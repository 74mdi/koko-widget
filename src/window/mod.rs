//! Native window integration for Wayland layer-shell, plain Wayland and X11.
//!
//! Rendering and media remain completely shared. Only surface-role and
//! stacking/repositioning concerns live in these backends.

mod fallback;
#[cfg(feature = "layer-shell")]
mod layer_shell;
mod wayland;
mod x11;

use anyhow::Result;
use gtk::prelude::*;
use tracing::{info, warn};

use crate::{
    config::{Config, Layer as ConfigLayer, PositionMode, WidgetKind, WidgetSpot},
    ui::layout::{self, HORIZONTAL_MARGIN, Rect},
};

pub use self::x11::X11Backend;

pub fn layer_shell_available() -> bool {
    wayland::is_layer_shell_available()
}

pub fn layer_shell_compiled() -> bool {
    wayland::layer_shell_build_enabled()
}

pub fn layer_shell_namespace() -> &'static str {
    wayland::namespace()
}

pub fn niri_liquid_glass_rule() -> &'static str {
    wayland::NIRI_LIQUID_GLASS_RULE
}

/// Configure the settings window as a centred compositor overlay where
/// possible. Returns true when the settings surface is a layer surface.
pub fn setup_settings_window(window: &gtk::Window) -> bool {
    let display = gtk::gdk::Display::default();
    let is_wayland = display
        .as_ref()
        .is_some_and(|display| display.type_().name().contains("Wayland"));
    #[cfg(feature = "layer-shell")]
    if is_wayland && wayland::is_layer_shell_available() {
        match layer_shell::init_settings(window) {
            Ok(()) => {
                info!("using layer-shell settings overlay");
                return true;
            }
            Err(error) => warn!(%error, "could not use layer-shell for settings"),
        }
    }
    let _ = is_wayland;
    false
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendKind {
    LayerShell,
    WaylandToplevel,
    X11,
    Other,
}

impl BackendKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::LayerShell => "Wayland layer-shell",
            Self::WaylandToplevel => "Wayland toplevel (fallback)",
            Self::X11 => "X11",
            Self::Other => "unknown",
        }
    }

    pub fn has_guaranteed_stacking(self) -> bool {
        matches!(self, Self::LayerShell | Self::X11)
    }
}

/// Where a drag ended, expressed relative to the selected monitor.
#[derive(Debug, Clone)]
pub struct DragPlacement {
    pub display: Option<String>,
    pub x: i32,
    pub y: i32,
}

/// Fully resolved placement for one widget window.
///
/// Every widget owns a window, so placement is resolved per widget: either from
/// a saved [`WidgetSpot`] or from the automatic row layout. Both produce
/// monitor-relative coordinates, which keeps the layer-shell and X11 backends
/// trivial.
#[derive(Debug, Clone)]
pub struct Placement {
    pub display: Option<String>,
    /// Canvas left edge, relative to the selected monitor.
    pub x: i32,
    pub y: i32,
    pub layer: ConfigLayer,
    pub canvas_width: f64,
    pub canvas_height: f64,
}

impl Placement {
    /// Resolve the placement for `kind`.
    pub fn resolve(config: &Config, kind: WidgetKind) -> Self {
        let (canvas_width, canvas_height) = layout::canvas_size(kind, &config.widgets);
        let spot = config.widgets.positions.get(kind);
        let display = spot
            .and_then(|spot| spot.display.clone())
            .or_else(|| config.position.display.clone());

        let (x, y) = if let Some(spot) = spot {
            (spot.x, spot.y)
        } else {
            auto_origin(config, kind, canvas_height)
        };

        Self {
            display,
            x,
            y,
            layer: config.position.layer,
            canvas_width,
            canvas_height,
        }
    }

    /// Convert to a saved spot after a drag.
    pub fn to_spot(&self) -> WidgetSpot {
        WidgetSpot {
            display: self.display.clone(),
            x: self.x,
            y: self.y,
        }
    }
}

