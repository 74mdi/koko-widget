//! FreeBSD platform discovery for the diagnostics report.

pub fn version_name() -> Option<String> {
    // FreeBSD exposes its version through sysctl, but KOKO WIDGET intentionally
    // avoids linking a platform-specific sysctl crate just for diagnostics.
    // `std::env::consts::OS` and the kernel name are enough for the report.
    Some("FreeBSD".to_owned())
}
