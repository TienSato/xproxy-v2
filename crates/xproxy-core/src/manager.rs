//! Quản lý toàn bộ dải cổng — port từ `ProxyManager.swift`, bỏ hết phần tiến trình ngoài.
//!
//! Những thứ ở bản Swift mà ở đây KHÔNG còn tồn tại:
//! - `killOrphanGost()` — không có tiến trình con thì không có tiến trình mồ côi.
//! - `terminateAndWait()` — không rebind thì không phải đợi socket nhả.
//! - `killProcessOnPort()` — không cần `lsof`/`kill -9`.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::error::{Error, Result};
use crate::relay::{CounterSnapshot, GlobalLimit, SlotConfig, SlotHandle, Upstream};
use crate::rlimit;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SlotStatus {
    /// Cổng chưa mở.
    Idle,
    /// Cổng đã mở nhưng chưa có upstream.
    Listening,
    /// Đang gọi API lấy IP.
    Starting,
    /// Đang chạy bình thường.
    Running,
    Error,
}

/// Trạng thái đầy đủ của một cổng (phần app giữ, không phải phần relay giữ).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlotState {
    pub port: u16,
    /// Cổng gửi cho API nhà cung cấp (443..3000) — giữ nguyên ý niệm của bản Swift.
    pub provider_port: u16,
    pub provider_id: String,
    pub country: String,
    pub state: String,
    pub city: String,
    pub status: SlotStatus,
    /// host:port của proxy nhà cung cấp trả về. Đây là thứ hiện ra bảng — nó đến từ
    /// chính phản hồi API nên KHÔNG tốn thêm một byte nào để biết.
    pub upstream_host: String,
    pub upstream_port: u16,
    /// IP thoát thật. Chỉ có giá trị khi người dùng tự bấm nút kiểm tra, vì lấy nó
    /// phải đi qua proxy và tốn băng thông của nhà cung cấp.
    pub exit_ip: String,
    pub alive: Option<bool>,
    pub message: String,
}

impl SlotState {
    pub fn new(port: u16, provider_port: u16) -> Self {
        Self {
            port,
            provider_port,
            provider_id: "cliproxy".into(),
            country: String::new(),
            state: String::new(),
            city: String::new(),
            status: SlotStatus::Idle,
            upstream_host: String::new(),
            upstream_port: 0,
            exit_ip: String::new(),
            alive: None,
            message: String::new(),
        }
    }

    pub fn region_text(&self) -> String {
        [&self.country, &self.state, &self.city]
            .into_iter()
            .filter(|s| !s.is_empty())
            .cloned()
            .collect::<Vec<_>>()
            .join(" / ")
    }
}

/// Trạng thái + số liệu, dạng gửi thẳng lên UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlotView {
    #[serde(flatten)]
    pub state: SlotState,
    pub stats: CounterSnapshot,
    pub region_text: String,
}

#[derive(Default)]
struct Inner {
    states: BTreeMap<u16, SlotState>,
    handles: BTreeMap<u16, SlotHandle>,
}

/// Sức khoẻ chung của cả app — để UI cảnh báo TRƯỚC khi các cổng bắt đầu chết vì
/// cạn file descriptor, thay vì để người dùng tự đoán.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Health {
    /// Tổng kết nối đang mở của mọi cổng.
    pub open_conns: usize,
    /// Trần kết nối của cả app, suy ra từ giới hạn file descriptor.
    pub max_conns: usize,
    /// Giới hạn fd thật sau khi app đã tự nâng lúc khởi động.
    pub fd_limit: u64,
}

/// Quản lý tập hợp cổng đang chạy.
#[derive(Clone)]
pub struct Manager {
    inner: Arc<RwLock<Inner>>,
    config: Arc<RwLock<SlotConfig>>,
    global: Arc<GlobalLimit>,
    fd_limit: u64,
    /// Những cổng đã bị bỏ qua lúc tạo dải vì chương trình khác đang giữ.
    skipped: Arc<RwLock<Vec<u16>>>,
}

