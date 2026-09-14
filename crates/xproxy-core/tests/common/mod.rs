//! Đồ giả dùng cho test: một SOCKS5 upstream giả và một server đích giả.
//!
//! Nhờ có mấy thứ này, test chạy hoàn toàn trên loopback — không cần mạng, không cần
//! tài khoản nhà cung cấp, chạy được trong CI.

#![allow(dead_code)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// Server đích: mỗi kết nối, đọc tới khi client đóng nửa chiều ghi, rồi trả về
/// `banner + dữ liệu nhận được` (echo có gắn nhãn, để biết đi qua upstream nào).
pub struct Origin {
    pub addr: std::net::SocketAddr,
}

impl Origin {
    pub async fn start(banner: &'static str) -> Origin {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else { break };
                tokio::spawn(async move {
                    let mut buf = Vec::new();
                    let _ = sock.read_to_end(&mut buf).await;
                    let _ = sock.write_all(banner.as_bytes()).await;
                    let _ = sock.write_all(&buf).await;
                    let _ = sock.shutdown().await;
                });
            }
        });
        Origin { addr }
    }
}

/// SOCKS5 upstream giả — đóng vai proxy của nhà cung cấp.
pub struct FakeUpstream {
    pub addr: std::net::SocketAddr,
    /// Số lần có kết nối đi qua — để test kiểm tra traffic thật sự đổi đường.
    pub hits: Arc<AtomicUsize>,
    /// Tên miền cuối cùng mà client yêu cầu (dạng chuỗi, chưa resolve).
    pub last_domain: Arc<tokio::sync::Mutex<Option<String>>>,
}

impl FakeUpstream {
    /// `creds = None` → không đòi xác thực. `Some((u,p))` → bắt buộc đúng user/pass.
    pub async fn start(creds: Option<(&'static str, &'static str)>) -> FakeUpstream {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let last_domain = Arc::new(tokio::sync::Mutex::new(None));

        let h = hits.clone();
        let d = last_domain.clone();
        tokio::spawn(async move {
            loop {
                let Ok((sock, _)) = listener.accept().await else { break };
                let h = h.clone();
                let d = d.clone();
                tokio::spawn(async move {
                    let _ = serve(sock, creds, h, d).await;
                });
            }
        });

        FakeUpstream {
            addr,
            hits,
            last_domain,
        }
    }
}

async fn serve(
    mut s: TcpStream,
    creds: Option<(&'static str, &'static str)>,
    hits: Arc<AtomicUsize>,
    last_domain: Arc<tokio::sync::Mutex<Option<String>>>,
) -> std::io::Result<()> {
    // Greeting
    let ver = s.read_u8().await?;
    assert_eq!(ver, 0x05, "upstream giả chỉ nói SOCKS5");
    let n = s.read_u8().await? as usize;
    let mut methods = vec![0u8; n];
    s.read_exact(&mut methods).await?;

    match creds {
        Some((want_u, want_p)) => {
            assert!(methods.contains(&0x02), "client phải chào được user/pass");
            s.write_all(&[0x05, 0x02]).await?;

            assert_eq!(s.read_u8().await?, 0x01);
            let ul = s.read_u8().await? as usize;
            let mut u = vec![0u8; ul];
            s.read_exact(&mut u).await?;
            let pl = s.read_u8().await? as usize;
            let mut p = vec![0u8; pl];
            s.read_exact(&mut p).await?;

            let ok = u == want_u.as_bytes() && p == want_p.as_bytes();
            s.write_all(&[0x01, if ok { 0x00 } else { 0x01 }]).await?;
            if !ok {
                return Ok(());
            }
        }
        None => {
            s.write_all(&[0x05, 0x00]).await?;
        }
    }

    // Request
    let mut head = [0u8; 3];
    s.read_exact(&mut head).await?;
    assert_eq!(head[1], 0x01, "chỉ hỗ trợ CONNECT");

    let atyp = s.read_u8().await?;
    let target = match atyp {
        0x01 => {
            let mut b = [0u8; 4];
            s.read_exact(&mut b).await?;
            let port = s.read_u16().await?;
            format!("{}.{}.{}.{}:{}", b[0], b[1], b[2], b[3], port)
        }
        0x03 => {
            let len = s.read_u8().await? as usize;
            let mut b = vec![0u8; len];
            s.read_exact(&mut b).await?;
            let port = s.read_u16().await?;
            let host = String::from_utf8(b).unwrap();
            *last_domain.lock().await = Some(host.clone());
            format!("{host}:{port}")
        }
        _ => panic!("ATYP không dùng trong test"),
    };

    hits.fetch_add(1, Ordering::Relaxed);

    match TcpStream::connect(&target).await {
        Ok(mut out) => {
            s.write_all(&[0x05, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0]).await?;
            tokio::io::copy_bidirectional(&mut s, &mut out).await.ok();
        }
        Err(_) => {
            s.write_all(&[0x05, 0x05, 0x00, 0x01, 0, 0, 0, 0, 0, 0]).await?;
        }
    }
    Ok(())
}

// ── Client SOCKS5 tối giản, dùng để nói chuyện với relay đang test ────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SocksReply {
    /// Mã REP của SOCKS5, hoặc `0xFF` khi relay đóng thẳng kết nối (quá tải).
    pub code: u8,
}

/// `0xFF` — relay đóng kết nối mà không trả lời (chạm trần số kết nối).
pub const CLOSED: u8 = 0xFF;

/// Bắt tay SOCKS5 (không auth) với `relay_port`, yêu cầu CONNECT tới `host:port` dưới
/// dạng TÊN MIỀN. Trả về stream nếu thành công, hoặc mã lỗi.
///
/// Không `unwrap` giữa chừng: relay có quyền đóng kết nối khi quá tải, và test phải
/// phân biệt được "bị từ chối" với "test hỏng".
pub async fn socks_connect_domain(
    relay_port: u16,
    host: &str,
    port: u16,
) -> Result<TcpStream, SocksReply> {
    let closed = || SocksReply { code: CLOSED };

    let mut s = TcpStream::connect(("127.0.0.1", relay_port))
        .await
        .map_err(|_| closed())?;

    s.write_all(&[0x05, 0x01, 0x00]).await.map_err(|_| closed())?;
    let mut m = [0u8; 2];
    s.read_exact(&mut m).await.map_err(|_| closed())?;
    if m != [0x05, 0x00] {
        return Err(SocksReply { code: CLOSED });
    }

    let mut req = vec![0x05, 0x01, 0x00, 0x03, host.len() as u8];
    req.extend_from_slice(host.as_bytes());
    req.extend_from_slice(&port.to_be_bytes());
    s.write_all(&req).await.map_err(|_| closed())?;

    let mut head = [0u8; 3];
    s.read_exact(&mut head).await.map_err(|_| closed())?;

    // Bỏ qua BND.ADDR + BND.PORT
    let atyp = s.read_u8().await.map_err(|_| closed())?;
    let n = match atyp {
        0x01 => 4,
        0x04 => 16,
        0x03 => s.read_u8().await.map_err(|_| closed())? as usize,
        _ => 0,
    };
    let mut skip = vec![0u8; n + 2];
    s.read_exact(&mut skip).await.map_err(|_| closed())?;

    if head[1] == 0x00 {
        Ok(s)
    } else {
        Err(SocksReply { code: head[1] })
    }
}

/// Gửi `payload` rồi đọc toàn bộ phản hồi.
pub async fn round_trip(mut s: TcpStream, payload: &[u8]) -> String {
    s.write_all(payload).await.unwrap();
    s.shutdown().await.unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).await.unwrap();
    out
}
