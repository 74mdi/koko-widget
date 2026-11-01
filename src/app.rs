//! GTK application entry point and visual style installation.

use std::{cell::RefCell, rc::Rc, sync::Arc, time::Duration};

use anyhow::Result;
use gtk::{glib, prelude::*};
use tracing::{info, warn};

use crate::{
    config::Config,
    ipc::{self, Action},
    platform,
    ui::{UiController, controller::UiOptions},
};

const CSS: &str = r#"
window.koko-window {
    background-color: transparent;
    background-image: none;
}

.koko-root, .koko-canvas {
    background-color: transparent;
    background-image: none;
}

button.koko-control {
    opacity: 0;
    background-color: transparent;
    background-image: none;
    border: none;
    box-shadow: none;
    padding: 0;
    min-width: 0;
    min-height: 0;
    outline: none;
}

button.koko-control:hover,
button.koko-control:active,
button.koko-control:focus {
    background-color: transparent;
    background-image: none;
    border: none;
    box-shadow: none;
    outline: none;
}

popover.koko-menu > contents {
    background-color: rgba(20, 19, 26, 0.985);
    color: #f4f2f8;
    border: 1px solid rgba(255, 255, 255, 0.075);
    border-radius: 12px;
    box-shadow: 0 10px 28px rgba(0, 0, 0, 0.46);
    padding: 0;
}

.koko-menu-content {
    padding: 0;
}

.koko-menu-header {
    padding: 11px 14px 9px 14px;
    border-bottom: 1px solid rgba(255, 255, 255, 0.06);
}

.koko-menu-title {
    color: rgba(255, 255, 255, 0.92);
    font-weight: 600;
    font-size: 11.5px;
    letter-spacing: 0.05em;
}

.koko-menu-subtitle {
    color: rgba(255, 255, 255, 0.44);
    font-size: 10.5px;
}

.koko-menu-group {
    color: rgba(255, 255, 255, 0.34);
    font-size: 9.5px;
    font-weight: 600;
    letter-spacing: 0.07em;
    padding: 9px 14px 3px 14px;
}

button.koko-menu-item {
    background-color: transparent;
    background-image: none;
    border: none;
    box-shadow: none;
    border-radius: 0;
    color: rgba(245, 243, 250, 0.94);
    padding: 6px 14px;
    font-weight: 400;
    font-size: 12px;
    min-height: 22px;
}

button.koko-menu-item:hover {
    background-color: rgba(255, 255, 255, 0.055);
}

button.koko-menu-item:active {
    background-color: rgba(255, 255, 255, 0.09);
}

.koko-menu-on {
    color: #9d8cff;
    font-size: 10px;
    font-weight: 600;
}

.koko-menu-off {
    color: rgba(255, 255, 255, 0.26);
    font-size: 10px;
}

.koko-menu-footer {
    border-top: 1px solid rgba(255, 255, 255, 0.06);
    padding: 4px 0 4px 0;
}

button.koko-menu-danger {
    color: #f0a3a3;
}

.koko-settings {
    background-color: #12111a;
    color: #f5f3f8;
    font-family: "Noto Sans", "Cantarell", sans-serif;
}

.koko-settings-root {
    background-color: #12111a;
}

.koko-sidebar {
    background-color: #0d0c13;
    border-right: 1px solid rgba(255, 255, 255, 0.055);
    padding: 14px 10px;
}

button.koko-nav {
    background-color: transparent;
    background-image: none;
    border: none;
    box-shadow: none;
    border-radius: 8px;
    color: rgba(240, 238, 248, 0.66);
    font-size: 12.5px;
    font-weight: 500;
    padding: 8px 11px;
    min-height: 20px;
}

button.koko-nav:hover {
    background-color: rgba(255, 255, 255, 0.05);
    color: #f2f0f8;
}

button.koko-nav:checked {
    background-color: rgba(255, 255, 255, 0.10);
    color: #ffffff;
    font-weight: 600;
}

.koko-stack {
    background-color: #12111a;
}

.koko-settings-header {
    background-color: #16141d;
    background-image: none;
    border-bottom: 1px solid rgba(255, 255, 255, 0.07);
    box-shadow: none;
    min-height: 46px;
}

.koko-settings-title {
    color: #ffffff;
    font-weight: 700;
    font-size: 14px;
}

.koko-settings-subtitle {
    color: rgba(255, 255, 255, 0.46);
    font-size: 10px;
}

