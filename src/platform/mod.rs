//! Cross-platform detection and diagnostics.

#[cfg(target_os = "freebsd")]
pub mod freebsd;
#[cfg(target_os = "linux")]
pub mod linux;

pub mod capabilities;

pub use capabilities::{Capabilities, DisplayBackend, MonitorInfo, detect};

pub fn os_name() -> &'static str {
    std::env::consts::OS
}

pub fn distribution_name() -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        return linux::distribution_name();
    }
    #[cfg(target_os = "freebsd")]
    {
        return freebsd::version_name();
    }
    #[allow(unreachable_code)]
    None
}
