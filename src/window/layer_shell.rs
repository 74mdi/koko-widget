//! Wayland layer-shell backend using `zwlr_layer_shell_v1` through
//! gtk4-layer-shell.
//!
//! Niri, Sway, Hyprland, KWin (where exposed) and other layer-shell compositors
//! use the same path. Capabilities are detected at runtime instead of checking
//! compositor names. The layer namespace is deliberately stable so compositor
//! blur rules can target `koko-widget`.

use anyhow::{Context, Result};
use gtk::prelude::*;
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use tracing::debug;

use crate::{
    config::Layer as ConfigLayer,
    window::{DragPlacement, Placement, select_monitor, wayland},
};

/// State for a free-position drag on a layer surface.
#[derive(Debug, Clone)]
pub struct LayerDrag {
    monitor_width: i32,
    monitor_height: i32,
    canvas_width: f64,
    canvas_height: f64,
    x: i32,
    y: i32,
    display: Option<String>,
}

/// Turn the settings toplevel into a centred overlay layer surface. This keeps
/// it usable above fullscreen windows on layer-shell compositors.
pub fn init_settings(window: &gtk::Window) -> Result<()> {
    if !wayland::is_layer_shell_available() {
        anyhow::bail!("compositor does not support zwlr_layer_shell_v1");
    }
    window.init_layer_shell();
    window.set_namespace(Some("koko-settings"));
    window.set_keyboard_mode(KeyboardMode::OnDemand);
    window.set_exclusive_zone(0);
    window.set_layer(Layer::Overlay);
    for edge in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
        window.set_anchor(edge, false);
        window.set_margin(edge, 0);
    }
    Ok(())
}

pub fn new(window: &gtk::ApplicationWindow, placement: &Placement) -> Result<()> {
    if !wayland::is_layer_shell_available() {
        anyhow::bail!("compositor does not support zwlr_layer_shell_v1");
    }
    window.init_layer_shell();
    window.set_namespace(Some(wayland::namespace()));
    window.set_keyboard_mode(KeyboardMode::OnDemand);
    window.set_exclusive_zone(0);
    apply(window, placement).context("could not apply initial layer-shell placement")?;
    debug!(
        namespace = wayland::namespace(),
        layer = config_layer_label(placement.layer),
        "layer-shell surface initialized"
    );
    Ok(())
}

pub fn apply(window: &gtk::ApplicationWindow, placement: &Placement) -> Result<()> {
    if !window.is_layer_window() {
        anyhow::bail!("window is not a layer-shell surface");
    }
    window.set_layer(match placement.layer {
        ConfigLayer::Top => Layer::Top,
        ConfigLayer::Overlay => Layer::Overlay,
    });
    window.set_keyboard_mode(KeyboardMode::OnDemand);
    window.set_exclusive_zone(0);

    for edge in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
        window.set_anchor(edge, false);
        window.set_margin(edge, 0);
    }

    // Anchor top-left and use margins as an absolute monitor-relative position.
    // This is what lets every widget window sit wherever it likes.
    window.set_anchor(Edge::Top, true);
    window.set_anchor(Edge::Left, true);
    window.set_margin(Edge::Left, placement.x.max(0));
    window.set_margin(Edge::Top, placement.y.max(0));

    let monitor = select_monitor(placement.display.as_deref());
    if placement.display.is_some() && monitor.is_none() {
        tracing::warn!(
            selector = ?placement.display,
            "configured display was not found; letting the compositor choose"
        );
    }
    window.set_monitor(monitor.as_ref());
    Ok(())
}

fn config_layer_label(layer: ConfigLayer) -> &'static str {
    match layer {
        ConfigLayer::Top => "top",
        ConfigLayer::Overlay => "overlay",
    }
}

/// Enter a free drag. The surface already uses absolute margins, so the drag
/// simply tracks the starting point and clamps to the monitor.
pub fn begin_drag(window: &gtk::ApplicationWindow, placement: &Placement) -> Option<LayerDrag> {
    if !window.is_layer_window() {
        return None;
    }
    let display = gtk::gdk::Display::default()?;
    let monitor = window
        .surface()
        .and_then(|surface| display.monitor_at_surface(&surface))
        .or_else(|| select_monitor(placement.display.as_deref()))?;
    let geometry = monitor.geometry();
    let max_x = (geometry.width() - placement.canvas_width as i32).max(0);
    let max_y = (geometry.height() - placement.canvas_height as i32).max(0);
    let x = placement.x.clamp(0, max_x);
    let y = placement.y.clamp(0, max_y);

    window.set_margin(Edge::Left, x);
    window.set_margin(Edge::Top, y);

    debug!(x, y, "started layer-shell drag");
    Some(LayerDrag {
        monitor_width: geometry.width(),
        monitor_height: geometry.height(),
        canvas_width: placement.canvas_width,
        canvas_height: placement.canvas_height,
        x,
        y,
        display: monitor.connector().map(|connector| connector.to_string()),
    })
}

/// Move the layer surface by a logical-pixel delta.
pub fn drag_by(
    window: &gtk::ApplicationWindow,
    drag: &mut LayerDrag,
    dx: f64,
    dy: f64,
) -> Result<()> {
    if !window.is_layer_window() {
        anyhow::bail!("window is no longer a layer-shell surface");
    }
    let max_x = (drag.monitor_width - drag.canvas_width as i32).max(0);
    let max_y = (drag.monitor_height - drag.canvas_height as i32).max(0);
    drag.x = ((drag.x as f64 + dx).round() as i32).clamp(0, max_x);
    drag.y = ((drag.y as f64 + dy).round() as i32).clamp(0, max_y);
    window.set_margin(Edge::Left, drag.x);
    window.set_margin(Edge::Top, drag.y);
    Ok(())
}

pub fn finish_drag(drag: LayerDrag) -> DragPlacement {
    DragPlacement {
        display: drag.display,
        x: drag.x,
        y: drag.y,
    }
}
