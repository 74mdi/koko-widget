//! X11 overlay backend.
//!
//! GTK provides the render surface and ARGB visual. x11rb is used for the EWMH
//! properties that turn the window into a non-focus-stealing desktop overlay
//! and for monitor-aware positioning.

use std::{ffi::c_void, os::raw::c_ulong};

use anyhow::{Context, Result, bail};
use gtk::prelude::*;
use tracing::{debug, warn};
use x11rb::{
    connection::Connection,
    protocol::xproto::{
        AtomEnum, ClientMessageEvent, ConnectionExt as XprotoConnectionExt, EventMask, PropMode,
    },
    rust_connection::RustConnection,
    wrapper::ConnectionExt as WrapperConnectionExt,
};

use crate::{
    window::{DragPlacement, Placement, select_monitor},
};

unsafe extern "C" {
    fn gdk_x11_surface_get_xid(surface: *mut c_void) -> c_ulong;
}

const ATOM_CARDINAL: AtomEnum = AtomEnum::CARDINAL;
const X11_ATOMS: &[&str] = &[
    "_NET_WM_WINDOW_TYPE",
    "_NET_WM_WINDOW_TYPE_DOCK",
    "_NET_WM_STATE",
    "_NET_WM_STATE_ABOVE",
    "_NET_WM_STATE_SKIP_TASKBAR",
    "_NET_WM_STATE_SKIP_PAGER",
    "_NET_WM_STATE_STICKY",
    "_NET_WM_DESKTOP",
    "_NET_WM_USER_TIME",
    "_NET_WM_BYPASS_COMPOSITOR",
    "_NET_MOVERESIZE_WINDOW",
    "_NET_SUPPORTING_WM_CHECK",
];

pub struct X11Backend {
    connection: RustConnection,
    screen: usize,
    atoms: X11Atoms,
    drag: Option<X11Drag>,
}

#[derive(Debug, Clone)]
struct X11Drag {
    window_id: u32,
    x: i32,
    y: i32,
    canvas_width: i32,
    canvas_height: i32,
    monitor_x: i32,
    monitor_y: i32,
    monitor_width: i32,
    monitor_height: i32,
    scale: i32,
    display: Option<String>,
}

#[derive(Debug, Clone, Copy)]
struct X11Atoms {
    window_type: u32,
    window_type_dock: u32,
    state: u32,
    state_above: u32,
    state_skip_taskbar: u32,
    state_skip_pager: u32,
    state_sticky: u32,
    desktop: u32,
    user_time: u32,
    bypass_compositor: u32,
    move_resize: u32,
    supporting_wm_check: u32,
}

impl X11Backend {
    pub fn new(_placement: &Placement) -> Result<Self> {
        let (connection, screen) =
            x11rb::connect(None).context("could not connect to the X server")?;
        let atoms = intern_atoms(&connection)?;
        Ok(Self {
            connection,
            screen,
            atoms,
            drag: None,
        })
    }

    /// Attach EWMH properties to the native X11 window. Safe to call again after
    /// a settings change or monitor event.
    pub fn attach(&mut self, window: &gtk::ApplicationWindow) -> Result<()> {
        let xid = xid_for(window)?;
        let atoms = self.atoms;

        self.connection
            .change_property32(
                PropMode::REPLACE,
                xid,
                atoms.window_type,
                AtomEnum::ATOM,
                &[atoms.window_type_dock],
            )
            .context("could not set _NET_WM_WINDOW_TYPE")?;

        self.connection
            .change_property32(
                PropMode::REPLACE,
                xid,
                atoms.state,
                AtomEnum::ATOM,
                &[
                    atoms.state_above,
                    atoms.state_skip_taskbar,
                    atoms.state_skip_pager,
                    atoms.state_sticky,
                ],
            )
            .context("could not set _NET_WM_STATE")?;

        self.connection
            .change_property32(
                PropMode::REPLACE,
                xid,
                atoms.desktop,
                ATOM_CARDINAL,
                &[0xFFFF_FFFF],
            )
            .context("could not pin the window across desktops")?;

        // A zero user time tells most EWMH window managers that this window is
        // not the result of direct user activation, so it should not steal focus.
        self.connection
            .change_property32(PropMode::REPLACE, xid, atoms.user_time, ATOM_CARDINAL, &[0])
            .context("could not set _NET_WM_USER_TIME")?;

        // 2 means "never bypass the compositor"; transparency needs the
        // compositor to remain in the path.
        self.connection
            .change_property32(
                PropMode::REPLACE,
                xid,
                atoms.bypass_compositor,
                ATOM_CARDINAL,
                &[2],
            )
            .context("could not request compositor unredirect protection")?;

        debug!(xid, screen = self.screen, "X11 overlay properties set");
        self.connection
            .flush()
            .context("could not flush X11 requests")?;
        Ok(())
    }

