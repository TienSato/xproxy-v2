//! # xproxy-core
//!
//! Lõi của xproxy: relay proxy, API nhà cung cấp, tiện ích mạng.
//!
//! Crate này **không phụ thuộc Tauri** — cố ý. Nhờ vậy nó build và test được trên bất kỳ
//! máy nào có Rust (kể cả CI Linux không có WebView), và có thể chạy dạng CLI để đo tốc
//! độ so với gost mà không cần dựng giao diện.

pub mod addr;
pub mod error;
pub mod manager;
pub mod net;
pub mod provider;
pub mod relay;
pub mod rlimit;

pub use error::{Error, Result};
pub use manager::{Health, Manager, SlotState, SlotStatus, SlotView};
pub use relay::{
    CounterSnapshot, GlobalLimit, ListenAuth, RateLimiter, Scheme, SlotConfig, SlotHandle, Upstream,
};
