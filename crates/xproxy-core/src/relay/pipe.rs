//! Chuyển tiếp hai chiều + đếm byte.
//!
//! Không dùng `tokio::io::copy_bidirectional` vì nó chỉ trả tổng số byte KHI XONG.
//! Ta cần số liệu cập nhật liên tục để UI hiện băng thông từng cổng theo thời gian
//! thực — thứ mà bản Swift dùng gost không làm được, vì byte đi qua tiến trình ngoài
//! nên app không nhìn thấy.
//!
//! Kết nối sống bao lâu là việc của hai đầu, không phải việc của relay: thiết bị cắm
//! vào rồi để đó cả ngày cũng được. Ở đây không có timeout nằm im.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use super::limit::RateLimiter;

/// Bộ đếm dùng chung của một cổng. Ghi bằng atomic nên không cần khoá.
#[derive(Debug, Default)]
pub struct Counters {
    /// Byte thiết bị LAN gửi ra Internet.
    pub up: AtomicU64,
    /// Byte Internet trả về thiết bị LAN.
    pub down: AtomicU64,
    /// Số kết nối TCP đang mở.
    pub open: AtomicU64,
    /// Tổng số kết nối đã phục vụ.
    pub total: AtomicU64,
    /// Không nối được tới proxy: hết giờ, bị từ chối, sai user/pass. Lỗi THẬT.
    pub failed: AtomicU64,
    /// Bị từ chối vì chạm trần số kết nối (của cổng hoặc của cả app).
    pub rejected: AtomicU64,
    /// Client xin UDP/BIND — bình thường, KHÔNG phải lỗi.
    pub udp_refused: AtomicU64,
    /// Kết nối rớt kiểu bình thường: client đóng giữa chừng, peer reset, EOF sớm.
    pub churn: AtomicU64,
}

impl Counters {
    pub fn snapshot(&self) -> CounterSnapshot {
        CounterSnapshot {
            up: self.up.load(Ordering::Relaxed),
            down: self.down.load(Ordering::Relaxed),
            open: self.open.load(Ordering::Relaxed),
            total: self.total.load(Ordering::Relaxed),
            failed: self.failed.load(Ordering::Relaxed),
            rejected: self.rejected.load(Ordering::Relaxed),
            udp_refused: self.udp_refused.load(Ordering::Relaxed),
            churn: self.churn.load(Ordering::Relaxed),
            devices: 0,
        }
    }

    /// Xoá số liệu (giữ nguyên số kết nối đang mở).
    pub fn reset_traffic(&self) {
        self.up.store(0, Ordering::Relaxed);
        self.down.store(0, Ordering::Relaxed);
        self.total.store(0, Ordering::Relaxed);
        self.failed.store(0, Ordering::Relaxed);
        self.rejected.store(0, Ordering::Relaxed);
        self.udp_refused.store(0, Ordering::Relaxed);
        self.churn.store(0, Ordering::Relaxed);
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CounterSnapshot {
    pub up: u64,
    pub down: u64,
    pub open: u64,
    pub total: u64,
    pub failed: u64,
    pub rejected: u64,
    pub udp_refused: u64,
    pub churn: u64,
    /// Số THIẾT BỊ đang cắm vào cổng (đếm theo IP khác nhau, không phải số kết nối).
    pub devices: u64,
}

const BUF: usize = 32 * 1024;

/// Copy một chiều, cộng thẳng vào counter chung sau mỗi lần ghi thành công.
async fn copy_counted<R, W>(
    mut r: R,
    mut w: W,
    counter: &AtomicU64,
    rate: &RateLimiter,
) -> u64
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut buf = vec![0u8; BUF];
    let mut total = 0u64;
    loop {
        let n = match r.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        // Ghìm tốc độ TRƯỚC khi ghi. Khi không đặt giới hạn thì đây là một phép so sánh
        // số nguyên rồi return — không chạm mutex, không ảnh hưởng đường chạy mặc định.
        rate.acquire(n as u64).await;
        if w.write_all(&buf[..n]).await.is_err() {
            break;
        }
        total += n as u64;
        counter.fetch_add(n as u64, Ordering::Relaxed);
    }
    // Đóng nửa chiều ghi để bên kia biết đã hết dữ liệu (quan trọng với HTTP/1.1 không
    // có Content-Length: bên nhận chỉ biết hết body khi thấy EOF).
    let _ = w.shutdown().await;
    total
}

/// Nối `client` (thiết bị LAN) với `server` (kết nối đã mở qua upstream).
/// Chạy tới khi cả hai chiều đóng. Trả về (byte lên, byte xuống).
pub async fn splice<A, B>(
    client: A,
    server: B,
    counters: Arc<Counters>,
    rate: Arc<RateLimiter>,
) -> (u64, u64)
where
    A: AsyncRead + AsyncWrite + Unpin,
    B: AsyncRead + AsyncWrite + Unpin,
{
    let (cr, cw) = tokio::io::split(client);
    let (sr, sw) = tokio::io::split(server);

    // Hai chiều dùng CHUNG một gáo token → giới hạn là tổng băng thông của cổng,
    // đúng với cách proxy dân cư tính tiền (cộng cả lên lẫn xuống).
    let to_upstream = copy_counted(cr, sw, &counters.up, &rate);
    let to_client = copy_counted(sr, cw, &counters.down, &rate);

    tokio::join!(to_upstream, to_client)
}
