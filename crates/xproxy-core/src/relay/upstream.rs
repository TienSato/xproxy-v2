//! Nối tới proxy upstream của nhà cung cấp.
//!
//! Thay cho `gost -F socks5://user:pass@host:port`. Tự viết client SOCKS5 (RFC 1928 +
//! RFC 1929) thay vì dùng crate ngoài để kiểm soát timeout và thông điệp lỗi.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;

use crate::addr::TargetAddr;
use crate::error::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scheme {
    Socks5,
    Http,
}

impl Default for Scheme {
    fn default() -> Self {
        Scheme::Socks5
    }
}

/// Một proxy upstream do nhà cung cấp trả về (`host:port:user:pass`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Upstream {
    #[serde(default)]
    pub scheme: Scheme,
    pub host: String,
    pub port: u16,
    #[serde(default)]
    pub user: String,
    #[serde(default)]
    pub pass: String,
}

impl Upstream {
    pub fn new(host: impl Into<String>, port: u16) -> Self {
        Self {
            scheme: Scheme::Socks5,
            host: host.into(),
            port,
            user: String::new(),
            pass: String::new(),
        }
    }

    pub fn with_auth(mut self, user: impl Into<String>, pass: impl Into<String>) -> Self {
        self.user = user.into();
        self.pass = pass.into();
        self
    }

    pub fn with_scheme(mut self, scheme: Scheme) -> Self {
        self.scheme = scheme;
        self
    }

    pub fn endpoint(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    fn has_auth(&self) -> bool {
        !self.user.is_empty() || !self.pass.is_empty()
    }
}

/// Mở một kết nối tới `target` THÔNG QUA upstream.
pub async fn connect(up: &Upstream, target: &TargetAddr, dial_timeout: Duration) -> Result<TcpStream> {
    let fut = async {
        let mut stream = TcpStream::connect(up.endpoint()).await?;
        stream.set_nodelay(true).ok();
        match up.scheme {
            Scheme::Socks5 => socks5_handshake(&mut stream, up, target).await?,
            Scheme::Http => http_connect(&mut stream, up, target).await?,
        }
        Ok::<_, Error>(stream)
    };

    match timeout(dial_timeout, fut).await {
        Ok(r) => r,
        Err(_) => Err(Error::Timeout),
    }
}

// ── SOCKS5 (RFC 1928) ────────────────────────────────────────────────────────

async fn socks5_handshake(s: &mut TcpStream, up: &Upstream, target: &TargetAddr) -> Result<()> {
    // 1) Chào + danh sách phương thức xác thực ta hỗ trợ.
    if up.has_auth() {
        s.write_all(&[0x05, 0x02, 0x00, 0x02]).await?;
    } else {
        s.write_all(&[0x05, 0x01, 0x00]).await?;
    }
    s.flush().await?;

    let mut rep = [0u8; 2];
    s.read_exact(&mut rep).await?;
    if rep[0] != 0x05 {
        return Err(Error::Socks(format!("upstream trả về VER=0x{:02x}", rep[0])));
    }
    match rep[1] {
        0x00 => {}
        0x02 => auth_userpass(s, up).await?,
        0xFF => {
            return Err(Error::UpstreamRefused(
                "upstream không chấp nhận phương thức xác thực nào (sai user/pass?)".into(),
            ))
        }
        m => return Err(Error::Socks(format!("phương thức lạ: 0x{m:02x}"))),
    }

    // 2) Yêu cầu CONNECT tới đích.
    let mut req = Vec::with_capacity(32);
    req.extend_from_slice(&[0x05, 0x01, 0x00]); // VER, CMD=CONNECT, RSV
    target.write_to_buf(&mut req);
    s.write_all(&req).await?;
    s.flush().await?;

    // 3) Đọc trả lời.
    let mut head = [0u8; 3]; // VER REP RSV
    s.read_exact(&mut head).await?;
    if head[1] != 0x00 {
        return Err(Error::UpstreamRefused(socks_reply_text(head[1]).to_string()));
    }
    TargetAddr::skip(s).await?;
    Ok(())
}

/// Xác thực user/pass theo RFC 1929.
async fn auth_userpass(s: &mut TcpStream, up: &Upstream) -> Result<()> {
    let u = up.user.as_bytes();
    let p = up.pass.as_bytes();
    if u.len() > 255 || p.len() > 255 {
        return Err(Error::Socks("user/pass dài quá 255 byte".into()));
    }
    let mut buf = Vec::with_capacity(3 + u.len() + p.len());
    buf.push(0x01); // version của sub-negotiation
    buf.push(u.len() as u8);
    buf.extend_from_slice(u);
    buf.push(p.len() as u8);
    buf.extend_from_slice(p);
    s.write_all(&buf).await?;
    s.flush().await?;

    let mut rep = [0u8; 2];
    s.read_exact(&mut rep).await?;
    if rep[1] != 0x00 {
        return Err(Error::UpstreamRefused("sai user/pass của upstream".into()));
    }
    Ok(())
}

pub fn socks_reply_text(code: u8) -> &'static str {
    match code {
        0x01 => "upstream lỗi chung",
        0x02 => "upstream không cho phép kết nối này",
        0x03 => "mạng không tới được",
        0x04 => "máy đích không tới được",
        0x05 => "đích từ chối kết nối",
        0x06 => "TTL hết hạn",
        0x07 => "upstream không hỗ trợ lệnh CONNECT",
        0x08 => "upstream không hỗ trợ kiểu địa chỉ này",
        _ => "upstream trả về mã lỗi lạ",
    }
}

// ── HTTP CONNECT ─────────────────────────────────────────────────────────────

async fn http_connect(s: &mut TcpStream, up: &Upstream, target: &TargetAddr) -> Result<()> {
    let host = target.to_string();
    let mut req = format!("CONNECT {host} HTTP/1.1\r\nHost: {host}\r\n");
    if up.has_auth() {
        let token = base64(format!("{}:{}", up.user, up.pass).as_bytes());
        req.push_str(&format!("Proxy-Authorization: Basic {token}\r\n"));
    }
    req.push_str("Proxy-Connection: Keep-Alive\r\n\r\n");
    s.write_all(req.as_bytes()).await?;
    s.flush().await?;

    // Đọc tới hết phần header (kết thúc bằng \r\n\r\n).
    let mut buf = Vec::with_capacity(256);
    let mut byte = [0u8; 1];
    loop {
        let n = s.read(&mut byte).await?;
        if n == 0 {
            return Err(Error::Http("upstream đóng kết nối khi đang CONNECT".into()));
        }
        buf.push(byte[0]);
        if buf.len() >= 4 && buf[buf.len() - 4..] == *b"\r\n\r\n" {
            break;
        }
        if buf.len() > 16 * 1024 {
            return Err(Error::Http("header trả về quá dài".into()));
        }
    }

    let head = String::from_utf8_lossy(&buf);
    let status = head
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .unwrap_or("");
    if status != "200" {
        return Err(Error::UpstreamRefused(format!(
            "upstream HTTP trả về {}",
            if status.is_empty() { "phản hồi lạ" } else { status }
        )));
    }
    Ok(())
}

/// Base64 chuẩn, viết tay để khỏi thêm dependency.
fn base64(input: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(T[(n >> 18 & 63) as usize] as char);
        out.push(T[(n >> 12 & 63) as usize] as char);
        out.push(if chunk.len() > 1 { T[(n >> 6 & 63) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { T[(n & 63) as usize] as char } else { '=' });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_khop_chuan() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64(b"user:pass"), "dXNlcjpwYXNz");
    }
}
