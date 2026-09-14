//! Vòng đời một cổng chuyển tiếp.
//!
//! # Khác biệt cốt lõi so với bản Swift
//!
//! Bản Swift spawn một tiến trình `gost` cho mỗi cổng. Muốn đổi IP thì phải giết tiến
//! trình đó rồi bind lại cùng số cổng — mà `terminate()` chỉ gửi SIGTERM nên socket
//! chưa nhả ngay. Từ đó sinh ra `terminateAndWait` (poll 30 vòng × 50ms), rồi
//! `killProcessOnPort` (gọi `lsof` tìm PID, `kill -9`), rồi `killOrphanGost` (gọi
//! `pkill` lúc khởi động). Khoảng 80 dòng chỉ để dọn hậu quả.
//!
//! Ở đây listener nằm trong tiến trình app và KHÔNG BAO GIỜ bị đóng khi đổi IP —
//! upstream nằm sau một `RwLock`, đổi IP chỉ là ghi vào đó. Kết nối mới đi upstream mới
//! ngay lập tức; thiết bị đầu kia không thấy cổng rớt một nhịp nào.
//!
//! # Chạy nhiều cổng cùng lúc
//!
//! Mỗi kết nối là một task tokio độc lập trên runtime nhiều luồng, nên không có chuyện
//! kết nối chậm của cổng này chặn cổng kia. Thứ thật sự gây hại giữa các cổng là **cạn
//! file descriptor** — giới hạn fd là của cả tiến trình. Xem `limit.rs` và `rlimit.rs`.

mod cancel;
mod http;
pub mod limit;
mod pipe;
mod socks5;
pub mod upstream;

pub use limit::RateLimiter;
pub use pipe::{CounterSnapshot, Counters};
pub use socks5::ListenAuth;
pub use upstream::{Scheme, Upstream};

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use tokio::net::{TcpListener, TcpStream};
use tokio::sync::RwLock;
use tracing::{debug, trace, warn};

use cancel::Cancel;

use crate::addr::write_socks_reply;
use crate::error::{Error, Result};

/// Cấu hình phía nghe của một cổng (thiết bị LAN cắm vào).
#[derive(Debug, Clone)]
pub struct SlotConfig {
    /// Giao thức cổng LAN nói: SOCKS5 hay HTTP.
    pub listen_scheme: Scheme,
    /// Bắt thiết bị LAN đăng nhập (để trống = không cần).
    pub listen_auth: ListenAuth,
    /// Thời gian tối đa chờ nối được upstream.
    pub dial_timeout: Duration,
    /// Khi đổi IP, có ngắt luôn các kết nối đang mở không.
    ///
    /// `false` (mặc định): kết nối đang tải chạy nốt trên IP cũ, kết nối mới dùng IP mới.
    /// `true`: ngắt hết — thiết bị buộc phải dùng IP mới ngay.
    pub drop_on_rotate: bool,
    /// Trần kết nối đồng thời của riêng cổng này.
    ///
    /// `None` = **tự động**: không chặn gì khi máy còn rảnh, và tự chia đều khi cả app
    /// sắp chạm giới hạn. Đây là mặc định, người dùng không phải hiểu gì cả.
    /// `Some(n)` = tự đặt cứng ở n.
    pub max_conns: Option<usize>,
    /// Trần băng thông của cổng, byte/giây. `0` = không giới hạn (mặc định).
    pub rate_limit_bps: u64,
}

impl Default for SlotConfig {
    fn default() -> Self {
        Self {
            listen_scheme: Scheme::Socks5,
            listen_auth: ListenAuth::default(),
            dial_timeout: Duration::from_secs(15),
            drop_on_rotate: false,
            max_conns: None,
            rate_limit_bps: 0,
        }
    }
}

