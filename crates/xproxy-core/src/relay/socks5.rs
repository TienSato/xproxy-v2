//! Server SOCKS5 — phía nghe kết nối từ thiết bị trong LAN.
//!
//! Thay cho `gost -L socks5://0.0.0.0:<port>`.

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::addr::{write_socks_reply, TargetAddr};
use crate::error::{Error, Result};

/// Thông tin đăng nhập bắt buộc với thiết bị LAN (tuỳ chọn — mặc định không cần).
#[derive(Debug, Clone, Default)]
pub struct ListenAuth {
    pub user: String,
    pub pass: String,
}

impl ListenAuth {
    pub fn enabled(&self) -> bool {
        !self.user.is_empty()
    }
}

/// Bắt tay SOCKS5 với client và đọc ra địa chỉ đích nó muốn tới.
///
/// Trả về `TargetAddr`. Việc trả lời thành công cho client được làm SAU khi nối được
/// upstream (xem `relay::mod`), để client biết chính xác thành/bại.
pub async fn accept(stream: &mut TcpStream, auth: &ListenAuth) -> Result<TargetAddr> {
    // ── 1) Greeting: VER NMETHODS METHODS[] ──
    let ver = stream.read_u8().await?;
    if ver != 0x05 {
        return Err(Error::Socks(format!(
            "client gửi VER=0x{ver:02x}, không phải SOCKS5"
        )));
    }
    let n = stream.read_u8().await? as usize;
    if n == 0 {
        return Err(Error::Socks("client không đưa phương thức nào".into()));
    }
    let mut methods = vec![0u8; n];
    stream.read_exact(&mut methods).await?;

    // ── 2) Chọn phương thức ──
    if auth.enabled() {
        if !methods.contains(&0x02) {
            stream.write_all(&[0x05, 0xFF]).await?;
            return Err(Error::Socks("client không hỗ trợ xác thực user/pass".into()));
        }
        stream.write_all(&[0x05, 0x02]).await?;
        stream.flush().await?;
        check_userpass(stream, auth).await?;
    } else {
        if !methods.contains(&0x00) {
            stream.write_all(&[0x05, 0xFF]).await?;
            return Err(Error::Socks("client đòi xác thực nhưng cổng không bật".into()));
        }
        stream.write_all(&[0x05, 0x00]).await?;
        stream.flush().await?;
    }

    // ── 3) Request: VER CMD RSV ATYP ADDR PORT ──
    let mut head = [0u8; 3];
    stream.read_exact(&mut head).await?;
    if head[0] != 0x05 {
        return Err(Error::Socks("VER sai ở gói request".into()));
    }
    if head[1] != 0x01 {
        // Chỉ hỗ trợ CONNECT. BIND và UDP ASSOCIATE không dùng tới trong app này.
        let _ = TargetAddr::read_from(stream).await;
        write_socks_reply(stream, 0x07).await?;
        return Err(Error::UnsupportedCommand(head[1]));
    }

    TargetAddr::read_from(stream).await
}

async fn check_userpass(stream: &mut TcpStream, auth: &ListenAuth) -> Result<()> {
    let ver = stream.read_u8().await?;
    if ver != 0x01 {
        return Err(Error::Socks("sai version sub-negotiation".into()));
    }
    let ul = stream.read_u8().await? as usize;
    let mut u = vec![0u8; ul];
    stream.read_exact(&mut u).await?;
    let pl = stream.read_u8().await? as usize;
    let mut p = vec![0u8; pl];
    stream.read_exact(&mut p).await?;

    let ok = u == auth.user.as_bytes() && p == auth.pass.as_bytes();
    stream.write_all(&[0x01, if ok { 0x00 } else { 0x01 }]).await?;
    stream.flush().await?;
    if ok {
        Ok(())
    } else {
        Err(Error::Socks("sai user/pass của cổng LAN".into()))
    }
}

/// Mã trả lời SOCKS5 tương ứng với lỗi khi nối upstream.
pub fn reply_code_for(err: &Error) -> u8 {
    match err {
        Error::Timeout => 0x06,                 // TTL expired
        Error::UpstreamRefused(_) => 0x05,      // connection refused
        Error::NoUpstream(_) => 0x02,           // not allowed
        Error::Io(e) => match e.kind() {
            std::io::ErrorKind::ConnectionRefused => 0x05,
            std::io::ErrorKind::TimedOut => 0x06,
            _ => 0x01,
        },
        _ => 0x01,
    }
}
