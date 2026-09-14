use std::io;

/// Lỗi của lõi xproxy. Thông điệp viết tiếng Việt vì hiện ra thẳng UI.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("lỗi I/O: {0}")]
    Io(#[from] io::Error),

    #[error("hết thời gian chờ")]
    Timeout,

    #[error("upstream từ chối: {0}")]
    UpstreamRefused(String),

    #[error("sai giao thức SOCKS5: {0}")]
    Socks(String),

    #[error("sai giao thức HTTP: {0}")]
    Http(String),

    #[error("cổng {0} chưa được gán proxy")]
    NoUpstream(u16),

    #[error("cổng {0} đang bị chương trình khác chiếm — đổi dải cổng trong Cài đặt. \
Trên macOS, cổng 5000 và 7000 thường do AirPlay Receiver giữ \
(tắt ở System Settings → General → AirDrop & Handoff).")]
    PortInUse(u16),

    /// Client xin lệnh SOCKS5 mà relay không làm: BIND (0x02) hoặc UDP ASSOCIATE (0x03).
    /// Đây là chuyện BÌNH THƯỜNG, không phải hỏng — client sẽ tự lùi về TCP.
    #[error("client xin lệnh SOCKS5 0x{0:02x} (relay chỉ làm CONNECT)")]
    UnsupportedCommand(u8),

    #[error("lỗi mạng: {0}")]
    Net(String),

    #[error("{0}")]
    Api(String),
}

impl From<reqwest::Error> for Error {
    fn from(e: reqwest::Error) -> Self {
        Error::Net(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;
