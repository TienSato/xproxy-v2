use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::error::{Error, Result};

/// Địa chỉ đích của một kết nối.
///
/// Tên miền được GIỮ NGUYÊN dạng chuỗi, không resolve tại máy này — để upstream tự
/// resolve. Đây là điểm quan trọng: nếu ta resolve ở đây thì DNS đi ra từ IP thật của
/// máy, lộ vị trí dù traffic đi qua proxy (tương đương `--socks5-hostname` của curl).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetAddr {
    Ip(SocketAddr),
    Domain(String, u16),
}

impl TargetAddr {
    pub fn port(&self) -> u16 {
        match self {
            TargetAddr::Ip(sa) => sa.port(),
            TargetAddr::Domain(_, p) => *p,
        }
    }

    /// Đọc phần ATYP + ADDR + PORT của một gói SOCKS5.
    pub async fn read_from<R>(r: &mut R) -> Result<Self>
    where
        R: AsyncReadExt + Unpin,
    {
        let atyp = r.read_u8().await?;
        match atyp {
            0x01 => {
                let mut b = [0u8; 4];
                r.read_exact(&mut b).await?;
                let port = r.read_u16().await?;
                Ok(TargetAddr::Ip(SocketAddr::new(
                    IpAddr::V4(Ipv4Addr::from(b)),
                    port,
                )))
            }
            0x03 => {
                let len = r.read_u8().await? as usize;
                if len == 0 {
                    return Err(Error::Socks("tên miền rỗng".into()));
                }
                let mut b = vec![0u8; len];
                r.read_exact(&mut b).await?;
                let host = String::from_utf8(b)
                    .map_err(|_| Error::Socks("tên miền không phải UTF-8".into()))?;
                let port = r.read_u16().await?;
                Ok(TargetAddr::Domain(host, port))
            }
            0x04 => {
                let mut b = [0u8; 16];
                r.read_exact(&mut b).await?;
                let port = r.read_u16().await?;
                Ok(TargetAddr::Ip(SocketAddr::new(
                    IpAddr::V6(Ipv6Addr::from(b)),
                    port,
                )))
            }
            other => Err(Error::Socks(format!("ATYP không hỗ trợ: 0x{other:02x}"))),
        }
    }

    /// Ghi ATYP + ADDR + PORT vào buffer (dùng khi gửi CONNECT lên upstream).
    pub fn write_to_buf(&self, buf: &mut Vec<u8>) {
        match self {
            TargetAddr::Ip(SocketAddr::V4(v4)) => {
                buf.push(0x01);
                buf.extend_from_slice(&v4.ip().octets());
                buf.extend_from_slice(&v4.port().to_be_bytes());
            }
            TargetAddr::Ip(SocketAddr::V6(v6)) => {
                buf.push(0x04);
                buf.extend_from_slice(&v6.ip().octets());
                buf.extend_from_slice(&v6.port().to_be_bytes());
            }
            TargetAddr::Domain(host, port) => {
                let bytes = host.as_bytes();
                let len = bytes.len().min(255);
                buf.push(0x03);
                buf.push(len as u8);
                buf.extend_from_slice(&bytes[..len]);
                buf.extend_from_slice(&port.to_be_bytes());
            }
        }
    }

    /// Bỏ qua phần địa chỉ trong gói trả lời của upstream (ta không cần BND.ADDR).
    pub async fn skip<R>(r: &mut R) -> Result<()>
    where
        R: AsyncReadExt + Unpin,
    {
        let atyp = r.read_u8().await?;
        let n = match atyp {
            0x01 => 4,
            0x04 => 16,
            0x03 => r.read_u8().await? as usize,
            other => return Err(Error::Socks(format!("ATYP lạ trong reply: 0x{other:02x}"))),
        };
        let mut buf = vec![0u8; n + 2]; // + 2 byte port
        r.read_exact(&mut buf).await?;
        Ok(())
    }

    /// Parse chuỗi dạng `host:port` (dùng cho HTTP CONNECT).
    pub fn parse_host_port(s: &str, default_port: u16) -> Result<Self> {
        // IPv6 có dấu ngoặc: [::1]:8080
        if let Some(rest) = s.strip_prefix('[') {
            let (ip, tail) = rest
                .split_once(']')
                .ok_or_else(|| Error::Http(format!("địa chỉ IPv6 sai: {s}")))?;
            let port = tail
                .strip_prefix(':')
                .and_then(|p| p.parse().ok())
                .unwrap_or(default_port);
            let ip: Ipv6Addr = ip
                .parse()
                .map_err(|_| Error::Http(format!("IPv6 sai: {ip}")))?;
            return Ok(TargetAddr::Ip(SocketAddr::new(IpAddr::V6(ip), port)));
        }
        let (host, port) = match s.rsplit_once(':') {
            Some((h, p)) if !h.is_empty() => {
                let port = p
                    .parse()
                    .map_err(|_| Error::Http(format!("cổng sai: {p}")))?;
                (h, port)
            }
            _ => (s, default_port),
        };
        if host.is_empty() {
            return Err(Error::Http("thiếu host".into()));
        }
        match host.parse::<IpAddr>() {
            Ok(ip) => Ok(TargetAddr::Ip(SocketAddr::new(ip, port))),
            Err(_) => Ok(TargetAddr::Domain(host.to_string(), port)),
        }
    }
}

impl fmt::Display for TargetAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TargetAddr::Ip(sa) => write!(f, "{sa}"),
            TargetAddr::Domain(h, p) => write!(f, "{h}:{p}"),
        }
    }
}

/// Trả lời SOCKS5 thành công (BND.ADDR = 0.0.0.0:0 — client không dùng tới).
pub async fn write_socks_reply<W>(w: &mut W, rep: u8) -> Result<()>
where
    W: AsyncWriteExt + Unpin,
{
    w.write_all(&[0x05, rep, 0x00, 0x01, 0, 0, 0, 0, 0, 0]).await?;
    w.flush().await?;
    Ok(())
}