impl Default for Manager {
    fn default() -> Self {
        Self::new(SlotConfig::default())
    }
}

impl Manager {
    /// Nâng giới hạn file descriptor rồi suy ra trần kết nối chung từ đó.
    ///
    /// Phải làm ngay lúc dựng: trên macOS, app chạy từ Finder chỉ có 256 fd mềm — chạy
    /// vài chục cổng là chạm trần, và khi chạm thì MỌI cổng cùng hỏng.
    pub fn new(config: SlotConfig) -> Self {
        let fd_limit = rlimit::raise_fd_limit();
        let max = rlimit::safe_global_conns();
        tracing::info!(fd_limit, max_conns = max, "giới hạn tài nguyên");
        Self {
            inner: Arc::new(RwLock::new(Inner::default())),
            config: Arc::new(RwLock::new(config)),
            global: GlobalLimit::new(max),
            fd_limit,
            skipped: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Những cổng đã bỏ qua ở lần tạo dải gần nhất.
    pub async fn skipped_ports(&self) -> Vec<u16> {
        self.skipped.read().await.clone()
    }

    pub fn health(&self) -> Health {
        Health {
            open_conns: self.global.open(),
            max_conns: self.global.max(),
            fd_limit: self.fd_limit,
        }
    }

    pub async fn config(&self) -> SlotConfig {
        self.config.read().await.clone()
    }

    /// Đổi cấu hình chung, áp cho mọi cổng đang chạy.
    pub async fn set_config(&self, cfg: SlotConfig) {
        *self.config.write().await = cfg.clone();
        let inner = self.inner.read().await;
        for h in inner.handles.values() {
            h.set_config(cfg.clone()).await;
        }
    }

    /// Tạo dải cổng: lấy `count` cổng TRỐNG tính từ `start`.
    ///
    /// **Tự nhảy qua cổng đang bị chương trình khác giữ** thay vì tạo ra một cổng chết
    /// rồi để người dùng bấm Gán mới phát hiện. Trên macOS chuyện này gặp ngay ở cổng
    /// đầu tiên: AirPlay Receiver giữ cổng 5000 và 7000. Danh sách cổng bị nhảy qua
    /// được giữ lại để UI nói cho người dùng biết.
    pub async fn generate_ports(&self, start: u16, count: u16) -> Vec<u16> {
        self.shutdown_all().await;

        let mut ports: Vec<u16> = Vec::with_capacity(count as usize);
        let mut skipped: Vec<u16> = Vec::new();
        let mut p = start;
        let mut tried = 0u32;

        while ports.len() < count as usize && tried < 5000 {
            tried += 1;
            if crate::net::is_port_free(p).await {
                ports.push(p);
            } else {
                skipped.push(p);
            }
            match p.checked_add(1) {
                Some(next) => p = next,
                None => break,
            }
        }

        {
            let mut inner = self.inner.write().await;
            inner.states.clear();
            for (i, port) in ports.iter().enumerate() {
                let provider_port = (443u32 + i as u32).min(3000) as u16;
                inner.states.insert(*port, SlotState::new(*port, provider_port));
            }
        }
        *self.skipped.write().await = skipped.clone();
        if !skipped.is_empty() {
            tracing::info!(?skipped, "đã nhảy qua cổng đang bị chiếm");
        }
        skipped
    }

    pub async fn views(&self) -> Vec<SlotView> {
        let inner = self.inner.read().await;
        inner
            .states
            .values()
            .map(|s| SlotView {
                stats: inner
                    .handles
                    .get(&s.port)
                    .map(|h| h.stats())
                    .unwrap_or_default(),
                region_text: s.region_text(),
                state: s.clone(),
            })
            .collect()
    }

    pub async fn view(&self, port: u16) -> Option<SlotView> {
        let inner = self.inner.read().await;
        inner.states.get(&port).map(|s| SlotView {
            stats: inner.handles.get(&port).map(|h| h.stats()).unwrap_or_default(),
            region_text: s.region_text(),
            state: s.clone(),
        })
    }

    pub async fn running_count(&self) -> usize {
        self.inner
            .read()
            .await
            .states
            .values()
            .filter(|s| s.status == SlotStatus::Running)
            .count()
    }

    pub async fn idle_ports(&self) -> Vec<u16> {
        self.inner
            .read()
            .await
            .states
            .values()
            .filter(|s| matches!(s.status, SlotStatus::Idle | SlotStatus::Error))
            .map(|s| s.port)
            .collect()
    }

    /// Mở cổng (bind) nếu chưa mở. Trả về handle.
    pub async fn ensure_open(&self, port: u16) -> Result<SlotHandle> {
        // Lấy bản sao rồi THẢ read guard ngay. Nếu giữ guard qua tới `write()` bên dưới
        // thì RwLock của tokio sẽ kẹt cứng (read và write trong cùng một task).
        let existing = self.inner.read().await.handles.get(&port).cloned();
        if let Some(h) = existing {
            if !h.is_closed() {
                return Ok(h);
            }
        }
        let cfg = self.config.read().await.clone();
        let handle = SlotHandle::bind(port, cfg, self.global.clone()).await?;
        let mut inner = self.inner.write().await;
        inner.handles.insert(port, handle.clone());
        if let Some(s) = inner.states.get_mut(&port) {
            if s.status == SlotStatus::Idle {
                s.status = SlotStatus::Listening;
            }
        } else {
            let mut st = SlotState::new(port, 443);
            st.status = SlotStatus::Listening;
            inner.states.insert(port, st);
        }
        Ok(handle)
    }

    /// Gán / đổi upstream cho một cổng.
    ///
    /// **Cổng không bị đóng và mở lại** — khác hẳn bản Swift. Nghĩa là thiết bị đang cắm
    /// vào cổng này không cần cấu hình lại gì, và không có khoảng thời gian chết.
    pub async fn assign(&self, port: u16, up: Upstream) -> Result<()> {
        let handle = self.ensure_open(port).await?;
        let host = up.host.clone();
        let uport = up.port;
        handle.set_upstream(up).await;

        let mut inner = self.inner.write().await;
        let s = inner
            .states
            .get_mut(&port)
            .ok_or_else(|| Error::Api(format!("không có cổng {port}")))?;
        s.upstream_host = host;
        s.upstream_port = uport;
        s.status = SlotStatus::Running;
        s.exit_ip.clear();
        s.alive = None;
        Ok(())
    }

    /// Gỡ cổng: đóng listener, nhả socket ngay.
    pub async fn unassign(&self, port: u16) {
        let mut inner = self.inner.write().await;
        if let Some(h) = inner.handles.remove(&port) {
            h.shutdown();
        }
        if let Some(s) = inner.states.get_mut(&port) {
            let mut fresh = SlotState::new(s.port, s.provider_port);
            fresh.provider_id = std::mem::take(&mut s.provider_id);
            *s = fresh;
        }
    }

    pub async fn shutdown_all(&self) {
        let mut inner = self.inner.write().await;
        for (_, h) in std::mem::take(&mut inner.handles) {
            h.shutdown();
        }
        for s in inner.states.values_mut() {
            s.status = SlotStatus::Idle;
            s.upstream_host.clear();
            s.upstream_port = 0;
            s.exit_ip.clear();
            s.alive = None;
        }
    }

    /// Cập nhật một trường bất kỳ của trạng thái cổng.
    pub async fn update<F: FnOnce(&mut SlotState)>(&self, port: u16, f: F) {
        if let Some(s) = self.inner.write().await.states.get_mut(&port) {
            f(s);
        }
    }

    pub async fn handle(&self, port: u16) -> Option<SlotHandle> {
        self.inner.read().await.handles.get(&port).cloned()
    }
}