/// Automatic row layout: every canvas shares a vertical centre, and cards keep
/// the configured gap between them.
fn auto_origin(config: &Config, kind: WidgetKind, canvas_height: f64) -> (i32, i32) {
    let (monitor_w, monitor_h) = monitor_size(config.position.display.as_deref());
    let margin = config.position.margin.max(0);

    let row_width: f64 = config
        .widgets
        .active()
        .iter()
        .map(|entry| layout::card_size(*entry, &config.widgets).0)
        .sum::<f64>()
        + config.widgets.gap.max(0.0) * config.widgets.active().len().saturating_sub(1) as f64;

    let row_left = match config.position.mode {
        PositionMode::Center | PositionMode::TopCenter | PositionMode::BottomCenter => {
            (f64::from(monitor_w) - row_width) * 0.5
        }
        PositionMode::Custom => f64::from(config.position.custom_x.max(0)),
    };

    let centre_y = match config.position.mode {
        PositionMode::Center => f64::from(monitor_h) * 0.5,
        PositionMode::Custom => f64::from(config.position.custom_y.max(0)) + canvas_height * 0.5,
        PositionMode::TopCenter => f64::from(margin) + canvas_height * 0.5,
        PositionMode::BottomCenter => {
            f64::from(monitor_h) - f64::from(margin) - canvas_height * 0.5
        }
    };

    let card_left = row_left + layout::row_offset(kind, &config.widgets);
    let x = card_left - HORIZONTAL_MARGIN;
    let y = centre_y - canvas_height * 0.5;

    (x.round() as i32, y.round() as i32)
}

fn monitor_size(selector: Option<&str>) -> (i32, i32) {
    select_monitor(selector)
        .or_else(|| {
            gtk::gdk::Display::default()
                .and_then(|display| display.monitors().item(0))
                .and_downcast::<gtk::gdk::Monitor>()
        })
        .map(|monitor| {
            let geometry = monitor.geometry();
            (geometry.width(), geometry.height())
        })
        .unwrap_or((1920, 1080))
}

pub struct WindowBackend {
    pub kind: BackendKind,
    pub input_shapes: bool,
    x11: Option<X11Backend>,
    x11_attached: bool,
    #[cfg(feature = "layer-shell")]
    layer_drag: Option<layer_shell::LayerDrag>,
}

impl WindowBackend {
    /// Configure the window before its first present. Layer-shell is
    /// initialised here, before the surface is realized.
    pub fn setup(window: &gtk::ApplicationWindow, placement: &Placement) -> Result<Self> {
        prepare_common_window(window);
        let display = gtk::gdk::Display::default();
        let type_name = display
            .as_ref()
            .map(|display| display.type_().name().to_string())
            .unwrap_or_default();
        let is_wayland = type_name.contains("Wayland");
        let is_x11 = type_name.contains("X11");
        let input_shapes = display
            .as_ref()
            .map(|display| display.supports_input_shapes())
            .unwrap_or(false);

        let mut backend = Self {
            kind: BackendKind::Other,
            input_shapes,
            x11: None,
            x11_attached: false,
            #[cfg(feature = "layer-shell")]
            layer_drag: None,
        };

        if is_wayland {
            #[cfg(feature = "layer-shell")]
            {
                if wayland::is_layer_shell_available() {
                    match layer_shell::new(window, placement) {
                        Ok(()) => {
                            backend.kind = BackendKind::LayerShell;
                            info!("using Wayland layer-shell backend (namespace koko-widget)");
                            return Ok(backend);
                        }
                        Err(error) => {
                            warn!(%error, "layer-shell initialisation failed; using a toplevel");
                        }
                    }
                }
            }
            backend.kind = BackendKind::WaylandToplevel;
            fallback::configure(window);
            info!("Wayland layer-shell unavailable; using a plain toplevel");
        } else if is_x11 {
            backend.kind = BackendKind::X11;
            backend.x11 = Some(X11Backend::new(placement)?);
            info!("using X11 overlay backend");
        } else {
            warn!(%type_name, "unknown GDK display backend");
        }

        window.set_default_size(
            placement.canvas_width as i32,
            placement.canvas_height as i32,
        );
        Ok(backend)
    }

    /// Finish backend setup after GTK has realized the native surface but before
    /// presenting the window.
    pub fn after_realize(&mut self, window: &gtk::ApplicationWindow, _placement: &Placement) {
        if self.kind == BackendKind::X11 && !self.x11_attached {
            if let Some(x11) = self.x11.as_mut() {
                match x11.attach(window) {
                    Ok(()) => self.x11_attached = true,
                    Err(error) => warn!(%error, "could not attach to the X11 window"),
                }
            }
        }
    }

