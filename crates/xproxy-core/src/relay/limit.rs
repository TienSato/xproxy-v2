//! Giới hạn tài nguyên theo từng cổng — để một cổng không kéo cả app xuống.
//!
//! # Cổng này có hút băng thông của cổng kia không?
//!
//! Có ba đường mà một cổng làm hại cổng khác, và chúng rất khác nhau:
//!
//! **1. Cạn file descriptor (nguy hiểm nhất, và âm thầm).** Giới hạn fd là của cả
//! tiến trình, không phải của từng cổng. Một cổng mở 3000 kết nối là `accept()` của
//! MỌI cổng khác bắt đầu trả `EMFILE` — cổng khác chết mà không hiểu tại sao. Đây là
//! đường gây hại thật sự và nó không liên quan gì tới băng thông. Chặn bằng
//! `max_conns_per_slot` (semaphore từng cổng) cộng với việc nâng giới hạn fd lúc khởi
//! động (xem `crate::rlimit`).
//!
//! **2. Nghẽn đầu hàng (head-of-line blocking).** Nếu nhiều kết nối dùng chung một
//! vòng lặp thì kết nối chậm chặn kết nối nhanh. Thiết kế ở đây không dính: mỗi kết nối
//! là một task tokio riêng, chạy trên runtime nhiều luồng. Thêm nữa tokio có cơ chế
//! "coop budget" tự bắt task nhường lượt sau ~128 thao tác I/O, nên không task nào giữ
//! luồng mãi.
//!
//! **3. Tranh băng thông thật.** Đường mạng của máy chỉ có một. Ở đây TCP đã chia
//! tương đối đều giữa các kết nối rồi — và vì mỗi cổng có kết nối riêng nên các cổng
//! tự cạnh tranh sòng phẳng. Muốn ép cứng hơn (ví dụ giữ chỗ cho một cổng quan trọng,
//! hoặc ghìm một cổng lại vì proxy tính tiền theo GB) thì đặt `rate_limit` — gáo token
//! bên dưới.

use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::Mutex;

/// Gáo token: cho phép trung bình `rate` byte/giây, cho dồn tối đa `burst` byte.
///
/// `rate = 0` nghĩa là không giới hạn — và khi đó `acquire` trả về ngay, không chạm
/// tới mutex, nên đường chạy mặc định không tốn gì.
pub struct RateLimiter {
    rate: u64,
    burst: u64,
    bucket: Mutex<Bucket>,
}

struct Bucket {
    tokens: f64,
    last: Instant,
}

impl RateLimiter {
    /// `bytes_per_sec = 0` → không giới hạn.
    pub fn new(bytes_per_sec: u64) -> Arc<Self> {
        // burst phải lớn hơn một lần đọc (32KB) nếu không `acquire` sẽ không bao giờ
        // đủ token và treo vĩnh viễn.
        let burst = bytes_per_sec.max(256 * 1024);
        Arc::new(Self {
            rate: bytes_per_sec,
            burst,
            bucket: Mutex::new(Bucket {
                tokens: burst as f64,
                last: Instant::now(),
            }),
        })
    }

    pub fn unlimited() -> Arc<Self> {
        Self::new(0)
    }

    pub fn is_limited(&self) -> bool {
        self.rate > 0
    }

    /// Chờ tới khi được phép truyền `n` byte.
    pub async fn acquire(&self, n: u64) {
        if self.rate == 0 || n == 0 {
            return;
        }
        let want = (n as f64).min(self.burst as f64);
        loop {
            let wait = {
                let mut b = self.bucket.lock().await;
                let now = Instant::now();
                let dt = now.duration_since(b.last).as_secs_f64();
                b.last = now;
                b.tokens = (b.tokens + dt * self.rate as f64).min(self.burst as f64);

                if b.tokens >= want {
                    b.tokens -= want;
                    None
                } else {
                    let need = want - b.tokens;
                    Some(Duration::from_secs_f64(need / self.rate as f64))
                }
            };
            match wait {
                None => return,
                // Ngủ từng nhịp ngắn thay vì một giấc dài: nếu người dùng nới giới hạn
                // giữa chừng thì phản ứng nhanh, và không giữ mutex trong lúc ngủ.
                Some(d) => tokio::time::sleep(d.min(Duration::from_millis(100))).await,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn khong_gioi_han_thi_khong_cho() {
        let l = RateLimiter::unlimited();
        let t = Instant::now();
        for _ in 0..1000 {
            l.acquire(32 * 1024).await;
        }
        assert!(t.elapsed() < Duration::from_millis(50));
    }

    #[tokio::test]
    async fn gioi_han_thi_ghim_dung_toc_do() {
        // rate = 512 KB/s → burst = max(rate, 256KB) = 512KB, và gáo khởi đầu đầy.
        // Xin 24 × 32KB = 768KB: 512KB lấy ngay, 256KB còn lại chờ 256/512 = 0.5 giây.
        let l = RateLimiter::new(512 * 1024);
        let t = Instant::now();
        for _ in 0..24 {
            l.acquire(32 * 1024).await;
        }
        let dt = t.elapsed();
        assert!(dt >= Duration::from_millis(350), "quá nhanh: {dt:?}");
        assert!(dt < Duration::from_millis(2000), "quá chậm: {dt:?}");
    }

    /// Xin nhiều hơn dung lượng gáo cũng không được treo vĩnh viễn.
    #[tokio::test]
    async fn xin_lon_hon_gao_van_qua_duoc() {
        let l = RateLimiter::new(1024 * 1024);
        tokio::time::timeout(Duration::from_secs(3), l.acquire(100 * 1024 * 1024))
            .await
            .expect("không được treo");
    }
}
