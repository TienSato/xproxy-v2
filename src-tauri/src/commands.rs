//! Mặt tiếp xúc giữa UI (web) và lõi Rust.
//!
//! Quy ước: mỗi `#[tauri::command]` chỉ bóc tham số rồi gọi một hàm thuần bên dưới nhận
//! `&Manager` + `Settings`. Nhờ vậy logic gọi lại được lẫn nhau (ví dụ "gán hàng loạt"
//! gọi lại "gán một cổng") mà không phải chuyền `State` vòng quanh.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use tokio::sync::RwLock;

use xproxy_core::provider::{self, LineFormat, Provider, UrlOverrides};
use xproxy_core::{net, Health, Manager, Scheme, SlotConfig, SlotStatus, SlotView};

use crate::settings::{self, Settings};

#[derive(Clone)]
pub struct AppState {
    pub manager: Manager,
    pub settings: Arc<RwLock<Settings>>,
    /// Cờ xin dừng đợt "gán hàng loạt" đang chạy.
    pub batch_cancel: Arc<AtomicBool>,
}

type R<T> = Result<T, String>;

pub fn slot_config(s: &Settings) -> SlotConfig {
    SlotConfig {
        listen_scheme: s.listen_scheme,
        listen_auth: Default::default(),
        dial_timeout: Duration::from_secs(s.dial_timeout_secs.max(3)),
        drop_on_rotate: s.drop_on_rotate,
        // 0 ở UI nghĩa là "tự động" — lõi hiểu đó là None.
        max_conns: (s.max_conns_per_slot > 0).then_some(s.max_conns_per_slot),
        rate_limit_bps: s.rate_limit_kbps.saturating_mul(1024),
    }
}

fn overrides(s: &Settings, provider_port: u16) -> UrlOverrides {
    UrlOverrides {
        country: s.country.clone(),
        state: s.state.clone(),
        city: s.city.clone(),
        provider_port: Some(provider_port),
    }
}

// ══ Cấu hình ═════════════════════════════════════════════════════════════════

#[tauri::command]
pub async fn get_settings(state: State<'_, AppState>) -> R<Settings> {
    Ok(state.settings.read().await.clone())
}

#[tauri::command]
pub async fn save_settings(state: State<'_, AppState>, value: Settings) -> R<()> {
    let mut value = value;
    value.ensure_builtins();
    settings::save(&value)?;
    state.manager.set_config(slot_config(&value)).await;
    *state.settings.write().await = value;
    Ok(())
}

#[tauri::command]
pub async fn lan_ip() -> R<String> {
    Ok(net::lan_ip().unwrap_or_else(|| "—".into()))
}

/// Thử URL của một nhà cung cấp mà KHÔNG gán vào cổng nào — xem đúng chưa rồi hãy dùng.
#[tauri::command]
pub async fn test_provider(state: State<'_, AppState>, provider_id: String) -> R<TestResult> {
    let s = state.settings.read().await.clone();
    let p = s.provider(&provider_id);

    let url = provider::build_url(&p.url, &overrides(&s, 443), p.vary_port)
        .map_err(|e| e.to_string())?;
    let (shown, body) = provider::raw_get(url.as_str()).await;

    let parsed = provider::parse_upstreams(&body, p.line_format);
    Ok(TestResult {
        url: shown,
        body,
        parsed: parsed
            .iter()
            .map(|u| format!("{}:{}  (user: {})", u.host, u.port, u.user))
            .collect(),
    })
}

#[derive(Serialize)]
pub struct TestResult {
    pub url: String,
    pub body: String,
    /// Những dòng app đọc được — trống nghĩa là URL sai hoặc chọn nhầm định dạng dòng.
    pub parsed: Vec<String>,
}

// ══ Cổng ═════════════════════════════════════════════════════════════════════

#[tauri::command]
pub async fn generate_ports(state: State<'_, AppState>) -> R<Vec<SlotView>> {
    let s = state.settings.read().await.clone();
    let _ = state.manager.generate_ports(s.start_port, s.port_count).await;
    Ok(state.manager.views().await)
}

#[tauri::command]
pub async fn list_slots(state: State<'_, AppState>) -> R<Vec<SlotView>> {
    Ok(state.manager.views().await)
}

#[derive(Serialize, Clone)]
pub struct Summary {
    pub running: usize,
    pub total: usize,
    pub lan_ip: String,
    pub health: Health,
    /// Cổng đã tự nhảy qua vì chương trình khác đang giữ.
    pub skipped_ports: Vec<u16>,
}