    pub fn present(&self, window: &gtk::ApplicationWindow) {
        window.present();
    }

    /// Called after the window has been mapped. X11 window managers commonly
    /// ignore a pre-map ConfigureRequest, so positioning is repeated here.
    pub fn after_map(&mut self, window: &gtk::ApplicationWindow, placement: &Placement) {
        if self.kind == BackendKind::X11 {
            if let Some(x11) = self.x11.as_mut() {
                if !self.x11_attached {
                    if let Err(error) = x11.attach(window) {
                        warn!(%error, "could not attach to the X11 window after mapping");
                        return;
                    }
                    self.x11_attached = true;
                }
                if let Err(error) = x11.reposition(window, placement) {
                    warn!(%error, "could not position the X11 overlay");
                }
            }
        }
    }

    /// Reapply positioning and stacking after a settings or monitor change.
    pub fn reapply(&mut self, window: &gtk::ApplicationWindow, placement: &Placement) {
        match self.kind {
            #[cfg(feature = "layer-shell")]
            BackendKind::LayerShell => {
                if let Err(error) = layer_shell::apply(window, placement) {
                    warn!(%error, "could not reapply layer-shell placement");
                }
            }
            BackendKind::X11 => {
                if let Some(x11) = self.x11.as_mut() {
                    if self.x11_attached {
                        if let Err(error) = x11.attach(window) {
                            warn!(%error, "could not update X11 window properties");
                        }
                        if let Err(error) = x11.reposition(window, placement) {
                            warn!(%error, "could not reposition X11 overlay");
                        }
                    }
                }
            }
            _ => {}
        }
    }

    /// Constrain pointer events to the visible card. Transparent margins then
    /// pass clicks through to the desktop.
    pub fn update_input_region(&self, window: &gtk::ApplicationWindow, card: Rect) {
        if !self.input_shapes {
            return;
        }
        let Some(surface) = window.surface() else {
            return;
        };
        let region = cairo::Region::create_rectangle(&glass_input_rect(card));
        surface.set_input_region(Some(&region));
    }

    pub fn clear_input_region(&self, window: &gtk::ApplicationWindow) {
        if let Some(surface) = window.surface() {
            let region = cairo::Region::create();
            surface.set_input_region(Some(&region));
        }
    }

    /// Whether the current backend can move the surface from pointer input.
    pub fn can_drag(&self) -> bool {
        match self.kind {
            #[cfg(feature = "layer-shell")]
            BackendKind::LayerShell => true,
            BackendKind::X11 => true,
            _ => false,
        }
    }

    /// Begin a pointer drag from the placement this window currently has.
    pub fn begin_drag(&mut self, window: &gtk::ApplicationWindow, placement: &Placement) -> bool {
        match self.kind {
            #[cfg(feature = "layer-shell")]
            BackendKind::LayerShell => {
                let drag = layer_shell::begin_drag(window, placement);
                let started = drag.is_some();
                self.layer_drag = drag;
                started
            }
            BackendKind::X11 => self
                .x11
                .as_mut()
                .is_some_and(|x11| x11.begin_drag(window, placement)),
            _ => false,
        }
    }

    /// Move an active drag by a logical-pixel delta from GTK pointer input.
    pub fn drag_by(&mut self, window: &gtk::ApplicationWindow, dx: f64, dy: f64) -> bool {
        match self.kind {
            #[cfg(feature = "layer-shell")]
            BackendKind::LayerShell => {
                let Some(drag) = self.layer_drag.as_mut() else {
                    return false;
                };
                if let Err(error) = layer_shell::drag_by(window, drag, dx, dy) {
                    warn!(%error, "could not move layer-shell surface");
                    return false;
                }
                true
            }
            BackendKind::X11 => self.x11.as_mut().is_some_and(|x11| x11.drag_by(dx, dy)),
            _ => false,
        }
    }