    pub fn reposition(
        &mut self,
        window: &gtk::ApplicationWindow,
        placement: &Placement,
    ) -> Result<()> {
        let xid = xid_for(window)?;
        let monitor = select_monitor(placement.display.as_deref()).or_else(first_monitor);
        let (mx, my, scale) = if let Some(monitor) = monitor {
            let geometry = monitor.geometry();
            (
                f64::from(geometry.x()),
                f64::from(geometry.y()),
                f64::from(monitor.scale_factor().max(1)),
            )
        } else {
            (0.0, 0.0, 1.0)
        };

        // Placement coordinates are monitor-relative logical pixels; X11 wants
        // root coordinates in physical pixels.
        let x = mx + f64::from(placement.x) * scale;
        let y = my + f64::from(placement.y) * scale;

        self.move_window(xid, x.round() as i32, y.round() as i32)?;
        debug!(xid, x, y, "X11 overlay positioned");
        Ok(())
    }

    pub fn screen(&self) -> usize {
        self.screen
    }

    pub fn begin_drag(&mut self, window: &gtk::ApplicationWindow, placement: &Placement) -> bool {
        if self.drag.is_some() {
            return false;
        }
        match self.begin_drag_inner(window, placement) {
            Ok(()) => true,
            Err(error) => {
                warn!(%error, "could not start X11 drag");
                false
            }
        }
    }

    fn begin_drag_inner(
        &mut self,
        window: &gtk::ApplicationWindow,
        placement: &Placement,
    ) -> Result<()> {
        let xid = xid_for(window)?;
        let root = self.connection.setup().roots[self.screen].root;
        let reply = self
            .connection
            .translate_coordinates(xid, root, 0, 0)
            .context("could not query X11 window position")?
            .reply()
            .context("X11 position query failed")?;
        let surface = window
            .surface()
            .context("the GTK window has not been realized")?;
        let monitor = gtk::gdk::Display::default()
            .and_then(|display| display.monitor_at_surface(&surface))
            .or_else(first_monitor)
            .context("no X11 monitor available")?;
        let geometry = monitor.geometry();
        let scale = monitor.scale_factor().max(1);
        let monitor_x = geometry.x() * scale;
        let monitor_y = geometry.y() * scale;
        let monitor_width = geometry.width() * scale;
        let monitor_height = geometry.height() * scale;
        let canvas_width = placement.canvas_width * f64::from(scale);
        let canvas_height = placement.canvas_height * f64::from(scale);
        let max_x = (monitor_x + monitor_width - canvas_width.round() as i32).max(monitor_x);
        let max_y = (monitor_y + monitor_height - canvas_height.round() as i32).max(monitor_y);
        let x = (i32::from(reply.dst_x)).clamp(monitor_x, max_x);
        let y = (i32::from(reply.dst_y)).clamp(monitor_y, max_y);
        self.drag = Some(X11Drag {
            window_id: xid,
            x,
            y,
            canvas_width: canvas_width.round() as i32,
            canvas_height: canvas_height.round() as i32,
            monitor_x,
            monitor_y,
            monitor_width,
            monitor_height,
            scale,
            display: monitor.connector().map(|connector| connector.to_string()),
        });
        debug!(xid, x, y, "started X11 drag");
        Ok(())
    }