#[tauri::command]
pub async fn summary(state: State<'_, AppState>) -> R<Summary> {
    let views = state.manager.views().await;
    Ok(Summary {
        running: views
            .iter()
            .filter(|v| v.state.status == SlotStatus::Running)
            .count(),
        total: views.len(),
        lan_ip: net::lan_ip().unwrap_or_else(|| "—".into()),
        health: state.manager.health(),
        skipped_ports: state.manager.skipped_ports().await,
    })
}

/// Gán / đổi IP cho một cổng: gọi URL của nhà cung cấp, lấy một dòng proxy, cắm vào cổng.
///
/// Cổng KHÔNG bị đóng rồi mở lại. Số cổng không đổi, thiết bị đang cắm vào không thấy
/// gián đoạn, và không còn tình huống "cổng bị chiếm → nhảy sang cổng khác".
pub async fn do_assign(
    app: &AppHandle,
    st: &AppState,
    s: &Settings,
    port: u16,
) -> Result<(), String> {
    let mgr = &st.manager;
    let p = s.active();

    let provider_port = mgr
        .view(port)
        .await
        .map(|v| v.state.provider_port)
        .unwrap_or(443);

    mgr.update(port, |x| {
        x.status = SlotStatus::Starting;
        x.message.clear();
    })
    .await;

    match provider::fetch_one(&p, &overrides(s, provider_port)).await {
        Ok(up) => {
            // Mở cổng có thể hỏng (cổng đang bị chương trình khác chiếm). Trước đây
            // chỗ này `?` thẳng ra ngoài, nên trạng thái kẹt ở "Đang lấy IP" vĩnh viễn
            // và người dùng không thấy lý do. Phải ghi lỗi vào đúng cổng đó.
            if let Err(e) = mgr.assign(port, up).await {
                let msg = e.to_string();
                let m2 = msg.clone();
                mgr.update(port, |x| {
                    x.status = SlotStatus::Error;
                    x.message = m2;
                })
                .await;
                return Err(msg);
            }
            let (pid, c, st_, ci) = (p.id.clone(), s.country.clone(), s.state.clone(), s.city.clone());
            mgr.update(port, |x| {
                x.provider_id = pid;
                x.country = c;
                x.state = st_;
                x.city = ci;
                x.message.clear();
            })
            .await;
            let _ = app.emit("slot-updated", port);
            Ok(())
        }
        Err(e) => {
            let msg = e.to_string();
            let m2 = msg.clone();
            mgr.update(port, |x| {
                x.status = SlotStatus::Error;
                x.message = m2;
            })
            .await;
            Err(msg)
        }
    }
}

#[tauri::command]
pub async fn assign_slot(app: AppHandle, state: State<'_, AppState>, port: u16) -> R<SlotView> {
    let s = state.settings.read().await.clone();
    do_assign(&app, &state, &s, port).await?;
    state
        .manager
        .view(port)
        .await
        .ok_or_else(|| format!("không có cổng {port}"))
}

/// Dán một dòng proxy thủ công.
#[tauri::command]
pub async fn assign_manual(
    app: AppHandle,
    state: State<'_, AppState>,
    port: u16,
    line: String,
    cred_at_host: bool,
) -> R<SlotView> {
    let s = state.settings.read().await.clone();
    let fmt = if cred_at_host {
        LineFormat::CredAtHost
    } else {
        LineFormat::HostFirst
    };
    let mut up = provider::parse_line(&line, fmt)
        .ok_or_else(|| "sai định dạng — cần host:port:user:pass".to_string())?;
    up.scheme = s.active().upstream_scheme;

    if let Err(e) = state.manager.assign(port, up).await {
        let msg = e.to_string();
        let m2 = msg.clone();
        state
            .manager
            .update(port, |x| {
                x.status = SlotStatus::Error;
                x.message = m2;
            })
            .await;
        return Err(msg);
    }
    state
        .manager
        .update(port, |x| {
            x.message = "Dán thủ công".into();
            x.country.clear();
            x.state.clear();
            x.city.clear();
        })
        .await;
    let _ = app.emit("slot-updated", port);

    state
        .manager
        .view(port)
        .await
        .ok_or_else(|| format!("không có cổng {port}"))
}

#[tauri::command]
pub async fn unassign_slot(state: State<'_, AppState>, port: u16) -> R<()> {
    state.manager.unassign(port).await;
    Ok(())
}

#[tauri::command]
pub async fn stop_all(state: State<'_, AppState>) -> R<()> {
    state.manager.shutdown_all().await;
    Ok(())
}

