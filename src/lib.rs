//! KOKO WIDGET: a native, animated MPRIS music overlay.

pub mod animation;
pub mod app;
pub mod cli;
pub mod config;
pub mod diagnostics;
pub mod ipc;
pub mod media;
pub mod platform;
pub mod ui;
pub mod utils;
pub mod window;

/// Reverse-DNS application identifier.
pub const APP_ID: &str = "io.github.kokowidget.KokoWidget";
/// Application name used in UI and logs.
pub const APP_NAME: &str = "KOKO WIDGET";
/// Version from Cargo.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