.koko-header-close {
    background-color: transparent;
    background-image: none;
    border: none;
    box-shadow: none;
    color: rgba(255, 255, 255, 0.66);
    border-radius: 8px;
    min-width: 26px;
    min-height: 26px;
    padding: 1px 8px;
    font-size: 16px;
}

.koko-header-close:hover {
    background-color: rgba(255, 255, 255, 0.10);
    color: #ffffff;
}

.koko-mark {
    color: #ffffff;
    background-image: linear-gradient(145deg, #9a8cff, #5c4ab8);
    background-color: #7c68e8;
    border-radius: 9px;
    padding: 3px 9px;
    font-weight: 800;
    font-size: 14px;
}

.koko-settings-page {
    background-color: transparent;
}

.koko-settings scrolledwindow,
.koko-settings viewport {
    background-color: transparent;
}

.koko-section-title {
    color: rgba(255, 255, 255, 0.44);
    font-weight: 600;
    font-size: 10.5px;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    margin: 6px 0 0 2px;
}

.koko-setting-row {
    background-color: rgba(255, 255, 255, 0.035);
    border: none;
    border-radius: 10px;
    padding: 9px 14px;
    min-height: 40px;
}

.koko-setting-title {
    color: #f5f3f8;
    font-weight: 500;
    font-size: 12.5px;
}

.koko-setting-subtitle {
    color: rgba(255, 255, 255, 0.48);
    font-size: 10px;
}

.koko-value {
    color: rgba(255, 255, 255, 0.55);
    font-size: 10px;
}

.koko-settings footer,
.koko-settings-footer {
    background-color: #0d0c13;
    border-top: 1px solid rgba(255, 255, 255, 0.055);
    padding: 10px 20px;
}

.koko-config-path {
    color: rgba(255, 255, 255, 0.30);
    font-size: 9px;
}

.koko-close {
    background-color: rgba(255, 255, 255, 0.09);
    background-image: none;
    color: #ffffff;
    border: 1px solid rgba(255, 255, 255, 0.10);
    border-radius: 9px;
    padding: 6px 18px;
    box-shadow: none;
}

.koko-close:hover {
    background-color: rgba(255, 255, 255, 0.14);
}

.koko-secondary {
    background-color: rgba(255, 255, 255, 0.07);
    background-image: none;
    color: #e8e4ff;
    border: 1px solid rgba(255, 255, 255, 0.10);
    border-radius: 8px;
    padding: 6px 13px;
    box-shadow: none;
}

.koko-secondary:hover {
    background-color: rgba(139, 124, 246, 0.22);
}

.koko-settings dropdown > button,
.koko-settings spinbutton,
.koko-settings entry {
    background-color: rgba(255, 255, 255, 0.06);
    background-image: none;
    color: #f5f3f8;
    border: 1px solid rgba(255, 255, 255, 0.08);
    border-radius: 8px;
    box-shadow: none;
}

.koko-settings dropdown > button:hover,
.koko-settings spinbutton:hover,
.koko-settings entry:hover {
    background-color: rgba(255, 255, 255, 0.09);
}

.koko-settings scale trough {
    background-color: rgba(255, 255, 255, 0.10);
    border-radius: 6px;
    min-height: 4px;
}

.koko-settings scale highlight {
    background-color: #8b7cf6;
    border-radius: 6px;
}

.koko-settings scale slider {
    background-color: #ffffff;
    border: none;
    box-shadow: 0 1px 5px rgba(0, 0, 0, 0.35);
    min-width: 13px;
    min-height: 13px;
}

.koko-widget-name {
    color: #e9e6f4;
    font-size: 12px;
}

button.koko-order-button {
    min-width: 26px;
    min-height: 22px;
    padding: 0 6px;
    font-size: 13px;
}

.koko-widget-toggle {
    color: #e9e6f4;
    font-size: 12px;
    padding: 2px 0;
}

.koko-widget-toggle check {
    background-color: rgba(255, 255, 255, 0.09);
    background-image: none;
    border: 1px solid rgba(255, 255, 255, 0.14);
    border-radius: 5px;
    min-width: 16px;
    min-height: 16px;
}

.koko-widget-toggle check:checked {
    background-color: #7c68e8;
    border-color: #7c68e8;
    color: #ffffff;
}

.koko-settings switch:checked {
    background-color: #7c68e8;
}

.koko-settings switch:checked slider {
    background-color: #ffffff;
}
"#;

pub fn run(cli: crate::cli::Cli) -> Result<()> {
    let app = gtk::Application::builder()
        .application_id(crate::APP_ID)
        .build();
    let controller_slot: Rc<RefCell<Option<Rc<UiController>>>> = Rc::new(RefCell::new(None));
    let initial_action = cli.action();
    let start_hidden = cli.start_hidden;

    app.connect_startup(move |_| {
        install_css();
        info!(version = crate::VERSION, "starting KOKO WIDGET");
    });

    {
        let slot = Rc::clone(&controller_slot);
        app.connect_activate(move |app| {
            // Hiding the final window (for example while settings is open) must
            // not implicitly terminate the widget. The guard is intentionally
            // leaked for the lifetime of the process.
            let _hold = app.hold();
            if let Some(existing) = slot.borrow().as_ref() {
                existing.handle_action(Action::Show);
                return;
            }
            let outcome = Config::load();
            for warning in &outcome.warnings {
                warn!("{warning}");
            }
            let config = outcome.config.clone();
            if !outcome.path.exists()
                && let Err(error) = config.save()
            {
                warn!(%error, "could not create the initial configuration file");
            }
            let capabilities = platform::detect();
            info!(
                backend = ?capabilities.backend,
                layer_shell = capabilities.layer_shell,
                compositor = ?capabilities.compositor_hint,
                "desktop capabilities detected"
            );
            match UiController::build(app, config, UiOptions { start_hidden }) {
                Ok(controller) => {
                    if let Err(error) = start_control_service(&controller) {
                        warn!(%error, "single-instance service could not start");
                        if ipc::try_send_action(Action::Show) {
                            warn!("another primary owns the control name; exiting this duplicate");
                            app.quit();
                            return;
                        }
                    }
                    *slot.borrow_mut() = Some(Rc::clone(&controller));
                    match initial_action {
                        Some(action) => apply_initial_action(&controller, action),
                        // A plain launch from the app launcher should present the
                        // menu rather than silently showing a bare pill.
                        None if !start_hidden => controller.open_main_menu(),
                        None => {}
                    }
                }
                Err(error) => {
                    eprintln!("KOKO WIDGET: could not create the overlay: {error:#}");
                    warn!(%error, "overlay construction failed");
                    app.quit();
                }
            }
        });
    }

    {
        let slot = Rc::clone(&controller_slot);
        app.connect_shutdown(move |_| {
            if let Some(controller) = slot.borrow().as_ref() {
                controller.flush_config();
            }
        });
    }

    let status = app.run_with_args(&["koko-widget"]);
    if status != glib::ExitCode::SUCCESS {
        anyhow::bail!("GTK application exited with status {:?}", status);
    }
    Ok(())
}

fn install_css() {
    let Some(display) = gtk::gdk::Display::default() else {
        eprintln!("KOKO WIDGET: no GDK display is available");
        return;
    };
    let provider = gtk::CssProvider::new();
    provider.load_from_string(CSS);
    gtk::style_context_add_provider_for_display(
        &display,
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}

fn start_control_service(controller: &Rc<UiController>) -> Result<()> {
    let (action_tx, action_rx) = async_channel::unbounded::<Action>();
    ipc::spawn_server(Arc::new(move |action| {
        let _ = action_tx.try_send(action);
    }))?;
    let weak = Rc::downgrade(controller);
    glib::MainContext::default().spawn_local(async move {
        while let Ok(action) = action_rx.recv().await {
            let Some(controller) = weak.upgrade() else {
                break;
            };
            controller.handle_action(action);
        }
    });
    Ok(())
}

fn apply_initial_action(controller: &Rc<UiController>, action: Action) {
    match action {
        Action::Next | Action::Previous | Action::PlayPause => {
            // Starting the process and sending a command at the same instant can
            // race MPRIS discovery. Give the worker one short grace period; this
            // only happens once, and only when the app was not already running.
            let weak = Rc::downgrade(controller);
            glib::timeout_add_local_once(Duration::from_millis(800), move || {
                if let Some(controller) = weak.upgrade() {
                    controller.handle_action(action);
                }
            });
        }
        Action::Hide | Action::Collapse => {
            // These are no-ops if there was no primary instance to control.
        }
        other => controller.handle_action(other),
    }
}
