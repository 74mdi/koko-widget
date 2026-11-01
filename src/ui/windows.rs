//! One native window per widget.
//!
//! Each pill is an independent surface with its own transparent canvas, its own
//! backend placement and its own input region. That is what lets the clock, the
//! date and the note be moved separately from the media card, and what lets a
//! widget float without a background at all.
//!
//! All windows share one controller: media, artwork, config and animation state
//! stay global, while geometry and pointer handling are per window.

use std::{cell::RefCell, rc::Rc};

use anyhow::Result;
use gtk::prelude::*;
use tracing::{debug, info, warn};

use crate::{
    config::{Config, WidgetKind},
    ui::{
        draw::{self, ViewData},
        layout::{self, FrameLayout, Rect},
    },
    window::{DragPlacement, Placement, WindowBackend},
};

/// Interactive controls that only the media window needs as native widgets.
pub struct MediaButtons {
    pub previous: gtk::Button,
    pub play: gtk::Button,
    pub next: gtk::Button,
}

/// One widget's window, canvas and backend.
pub struct WidgetWindow {
    pub kind: WidgetKind,
    pub window: gtk::ApplicationWindow,
    pub canvas: gtk::DrawingArea,
    pub fixed: gtk::Fixed,
    pub backend: RefCell<WindowBackend>,
    pub buttons: Option<MediaButtons>,
    /// Last input region we applied, to avoid redundant surface requests.
    last_input: RefCell<Option<(i32, i32, i32, i32)>>,
    /// Press origin while a click-or-drag gesture is in progress.
    press_origin: RefCell<Option<(f64, f64)>>,
    dragging: RefCell<bool>,
}

impl WidgetWindow {
    /// Build the window for `kind`, sized and placed by its own `Placement`.
    pub fn build(app: &gtk::Application, config: &Config, kind: WidgetKind) -> Result<Self> {
        let placement = Placement::resolve(config, kind);
        let window = gtk::ApplicationWindow::builder()
            .application(app)
            .title(window_title(kind))
            .build();
        window.add_css_class("koko-window");
        window.set_decorated(false);
        window.set_resizable(false);
        window.set_default_size(
            placement.canvas_width as i32,
            placement.canvas_height as i32,
        );

        let fixed = gtk::Fixed::new();
        fixed.add_css_class("koko-root");
        fixed.set_size_request(
            placement.canvas_width as i32,
            placement.canvas_height as i32,
        );

        let canvas = gtk::DrawingArea::new();
        canvas.add_css_class("koko-canvas");
        canvas.set_content_width(placement.canvas_width as i32);
        canvas.set_content_height(placement.canvas_height as i32);
        canvas.set_size_request(
            placement.canvas_width as i32,
            placement.canvas_height as i32,
        );
        canvas.set_hexpand(true);
        canvas.set_vexpand(true);
        canvas.set_accessible_role(gtk::AccessibleRole::Group);
        fixed.put(&canvas, 0.0, 0.0);

        let buttons = if kind == WidgetKind::Media {
            let previous = control_button("Previous track");
            let play = control_button("Play");
            let next = control_button("Next track");
            fixed.put(&previous, 0.0, 0.0);
            fixed.put(&play, 0.0, 0.0);
            fixed.put(&next, 0.0, 0.0);
            previous.set_visible(false);
            Some(MediaButtons {
                previous,
                play,
                next,
            })
        } else {
            None
        };

        window.set_child(Some(&fixed));

        let mut backend = WindowBackend::setup(&window, &placement)?;
        if !window.is_realized() {
            gtk::prelude::WidgetExt::realize(&window);
        }
        backend.after_realize(&window, &placement);

        debug!(
            ?kind,
            x = placement.x,
            y = placement.y,
            width = placement.canvas_width,
            height = placement.canvas_height,
            "widget window created"
        );

        Ok(Self {
            kind,
            window,
            canvas,
            fixed,
            backend: RefCell::new(backend),
            buttons,
            last_input: RefCell::new(None),
            press_origin: RefCell::new(None),
            dragging: RefCell::new(false),
        })
    }

    pub fn placement(&self, config: &Config) -> Placement {
        Placement::resolve(config, self.kind)
    }

