//! Tiện ích mạng — thay toàn bộ chỗ bản Swift gọi công cụ ngoài.
//!
//! | Bản Swift | Ở đây |
//! |---|---|
//! | `getifaddrs` viết tay, chỉ en0/en1 | UDP socket trick (chạy cả Windows, không cần crate) |
//! | `isPortFree` với socket/bind thủ công | `TcpListener::bind` |
//! | `nc -z host port` | `TcpStream::connect` có timeout |
//! | `curl --socks5-hostname ...` | `reqwest` với proxy SOCKS5, **chỉ khi người dùng bấm** |
//!
//! Phần tra vị trí / múi giờ qua `ip-api.com` đã bỏ hẳn: không dùng tới, mà mỗi lần gán
//! lại thêm một lần gọi mạng.

use std::collections::HashSet;
use std::net::IpAddr;
use std::time::Duration;

use tokio::net::{TcpListener, TcpStream};
use tokio::time::timeout;

use crate::error::{Error, Result};

/// IP LAN của máy này (để hiện cho người dùng biết thiết bị khác trỏ vào đâu).
///
/// Mẹo không cần thư viện và chạy giống nhau trên macOS/Windows/Linux: mở một UDP socket
/// rồi `connect` tới một địa chỉ ngoài. UDP không bắt tay nên **không gói tin nào được
/// gửi đi**; hệ điều hành chỉ chọn route và gán địa chỉ nguồn — đọc ra là xong.
pub fn lan_ip() -> Option<String> {
    let sock = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.connect("8.8.8.8:80").ok()?;
    let ip = sock.local_addr().ok()?.ip();
    if ip.is_loopback() || ip.is_unspecified() {
        None
    } else {
        Some(ip.to_string())
    }
}

/// Cổng có trống để bind không. Hỏi thẳng hệ điều hành, không cần `lsof`.
pub async fn is_port_free(port: u16) -> bool {
    TcpListener::bind(("0.0.0.0", port)).await.is_ok()
}

/// Cổng trống đầu tiên >= `start`, bỏ qua các cổng trong `taken`.
pub async fn first_free_port(start: u16, taken: &HashSet<u16>, limit: u16) -> Option<u16> {
    let mut p = start;
    let mut tried = 0u16;
    while tried < limit {
        if !taken.contains(&p) && is_port_free(p).await {
            return Some(p);
        }
        p = p.checked_add(1)?;
        tried += 1;
    }
    None
}

/// Ping nhẹ tới upstream: chỉ bắt tay TCP, không truyền dữ liệu.
///
/// Thay `nc -z -G 4 -w 4 host port`. Quan trọng: KHÔNG đi qua cổng relay đang tải, nên
/// không tranh băng thông với thiết bị đang dùng proxy — giữ đúng ý đồ của bản Swift.
pub async fn ping_upstream(host: &str, port: u16, dur: Duration) -> bool {
    matches!(
        timeout(dur, TcpStream::connect((host, port))).await,
        Ok(Ok(_))
    )
}

const IP_ENDPOINTS: [&str; 3] = [
    // Sắp theo độ "nhẹ": thân phản hồi chỉ là một dòng IP, vài chục byte.
    "http://checkip.amazonaws.com",
    "http://icanhazip.com",
    "https://api.ipify.org",
];

/// Lấy IP thoát QUA cổng relay — tức IP mà website nhìn thấy.
///
/// ⚠️ **Hàm này đi qua proxy nên tốn băng thông của nhà cung cấp.** Không có chỗ nào
/// trong app tự gọi nó: chỉ chạy khi người dùng bấm nút trên đúng cổng đó. Một lần gọi
/// tốn cỡ vài trăm byte (bắt tay TCP + một request HTTP trần), nhưng với proxy tính
/// tiền theo GB thì nguyên tắc vẫn là: không tự ý tiêu của người dùng.
///
/// `socks5h://` (chữ h) = để proxy resolve DNS, không resolve tại máy — nếu không thì
/// DNS đi ra từ IP thật, lộ vị trí dù traffic qua proxy.
pub async fn exit_ip(local_port: u16, scheme_is_http: bool) -> Result<String> {
    let proxy_url = if scheme_is_http {
        format!("http://127.0.0.1:{local_port}")
    } else {
        format!("socks5h://127.0.0.1:{local_port}")
    };
    let client = reqwest::Client::builder()
        .proxy(reqwest::Proxy::all(&proxy_url).map_err(|e| Error::Net(e.to_string()))?)
        .timeout(Duration::from_secs(12))
        // Kết nối mới mỗi lần, không tái dùng socket cũ — tránh trả về IP của phiên
        // trước khi vừa bấm Đổi.
        .pool_max_idle_per_host(0)
        .build()?;

    let mut last = String::from("không gọi được endpoint nào");
    for url in IP_ENDPOINTS {
        match client.get(url).send().await {
            Ok(resp) => match resp.text().await {
                Ok(body) => {
                    let ip = body.trim();
                    if ip.parse::<IpAddr>().is_ok() {
                        return Ok(ip.to_string());
                    }
                    last = format!("{url} trả về nội dung lạ");
                }
                Err(e) => last = e.to_string(),
            },
            Err(e) => last = e.to_string(),
        }
    }
    Err(Error::Net(last))
}
