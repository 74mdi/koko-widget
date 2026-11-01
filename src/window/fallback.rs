//! Fallback decorations and stacking policy for plain Wayland toplevels.
//!
//! A regular xdg_toplevel cannot request global stacking or a position on
//! Wayland. KOKO WIDGET keeps the full interface working here and documents that
//! the compositor decides placement and may place the surface behind focused
//! windows.

use gtk::prelude::*;

pub fn configure(window: &gtk::ApplicationWindow) {
    window.set_decorated(false);
    window.set_resizable(false);
    window.set_title(Some(crate::APP_NAME));
    window.add_css_class("koko-toplevel-fallback");
}