/// Bộ đếm kết nối đang mở của TOÀN BỘ app.
///
/// Đây là hàng rào chung: fd là tài nguyên của cả tiến trình, nên phải có một chỗ đếm
/// chung thì mới ngăn được cổng này làm chết cổng kia.
/// Sàn của phần chia đều: dù chia kiểu gì mỗi cổng cũng được ít nhất chừng này.
const MIN_SHARE: usize = 16;
/// Dưới ngưỡng này coi như máy còn rảnh → không chặn ai cả.
const PRESSURE_PERCENT: usize = 60;

#[derive(Debug)]
pub struct GlobalLimit {
    open: AtomicUsize,
    max: AtomicUsize,
    /// Số cổng đang mở — mẫu số của phép chia đều.
    slots: AtomicUsize,
}

impl GlobalLimit {
    pub fn new(max: usize) -> Arc<Self> {
        Arc::new(Self {
            open: AtomicUsize::new(0),
            max: AtomicUsize::new(max),
            slots: AtomicUsize::new(0),
        })
    }

    pub fn set_max(&self, max: usize) {
        self.max.store(max, Ordering::Relaxed);
    }

    pub fn open(&self) -> usize {
        self.open.load(Ordering::Relaxed)
    }

    pub fn max(&self) -> usize {
        self.max.load(Ordering::Relaxed)
    }

    pub fn slots(&self) -> usize {
        self.slots.load(Ordering::Relaxed)
    }

    /// Phần chia đều cho mỗi cổng khi máy đang căng.
    pub fn fair_share(&self) -> usize {
        (self.max() / self.slots().max(1)).max(MIN_SHARE)
    }

    fn register_slot(&self) {
        self.slots.fetch_add(1, Ordering::Relaxed);
    }

    fn unregister_slot(&self) {
        self.slots.fetch_sub(1, Ordering::Relaxed);
    }

    /// Xin một chỗ cho kết nối mới.
    ///
    /// Ba lớp, xét theo thứ tự:
    /// 1. Trần chung của cả app (suy ra từ giới hạn file descriptor) — không ai vượt được.
    /// 2. Trần cứng của cổng, nếu người dùng tự đặt.
    /// 3. **Chia đều tự động** khi không tự đặt: lúc máy còn rảnh (dưới 60% trần chung)
    ///    thì cho thoải mái — một cổng chạy một mình được dùng hết công suất. Chỉ khi
    ///    đã căng mới ép mỗi cổng về phần chia đều của nó, để cổng đang mở nhiều không
    ///    lấn sang cổng khác.
    /// `std::result::Result` chứ không phải alias `Result<T>` của crate này — ở đây
    /// kiểu lỗi là một chuỗi lý do để ghi log, không phải `crate::Error`.
    fn try_enter(
        self: &Arc<Self>,
        slot_open: usize,
        cap: Option<usize>,
    ) -> std::result::Result<GlobalGuard, &'static str> {
        let max = self.max.load(Ordering::Relaxed);
        let before = self.open.fetch_add(1, Ordering::Relaxed);
        // Tạo guard NGAY để mọi đường thoát bên dưới đều trả chỗ lại.
        let guard = GlobalGuard(self.clone());

        if before >= max {
            return Err("cả app đã chạm trần kết nối");
        }
        match cap {
            Some(c) if c > 0 && slot_open >= c => Err("cổng đã chạm trần tự đặt"),
            Some(_) => Ok(guard),
            None => {
                if before * 100 >= max * PRESSURE_PERCENT && slot_open >= self.fair_share() {
                    Err("đang căng, cổng đã dùng hết phần chia đều")
                } else {
                    Ok(guard)
                }
            }
        }
    }
}

/// Trả chỗ về cho `GlobalLimit` khi kết nối kết thúc, kể cả khi task panic.
struct GlobalGuard(Arc<GlobalLimit>);

impl Drop for GlobalGuard {
    fn drop(&mut self) {
        self.0.open.fetch_sub(1, Ordering::Relaxed);
    }
}