    pub fn drag_by(&mut self, dx: f64, dy: f64) -> bool {
        let (xid, x, y) = {
            let Some(drag) = self.drag.as_mut() else {
                return false;
            };
            let scale = f64::from(drag.scale.max(1));
            let max_x =
                (drag.monitor_x + drag.monitor_width - drag.canvas_width).max(drag.monitor_x);
            let max_y =
                (drag.monitor_y + drag.monitor_height - drag.canvas_height).max(drag.monitor_y);
            drag.x = (drag.x + (dx * scale).round() as i32).clamp(drag.monitor_x, max_x);
            drag.y = (drag.y + (dy * scale).round() as i32).clamp(drag.monitor_y, max_y);
            (drag.window_id, drag.x, drag.y)
        };
        match self.move_window(xid, x, y) {
            Ok(()) => true,
            Err(error) => {
                warn!(%error, "could not move X11 overlay");
                false
            }
        }
    }

    pub fn end_drag(&mut self) -> Option<DragPlacement> {
        let drag = self.drag.take()?;
        let scale = f64::from(drag.scale.max(1));
        Some(DragPlacement {
            display: drag.display,
            x: ((drag.x - drag.monitor_x) as f64 / scale).round() as i32,
            y: ((drag.y - drag.monitor_y) as f64 / scale).round() as i32,
        })
    }

    pub fn cancel_drag(&mut self) {
        self.drag = None;
    }

    fn move_window(&self, xid: u32, x: i32, y: i32) -> Result<()> {
        let root = self.connection.setup().roots[self.screen].root;
        if self.has_window_manager()? {
            let flags: u32 = 0x70A; // static gravity, source=application, x+y present
            let event = ClientMessageEvent::new(
                32,
                xid,
                self.atoms.move_resize,
                [flags, x as u32, y as u32, 0, 0],
            );
            self.connection
                .send_event(
                    false,
                    root,
                    EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
                    event,
                )
                .context("could not send _NET_MOVERESIZE_WINDOW")?;
        } else {
            self.connection
                .configure_window(
                    xid,
                    &x11rb::protocol::xproto::ConfigureWindowAux::new().x(x).y(y),
                )
                .context("could not position the X11 overlay")?;
        }
        self.connection.flush().ok();
        Ok(())
    }

    fn has_window_manager(&self) -> Result<bool> {
        let root = self.connection.setup().roots[self.screen].root;
        let reply = self
            .connection
            .get_property(
                false,
                root,
                self.atoms.supporting_wm_check,
                AtomEnum::WINDOW,
                0,
                1,
            )
            .context("could not query _NET_SUPPORTING_WM_CHECK")?
            .reply()
            .context("could not read _NET_SUPPORTING_WM_CHECK")?;
        Ok(reply.value_len > 0)
    }
}

fn first_monitor() -> Option<gtk::gdk::Monitor> {
    let display = gtk::gdk::Display::default()?;
    display
        .monitors()
        .item(0)
        .and_downcast::<gtk::gdk::Monitor>()
}

fn xid_for(window: &gtk::ApplicationWindow) -> Result<u32> {
    let surface = window
        .surface()
        .context("the GTK window has not been realized")?;
    if !surface.type_().name().contains("X11") {
        bail!("the GTK surface is not an X11 surface");
    }
    use glib::translate::ToGlibPtr;
    let raw: *mut gtk::gdk::ffi::GdkSurface = surface.to_glib_none().0;
    let raw = raw as *mut c_void;
    let xid = unsafe { gdk_x11_surface_get_xid(raw) };
    if xid == 0 {
        bail!("GDK returned an invalid X11 window id");
    }
    Ok(xid as u32)
}

fn intern_atoms(connection: &RustConnection) -> Result<X11Atoms> {
    let mut values = Vec::with_capacity(X11_ATOMS.len());
    for name in X11_ATOMS {
        values.push(
            connection
                .intern_atom(false, name.as_bytes())
                .with_context(|| format!("could not intern {name}"))?
                .reply()
                .with_context(|| format!("could not receive atom {name}"))?
                .atom,
        );
    }
    Ok(X11Atoms {
        window_type: values[0],
        window_type_dock: values[1],
        state: values[2],
        state_above: values[3],
        state_skip_taskbar: values[4],
        state_skip_pager: values[5],
        state_sticky: values[6],
        desktop: values[7],
        user_time: values[8],
        bypass_compositor: values[9],
        move_resize: values[10],
        supporting_wm_check: values[11],
    })
}
