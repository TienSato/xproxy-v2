//! Nhà cung cấp proxy.
//!
//! Mô hình chỉ có MỘT kiểu: người dùng dán nguyên URL lấy IP từ trang nhà cung cấp.
//! Không còn chuyện app phải biết tên tham số của từng hãng (`ParamNames`), không còn
//! hai chế độ "tham số" / "URL mẫu" như bản Swift. Thêm nhà cung cấp mới = dán thêm
//! một URL.
//!
//! URL mẫu trông như:
//! ```text
//! https://webipapi.cliproxy.com/api/getIpInfo?key=...&port=443&num=1&country=US&state=&type=2
//! ```
//! và trả về mỗi dòng một proxy:
//! ```text
//! 64.205.177.44:443:6ab9171c2721211f:kd7c2apyfaxslfb61syy
//! ```

mod api;
pub mod regions;

pub use api::{build_url, fetch_one, fetch_upstreams, parse_line, parse_upstreams, raw_get, UrlOverrides};

use serde::{Deserialize, Serialize};

use crate::relay::Scheme;

/// Dạng dòng proxy mà API trả về.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LineFormat {
    /// `host:port:user:pass` — dạng phổ biến nhất.
    #[default]
    HostFirst,
    /// `user:pass@host:port`
    CredAtHost,
}

/// Một nhà cung cấp = một cái tên + một URL. Hết.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Provider {
    pub id: String,
    pub name: String,
    /// URL lấy IP, dán nguyên từ trang nhà cung cấp. Key nằm luôn trong URL.
    pub url: String,
    /// Trang quản lý, để mở nhanh khi cần lấy URL mới.
    pub dashboard_url: String,
    /// Giao thức của proxy upstream mà API trả về.
    pub upstream_scheme: Scheme,
    pub line_format: LineFormat,
    /// Nếu URL có tham số `port`, đổi nó theo từng cổng (443, 444, 445…).
    /// Một số nhà cung cấp dùng tham số này để tách phiên; tắt đi nếu hãng của bạn không.
    pub vary_port: bool,
    /// Nhà cung cấp dựng sẵn (không xoá được, nhưng sửa URL được).
    pub builtin: bool,
}

impl Default for Provider {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            url: String::new(),
            dashboard_url: String::new(),
            upstream_scheme: Scheme::Socks5,
            line_format: LineFormat::HostFirst,
            vary_port: true,
            builtin: false,
        }
    }
}

impl Provider {
    pub fn cliproxy() -> Self {
        Self {
            id: "cliproxy".into(),
            name: "CliProxy".into(),
            url: "https://webipapi.cliproxy.com/api/getIpInfo?key=&port=443&num=1&country=US&state=&type=2".into(),
            dashboard_url: "https://dash.cliproxy.com".into(),
            builtin: true,
            ..Default::default()
        }
    }

    pub fn novproxy() -> Self {
        Self {
            id: "novproxy".into(),
            name: "NovProxy".into(),
            url: String::new(),
            dashboard_url: "https://novproxy.com/login/".into(),
            builtin: true,
            ..Default::default()
        }
    }

    pub fn builtins() -> Vec<Provider> {
        vec![Self::cliproxy(), Self::novproxy()]
    }

    /// URL đã cấu hình xong chưa (có host và có vẻ đã điền key).
    pub fn configured(&self) -> bool {
        let u = self.url.trim();
        !u.is_empty() && u.starts_with("http") && !u.contains("key=&")
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Country {
    pub code: &'static str,
    pub name: &'static str,
}

pub const COUNTRIES: &[Country] = &[
    Country { code: "US", name: "United States" },
    Country { code: "GB", name: "United Kingdom" },
    Country { code: "CA", name: "Canada" },
    Country { code: "AU", name: "Australia" },
    Country { code: "DE", name: "Germany" },
    Country { code: "FR", name: "France" },
    Country { code: "NL", name: "Netherlands" },
    Country { code: "JP", name: "Japan" },
    Country { code: "KR", name: "South Korea" },
    Country { code: "SG", name: "Singapore" },
    Country { code: "HK", name: "Hong Kong" },
    Country { code: "TW", name: "Taiwan" },
    Country { code: "VN", name: "Vietnam" },
    Country { code: "TH", name: "Thailand" },
    Country { code: "ID", name: "Indonesia" },
    Country { code: "IN", name: "India" },
    Country { code: "BR", name: "Brazil" },
    Country { code: "ES", name: "Spain" },
    Country { code: "IT", name: "Italy" },
    Country { code: "RU", name: "Russia" },
];
