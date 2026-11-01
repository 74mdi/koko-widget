//! Native settings window with the same restrained visual language as the
//! overlay.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gtk::{gdk, prelude::*};

use crate::{
    config::{
        Config, FontWeight, HourFormat, Layer, PositionMode, ReducedMotion, TextAlign, Theme,
        WidgetKind,
    },
    media::metadata::PlayerInfo,
    utils::{config_file, format_hex_color, parse_hex_color},
};

pub struct SettingsHandle {
    pub window: gtk::ApplicationWindow,
    update_players: Box<dyn Fn(&[PlayerInfo])>,
}

impl SettingsHandle {
    pub fn present(&self) {
        self.window.present();
    }

    pub fn update_players(&self, players: &[PlayerInfo]) {
        (self.update_players)(players);
    }
}

struct SettingsContext {
    config: Rc<RefCell<Config>>,
    on_change: Rc<dyn Fn(Config)>,
}

impl SettingsContext {
    fn update(&self, mutate: impl FnOnce(&mut Config)) {
        let config = {
            let mut config = self.config.borrow_mut();
            mutate(&mut config);
            config.clone()
        };
        (self.on_change)(config);
    }
}

pub fn open(
    parent: &gtk::ApplicationWindow,
    app: &gtk::Application,
    config: Config,
    players: Vec<PlayerInfo>,
    on_change: Rc<dyn Fn(Config)>,
    on_close: Rc<dyn Fn()>,
) -> SettingsHandle {
    let config_state = Rc::new(RefCell::new(config.clone()));
    let context = Rc::new(SettingsContext {
        config: Rc::clone(&config_state),
        on_change,
    });

    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title("KOKO WIDGET Settings")
        .default_width(760)
        .default_height(780)
        .build();
    let layered = crate::window::setup_settings_window(window.upcast_ref());
    if layered {
        window.set_decorated(false);
    } else {
        if parent.is_visible() {
            window.set_transient_for(Some(parent));
        }
        window.set_modal(false);
    }
    window.add_css_class("koko-settings");

    let header = gtk::HeaderBar::new();
    header.add_css_class("koko-settings-header");
    header.set_show_title_buttons(false);
    let title_box = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let mark = gtk::Label::new(Some("K"));
    mark.add_css_class("koko-mark");
    title_box.append(&mark);
    let titles = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let title = gtk::Label::new(Some("KOKO WIDGET"));
    title.add_css_class("koko-settings-title");
    title.set_xalign(0.0);
    let subtitle = gtk::Label::new(Some("Settings"));
    subtitle.add_css_class("koko-settings-subtitle");
    subtitle.set_xalign(0.0);
    titles.append(&title);
    titles.append(&subtitle);
    title_box.append(&titles);
    header.set_title_widget(Some(&title_box));
    let header_close = gtk::Button::with_label("×");
    header_close.add_css_class("koko-header-close");
    header_close.set_tooltip_text(Some("Close settings"));
    header_close.update_property(&[gtk::accessible::Property::Label("Close settings")]);
    {
        let window_close = window.clone();
        let on_close = Rc::clone(&on_close);
        header_close.connect_clicked(move |_| {
            window_close.destroy();
            on_close();
        });
    }
    header.pack_end(&header_close);
    window.set_titlebar(Some(&header));

    // Sidebar + page stack. A settings window is a navigation surface, not one
    // endless list: each area gets its own page and its own scroll position.
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.add_css_class("koko-settings-root");
    window.set_child(Some(&root));

    let body = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    body.set_vexpand(true);
    root.append(&body);

    let sidebar = gtk::Box::new(gtk::Orientation::Vertical, 2);
    sidebar.add_css_class("koko-sidebar");
    sidebar.set_size_request(184, -1);
    body.append(&sidebar);

    let stack = gtk::Stack::new();
    stack.set_transition_type(gtk::StackTransitionType::Crossfade);
    stack.set_transition_duration(110);
    stack.set_hexpand(true);
    stack.set_vexpand(true);
    stack.add_css_class("koko-stack");
    body.append(&stack);

    let page_for = |title: &str, key: &str| {
        let page = gtk::Box::new(gtk::Orientation::Vertical, 18);
        page.set_margin_top(22);
        page.set_margin_bottom(28);
        page.set_margin_start(26);
        page.set_margin_end(26);
        page.add_css_class("koko-settings-page");
        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .child(&page)
            .build();
        scroller.set_vexpand(true);
        stack.add_titled(&scroller, Some(key), title);
        page
    };
    let appearance_page = page_for("Appearance", "appearance");
    let widgets_page = page_for("Widgets", "widgets");
    let window_page = page_for("Window", "window");
    let players_page = page_for("Players", "players");

    let nav_names = [
        ("Appearance", "appearance"),
        ("Widgets", "widgets"),
        ("Window", "window"),
        ("Players", "players"),
    ];
    let mut nav_buttons: Vec<(String, gtk::ToggleButton)> = Vec::new();
    let first = nav_names[0].1.to_owned();
    for (label, key) in nav_names {
        let button = gtk::ToggleButton::with_label(label);
        button.add_css_class("koko-nav");
        button.set_active(key == first);
        button.update_property(&[gtk::accessible::Property::Label(label)]);
        {
            let stack = stack.clone();
            let key = key.to_owned();
            let buttons = Rc::new(RefCell::new(Vec::<(String, gtk::ToggleButton)>::new()));
            let _ = &buttons;
            button.connect_clicked(move |button| {
                stack.set_visible_child_name(&key);
                button.set_active(true);
            });
        }
        sidebar.append(&button);
        nav_buttons.push((key.to_owned(), button));
    }
    // Keep exactly one navigation item selected.
    for (key, button) in &nav_buttons {
        let stack = stack.clone();
        let key = key.clone();
        let peers: Vec<(String, gtk::ToggleButton)> = nav_buttons
            .iter()
            .map(|(k, b)| (k.clone(), b.clone()))
            .collect();
        button.connect_toggled(move |this| {
            if !this.is_active() {
                return;
            }
            stack.set_visible_child_name(&key);
            for (other_key, other) in &peers {
                if other_key != &key {
                    other.set_active(false);
                }
            }
        });
    }
    stack.set_visible_child_name("appearance");

    // ---- Appearance -----------------------------------------------------
    let appearance = section("Appearance");
    appearance_page.append(&appearance.container);

    let glass_switch = gtk::Switch::new();
    glass_switch.set_active(config.appearance.glass);
    let ctx = Rc::clone(&context);
    glass_switch.connect_active_notify(move |switch| {
        ctx.update(|config| config.appearance.glass = switch.is_active());
    });
    appearance.rows.append(&setting_row(
        "Glass effects",
        "Layered glass, highlights and grain",
        &glass_switch,
        "glass",
    ));

    let (opacity_scale, opacity_value) =
        percentage_scale(config.appearance.opacity, 0.35, 1.0, "Glass opacity");
    {
        let ctx = Rc::clone(&context);
        let value = opacity_value.clone();
        opacity_scale.connect_value_changed(move |scale| {
            let value_now = scale.value();
            value.set_text(&format!("{:.0}%", value_now * 100.0));
            ctx.update(|config| config.appearance.opacity = value_now);
        });
    }
    let opacity_box = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    opacity_box.append(&opacity_scale);
    opacity_box.append(&opacity_value);
    appearance.rows.append(&setting_row(
        "Opacity",
        "Lower values reveal more of the compositor background",
        &opacity_box,
        "opacity",
    ));

    let blur_switch = gtk::Switch::new();
    blur_switch.set_active(config.appearance.blur);
    let ctx = Rc::clone(&context);
    blur_switch.connect_active_notify(move |switch| {
        ctx.update(|config| config.appearance.blur = switch.is_active());
    });
    appearance.rows.append(&setting_row(
        "Prefer compositor blur",
        "Use a compositor rule for the koko-widget layer namespace",
        &blur_switch,
        "blur",
    ));

    let copy_glass = gtk::Button::with_label("Copy Niri rule");
    copy_glass.add_css_class("koko-secondary");
    copy_glass.set_tooltip_text(Some(
        "Copy a layer rule for niri's native liquid-glass effect",
    ));
    set_accessible_labels(
        &copy_glass,
        "Copy niri liquid-glass rule",
        "Copies a compositor layer rule to the clipboard",
    );
    {
        let copy_glass_btn = copy_glass.clone();
        copy_glass.connect_clicked(move |_| {
            if let Some(display) = gtk::gdk::Display::default() {
                display
                    .clipboard()
                    .set_text(crate::window::niri_liquid_glass_rule());
                copy_glass_btn.set_label("Copied");
            }
        });
    }
    appearance.rows.append(&setting_row(
        "Liquid glass (niri)",
        "Refraction requires the compositor; copy the ready layer rule",
        &copy_glass,
        "niri liquid glass rule",
    ));

    let theme_names = gtk::StringList::new(&["Dark", "Light", "Follow system"]);
    let theme_dropdown = gtk::DropDown::new(Some(theme_names), None::<&gtk::Expression>);
    theme_dropdown.set_selected(theme_index(config.appearance.theme));
    theme_dropdown.set_size_request(180, -1);
    set_accessible_labels(&theme_dropdown, "Theme", "Glass surface colour scheme");
    let ctx = Rc::clone(&context);
    theme_dropdown.connect_selected_notify(move |dropdown| {
        let theme = theme_from_index(dropdown.selected());
        ctx.update(|config| config.appearance.theme = theme);
    });
    appearance.rows.append(&setting_row(
        "Theme",
        "Choose a dark or light glass surface",
        &theme_dropdown,
        "theme",
    ));

    let accent = parse_hex_color(&config.appearance.accent).unwrap_or([0.545, 0.486, 0.965, 1.0]);
    let dialog = gtk::ColorDialog::builder()
        .with_alpha(false)
        .title("Accent colour")
        .build();
    let accent_button = gtk::ColorDialogButton::new(Some(dialog));
    accent_button.set_rgba(&gdk::RGBA::new(
        accent[0] as f32,
        accent[1] as f32,
        accent[2] as f32,
        1.0,
    ));
    set_accessible_labels(&accent_button, "Accent colour", "Tint used by the glass");
    let ctx = Rc::clone(&context);
    accent_button.connect_rgba_notify(move |button| {
        let rgba = button.rgba();
        let colour = format_hex_color([
            f64::from(rgba.red()),
            f64::from(rgba.green()),
            f64::from(rgba.blue()),
        ]);
        ctx.update(|config| config.appearance.accent = colour);
    });
    appearance.rows.append(&setting_row(
        "Accent",
        "Used when artwork colour adaptation is off",
        &accent_button,
        "accent",
    ));

    let adaptive_switch = gtk::Switch::new();
    adaptive_switch.set_active(config.appearance.adaptive_accent);
    let ctx = Rc::clone(&context);
    adaptive_switch.connect_active_notify(move |switch| {
        ctx.update(|config| config.appearance.adaptive_accent = switch.is_active());
    });
    appearance.rows.append(&setting_row(
        "Artwork colour adaptation",
        "Let the current cover add a restrained tint",
        &adaptive_switch,
        "adaptive accent",
    ));

    let (radius_scale, radius_value) = value_scale(
        config.appearance.corner_radius,
        18.0,
        48.0,
        "Corner radius",
        " px",
    );
    {
        let ctx = Rc::clone(&context);
        let value = radius_value.clone();
        radius_scale.connect_value_changed(move |scale| {
            let value_now = scale.value();
            value.set_text(&format!("{:.0} px", value_now));
            ctx.update(|config| config.appearance.corner_radius = value_now);
        });
    }
    let radius_box = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    radius_box.append(&radius_scale);
    radius_box.append(&radius_value);
    appearance.rows.append(&setting_row(
        "Corner radius",
        "Shared by the compact and expanded forms",
        &radius_box,
        "corner radius",
    ));

    // ---- Position -------------------------------------------------------
    let position = section("Position");
    window_page.append(&position.container);

    let position_names = gtk::StringList::new(&[
        PositionMode::Center.label(),
        PositionMode::TopCenter.label(),
        PositionMode::BottomCenter.label(),
        PositionMode::Custom.label(),
    ]);
    let position_dropdown = gtk::DropDown::new(Some(position_names), None::<&gtk::Expression>);
    position_dropdown.set_selected(
        PositionMode::ALL
            .iter()
            .position(|mode| *mode == config.position.mode)
            .unwrap_or(0) as u32,
    );
    position_dropdown.set_size_request(180, -1);
    set_accessible_labels(
        &position_dropdown,
        "Screen position",
        "Where the overlay rests",
    );
    position.rows.append(&setting_row(
        "Placement",
        "Screen centre, an edge, or a custom offset",
        &position_dropdown,
        "position",
    ));

    let mut display_options = vec![None];
    let mut display_labels = vec!["Automatic".to_owned()];
    if let Some(display) = gtk::gdk::Display::default() {
        let monitors = display.monitors();
        for index in 0..monitors.n_items() {
            if let Some(monitor) = monitors.item(index).and_downcast::<gtk::gdk::Monitor>() {
                let label = monitor
                    .connector()
                    .map(|connector| connector.to_string())
                    .or_else(|| monitor.model().map(|model| model.to_string()))
                    .unwrap_or_else(|| format!("Display {}", index + 1));
                display_options.push(Some(label.clone()));
                display_labels.push(label);
            }
        }
    }
    if let Some(selected) = config.position.display.as_ref()
        && !display_options
            .iter()
            .any(|option| option.as_deref() == Some(selected.as_str()))
    {
        display_options.push(Some(selected.clone()));
        display_labels.push(format!("{selected} (unavailable)"));
    }
    let display_labels_ref: Vec<&str> = display_labels.iter().map(String::as_str).collect();
    let display_names = gtk::StringList::new(&display_labels_ref);
    let display_dropdown = gtk::DropDown::new(Some(display_names), None::<&gtk::Expression>);
    let display_index = config
        .position
        .display
        .as_ref()
        .and_then(|selected| {
            display_options
                .iter()
                .position(|option| option.as_deref() == Some(selected.as_str()))
        })
        .unwrap_or(0) as u32;
    display_dropdown.set_selected(display_index);
    display_dropdown.set_size_request(180, -1);
    set_accessible_labels(
        &display_dropdown,
        "Display",
        "Which monitor the overlay uses",
    );
    let display_options = Rc::new(display_options);
    let ctx = Rc::clone(&context);
    display_dropdown.connect_selected_notify(move |dropdown| {
        let selected = display_options
            .get(dropdown.selected() as usize)
            .cloned()
            .flatten();
        ctx.update(|config| config.position.display = selected);
    });
    position.rows.append(&setting_row(
        "Display",
        "Automatic follows the compositor's active output",
        &display_dropdown,
        "display",
    ));

    let margin_spin = gtk::SpinButton::with_range(0.0, 500.0, 1.0);
    margin_spin.set_value(f64::from(config.position.margin));
    margin_spin.set_size_request(120, -1);
    set_accessible_labels(
        &margin_spin,
        "Edge margin",
        "Distance from the screen edge in pixels",
    );
    let ctx = Rc::clone(&context);
    margin_spin.connect_value_changed(move |spin| {
        ctx.update(|config| config.position.margin = spin.value().round() as i32);
    });
    position.rows.append(&setting_row(
        "Edge margin",
        "Used by the top and bottom placements",
        &margin_spin,
        "margin",
    ));

    let custom_x = gtk::SpinButton::with_range(0.0, 20_000.0, 1.0);
    custom_x.set_value(f64::from(config.position.custom_x));
    custom_x.set_size_request(120, -1);
    set_accessible_labels(
        &custom_x,
        "Custom horizontal offset",
        "Distance from the left edge",
    );
    let ctx = Rc::clone(&context);
    custom_x.connect_value_changed(move |spin| {
        ctx.update(|config| config.position.custom_x = spin.value().round() as i32);
    });
    let custom_row = setting_row(
        "Custom offset",
        "Horizontal and vertical offset in pixels",
        &custom_x,
        "custom horizontal offset",
    );
    position.rows.append(&custom_row);

    let custom_y = gtk::SpinButton::with_range(0.0, 20_000.0, 1.0);
    custom_y.set_value(f64::from(config.position.custom_y));
    custom_y.set_size_request(120, -1);
    set_accessible_labels(
        &custom_y,
        "Custom vertical offset",
        "Distance from the top edge",
    );
    let ctx = Rc::clone(&context);
    custom_y.connect_value_changed(move |spin| {
        ctx.update(|config| config.position.custom_y = spin.value().round() as i32);
    });
    let custom_y_row = setting_row(
        "Vertical offset",
        "Only applied with the custom placement",
        &custom_y,
        "custom vertical offset",
    );
    position.rows.append(&custom_y_row);

    let layer_names = gtk::StringList::new(&[Layer::Top.label(), Layer::Overlay.label()]);
    let layer_dropdown = gtk::DropDown::new(Some(layer_names), None::<&gtk::Expression>);
    layer_dropdown.set_selected(if config.position.layer == Layer::Top {
        0
    } else {
        1
    });
    layer_dropdown.set_size_request(180, -1);
    set_accessible_labels(
        &layer_dropdown,
        "Surface layer",
        "Stacking layer on layer-shell compositors",
    );
    let ctx = Rc::clone(&context);
    layer_dropdown.connect_selected_notify(move |dropdown| {
        let layer = if dropdown.selected() == 1 {
            Layer::Overlay
        } else {
            Layer::Top
        };
        ctx.update(|config| config.position.layer = layer);
    });
    position.rows.append(&setting_row(
        "Layer",
        "Overlay can remain visible above fullscreen surfaces",
        &layer_dropdown,
        "layer",
    ));

    let custom_position = config.position.mode == PositionMode::Custom;
    custom_row.set_sensitive(custom_position);
    custom_y_row.set_sensitive(custom_position);
    let ctx = Rc::clone(&context);
    let custom_x_row = custom_row.clone();
    let custom_y_row_clone = custom_y_row.clone();
    position_dropdown.connect_selected_notify(move |dropdown| {
        let mode = PositionMode::ALL
            .get(dropdown.selected() as usize)
            .copied()
            .unwrap_or(PositionMode::Center);
        custom_x_row.set_sensitive(mode == PositionMode::Custom);
        custom_y_row_clone.set_sensitive(mode == PositionMode::Custom);
        ctx.update(|config| config.position.mode = mode);
    });

    // ---- Behaviour ------------------------------------------------------
    let behaviour = section("Behaviour");
    window_page.append(&behaviour.container);

    let auto_expand = gtk::Switch::new();
    auto_expand.set_active(config.behavior.auto_expand_on_new_track);
    let ctx = Rc::clone(&context);
    auto_expand.connect_active_notify(move |switch| {
        ctx.update(|config| config.behavior.auto_expand_on_new_track = switch.is_active());
    });
    behaviour.rows.append(&setting_row(
        "Expand for a new track",
        "Show the progress and full transport briefly",
        &auto_expand,
        "auto expand",
    ));

    let collapse_spin = gtk::SpinButton::with_range(1.0, 30.0, 0.5);
    collapse_spin.set_digits(1);
    collapse_spin.set_value(config.behavior.auto_collapse_seconds);
    collapse_spin.set_size_request(120, -1);
    set_accessible_labels(
        &collapse_spin,
        "Auto-collapse timeout",
        "Seconds before returning to compact",
    );
    let ctx = Rc::clone(&context);
    collapse_spin.connect_value_changed(move |spin| {
        ctx.update(|config| config.behavior.auto_collapse_seconds = spin.value());
    });
    behaviour.rows.append(&setting_row(
        "Collapse after",
        "Seconds of inactivity before returning to the pill",
        &collapse_spin,
        "collapse timeout",
    ));

    let hover_switch = gtk::Switch::new();
    hover_switch.set_active(config.behavior.expand_on_hover);
    let ctx = Rc::clone(&context);
    hover_switch.connect_active_notify(move |switch| {
        ctx.update(|config| config.behavior.expand_on_hover = switch.is_active());
    });
    behaviour.rows.append(&setting_row(
        "Expand on hover",
        "Reveal details when the pointer enters the pill",
        &hover_switch,
        "expand on hover",
    ));

    let pin_switch = gtk::Switch::new();
    pin_switch.set_active(config.behavior.pin_expanded);
    let ctx = Rc::clone(&context);
    pin_switch.connect_active_notify(move |switch| {
        ctx.update(|config| config.behavior.pin_expanded = switch.is_active());
    });
    behaviour.rows.append(&setting_row(
        "Pin expanded",
        "Keep the full layout open and disable auto-collapse",
        &pin_switch,
        "pin expanded",
    ));

    let paused_switch = gtk::Switch::new();
    paused_switch.set_active(config.behavior.show_when_paused);
    let ctx = Rc::clone(&context);
    paused_switch.connect_active_notify(move |switch| {
        ctx.update(|config| config.behavior.show_when_paused = switch.is_active());
    });
    behaviour.rows.append(&setting_row(
        "Show when paused",
        "Keep the current track visible while playback is paused",
        &paused_switch,
        "show when paused",
    ));

    let idle_switch = gtk::Switch::new();
    idle_switch.set_active(config.behavior.hide_when_idle);
    let ctx = Rc::clone(&context);
    idle_switch.connect_active_notify(move |switch| {
        ctx.update(|config| config.behavior.hide_when_idle = switch.is_active());
    });
    behaviour.rows.append(&setting_row(
        "Hide when idle",
        "Do not occupy the desktop when nothing is loaded",
        &idle_switch,
        "hide when idle",
    ));

    let autostart_switch = gtk::Switch::new();
    autostart_switch.set_active(config.behavior.autostart);
    let ctx = Rc::clone(&context);
    autostart_switch.connect_active_notify(move |switch| {
        ctx.update(|config| config.behavior.autostart = switch.is_active());
    });
    behaviour.rows.append(&setting_row(
        "Start automatically",
        "Create a KOKO WIDGET-owned XDG autostart entry",
        &autostart_switch,
        "start automatically",
    ));

    // ---- Widgets --------------------------------------------------------
    let widgets = section("Widgets");
    widgets_page.append(&widgets.container);

    let order_rebuild_slot: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
    let enabled_box = gtk::Box::new(gtk::Orientation::Vertical, 4);
    enabled_box.set_halign(gtk::Align::Fill);
    enabled_box.set_hexpand(true);
    let mut widget_switches: Vec<(WidgetKind, gtk::CheckButton)> = Vec::new();
    for kind in WidgetKind::ALL {
        let toggle = gtk::CheckButton::with_label(kind.label());
        toggle.set_active(config.widgets.is_enabled(kind));
        toggle.set_tooltip_text(Some(kind.description()));
        toggle.add_css_class("koko-widget-toggle");
        set_accessible_labels(&toggle, kind.label(), kind.description());
        let ctx = Rc::clone(&context);
        let rebuild_slot = Rc::clone(&order_rebuild_slot);
        toggle.connect_toggled(move |button| {
            let active = button.is_active();
            ctx.update(|config| config.widgets.set_enabled(kind, active));
            if let Some(rebuild) = rebuild_slot.borrow().as_ref() {
                rebuild();
            }
        });
        enabled_box.append(&toggle);
        widget_switches.push((kind, toggle));
    }
    widgets.rows.append(&setting_row(
        "Enabled widgets",
        "Turn each card on or off; order is controlled below",
        &enabled_box,
        "enabled widgets",
    ));

    // Order control: one row per enabled widget with explicit move buttons.
    let order_box = gtk::Box::new(gtk::Orientation::Vertical, 4);
    order_box.set_halign(gtk::Align::Fill);
    order_box.set_hexpand(true);
    let order_list = gtk::Box::new(gtk::Orientation::Vertical, 2);
    order_box.append(&order_list);
    // Shared handle so each freshly created button can trigger a redraw of the
    // order list after it mutates the configuration.
    let rebuild_slot = Rc::clone(&order_rebuild_slot);
    let rebuild_order: Rc<dyn Fn()> = {
        let order_list = order_list.clone();
        let context = Rc::clone(&context);
        let config_state = Rc::clone(&config_state);
        let rebuild_slot = Rc::clone(&rebuild_slot);
        Rc::new(move || {
            while let Some(child) = order_list.first_child() {
                order_list.remove(&child);
            }
            let active = config_state.borrow().widgets.active();
            if active.len() < 2 {
                let hint = gtk::Label::new(Some("Enable two or more widgets to reorder"));
                hint.add_css_class("koko-value");
                hint.set_xalign(0.0);
                order_list.append(&hint);
                return;
            }
            for (index, kind) in active.iter().enumerate() {
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
                row.set_hexpand(true);
                let name = gtk::Label::new(Some(kind.label()));
                name.add_css_class("koko-widget-name");
                name.set_xalign(0.0);
                name.set_hexpand(true);
                row.append(&name);

                for (delta, glyph, tip) in [(-1i32, "‹", "Move left"), (1i32, "›", "Move right")]
                {
                    let button = gtk::Button::with_label(glyph);
                    button.add_css_class("koko-secondary");
                    button.add_css_class("koko-order-button");
                    let target = index as i32 + delta;
                    button.set_sensitive(target >= 0 && (target as usize) < active.len());
                    button.set_tooltip_text(Some(tip));
                    set_accessible_labels(
                        &button,
                        &format!("{tip}: {}", kind.label()),
                        "Changes the left-to-right widget order",
                    );
                    let ctx = Rc::clone(&context);
                    let config_state = Rc::clone(&config_state);
                    let rebuild_slot = Rc::clone(&rebuild_slot);
                    let kind = *kind;
                    button.connect_clicked(move |_| {
                        let active = config_state.borrow().widgets.active();
                        let Some(from) = active.iter().position(|entry| *entry == kind) else {
                            return;
                        };
                        let to = from as i32 + delta;
                        if to < 0 || to as usize >= active.len() {
                            return;
                        }
                        let mut order = config_state.borrow().widgets.order.clone();
                        for widget in &active {
                            if !order.contains(widget) {
                                order.push(*widget);
                            }
                        }
                        let a = order.iter().position(|entry| *entry == kind);
                        let b = order.iter().position(|entry| *entry == active[to as usize]);
                        if let (Some(a), Some(b)) = (a, b) {
                            order.swap(a, b);
                        }
                        ctx.update(|config| config.widgets.order = order);
                        if let Some(rebuild) = rebuild_slot.borrow().as_ref() {
                            rebuild();
                        }
                    });
                    row.append(&button);
                }
                order_list.append(&row);
            }
        })
    };
    *rebuild_slot.borrow_mut() = Some(Rc::clone(&rebuild_order));
    rebuild_order();
    widgets.rows.append(&setting_row(
        "Order",
        "Move any card left or right in the row",
        &order_box,
        "widget order",
    ));

    let gap_scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 48.0, 1.0);
    gap_scale.set_value(config.widgets.gap);
    gap_scale.set_draw_value(false);
    gap_scale.set_size_request(180, -1);
    let gap_value = gtk::Label::new(Some(&format!("{:.0} px", config.widgets.gap)));
    gap_value.add_css_class("koko-value");
    gap_value.set_width_chars(6);
    let gap_box = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    gap_box.append(&gap_scale);
    gap_box.append(&gap_value);
    set_accessible_labels(&gap_scale, "Widget gap", "Space between cards in pixels");
    let ctx = Rc::clone(&context);
    let gap_label = gap_value.clone();
    gap_scale.connect_value_changed(move |scale| {
        let value = scale.value();
        gap_label.set_text(&format!("{value:.0} px"));
        ctx.update(|config| config.widgets.gap = value);
    });
    widgets.rows.append(&setting_row(
        "Gap",
        "Space between adjacent cards",
        &gap_box,
        "widget gap",
    ));

    let align_names = gtk::StringList::new(&["Centered", "Top aligned", "Bottom aligned"]);
    let align_dropdown = gtk::DropDown::new(Some(align_names), None::<&gtk::Expression>);
    align_dropdown.set_selected(match config.widgets.align {
        crate::config::RowAlign::Center => 0,
        crate::config::RowAlign::Top => 1,
        crate::config::RowAlign::Bottom => 2,
    });
    align_dropdown.set_size_request(180, -1);
    set_accessible_labels(
        &align_dropdown,
        "Row alignment",
        "Vertical alignment of the cards",
    );
    let ctx = Rc::clone(&context);
    align_dropdown.connect_selected_notify(move |dropdown| {
        let align = match dropdown.selected() {
            1 => crate::config::RowAlign::Top,
            2 => crate::config::RowAlign::Bottom,
            _ => crate::config::RowAlign::Center,
        };
        ctx.update(|config| config.widgets.align = align);
    });
    widgets.rows.append(&setting_row(
        "Alignment",
        "How shorter cards line up with the tallest card",
        &align_dropdown,
        "row alignment",
    ));

    // ---- Media widget ---------------------------------------------------
    let media_widget = section("Now playing widget");
    widgets_page.append(&media_widget.container);

    let media_background = gtk::Switch::new();
    media_background.set_active(config.widgets.media.background);
    let ctx = Rc::clone(&context);
    media_background.connect_active_notify(move |switch| {
        ctx.update(|config| config.widgets.media.background = switch.is_active());
    });
    media_widget.rows.append(&setting_row(
        "Glass background",
        "Off leaves the artwork and text floating without a pill",
        &media_background,
        "media background",
    ));

    let media_artwork = gtk::Switch::new();
    media_artwork.set_active(config.widgets.media.artwork);
    let ctx = Rc::clone(&context);
    media_artwork.connect_active_notify(move |switch| {
        ctx.update(|config| config.widgets.media.artwork = switch.is_active());
    });
    media_widget.rows.append(&setting_row(
        "Album artwork",
        "Show the cover beside the track details",
        &media_artwork,
        "album artwork",
    ));

    let (art_radius_scale, art_radius_value) = value_scale(
        config.widgets.media.artwork_radius,
        0.0,
        48.0,
        "Artwork radius",
        " px",
    );
    {
        let ctx = Rc::clone(&context);
        let value = art_radius_value.clone();
        art_radius_scale.connect_value_changed(move |scale| {
            let now = scale.value();
            value.set_text(&format!("{now:.0} px"));
            ctx.update(|config| config.widgets.media.artwork_radius = now);
        });
    }
    media_widget.rows.append(&setting_row(
        "Artwork corner radius",
        "Rounded corners on the cover thumbnail",
        &art_radius_scale,
        "artwork radius",
    ));

    let show_artist = gtk::Switch::new();
    show_artist.set_active(config.widgets.media.show_artist);
    let ctx = Rc::clone(&context);
    show_artist.connect_active_notify(move |switch| {
        ctx.update(|config| config.widgets.media.show_artist = switch.is_active());
    });
    media_widget.rows.append(&setting_row(
        "Artist line",
        "Second line under the track title",
        &show_artist,
        "artist line",
    ));

    let show_controls = gtk::Switch::new();
    show_controls.set_active(config.widgets.media.show_controls);
    let ctx = Rc::clone(&context);
    show_controls.connect_active_notify(move |switch| {
        ctx.update(|config| config.widgets.media.show_controls = switch.is_active());
    });
    media_widget.rows.append(&setting_row(
        "Transport controls",
        "Previous, play/pause and next buttons",
        &show_controls,
        "transport controls",
    ));

    let show_progress = gtk::Switch::new();
    show_progress.set_active(config.widgets.media.show_progress);
    let ctx = Rc::clone(&context);
    show_progress.connect_active_notify(move |switch| {
        ctx.update(|config| config.widgets.media.show_progress = switch.is_active());
    });
    media_widget.rows.append(&setting_row(
        "Progress bar",
        "Seekable position and remaining time",
        &show_progress,
        "progress bar",
    ));

    let (title_scale, title_value) = value_scale(
        config.widgets.media.title_size,
        8.0,
        32.0,
        "Title size",
        " pt",
    );
    {
        let ctx = Rc::clone(&context);
        let value = title_value.clone();
        title_scale.connect_value_changed(move |scale| {
            let now = scale.value();
            value.set_text(&format!("{now:.1} pt"));
            ctx.update(|config| config.widgets.media.title_size = now);
        });
    }
    media_widget.rows.append(&setting_row(
        "Title size",
        "Track title type size",
        &title_scale,
        "title size",
    ));

    let (artist_scale, artist_value) = value_scale(
        config.widgets.media.artist_size,
        7.0,
        26.0,
        "Artist size",
        " pt",
    );
    {
        let ctx = Rc::clone(&context);
        let value = artist_value.clone();
        artist_scale.connect_value_changed(move |scale| {
            let now = scale.value();
            value.set_text(&format!("{now:.1} pt"));
            ctx.update(|config| config.widgets.media.artist_size = now);
        });
    }
    media_widget.rows.append(&setting_row(
        "Artist size",
        "Artist and album type size",
        &artist_scale,
        "artist size",
    ));

    // ---- Clock widget ---------------------------------------------------
    let clock_section = section("Clock widget");
    widgets_page.append(&clock_section.container);

    let clock_background = gtk::Switch::new();
    clock_background.set_active(config.widgets.clock.background);
    let ctx = Rc::clone(&context);
    clock_background.connect_active_notify(move |switch| {
        ctx.update(|config| config.widgets.clock.background = switch.is_active());
    });
    clock_section.rows.append(&setting_row(
        "Glass background",
        "Off renders the time as liquid-glass lettering on the desktop",
        &clock_background,
        "clock background",
    ));

    let show_seconds = gtk::Switch::new();
    show_seconds.set_active(config.widgets.clock.show_seconds);
    let ctx = Rc::clone(&context);
    show_seconds.connect_active_notify(move |switch| {
        ctx.update(|config| config.widgets.clock.show_seconds = switch.is_active());
    });
    clock_section.rows.append(&setting_row(
        "Show seconds",
        "Add a seconds field to the clock",
        &show_seconds,
        "show seconds",
    ));

    let hour_names = gtk::StringList::new(&[
        HourFormat::Auto.label(),
        HourFormat::TwelveHour.label(),
        HourFormat::TwentyFourHour.label(),
    ]);
    let hour_dropdown = gtk::DropDown::new(Some(hour_names), None::<&gtk::Expression>);
    hour_dropdown.set_selected(match config.widgets.clock.hour_format {
        HourFormat::Auto => 0,
        HourFormat::TwelveHour => 1,
        HourFormat::TwentyFourHour => 2,
    });
    hour_dropdown.set_size_request(200, -1);
    set_accessible_labels(
        &hour_dropdown,
        "Hour format",
        "12-hour, 24-hour or locale default",
    );
    let ctx = Rc::clone(&context);
    hour_dropdown.connect_selected_notify(move |dropdown| {
        let format = match dropdown.selected() {
            1 => HourFormat::TwelveHour,
            2 => HourFormat::TwentyFourHour,
            _ => HourFormat::Auto,
        };
        ctx.update(|config| config.widgets.clock.hour_format = format);
    });
    clock_section.rows.append(&setting_row(
        "Hour format",
        "How the time is written",
        &hour_dropdown,
        "hour format",
    ));

    let clock_label = gtk::Switch::new();
    clock_label.set_active(config.widgets.clock.show_label);
    let ctx = Rc::clone(&context);
    clock_label.connect_active_notify(move |switch| {
        ctx.update(|config| config.widgets.clock.show_label = switch.is_active());
    });
    clock_section.rows.append(&setting_row(
        "Caption",
        "Small label above the time",
        &clock_label,
        "clock caption",
    ));

    let clock_caption = text_entry(&config.widgets.clock.label, "Local time");
    let ctx = Rc::clone(&context);
    clock_caption.connect_changed(move |entry| {
        let text = entry.text().to_string();
        ctx.update(|config| config.widgets.clock.label = text);
    });
    clock_section.rows.append(&setting_row(
        "Caption text",
        "Shown only when the caption is enabled",
        &clock_caption,
        "clock caption text",
    ));

    let (clock_size, clock_size_value) = value_scale(
        config.widgets.clock.font_size,
        10.0,
        96.0,
        "Clock size",
        " pt",
    );
    {
        let ctx = Rc::clone(&context);
        let value = clock_size_value.clone();
        clock_size.connect_value_changed(move |scale| {
            let now = scale.value();
            value.set_text(&format!("{now:.0} pt"));
            ctx.update(|config| config.widgets.clock.font_size = now);
        });
    }
    clock_section.rows.append(&setting_row(
        "Time size",
        "Type size of the clock digits",
        &clock_size,
        "clock size",
    ));

    let (clock_width, clock_width_value) = value_scale(
        config.widgets.clock.width,
        96.0,
        420.0,
        "Clock width",
        " px",
    );
    {
        let ctx = Rc::clone(&context);
        let value = clock_width_value.clone();
        clock_width.connect_value_changed(move |scale| {
            let now = scale.value();
            value.set_text(&format!("{now:.0} px"));
            ctx.update(|config| config.widgets.clock.width = now);
        });
    }
    clock_section.rows.append(&setting_row(
        "Card width",
        "Width of the clock card",
        &clock_width,
        "clock card width",
    ));

    let (clock_height, clock_height_value) = value_scale(
        config.widgets.clock.height,
        72.0,
        320.0,
        "Clock height",
        " px",
    );
    {
        let ctx = Rc::clone(&context);
        let value = clock_height_value.clone();
        clock_height.connect_value_changed(move |scale| {
            let now = scale.value();
            value.set_text(&format!("{now:.0} px"));
            ctx.update(|config| config.widgets.clock.height = now);
        });
    }
    clock_section.rows.append(&setting_row(
        "Card height",
        "Height of the clock pill; each widget keeps its own shape",
        &clock_height,
        "clock card height",
    ));

    let clock_weight = weight_dropdown(config.widgets.clock.weight);
    {
        let ctx = Rc::clone(&context);
        let dropdown = clock_weight.clone();
        clock_weight.connect_selected_notify(move |_| {
            let weight = weight_from_index(dropdown.selected());
            ctx.update(|config| config.widgets.clock.weight = weight);
        });
    }
    clock_section.rows.append(&setting_row(
        "Weight",
        "Font weight of the clock digits",
        &clock_weight,
        "clock weight",
    ));

    let clock_colour = colour_button(config.widgets.clock.colour.as_deref(), "Clock colour");
    {
        let ctx = Rc::clone(&context);
        clock_colour.connect_rgba_notify(move |button| {
            let colour = button.rgba();
            let hex = format_hex_color([
                f64::from(colour.red()),
                f64::from(colour.green()),
                f64::from(colour.blue()),
            ]);
            ctx.update(|config| config.widgets.clock.colour = Some(hex));
        });
    }
    let clock_colour_row = setting_row(
        "Colour",
        "Leave at the theme colour unless you want a custom tint",
        &clock_colour,
        "clock colour",
    );
    let reset_clock_colour = gtk::Button::with_label("Reset");
    reset_clock_colour.add_css_class("koko-secondary");
    set_accessible_labels(
        &reset_clock_colour,
        "Reset clock colour",
        "Return to the theme text colour",
    );
    {
        let ctx = Rc::clone(&context);
        let button = clock_colour.clone();
        reset_clock_colour.connect_clicked(move |_| {
            ctx.update(|config| config.widgets.clock.colour = None);
            if let Some(colour) = crate::utils::parse_hex_color("#ffffff") {
                button.set_rgba(&gdk::RGBA::new(
                    colour[0] as f32,
                    colour[1] as f32,
                    colour[2] as f32,
                    colour[3] as f32,
                ));
            }
        });
    }
    clock_section.rows.append(&clock_colour_row);
    clock_section.rows.append(&setting_row(
        "Reset colour",
        "Use the theme text colour again",
        &reset_clock_colour,
        "reset clock colour",
    ));

    // ---- Date widget ----------------------------------------------------
    let date_section = section("Date widget");
    widgets_page.append(&date_section.container);

    let date_background = gtk::Switch::new();
    date_background.set_active(config.widgets.date.background);
    let ctx = Rc::clone(&context);
    date_background.connect_active_notify(move |switch| {
        ctx.update(|config| config.widgets.date.background = switch.is_active());
    });
    date_section.rows.append(&setting_row(
        "Glass background",
        "Off renders the date as liquid-glass lettering on the desktop",
        &date_background,
        "date background",
    ));

    let date_format = text_entry(&config.widgets.date.format, "%A %-d %B");
    let ctx = Rc::clone(&context);
    date_format.connect_changed(move |entry| {
        let text = entry.text().to_string();
        ctx.update(|config| config.widgets.date.format = text);
    });
    date_section.rows.append(&setting_row(
        "Format",
        "strftime pattern, for example %A %-d %B or %Y-%m-%d",
        &date_format,
        "date format",
    ));

    let date_wrap = gtk::Switch::new();
    date_wrap.set_active(config.widgets.date.wrap);
    let ctx = Rc::clone(&context);
    date_wrap.connect_active_notify(move |switch| {
        ctx.update(|config| config.widgets.date.wrap = switch.is_active());
    });
    date_section.rows.append(&setting_row(
        "Wrap onto two lines",
        "Break long dates at a word boundary",
        &date_wrap,
        "date wrapping",
    ));

    let date_label = gtk::Switch::new();
    date_label.set_active(config.widgets.date.show_label);
    let ctx = Rc::clone(&context);
    date_label.connect_active_notify(move |switch| {
        ctx.update(|config| config.widgets.date.show_label = switch.is_active());
    });
    date_section.rows.append(&setting_row(
        "Caption",
        "Small label above the date",
        &date_label,
        "date caption",
    ));

    let date_caption = text_entry(&config.widgets.date.label, "Today");
    let ctx = Rc::clone(&context);
    date_caption.connect_changed(move |entry| {
        let text = entry.text().to_string();
        ctx.update(|config| config.widgets.date.label = text);
    });
    date_section.rows.append(&setting_row(
        "Caption text",
        "Shown only when the caption is enabled",
        &date_caption,
        "date caption text",
    ));

    let (date_size, date_size_value) =
        value_scale(config.widgets.date.font_size, 8.0, 72.0, "Date size", " pt");
    {
        let ctx = Rc::clone(&context);
        let value = date_size_value.clone();
        date_size.connect_value_changed(move |scale| {
            let now = scale.value();
            value.set_text(&format!("{now:.0} pt"));
            ctx.update(|config| config.widgets.date.font_size = now);
        });
    }
    date_section.rows.append(&setting_row(
        "Date size",
        "Type size of the date",
        &date_size,
        "date size",
    ));

    let (date_width, date_width_value) =
        value_scale(config.widgets.date.width, 96.0, 420.0, "Date width", " px");
    {
        let ctx = Rc::clone(&context);
        let value = date_width_value.clone();
        date_width.connect_value_changed(move |scale| {
            let now = scale.value();
            value.set_text(&format!("{now:.0} px"));
            ctx.update(|config| config.widgets.date.width = now);
        });
    }
    date_section.rows.append(&setting_row(
        "Card width",
        "Width of the date card",
        &date_width,
        "date card width",
    ));

    let (date_height, date_height_value) = value_scale(
        config.widgets.date.height,
        72.0,
        320.0,
        "Date height",
        " px",
    );
    {
        let ctx = Rc::clone(&context);
        let value = date_height_value.clone();
        date_height.connect_value_changed(move |scale| {
            let now = scale.value();
            value.set_text(&format!("{now:.0} px"));
            ctx.update(|config| config.widgets.date.height = now);
        });
    }
    date_section.rows.append(&setting_row(
        "Card height",
        "Height of the date pill; each widget keeps its own shape",
        &date_height,
        "date card height",
    ));

    let date_weight = weight_dropdown(config.widgets.date.weight);
    {
        let ctx = Rc::clone(&context);
        let dropdown = date_weight.clone();
        date_weight.connect_selected_notify(move |_| {
            let weight = weight_from_index(dropdown.selected());
            ctx.update(|config| config.widgets.date.weight = weight);
        });
    }
    date_section.rows.append(&setting_row(
        "Weight",
        "Font weight of the date",
        &date_weight,
        "date weight",
    ));

    // ---- Text widget ----------------------------------------------------
    let text_section = section("Text widget");
    widgets_page.append(&text_section.container);

    let text_background = gtk::Switch::new();
    text_background.set_active(config.widgets.text.background);
    let ctx = Rc::clone(&context);
    text_background.connect_active_notify(move |switch| {
        ctx.update(|config| config.widgets.text.background = switch.is_active());
    });
    text_section.rows.append(&setting_row(
        "Glass background",
        "Off renders the message as liquid-glass lettering on the desktop",
        &text_background,
        "text background",
    ));

    let note_text = text_entry(&config.widgets.text.text, "Stay focused");
    let ctx = Rc::clone(&context);
    note_text.connect_changed(move |entry| {
        let text = entry.text().to_string();
        ctx.update(|config| config.widgets.text.text = text);
    });
    text_section.rows.append(&setting_row(
        "Text",
        "Any short message, up to 240 characters",
        &note_text,
        "text content",
    ));

    let text_label = gtk::Switch::new();
    text_label.set_active(config.widgets.text.show_label);
    let ctx = Rc::clone(&context);
    text_label.connect_active_notify(move |switch| {
        ctx.update(|config| config.widgets.text.show_label = switch.is_active());
    });
    text_section.rows.append(&setting_row(
        "Caption",
        "Small label above the message",
        &text_label,
        "text caption",
    ));

    let text_caption = text_entry(&config.widgets.text.label, "Note");
    let ctx = Rc::clone(&context);
    text_caption.connect_changed(move |entry| {
        let text = entry.text().to_string();
        ctx.update(|config| config.widgets.text.label = text);
    });
    text_section.rows.append(&setting_row(
        "Caption text",
        "Shown only when the caption is enabled",
        &text_caption,
        "text caption text",
    ));

    let (text_size, text_size_value) =
        value_scale(config.widgets.text.font_size, 8.0, 72.0, "Text size", " pt");
    {
        let ctx = Rc::clone(&context);
        let value = text_size_value.clone();
        text_size.connect_value_changed(move |scale| {
            let now = scale.value();
            value.set_text(&format!("{now:.0} pt"));
            ctx.update(|config| config.widgets.text.font_size = now);
        });
    }
    text_section.rows.append(&setting_row(
        "Text size",
        "Type size of the message",
        &text_size,
        "text size",
    ));

    let (text_width, text_width_value) =
        value_scale(config.widgets.text.width, 96.0, 420.0, "Text width", " px");
    {
        let ctx = Rc::clone(&context);
        let value = text_width_value.clone();
        text_width.connect_value_changed(move |scale| {
            let now = scale.value();
            value.set_text(&format!("{now:.0} px"));
            ctx.update(|config| config.widgets.text.width = now);
        });
    }
    text_section.rows.append(&setting_row(
        "Card width",
        "Width of the text card",
        &text_width,
        "text card width",
    ));

    let (text_height, text_height_value) = value_scale(
        config.widgets.text.height,
        72.0,
        320.0,
        "Text height",
        " px",
    );
    {
        let ctx = Rc::clone(&context);
        let value = text_height_value.clone();
        text_height.connect_value_changed(move |scale| {
            let now = scale.value();
            value.set_text(&format!("{now:.0} px"));
            ctx.update(|config| config.widgets.text.height = now);
        });
    }
    text_section.rows.append(&setting_row(
        "Card height",
        "Height of the text pill; each widget keeps its own shape",
        &text_height,
        "text card height",
    ));

    let text_weight = weight_dropdown(config.widgets.text.weight);
    {
        let ctx = Rc::clone(&context);
        let dropdown = text_weight.clone();
        text_weight.connect_selected_notify(move |_| {
            let weight = weight_from_index(dropdown.selected());
            ctx.update(|config| config.widgets.text.weight = weight);
        });
    }
    text_section.rows.append(&setting_row(
        "Weight",
        "Font weight of the message",
        &text_weight,
        "text weight",
    ));

    let align_names = gtk::StringList::new(&[
        TextAlign::Left.label(),
        TextAlign::Center.label(),
        TextAlign::Right.label(),
    ]);
    let text_align = gtk::DropDown::new(Some(align_names), None::<&gtk::Expression>);
    text_align.set_selected(match config.widgets.text.align {
        TextAlign::Left => 0,
        TextAlign::Center => 1,
        TextAlign::Right => 2,
    });
    text_align.set_size_request(180, -1);
    set_accessible_labels(
        &text_align,
        "Text alignment",
        "Horizontal alignment inside the card",
    );
    let ctx = Rc::clone(&context);
    text_align.connect_selected_notify(move |dropdown| {
        let align = match dropdown.selected() {
            0 => TextAlign::Left,
            2 => TextAlign::Right,
            _ => TextAlign::Center,
        };
        ctx.update(|config| config.widgets.text.align = align);
    });
    text_section.rows.append(&setting_row(
        "Alignment",
        "How the message sits inside its card",
        &text_align,
        "text alignment",
    ));

    // ---- Media ----------------------------------------------------------
    let media = section("Media players");
    players_page.append(&media.container);

    let player_options = Rc::new(RefCell::new(build_player_options(&config, &players)));
    let (player_dropdown, player_guard) = build_player_dropdown(&player_options.borrow(), &config);
    set_accessible_labels(
        &player_dropdown,
        "Preferred player",
        "Player to prefer over automatic selection",
    );
    let ctx = Rc::clone(&context);
    let options_for_change = Rc::clone(&player_options);
    let guard_for_change = player_guard.clone();
    player_dropdown.connect_selected_notify(move |dropdown| {
        if guard_for_change.get() {
            return;
        }
        let selected = options_for_change
            .borrow()
            .get(dropdown.selected() as usize)
            .cloned();
        let preferred = selected.and_then(|option| option.selector);
        ctx.update(|config| config.media.preferred_player = preferred);
    });
    media.rows.append(&setting_row(
        "Preferred player",
        "Automatic selection favours the most recently playing player",
        &player_dropdown,
        "preferred player",
    ));

    let auto_select = gtk::Switch::new();
    auto_select.set_active(config.media.auto_select);
    let ctx = Rc::clone(&context);
    auto_select.connect_active_notify(move |switch| {
        ctx.update(|config| config.media.auto_select = switch.is_active());
    });
    media.rows.append(&setting_row(
        "Automatic selection",
        "Follow new playing players as they appear",
        &auto_select,
        "automatic selection",
    ));

    let priority_entry = gtk::Entry::new();
    priority_entry.set_text(&config.media.player_priority.join(", "));
    priority_entry.set_placeholder_text(Some("vlc, spotify, firefox"));
    priority_entry.set_size_request(220, -1);
    set_accessible_labels(
        &priority_entry,
        "Fallback priority",
        "Comma-separated player names",
    );
    let ctx = Rc::clone(&context);
    priority_entry.connect_changed(move |entry| {
        let priority = entry
            .text()
            .split(',')
            .map(str::trim)
            .filter(|entry| !entry.is_empty())
            .map(str::to_owned)
            .collect::<Vec<_>>();
        ctx.update(|config| config.media.player_priority = priority);
    });
    media.rows.append(&setting_row(
        "Fallback priority",
        "Comma-separated names used to break ties",
        &priority_entry,
        "fallback priority",
    ));

    let remote_switch = gtk::Switch::new();
    remote_switch.set_active(config.media.allow_remote_artwork);
    let ctx = Rc::clone(&context);
    remote_switch.connect_active_notify(move |switch| {
        ctx.update(|config| config.media.allow_remote_artwork = switch.is_active());
    });
    media.rows.append(&setting_row(
        "Remote artwork",
        "Download cover URLs to the local cache; no other network access",
        &remote_switch,
        "remote artwork",
    ));

    // ---- Animation ------------------------------------------------------
    let animation = section("Animation");
    window_page.append(&animation.container);

    let animations = gtk::Switch::new();
    animations.set_active(config.animation.enabled);
    let ctx = Rc::clone(&context);
    animations.connect_active_notify(move |switch| {
        ctx.update(|config| config.animation.enabled = switch.is_active());
    });
    animation.rows.append(&setting_row(
        "Animations",
        "Morph between the compact and expanded forms",
        &animations,
        "animations",
    ));

    let (speed_scale, speed_value) =
        value_scale(config.animation.speed, 0.25, 2.0, "Animation speed", "×");
    {
        let ctx = Rc::clone(&context);
        let value = speed_value.clone();
        speed_scale.connect_value_changed(move |scale| {
            let value_now = scale.value();
            value.set_text(&format!("{value_now:.2}×"));
            ctx.update(|config| config.animation.speed = value_now);
        });
    }
    let speed_box = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    speed_box.append(&speed_scale);
    speed_box.append(&speed_value);
    animation.rows.append(&setting_row(
        "Speed",
        "1.0× matches the reference timing",
        &speed_box,
        "animation speed",
    ));

    let motion_names = gtk::StringList::new(&[
        ReducedMotion::Auto.label(),
        ReducedMotion::Always.label(),
        ReducedMotion::Never.label(),
    ]);
    let motion_dropdown = gtk::DropDown::new(Some(motion_names), None::<&gtk::Expression>);
    motion_dropdown.set_selected(match config.animation.reduced_motion {
        ReducedMotion::Auto => 0,
        ReducedMotion::Always => 1,
        ReducedMotion::Never => 2,
    });
    motion_dropdown.set_size_request(200, -1);
    set_accessible_labels(
        &motion_dropdown,
        "Reduced motion",
        "Disable the morph for accessibility",
    );
    let ctx = Rc::clone(&context);
    motion_dropdown.connect_selected_notify(move |dropdown| {
        let mode = match dropdown.selected() {
            1 => ReducedMotion::Always,
            2 => ReducedMotion::Never,
            _ => ReducedMotion::Auto,
        };
        ctx.update(|config| config.animation.reduced_motion = mode);
    });
    animation.rows.append(&setting_row(
        "Reduced motion",
        "The system setting can suppress all motion automatically",
        &motion_dropdown,
        "reduced motion",
    ));

    // ---- Footer ---------------------------------------------------------
    let footer = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    footer.add_css_class("koko-settings-footer");
    let config_path = gtk::Label::new(Some(&config_file().display().to_string()));
    config_path.add_css_class("koko-config-path");
    config_path.set_ellipsize(pango::EllipsizeMode::Middle);
    config_path.set_xalign(0.0);
    config_path.set_hexpand(true);
    footer.append(&config_path);
    let close = gtk::Button::with_label("Close");
    close.add_css_class("koko-close");
    let window_clone = window.clone();
    let on_close_footer = Rc::clone(&on_close);
    close.connect_clicked(move |_| {
        window_clone.destroy();
        on_close_footer();
    });
    footer.append(&close);
    root.append(&footer);

    // Update the player dropdown when the media worker discovers new players.
    let player_dropdown_weak = player_dropdown.downgrade();
    let player_options_update = Rc::clone(&player_options);
    let guard_update = player_guard.clone();
    let config_state_update = Rc::clone(&config_state);
    let update_players = Box::new(move |players: &[PlayerInfo]| {
        let Some(dropdown) = player_dropdown_weak.upgrade() else {
            return;
        };
        let config = config_state_update.borrow().clone();
        *player_options_update.borrow_mut() = build_player_options(&config, players);
        guard_update.set(true);
        let options = player_options_update.borrow();
        let labels = options
            .iter()
            .map(|option| option.label.as_str())
            .collect::<Vec<_>>();
        let list = gtk::StringList::new(&labels);
        dropdown.set_model(Some(&list));
        let preferred = config.media.preferred_player.as_deref();
        let selected = options
            .iter()
            .position(|option| option.selector.as_deref() == preferred)
            .unwrap_or(0);
        dropdown.set_selected(selected as u32);
        guard_update.set(false);
    });

    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let close_window = window.clone();
    let on_close_escape = Rc::clone(&on_close);
    keys.connect_key_pressed(move |_, key, _code, _modifiers| {
        if key == gdk::Key::Escape {
            close_window.destroy();
            on_close_escape();
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    window.add_controller(keys);
    window.present();
    SettingsHandle {
        window,
        update_players,
    }
}

struct Section {
    container: gtk::Box,
    rows: gtk::Box,
}

fn section(title: &str) -> Section {
    let container = gtk::Box::new(gtk::Orientation::Vertical, 10);
    let heading = gtk::Label::new(Some(title));
    heading.add_css_class("koko-section-title");
    heading.set_xalign(0.0);
    container.append(&heading);
    let rows = gtk::Box::new(gtk::Orientation::Vertical, 8);
    container.append(&rows);
    Section { container, rows }
}

fn setting_row(
    title: &str,
    subtitle: &str,
    control: &impl IsA<gtk::Widget>,
    accessible_title: &str,
) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    row.add_css_class("koko-setting-row");
    let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
    text.set_hexpand(true);
    let title_label = gtk::Label::new(Some(title));
    title_label.add_css_class("koko-setting-title");
    title_label.set_xalign(0.0);
    let subtitle_label = gtk::Label::new(Some(subtitle));
    subtitle_label.add_css_class("koko-setting-subtitle");
    subtitle_label.set_xalign(0.0);
    subtitle_label.set_wrap(true);
    text.append(&title_label);
    text.append(&subtitle_label);
    row.append(&text);
    control.as_ref().set_valign(gtk::Align::Center);
    control.as_ref().set_hexpand(false);
    row.append(control);
    set_accessible_labels(control, accessible_title, subtitle);
    row
}

fn set_accessible_labels<W: IsA<gtk::Widget>>(widget: &W, label: &str, description: &str) {
    let widget: &gtk::Widget = widget.as_ref();
    widget.update_property(&[
        gtk::accessible::Property::Label(label),
        gtk::accessible::Property::Description(description),
    ]);
}

fn text_entry(value: &str, placeholder: &str) -> gtk::Entry {
    let entry = gtk::Entry::new();
    entry.set_text(value);
    entry.set_placeholder_text(Some(placeholder));
    entry.set_size_request(220, -1);
    entry
}

fn weight_dropdown(weight: FontWeight) -> gtk::DropDown {
    let names = gtk::StringList::new(&[
        FontWeight::Light.label(),
        FontWeight::Normal.label(),
        FontWeight::Medium.label(),
        FontWeight::Semibold.label(),
        FontWeight::Bold.label(),
    ]);
    let dropdown = gtk::DropDown::new(Some(names), None::<&gtk::Expression>);
    dropdown.set_selected(match weight {
        FontWeight::Light => 0,
        FontWeight::Normal => 1,
        FontWeight::Medium => 2,
        FontWeight::Semibold => 3,
        FontWeight::Bold => 4,
    });
    dropdown.set_size_request(150, -1);
    set_accessible_labels(&dropdown, "Font weight", "Weight of the rendered text");
    dropdown
}

fn weight_from_index(index: u32) -> FontWeight {
    match index {
        0 => FontWeight::Light,
        1 => FontWeight::Normal,
        2 => FontWeight::Medium,
        4 => FontWeight::Bold,
        _ => FontWeight::Semibold,
    }
}

fn colour_button(colour: Option<&str>, label: &str) -> gtk::ColorDialogButton {
    let rgba = colour
        .and_then(parse_hex_color)
        .map(|[r, g, b, a]| gdk::RGBA::new(r as f32, g as f32, b as f32, a as f32))
        .unwrap_or_else(|| gdk::RGBA::new(1.0, 1.0, 1.0, 1.0));
    let dialog = gtk::ColorDialog::builder()
        .title(label)
        .with_alpha(false)
        .modal(true)
        .build();
    let button = gtk::ColorDialogButton::new(Some(dialog));
    button.set_rgba(&rgba);
    button.set_size_request(64, -1);
    set_accessible_labels(&button, label, "Choose a colour for this widget");
    button
}

fn percentage_scale(value: f64, min: f64, max: f64, _label: &str) -> (gtk::Scale, gtk::Label) {
    let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, min, max, 0.01);
    scale.set_value(value);
    scale.set_draw_value(false);
    scale.set_size_request(180, -1);
    let label = gtk::Label::new(Some(&format!("{:.0}%", value * 100.0)));
    label.add_css_class("koko-value");
    label.set_width_chars(5);
    (scale, label)
}

