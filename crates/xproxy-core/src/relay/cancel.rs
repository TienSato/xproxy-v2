//! Cờ huỷ dùng chung, dựng trên `tokio::sync::watch`.
//!
//! Cố ý không dùng `tokio-util::CancellationToken` — chỉ cần đúng 30 dòng này, đổi lại
//! bớt được một dependency khỏi cây build.

use std::sync::Arc;

use tokio::sync::watch;

#[derive(Clone)]
pub struct Cancel {
    tx: Arc<watch::Sender<bool>>,
}

impl Default for Cancel {
    fn default() -> Self {
        Self::new()
    }
}

impl Cancel {
    pub fn new() -> Self {
        let (tx, _) = watch::channel(false);
        Self { tx: Arc::new(tx) }
    }

    pub fn cancel(&self) {
        // KHÔNG dùng `send()`: nó trả Err và **bỏ luôn giá trị** khi không còn receiver
        // nào. `Cancel::new()` thả receiver ban đầu đi, nên nếu chưa ai chờ
        // `cancelled()` thì `cancel()` sẽ im lặng không làm gì — cờ huỷ không bao giờ
        // bật. `send_replace` luôn ghi, bất kể có ai đang nghe hay không.
        self.tx.send_replace(true);
    }

    pub fn is_cancelled(&self) -> bool {
        *self.tx.borrow()
    }

    /// Chờ tới khi bị huỷ. Trả về ngay nếu đã huỷ từ trước.
    pub async fn cancelled(&self) {
        let mut rx = self.tx.subscribe();
        loop {
            if *rx.borrow_and_update() {
                return;
            }
            if rx.changed().await.is_err() {
                // Sender đã rơi — coi như huỷ.
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn huy_roi_thi_cho_tra_ve_ngay() {
        let c = Cancel::new();
        assert!(!c.is_cancelled());
        c.cancel();
        assert!(c.is_cancelled());
        tokio::time::timeout(std::time::Duration::from_millis(50), c.cancelled())
            .await
            .expect("phải trả về ngay");
    }

    #[tokio::test]
    async fn ban_sao_thay_duoc_lenh_huy() {
        let a = Cancel::new();
        let b = a.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            b.cancel();
        });
        tokio::time::timeout(std::time::Duration::from_secs(1), a.cancelled())
            .await
            .expect("phải nhận được lệnh huỷ");
    }
}
