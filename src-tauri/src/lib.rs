mod commands;
mod settings;

use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use tauri::{Emitter, Manager as _};
use tokio::sync::RwLock;
use xproxy_core::{Manager, SlotStatus};

use commands::{slot_config, AppState};
use settings::Settings;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,xproxy_core=debug".into()),
        )
        .init();

    let cfg = settings::load();

    // Manager::new nâng giới hạn file descriptor ngay tại đây — phải làm trước khi mở
    // cổng nào. Trên macOS app chạy từ Finder chỉ có 256 fd mềm.
    let manager = Manager::new(slot_config(&cfg));

    let state = AppState {
        manager,
        settings: Arc::new(RwLock::new(cfg)),
        batch_cancel: Arc::new(AtomicBool::new(false)),
    };
    let bg = state.clone(); // mọi trường đều là Arc nên clone là dùng chung

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(state)
        .setup(move |app| {
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(background(handle, bg));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_settings,
            commands::save_settings,
            commands::list_providers,
            commands::test_provider,
            commands::lan_ip,
            commands::generate_ports,
            commands::list_slots,
            commands::summary,
            commands::assign_slot,
            commands::assign_manual,
            commands::unassign_slot,
            commands::stop_all,
            commands::assign_batch,
            commands::cancel_batch,
            commands::check_slot,
            commands::check_all,
            commands::check_exit_ip,
            commands::reset_traffic,
        ])
        .run(tauri::generate_context!())
        .expect("không khởi động được cửa sổ xproxy");
}

/// Một task nền lo ba việc: đẩy số liệu lên UI, tự kiểm tra, tự xoay IP.
///
/// Đọc lại cấu hình mỗi nhịp nên bật/tắt trong Cài đặt có hiệu lực ngay, không cần dựng
/// lại timer như bản Swift.
async fn background(app: tauri::AppHandle, st: AppState) {
    {
        let s = st.settings.read().await.clone();
        let _ = st.manager.generate_ports(s.start_port, s.port_count).await;
    }

    let mut tick = tokio::time::interval(Duration::from_secs(1));
    let mut elapsed: u64 = 0;

    loop {
        tick.tick().await;
        elapsed += 1;

        // 1) Đẩy trạng thái mọi cổng lên UI.
        let _ = app.emit("slots", st.manager.views().await);

        let s: Settings = st.settings.read().await.clone();

        // 2) Tự kiểm tra sống/chết (ping nhẹ, không qua cổng đang tải).
        if s.auto_check && s.check_interval_secs > 0 && elapsed % s.check_interval_secs == 0 {
            let st2 = st.clone();
            tokio::spawn(async move {
                let ports = running_ports(&st2.manager).await;
                for p in ports {
                    commands::do_check(&st2.manager, p).await;
                    tokio::time::sleep(Duration::from_millis(300)).await;
                }
            });
        }

        // 3) Tự xoay IP.
        if s.auto_rotate && s.rotate_minutes > 0 && elapsed % (s.rotate_minutes * 60) == 0 {
            let _ = app.emit("batch-progress", "đang tự xoay IP…".to_string());
            for p in running_ports(&st.manager).await {
                let _ = commands::rotate_one(&app, &st, &s, p).await;
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
            let _ = app.emit("batch-progress", "xoay IP xong".to_string());
        }
    }
}

async fn running_ports(mgr: &Manager) -> Vec<u16> {
    mgr.views()
        .await
        .into_iter()
        .filter(|v| v.state.status == SlotStatus::Running)
        .map(|v| v.state.port)
        .collect()
}
