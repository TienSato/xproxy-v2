//! Cấu hình — một file JSON, không hơn.
//!
//! Bản trước cất API key vào Keychain. Bỏ rồi: key nằm luôn trong URL lấy IP, mà URL đó
//! là **cấu hình chứ không phải bí mật** — dán vào một lần là xong. Cất nó vào Keychain
//! chỉ làm mọi thao tác (sửa, sao lưu, chuyển máy) phiền hơn mà chẳng đổi được gì về
//! an toàn: ai đọc được file cấu hình thì cũng đọc được Keychain của phiên đó.

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use xproxy_core::provider::Provider;
use xproxy_core::Scheme;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub active_provider: String,
    /// Mọi nhà cung cấp, kể cả hai cái dựng sẵn — để sửa URL trực tiếp được.
    pub providers: Vec<Provider>,

    pub start_port: u16,
    pub port_count: u16,

    /// Giao thức cổng LAN nói với thiết bị.
    pub listen_scheme: Scheme,
    /// Khi đổi IP có ngắt kết nối đang mở không.
    pub drop_on_rotate: bool,
    pub dial_timeout_secs: u64,
    /// Trần kết nối đồng thời của mỗi cổng. **0 = tự động cân bằng** (mặc định).
    pub max_conns_per_slot: usize,
    /// Trần băng thông mỗi cổng, KB/s. `0` = không giới hạn.
    pub rate_limit_kbps: u64,

    pub auto_rotate: bool,
    pub rotate_minutes: u64,
    pub auto_check: bool,
    pub check_interval_secs: u64,

    pub country: String,
    pub state: String,
    pub city: String,

    // ── Giao diện ──
    /// "system" | "light" | "dark"
    pub theme: String,
    /// Màu nhấn, dạng hex. Mặc định xanh dương.
    pub accent: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            active_provider: "cliproxy".into(),
            providers: Provider::builtins(),
            start_port: 5100,
            port_count: 50,
            listen_scheme: Scheme::Socks5,
            drop_on_rotate: false,
            dial_timeout_secs: 15,
            max_conns_per_slot: 0,
            rate_limit_kbps: 0,
            auto_rotate: false,
            rotate_minutes: 30,
            auto_check: false,
            check_interval_secs: 60,
            country: String::new(),
            state: String::new(),
            city: String::new(),
            theme: "system".into(),
            accent: "#2563eb".into(),
        }
    }
}

impl Settings {
    pub fn provider(&self, id: &str) -> Provider {
        self.providers
            .iter()
            .find(|p| p.id == id)
            .cloned()
            .unwrap_or_else(Provider::cliproxy)
    }

    pub fn active(&self) -> Provider {
        self.provider(&self.active_provider)
    }

    /// Bổ sung nhà cung cấp dựng sẵn nếu file cấu hình cũ chưa có (ví dụ người dùng
    /// nâng cấp từ bản trước, hoặc lỡ xoá).
    pub fn ensure_builtins(&mut self) {
        for b in Provider::builtins() {
            if !self.providers.iter().any(|p| p.id == b.id) {
                self.providers.push(b);
            }
        }
        if !self.providers.iter().any(|p| p.id == self.active_provider) {
            if let Some(first) = self.providers.first() {
                self.active_provider = first.id.clone();
            }
        }
    }
}

/// Thư mục cấu hình theo quy ước từng hệ điều hành, không cần crate ngoài:
/// - macOS:   `~/Library/Application Support/xproxy`
/// - Windows: `%APPDATA%\xproxy`
/// - Linux:   `$XDG_CONFIG_HOME/xproxy` (mặc định `~/.config/xproxy`)
pub fn config_dir() -> PathBuf {
    let dir = if cfg!(target_os = "macos") {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .map(|h| h.join("Library/Application Support/xproxy"))
    } else if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .map(|h| h.join("xproxy"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
            .map(|h| h.join("xproxy"))
    };
    let dir = dir.unwrap_or_else(|| PathBuf::from("."));
    let _ = fs::create_dir_all(&dir);
    dir
}

pub fn config_path() -> PathBuf {
    config_dir().join("settings.json")
}

pub fn load() -> Settings {
    let mut s: Settings = match fs::read_to_string(config_path()) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
        Err(_) => Settings::default(),
    };
    s.ensure_builtins();
    s
}

pub fn save(s: &Settings) -> Result<(), String> {
    let text = serde_json::to_string_pretty(s).map_err(|e| e.to_string())?;
    fs::write(config_path(), text).map_err(|e| e.to_string())
}