/// Gán hàng loạt các cổng đang trống, tuần tự, nghỉ 200ms giữa mỗi lần gọi API.
#[tauri::command]
pub async fn assign_batch(app: AppHandle, state: State<'_, AppState>, count: usize) -> R<String> {
    let targets: Vec<u16> = state
        .manager
        .idle_ports()
        .await
        .into_iter()
        .take(count)
        .collect();
    if targets.is_empty() {
        return Ok("không có cổng nào đang trống".into());
    }

    let s = state.settings.read().await.clone();
    state.batch_cancel.store(false, Ordering::Relaxed);

    let total = targets.len();
    let (mut ok, mut fail) = (0usize, 0usize);

    for (i, port) in targets.into_iter().enumerate() {
        if state.batch_cancel.load(Ordering::Relaxed) {
            let msg = format!("đã dừng ở {i}/{total} — thành công {ok}, lỗi {fail}");
            let _ = app.emit("batch-progress", &msg);
            return Ok(msg);
        }
        let _ = app.emit("batch-progress", format!("đang gán {}/{total}…", i + 1));

        match do_assign(&app, &state, &s, port).await {
            Ok(()) => ok += 1,
            Err(_) => fail += 1,
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    let msg = format!("hoàn tất: {ok} thành công, {fail} lỗi");
    let _ = app.emit("batch-progress", &msg);
    Ok(msg)
}

#[tauri::command]
pub async fn cancel_batch(state: State<'_, AppState>) -> R<()> {
    state.batch_cancel.store(true, Ordering::Relaxed);
    Ok(())
}

// ══ Kiểm tra ═════════════════════════════════════════════════════════════════

/// Ping nhẹ tới upstream — KHÔNG đi qua cổng relay đang tải, nên không tranh băng thông
/// với thiết bị đang dùng proxy.
pub async fn do_check(mgr: &Manager, port: u16) -> Option<bool> {
    let v = mgr.view(port).await?;
    if v.state.upstream_host.is_empty() {
        return None;
    }
    let alive = net::ping_upstream(
        &v.state.upstream_host,
        v.state.upstream_port,
        Duration::from_secs(4),
    )
    .await;
    mgr.update(port, |x| x.alive = Some(alive)).await;
    Some(alive)
}

#[tauri::command]
pub async fn check_slot(state: State<'_, AppState>, port: u16) -> R<Option<bool>> {
    Ok(do_check(&state.manager, port).await)
}

#[tauri::command]
pub async fn check_all(state: State<'_, AppState>) -> R<()> {
    let mgr = state.manager.clone();
    let ports: Vec<u16> = mgr
        .views()
        .await
        .into_iter()
        .filter(|v| v.state.status == SlotStatus::Running)
        .map(|v| v.state.port)
        .collect();
    for p in ports {
        do_check(&mgr, p).await;
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    Ok(())
}

/// Kiểm tra IP thoát của một cổng — **chỉ chạy khi người dùng bấm**.
///
/// Đây là lệnh DUY NHẤT trong app gửi dữ liệu qua proxy. Mọi thứ khác (hiện proxy đã
/// gán, ping kiểm tra sống) không tốn một byte băng thông nhà cung cấp nào.
#[tauri::command]
pub async fn check_exit_ip(state: State<'_, AppState>, port: u16) -> R<String> {
    let scheme = state.settings.read().await.listen_scheme;
    let ip = net::exit_ip(port, scheme == Scheme::Http)
        .await
        .map_err(|e| e.to_string())?;
    let out = ip.clone();
    state
        .manager
        .update(port, |x| {
            x.exit_ip = ip;
            x.alive = Some(true);
        })
        .await;
    Ok(out)
}

#[tauri::command]
pub async fn reset_traffic(state: State<'_, AppState>, port: u16) -> R<()> {
    if let Some(h) = state.manager.handle(port).await {
        h.reset_traffic();
    }
    Ok(())
}

// ══ Nền ══════════════════════════════════════════════════════════════════════

/// Dùng cho vòng tự xoay IP ở task nền.
pub async fn rotate_one(
    app: &AppHandle,
    st: &AppState,
    s: &Settings,
    port: u16,
) -> Result<(), String> {
    do_assign(app, st, s, port).await
}

/// Giữ lại cho UI cần biết danh sách nhà cung cấp mà không phải đọc cả Settings.
#[tauri::command]
pub async fn list_providers(state: State<'_, AppState>) -> R<Vec<Provider>> {
    Ok(state.settings.read().await.providers.clone())
}
