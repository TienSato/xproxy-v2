//! Server HTTP proxy — phía nghe kết nối từ thiết bị LAN khi cổng đặt chế độ HTTP.
//!
//! Thay cho `gost -L http://0.0.0.0:<port>`. Hỗ trợ:
//! - `CONNECT host:port` → tunnel (HTTPS)
//! - request absolute-form `GET http://host/path` → viết lại thành origin-form rồi
//!   chuyển tiếp (HTTP thường)

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::addr::TargetAddr;
use crate::error::{Error, Result};

const MAX_HEAD: usize = 64 * 1024;

pub struct HttpRequest {
    pub target: TargetAddr,
    /// `true` = CONNECT (chỉ cần trả 200 rồi tunnel).
    /// `false` = request thường, phần head đã viết lại nằm ở `rewritten_head`.
    pub is_connect: bool,
    pub rewritten_head: Vec<u8>,
}

/// Đọc và phân tích request đầu tiên của client.
pub async fn accept(stream: &mut TcpStream) -> Result<HttpRequest> {
    let head = read_head(stream).await?;
    let text = String::from_utf8_lossy(&head).into_owned();

    let mut lines = text.split("\r\n");
    let request_line = lines.next().unwrap_or("");
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let uri = parts.next().unwrap_or("").to_string();
    let version = parts.next().unwrap_or("HTTP/1.1").to_string();

    if method.is_empty() || uri.is_empty() {
        return Err(Error::Http("request line rỗng".into()));
    }

    if method.eq_ignore_ascii_case("CONNECT") {
        let target = TargetAddr::parse_host_port(&uri, 443)?;
        return Ok(HttpRequest {
            target,
            is_connect: true,
            rewritten_head: Vec::new(),
        });
    }

    // Absolute-form: http://host[:port]/path
    let rest = uri
        .strip_prefix("http://")
        .ok_or_else(|| Error::Http(format!("URI không phải absolute-form: {uri}")))?;
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let target = TargetAddr::parse_host_port(authority, 80)?;

    // Viết lại head: origin-form + bỏ các header chỉ dành cho proxy.
    let mut out = format!("{method} {path} {version}\r\n");
    for line in lines {
        if line.is_empty() {
            break;
        }
        let name = line.split(':').next().unwrap_or("").trim();
        if name.eq_ignore_ascii_case("proxy-connection")
            || name.eq_ignore_ascii_case("proxy-authorization")
        {
            continue;
        }
        out.push_str(line);
        out.push_str("\r\n");
    }
    out.push_str("\r\n");

    Ok(HttpRequest {
        target,
        is_connect: false,
        rewritten_head: out.into_bytes(),
    })
}

async fn read_head(stream: &mut TcpStream) -> Result<Vec<u8>> {
    let mut buf = Vec::with_capacity(1024);
    let mut byte = [0u8; 1];
    loop {
        let n = stream.read(&mut byte).await?;
        if n == 0 {
            return Err(Error::Http("client đóng kết nối giữa chừng".into()));
        }
        buf.push(byte[0]);
        if buf.len() >= 4 && buf[buf.len() - 4..] == *b"\r\n\r\n" {
            return Ok(buf);
        }
        if buf.len() > MAX_HEAD {
            return Err(Error::Http("header client gửi quá dài".into()));
        }
    }
}

pub async fn write_connect_ok(stream: &mut TcpStream) -> Result<()> {
    stream
        .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
        .await?;
    stream.flush().await?;
    Ok(())
}

pub async fn write_error(stream: &mut TcpStream, err: &Error) -> Result<()> {
    let (code, reason) = match err {
        Error::Timeout => (504, "Gateway Timeout"),
        Error::NoUpstream(_) => (503, "Service Unavailable"),
        _ => (502, "Bad Gateway"),
    };
    let body = err.to_string();
    let resp = format!(
        "HTTP/1.1 {code} {reason}\r\nContent-Type: text/plain; charset=utf-8\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(resp.as_bytes()).await?;
    stream.flush().await?;
    Ok(())
}