fn value_scale(
    value: f64,
    min: f64,
    max: f64,
    _label: &str,
    suffix: &str,
) -> (gtk::Scale, gtk::Label) {
    let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, min, max, 0.05);
    scale.set_value(value);
    scale.set_draw_value(false);
    scale.set_size_request(180, -1);
    let label = gtk::Label::new(Some(&format!("{value:.2}{suffix}")));
    label.add_css_class("koko-value");
    label.set_width_chars(6);
    (scale, label)
}

fn theme_index(theme: Theme) -> u32 {
    match theme {
        Theme::Dark => 0,
        Theme::Light => 1,
        Theme::Auto => 2,
    }
}

fn theme_from_index(index: u32) -> Theme {
    match index {
        1 => Theme::Light,
        2 => Theme::Auto,
        _ => Theme::Dark,
    }
}

#[derive(Clone)]
struct PlayerOption {
    label: String,
    selector: Option<String>,
}

fn build_player_options(config: &Config, players: &[PlayerInfo]) -> Vec<PlayerOption> {
    let mut options = vec![PlayerOption {
        label: "Automatic".to_owned(),
        selector: None,
    }];
    let mut seen = std::collections::HashSet::new();
    for player in players {
        if player.bus_name.ends_with(".playerctld") {
            continue;
        }
        let selector = player.suffix().to_owned();
        if !seen.insert(selector.clone()) {
            continue;
        }
        options.push(PlayerOption {
            label: format!("{}  ·  {}", player.identity, player.suffix()),
            selector: Some(selector),
        });
    }
    if let Some(preferred) = config.media.preferred_player.as_deref()
        && !options
            .iter()
            .any(|option| option.selector.as_deref() == Some(preferred))
    {
        options.push(PlayerOption {
            label: format!("{preferred} (unavailable)"),
            selector: Some(preferred.to_owned()),
        });
    }
    options
}

fn build_player_dropdown(
    options: &[PlayerOption],
    config: &Config,
) -> (gtk::DropDown, Rc<Cell<bool>>) {
    let labels = options
        .iter()
        .map(|option| option.label.as_str())
        .collect::<Vec<_>>();
    let list = gtk::StringList::new(&labels);
    let dropdown = gtk::DropDown::new(Some(list), None::<&gtk::Expression>);
    let selected = config
        .media
        .preferred_player
        .as_deref()
        .and_then(|preferred| {
            options
                .iter()
                .position(|option| option.selector.as_deref() == Some(preferred))
        })
        .unwrap_or(0);
    dropdown.set_selected(selected as u32);
    dropdown.set_size_request(220, -1);
    (dropdown, Rc::new(Cell::new(false)))
}
