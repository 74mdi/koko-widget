//! Runtime desktop capability detection.
//!
//! Nothing here assumes a particular compositor. The values are discovered from
//! GDK, the Wayland layer-shell runtime check, and the session environment.

use gtk::prelude::*;

use crate::window;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayBackend {
    Wayland,
    X11,
    Other,
}

impl DisplayBackend {
    pub fn label(self) -> &'static str {
        match self {
            Self::Wayland => "Wayland",
            Self::X11 => "X11",
            Self::Other => "unknown",
        }
    }
}

#[derive(Debug, Clone)]
pub struct MonitorInfo {
    pub connector: Option<String>,
    pub model: Option<String>,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub scale: i32,
}

impl MonitorInfo {
    pub fn label(&self, index: usize) -> String {
        self.connector
            .clone()
            .or_else(|| self.model.clone())
            .unwrap_or_else(|| format!("Display {}", index + 1))
    }
}

#[derive(Debug, Clone)]
pub struct Capabilities {
    pub os: String,
    pub distribution: Option<String>,
    pub session_type: Option<String>,
    pub backend: DisplayBackend,
    pub gdk_backend_name: String,
    pub layer_shell: bool,
    pub layer_shell_compiled: bool,
    pub layer_shell_namespace: &'static str,
    pub input_shapes: bool,
    pub compositor_hint: Option<String>,
    pub niri_hint: bool,
    pub monitors: Vec<MonitorInfo>,
    pub gtk_runtime: String,
    pub dbus_session: bool,
    pub dark_theme: bool,
    pub reduced_motion: bool,
}

impl Capabilities {
    pub fn summary_line(&self) -> String {
        format!(
            "{} · {} · {}",
            self.os,
            self.backend.label(),
            self.compositor_hint
                .as_deref()
                .unwrap_or("unknown compositor")
        )
    }
}

/// Detect capabilities. GTK must be initialized before calling this.
pub fn detect() -> Capabilities {
    let display = gtk::gdk::Display::default();
    let gdk_backend_name = display
        .as_ref()
        .map(|display| display.type_().name().to_string())
        .unwrap_or_else(|| "none".to_owned());
    let backend = if gdk_backend_name.contains("Wayland") {
        DisplayBackend::Wayland
    } else if gdk_backend_name.contains("X11") {
        DisplayBackend::X11
    } else {
        DisplayBackend::Other
    };
    let layer_shell = backend == DisplayBackend::Wayland && window::layer_shell_available();
    let input_shapes = display
        .as_ref()
        .map(|display| display.supports_input_shapes())
        .unwrap_or(false);
    let monitors = display.as_ref().map(monitor_info).unwrap_or_default();
    let compositor_hint = env_nonempty("XDG_CURRENT_DESKTOP")
        .or_else(|| env_nonempty("XDG_SESSION_DESKTOP"))
        .or_else(|| env_nonempty("NIRI_SOCKET").map(|_| "niri".to_owned()));
    let niri_hint = env_nonempty("NIRI_SOCKET").is_some()
        || compositor_hint
            .as_deref()
            .is_some_and(|desktop| desktop.to_ascii_lowercase().contains("niri"));
    let session_type = env_nonempty("XDG_SESSION_TYPE");
    let dbus_session = env_nonempty("DBUS_SESSION_BUS_ADDRESS").is_some();
    let (dark_theme, reduced_motion) = system_preferences();

    Capabilities {
        os: os_label(),
        distribution: crate::platform::distribution_name(),
        session_type,
        backend,
        gdk_backend_name,
        layer_shell,
        layer_shell_compiled: window::layer_shell_compiled(),
        layer_shell_namespace: window::layer_shell_namespace(),
        input_shapes,
        compositor_hint,
        niri_hint,
        monitors,
        gtk_runtime: format!(
            "{}.{}.{}",
            gtk::major_version(),
            gtk::minor_version(),
            gtk::micro_version()
        ),
        dbus_session,
        dark_theme,
        reduced_motion,
    }
}

fn monitor_info(display: &gtk::gdk::Display) -> Vec<MonitorInfo> {
    let monitors = display.monitors();
    let mut result = Vec::new();
    for index in 0..monitors.n_items() {
        if let Some(monitor) = monitors.item(index).and_downcast::<gtk::gdk::Monitor>() {
            let geometry = monitor.geometry();
            result.push(MonitorInfo {
                connector: monitor.connector().map(|value| value.to_string()),
                model: monitor.model().map(|value| value.to_string()),
                x: geometry.x(),
                y: geometry.y(),
                width: geometry.width(),
                height: geometry.height(),
                scale: monitor.scale_factor(),
            });
        }
    }
    result
}

fn env_nonempty(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

fn os_label() -> String {
    match crate::platform::distribution_name() {
        Some(distribution) => format!("{} ({})", std::env::consts::OS, distribution),
        None => std::env::consts::OS.to_owned(),
    }
}

fn system_preferences() -> (bool, bool) {
    let Some(settings) = gtk::Settings::default() else {
        return (true, false);
    };
    let theme = settings
        .gtk_theme_name()
        .map(|name| name.to_string())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let dark = settings.is_gtk_application_prefer_dark_theme() || theme.contains("dark");
    let reduced = !settings.property::<bool>("gtk-enable-animations");
    (dark, reduced)
}