    /// Reposition and restack this window from the current configuration.
    pub fn reapply(&self, config: &Config) {
        let placement = self.placement(config);
        self.backend.borrow_mut().reapply(&self.window, &placement);
        *self.last_input.borrow_mut() = None;
    }

    /// Match the transparent canvas to the configured card size.
    pub fn resize_canvas(&self, config: &Config) {
        let (width, height) = layout::canvas_size(self.kind, &config.widgets);
        let width = width as i32;
        let height = height as i32;
        if self.window.default_size() == (width, height) {
            return;
        }
        self.window.set_default_size(width, height);
        self.fixed.set_size_request(width, height);
        self.canvas.set_content_width(width);
        self.canvas.set_content_height(height);
        self.canvas.set_size_request(width, height);
        self.window.queue_resize();
        *self.last_input.borrow_mut() = None;
    }

    /// Redraw this window against the current shared state.
    pub fn queue_draw(&self) {
        self.canvas.queue_draw();
    }

    /// Apply the card's input region, skipping no-op updates.
    pub fn update_input(&self, layout: &FrameLayout) {
        let rect = layout.input_rect();
        let key = (
            rect.x.floor() as i32,
            rect.y.floor() as i32,
            rect.width.ceil() as i32,
            rect.height.ceil() as i32,
        );
        if *self.last_input.borrow() == Some(key) {
            return;
        }
        *self.last_input.borrow_mut() = Some(key);
        self.backend
            .borrow()
            .update_input_region(&self.window, rect);
    }

    pub fn clear_input(&self) {
        *self.last_input.borrow_mut() = None;
        self.backend.borrow().clear_input_region(&self.window);
    }

    pub fn set_cursor(&self, name: &str) {
        self.canvas.set_cursor_from_name(Some(name));
        self.fixed.set_cursor_from_name(Some(name));
    }

    pub fn begin_drag(&self, config: &Config) -> bool {
        let placement = self.placement(config);
        let started = self.backend.borrow_mut().begin_drag(&self.window, &placement);
        if started {
            *self.dragging.borrow_mut() = true;
            self.set_cursor("grabbing");
        }
        started
    }

    pub fn drag_by(&self, dx: f64, dy: f64) -> bool {
        self.backend.borrow_mut().drag_by(&self.window, dx, dy)
    }

    pub fn is_dragging(&self) -> bool {
        *self.dragging.borrow()
    }

    /// Finish a drag and return the persisted spot, if the backend supports it.
    pub fn end_drag(&self) -> Option<DragPlacement> {
        let was_dragging = std::mem::replace(&mut *self.dragging.borrow_mut(), false);
        let placement = self.backend.borrow_mut().end_drag();
        if was_dragging {
            self.set_cursor("grab");
        }
        placement
    }

    pub fn cancel_drag(&self) {
        *self.dragging.borrow_mut() = false;
        self.backend.borrow_mut().cancel_drag();
    }

    pub fn press_origin(&self) -> Option<(f64, f64)> {
        *self.press_origin.borrow()
    }

    pub fn set_press_origin(&self, origin: Option<(f64, f64)>) {
        *self.press_origin.borrow_mut() = origin;
    }

    pub fn show(&self) {
        self.window.present();
    }

    pub fn hide(&self) {
        self.clear_input();
        self.window.set_visible(false);
    }

    pub fn is_visible(&self) -> bool {
        self.window.is_visible()
    }
}

/// Every widget window, in display order.
pub struct WidgetWindows {
    windows: RefCell<Vec<Rc<WidgetWindow>>>,
}

impl WidgetWindows {
    pub fn new() -> Self {
        Self {
            windows: RefCell::new(Vec::new()),
        }
    }