    /// Finish a drag and return the position that should be persisted.
    pub fn end_drag(&mut self) -> Option<DragPlacement> {
        #[cfg(feature = "layer-shell")]
        if let Some(drag) = self.layer_drag.take() {
            return Some(layer_shell::finish_drag(drag));
        }
        match self.kind {
            BackendKind::X11 => self.x11.as_mut().and_then(|x11| x11.end_drag()),
            _ => None,
        }
    }

    /// Cancel a drag and leave the surface at its current location.
    pub fn cancel_drag(&mut self) {
        #[cfg(feature = "layer-shell")]
        {
            self.layer_drag = None;
        }
        if let Some(x11) = self.x11.as_mut() {
            x11.cancel_drag();
        }
    }
}

fn prepare_common_window(window: &gtk::ApplicationWindow) {
    window.set_decorated(false);
    window.set_resizable(false);
    window.set_title(Some(crate::APP_NAME));
    window.add_css_class("koko-window");
}

fn glass_input_rect(pill: Rect) -> cairo::RectangleInt {
    cairo::RectangleInt::new(
        pill.x.floor() as i32,
        pill.y.floor() as i32,
        pill.width.ceil() as i32,
        pill.height.ceil() as i32,
    )
}

/// Resolve a configured monitor selector against the current display.
pub fn select_monitor(selector: Option<&str>) -> Option<gtk::gdk::Monitor> {
    let display = gtk::gdk::Display::default()?;
    let monitors = display.monitors();
    let count = monitors.n_items();
    if count == 0 {
        return None;
    }
    let Some(selector) = selector.map(str::trim).filter(|value| !value.is_empty()) else {
        return None;
    };
    if selector.eq_ignore_ascii_case("primary") || selector.eq_ignore_ascii_case("first") {
        return monitors.item(0).and_downcast::<gtk::gdk::Monitor>();
    }
    if let Ok(index) = selector.parse::<u32>() {
        let index = index.saturating_sub(1);
        if index < count {
            return monitors.item(index).and_downcast::<gtk::gdk::Monitor>();
        }
        return None;
    }
    for index in 0..count {
        if let Some(monitor) = monitors.item(index).and_downcast::<gtk::gdk::Monitor>() {
            if monitor
                .connector()
                .is_some_and(|connector| connector.eq_ignore_ascii_case(selector))
            {
                return Some(monitor);
            }
            // Some backends expose the model where a connector is unavailable.
            if monitor
                .model()
                .is_some_and(|model| model.eq_ignore_ascii_case(selector))
            {
                return Some(monitor);
            }
        }
    }
    None
}

pub fn position_anchors(mode: PositionMode) -> (bool, bool, bool, bool) {
    // top, bottom, left, right
    match mode {
        PositionMode::Center => (false, false, false, false),
        PositionMode::TopCenter => (true, false, false, false),
        PositionMode::BottomCenter => (false, true, false, false),
        PositionMode::Custom => (true, false, true, false),
    }
}

pub fn config_layer(layer: ConfigLayer) -> &'static str {
    match layer {
        ConfigLayer::Top => "top",
        ConfigLayer::Overlay => "overlay",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn position_modes_have_sensible_anchors() {
        assert_eq!(
            position_anchors(PositionMode::Center),
            (false, false, false, false)
        );
        assert_eq!(
            position_anchors(PositionMode::TopCenter),
            (true, false, false, false)
        );
        assert_eq!(
            position_anchors(PositionMode::BottomCenter),
            (false, true, false, false)
        );
        assert_eq!(
            position_anchors(PositionMode::Custom),
            (true, false, true, false)
        );
    }

    #[test]
    fn input_rect_is_restricted_to_the_pill() {
        let rectangle = glass_input_rect(Rect::new(32.0, 94.0, 460.0, 112.0));
        assert_eq!(rectangle.x(), 32);
        assert_eq!(rectangle.y(), 94);
        assert_eq!(rectangle.width(), 460);
        assert_eq!(rectangle.height(), 112);
    }

    #[test]
    fn backend_stacking_capabilities_are_honest() {
        assert!(BackendKind::LayerShell.has_guaranteed_stacking());
        assert!(BackendKind::X11.has_guaranteed_stacking());
        assert!(!BackendKind::WaylandToplevel.has_guaranteed_stacking());
    }
}