/// Tay cầm điều khiển một cổng đang chạy. Clone được, dùng chung giữa các task.
#[derive(Clone)]
pub struct SlotHandle {
    port: u16,
    upstream: Arc<RwLock<Option<Upstream>>>,
    config: Arc<RwLock<SlotConfig>>,
    counters: Arc<Counters>,
    /// Các THIẾT BỊ đang cắm vào cổng, đếm theo IP (mỗi IP giữ nhiều kết nối TCP).
    /// Người dùng nghĩ theo đơn vị "thiết bị", không phải "kết nối TCP" — một trình
    /// duyệt mở hàng chục kết nối cho đúng một cái máy.
    peers: Arc<StdMutex<HashMap<IpAddr, u32>>>,
    /// Gáo token của cổng. Đổi giới hạn = thay cả Arc, kết nối đang chạy nhận ngay.
    rate: Arc<RwLock<Arc<RateLimiter>>>,
    global: Arc<GlobalLimit>,
    /// Huỷ toàn bộ cổng (gỡ cổng).
    cancel: Cancel,
    /// Huỷ riêng các kết nối của "đời" upstream hiện tại (dùng khi drop_on_rotate).
    generation: Arc<RwLock<Cancel>>,
}

impl SlotHandle {
    /// Bind cổng và bắt đầu nhận kết nối. Lỗi ngay nếu cổng đang bận —
    /// không cần `lsof` hay `isPortFree` trước, `bind` tự trả lời.
    ///
    /// Truyền `port = 0` để hệ điều hành tự chọn cổng trống (tiện cho test).
    pub async fn bind(port: u16, config: SlotConfig, global: Arc<GlobalLimit>) -> Result<Self> {
        let listener = TcpListener::bind(("0.0.0.0", port)).await.map_err(|e| {
            // "Address already in use" là tình huống người dùng gặp thật và sửa được,
            // nên đừng để nó lọt ra dưới dạng lỗi I/O chung chung.
            if e.kind() == std::io::ErrorKind::AddrInUse {
                Error::PortInUse(port)
            } else {
                Error::Io(e)
            }
        })?;
        let port = listener.local_addr()?.port();
        let rate = RateLimiter::new(config.rate_limit_bps);
        global.register_slot();

        let handle = SlotHandle {
            port,
            upstream: Arc::new(RwLock::new(None)),
            config: Arc::new(RwLock::new(config)),
            counters: Arc::new(Counters::default()),
            peers: Arc::new(StdMutex::new(HashMap::new())),
            rate: Arc::new(RwLock::new(rate)),
            global,
            cancel: Cancel::new(),
            generation: Arc::new(RwLock::new(Cancel::new())),
        };

        let h = handle.clone();
        tokio::spawn(async move { h.accept_loop(listener).await });
        Ok(handle)
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn stats(&self) -> CounterSnapshot {
        let mut s = self.counters.snapshot();
        s.devices = self.peers.lock().map(|m| m.len()).unwrap_or(0) as u64;
        s
    }

    fn peer_join(&self, ip: IpAddr) {
        if let Ok(mut m) = self.peers.lock() {
            *m.entry(ip).or_insert(0) += 1;
        }
    }

    fn peer_leave(&self, ip: IpAddr) {
        if let Ok(mut m) = self.peers.lock() {
            if let Some(n) = m.get_mut(&ip) {
                *n -= 1;
                if *n == 0 {
                    m.remove(&ip);
                }
            }
        }
    }

    pub fn reset_traffic(&self) {
        self.counters.reset_traffic();
    }

    pub async fn upstream(&self) -> Option<Upstream> {
        self.upstream.read().await.clone()
    }

    /// Gán / đổi upstream. **Không đóng listener** — đây là điểm ăn tiền của thiết kế.
    pub async fn set_upstream(&self, up: Upstream) {
        let drop_existing = self.config.read().await.drop_on_rotate;
        *self.upstream.write().await = Some(up);
        if drop_existing {
            self.rotate_generation().await;
        }
    }

    /// Gỡ upstream nhưng GIỮ cổng mở, để kết nối mới bị từ chối có lý do rõ ràng thay vì
    /// thiết bị thấy "connection refused" khó hiểu.
    pub async fn clear_upstream(&self) {
        *self.upstream.write().await = None;
        self.rotate_generation().await;
    }

    /// Đổi cấu hình. Trần băng thông có hiệu lực NGAY với cả kết nối đang chạy;
    /// trần số kết nối áp cho kết nối mới.
    pub async fn set_config(&self, cfg: SlotConfig) {
        let new_rate = cfg.rate_limit_bps;
        let old_rate = self.config.read().await.rate_limit_bps;
        *self.config.write().await = cfg;
        if new_rate != old_rate {
            *self.rate.write().await = RateLimiter::new(new_rate);
        }
    }

    /// Ngắt mọi kết nối đang mở, giữ cổng.
    pub async fn drop_connections(&self) {
        self.rotate_generation().await;
    }

    async fn rotate_generation(&self) {
        let mut g = self.generation.write().await;
        g.cancel();
        *g = Cancel::new();
    }

    /// Đóng hẳn cổng: dừng accept loop, ngắt mọi kết nối, nhả socket.
    pub fn shutdown(&self) {
        self.cancel.cancel();
    }

    pub fn is_closed(&self) -> bool {
        self.cancel.is_cancelled()
    }

    // ── Vòng lặp nhận kết nối ────────────────────────────────────────────────

    async fn accept_loop(self, listener: TcpListener) {
        loop {
            let accepted = tokio::select! {
                biased;
                _ = self.cancel.cancelled() => break,
                r = listener.accept() => r,
            };

            let (stream, peer) = match accepted {
                Ok(v) => v,
                Err(e) => {
                    // Cạn fd (EMFILE/ENFILE): nghỉ một nhịp rồi thử lại, đừng quay vòng
                    // nóng đốt CPU — lúc này các cổng khác cũng đang chật vật.
                    warn!(port = self.port, error = %e, "accept lỗi");
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    continue;
                }
            };
            stream.set_nodelay(true).ok();

            // ── Hàng rào trước khi nhận việc ──
            let cap = self.config.read().await.max_conns;
            let slot_open = self.counters.open.load(Ordering::Relaxed) as usize;

            let global_guard = match self.global.try_enter(slot_open, cap) {
                Ok(g) => g,
                Err(why) => {
                    // Đóng thẳng socket. Rẻ và đúng chuẩn cho server quá tải: client
                    // thấy kết nối đứt và tự thử lại. KHÔNG cố gửi mã lỗi SOCKS5 —
                    // muốn gửi thì phải chạy hết phần bắt tay, mà lúc đang chạm trần
                    // thì việc cần làm là giải phóng chỗ chứ không phải làm thêm việc.
                    self.counters.rejected.fetch_add(1, Ordering::Relaxed);
                    debug!(port = self.port, why, "từ chối kết nối");
                    drop(stream);
                    continue;
                }
            };

            // Đếm TRƯỚC khi spawn, không phải trong task: nếu đếm trong task thì giữa
            // lúc accept và lúc task chạy có một khe hở, và một loạt kết nối ùa vào cùng
            // lúc sẽ lọt qua trần.
            self.counters.open.fetch_add(1, Ordering::Relaxed);
            self.counters.total.fetch_add(1, Ordering::Relaxed);
            self.peer_join(peer.ip());

            let slot = self.clone();
            let generation = slot.generation.read().await.clone();
            tokio::spawn(async move {
                let _guard = global_guard; // nhả chỗ khi task kết thúc, kể cả khi panic

                let result = tokio::select! {
                    biased;
                    _ = generation.cancelled() => Ok(()),
                    _ = slot.cancel.cancelled() => Ok(()),
                    r = slot.handle_conn(stream) => r,
                };

                slot.counters.open.fetch_sub(1, Ordering::Relaxed);
                slot.peer_leave(peer.ip());
                match result {
                    Ok(()) => {}
                    // Nhiễu mạng bình thường: thiết bị đóng tab, tải xong thì cắt, peer
                    // reset. Mạng nào cũng đầy thứ này — gọi nó là "lỗi" chỉ làm người
                    // dùng tưởng app hỏng trong khi mọi thứ đang chạy tốt.
                    Err(ref e) if Self::is_churn(e) => {
                        slot.counters.churn.fetch_add(1, Ordering::Relaxed);
                        trace!(port = slot.port, %peer, "kết nối đóng sớm");
                    }
                    // UDP ASSOCIATE / BIND: client hỏi, relay trả lời "không làm", client
                    // tự lùi về TCP. Trình duyệt hỏi liên tục vì QUIC/HTTP-3 chạy trên
                    // UDP. Đây KHÔNG phải hỏng — đếm riêng và log ở mức trace để không
                    // làm người dùng tưởng app lỗi.
                    Err(Error::UnsupportedCommand(cmd)) => {
                        slot.counters.udp_refused.fetch_add(1, Ordering::Relaxed);
                        trace!(port = slot.port, %peer, cmd, "client xin lệnh không hỗ trợ");
                    }
                    Err(e) => {
                        slot.counters.failed.fetch_add(1, Ordering::Relaxed);
                        debug!(port = slot.port, %peer, error = %e, "kết nối kết thúc có lỗi");
                    }
                }
            });
        }
        // listener rơi khỏi scope ở đây → cổng được nhả ngay lập tức.
        self.global.unregister_slot();
        debug!(port = self.port, "đã đóng cổng");
    }

    async fn handle_conn(&self, mut stream: TcpStream) -> Result<()> {
        let cfg = self.config.read().await.clone();
        let rate = self.rate.read().await.clone();

        match cfg.listen_scheme {
            Scheme::Socks5 => {
                let target = socks5::accept(&mut stream, &cfg.listen_auth).await?;
                let up = self.require_upstream().await?;

                match upstream::connect(&up, &target, cfg.dial_timeout).await {
                    Ok(server) => {
                        write_socks_reply(&mut stream, 0x00).await?;
                        pipe::splice(stream, server, self.counters.clone(), rate).await;
                        Ok(())
                    }
                    Err(e) => {
                        let _ = write_socks_reply(&mut stream, socks5::reply_code_for(&e)).await;
                        Err(e)
                    }
                }
            }
            Scheme::Http => {
                let req = http::accept(&mut stream).await?;
                let up = match self.require_upstream().await {
                    Ok(u) => u,
                    Err(e) => {
                        let _ = http::write_error(&mut stream, &e).await;
                        return Err(e);
                    }
                };

                match upstream::connect(&up, &req.target, cfg.dial_timeout).await {
                    Ok(mut server) => {
                        if req.is_connect {
                            http::write_connect_ok(&mut stream).await?;
                        } else {
                            use tokio::io::AsyncWriteExt;
                            server.write_all(&req.rewritten_head).await?;
                            server.flush().await?;
                            self.counters
                                .up
                                .fetch_add(req.rewritten_head.len() as u64, Ordering::Relaxed);
                        }
                        pipe::splice(stream, server, self.counters.clone(), rate).await;
                        Ok(())
                    }
                    Err(e) => {
                        let _ = http::write_error(&mut stream, &e).await;
                        Err(e)
                    }
                }
            }
        }
    }

    /// Lỗi này là nhiễu bình thường hay hỏng thật?
    ///
    /// Phân biệt đúng chỗ này quan trọng hơn vẻ ngoài: đếm nhầm thì cột "lỗi" lúc nào
    /// cũng đỏ và người dùng học cách phớt lờ nó — đến lúc hỏng thật thì không ai thấy.
    fn is_churn(e: &Error) -> bool {
        match e {
            Error::Io(io) => matches!(
                io.kind(),
                std::io::ErrorKind::ConnectionReset
                    | std::io::ErrorKind::ConnectionAborted
                    | std::io::ErrorKind::BrokenPipe
                    | std::io::ErrorKind::UnexpectedEof
                    | std::io::ErrorKind::NotConnected
            ),
            _ => false,
        }
    }

    async fn require_upstream(&self) -> Result<Upstream> {
        self.upstream
            .read()
            .await
            .clone()
            .ok_or(Error::NoUpstream(self.port))
    }
}