    /// Create the windows needed for the current configuration and drop any
    /// windows whose widget was disabled. Existing windows are kept so their
    /// surfaces, positions and drag state survive a settings change.
    pub fn sync(&self, app: &gtk::Application, config: &Config) -> Result<Vec<WidgetKind>> {
        let active = config.widgets.active();
        let mut added = Vec::new();
        let mut windows = self.windows.borrow_mut();

        // Drop windows for widgets that are no longer enabled. GTK applications
        // keep a reference to every window they own, so hiding is not enough:
        // the surface must be destroyed or it lingers on screen.
        windows.retain(|window| {
            let keep = active.contains(&window.kind);
            if !keep {
                window.hide();
                window.window.destroy();
            }
            keep
        });

        for kind in &active {
            if windows.iter().any(|window| window.kind == *kind) {
                continue;
            }
            match WidgetWindow::build(app, config, *kind) {
                Ok(window) => {
                    windows.push(Rc::new(window));
                    added.push(*kind);
                }
                Err(error) => {
                    warn!(?kind, %error, "could not create a widget window");
                }
            }
        }

        // Keep the vector in display order.
        windows.sort_by_key(|window| {
            active
                .iter()
                .position(|kind| *kind == window.kind)
                .unwrap_or(usize::MAX)
        });

        Ok(added)
    }

    pub fn all(&self) -> Vec<Rc<WidgetWindow>> {
        self.windows.borrow().clone()
    }

    pub fn get(&self, kind: WidgetKind) -> Option<Rc<WidgetWindow>> {
        self.windows
            .borrow()
            .iter()
            .find(|window| window.kind == kind)
            .cloned()
    }

    pub fn media(&self) -> Option<Rc<WidgetWindow>> {
        self.get(WidgetKind::Media)
    }

    pub fn is_empty(&self) -> bool {
        self.windows.borrow().is_empty()
    }

    /// Draw one window. `view_for` produces the shared view data for a kind.
    pub fn draw(&self, kind: WidgetKind, cr: &gtk::cairo::Context, width: i32, height: i32) {
        let _ = (kind, cr, width, height);
    }

    pub fn queue_draw_all(&self) {
        for window in self.windows.borrow().iter() {
            window.queue_draw();
        }
    }

    pub fn hide_all(&self) {
        for window in self.windows.borrow().iter() {
            window.hide();
        }
    }
}

impl Default for WidgetWindows {
    fn default() -> Self {
        Self::new()
    }
}

/// Shared drawing entry point so the controller does not need to know which
/// widget a canvas belongs to.
pub fn draw_window(
    window: &WidgetWindow,
    cr: &gtk::cairo::Context,
    width: i32,
    height: i32,
    view: &ViewData<'_>,
) {
    draw::draw(
        window.kind,
        cr,
        f64::from(width),
        f64::from(height),
        view,
    );
}

fn control_button(label: &str) -> gtk::Button {
    let button = gtk::Button::new();
    button.add_css_class("koko-control");
    button.set_hexpand(false);
    button.set_vexpand(false);
    button.update_property(&[gtk::accessible::Property::Label(label)]);
    button.set_tooltip_text(Some(label));
    button.set_cursor_from_name(Some("pointer"));
    button
}

fn window_title(kind: WidgetKind) -> String {
    match kind {
        WidgetKind::Media => format!("{} — Now playing", crate::APP_NAME),
        WidgetKind::Clock => format!("{} — Clock", crate::APP_NAME),
        WidgetKind::Date => format!("{} — Date", crate::APP_NAME),
        WidgetKind::Text => format!("{} — Text", crate::APP_NAME),
    }
}

/// Intersect helper used by the controller when clamping a saved spot.
pub fn clamp_spot(spot: &mut crate::config::WidgetSpot, max_x: i32, max_y: i32) {
    spot.x = spot.x.clamp(0, max_x.max(0));
    spot.y = spot.y.clamp(0, max_y.max(0));
}

/// Convenience for the controller: the union of every card rectangle.
pub fn input_rects(windows: &[Rc<WidgetWindow>], layouts: &[(WidgetKind, FrameLayout)]) -> Vec<Rect> {
    let _ = windows;
    layouts.iter().map(|(_, layout)| layout.input_rect()).collect()
}

/// Log a summary of the current window set once, at startup.
pub fn log_summary(windows: &[Rc<WidgetWindow>]) {
    let kinds: Vec<&'static str> = windows.iter().map(|window| window.kind.label()).collect();
    info!(count = windows.len(), widgets = ?kinds, "widget windows ready");
}
