//! Main overlay controller: media state, interaction, visibility and animation
//! scheduling.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::Result;
use glib::ControlFlow;
use gtk::{gdk, glib, prelude::*};
use tracing::{debug, info, warn};

use crate::{
    animation::{Morph, Phase},
    config::Config,
    config::WidgetKind,
    ipc::Action,
    media::{
        artwork::{ArtworkCallback, ArtworkLoader, ArtworkResponse},
        metadata::{PlaybackStatus, PlayerInfo, PlayerSnapshot},
        service::{MediaCallback, MediaCommand, MediaEvent, MediaService},
    },
    ui::{
        artwork::ArtworkVisual,
        draw::{self, ViewData},
        layout::{self, Control, FrameLayout, Rect},
        settings,
        windows::{WidgetWindow, WidgetWindows},
    },
};

/// Actions offered by the overlay's context menu.
#[derive(Debug, Clone, Copy)]
enum MenuAction {
    Pin,
    ToggleExpanded,
    Center,
    ToggleWidget(WidgetKind),
    Settings,
    Hide,
    Quit,
}

#[derive(Default)]
pub struct UiOptions {
    pub start_hidden: bool,
}

pub struct UiController {
    pub windows: WidgetWindows,
    state: Rc<RefCell<UiState>>,
    app: gtk::Application,
    self_ref: RefCell<std::rc::Weak<UiController>>,
    media: RefCell<Option<MediaService>>,
    artwork_loader: RefCell<Option<ArtworkLoader>>,
    settings: RefCell<Option<settings::SettingsHandle>>,
    frame_source: RefCell<Option<glib::SourceId>>,
    progress_source: RefCell<Option<glib::SourceId>>,
    auto_source: RefCell<Option<glib::SourceId>>,
    clock_source: RefCell<Option<glib::SourceId>>,
    save_source: RefCell<Option<glib::SourceId>>,
    popover: RefCell<Option<gtk::Popover>>,
    quitting: Cell<bool>,
}

struct UiState {
    config: Config,
    snapshot: Option<PlayerSnapshot>,
    players: Vec<PlayerInfo>,
    morph: Morph,
    artwork: ArtworkVisual,
    hover_control: Option<Control>,
    focused_control: Option<Control>,
    focus_visible: bool,
    pointer_inside: bool,
    suppress_hover_until_leave: bool,
    seeking_seconds: Option<f64>,
    seek_dragging: bool,
    auto_expanded: bool,
    hover_expanded: bool,
    manual_expanded: bool,
    user_hidden: bool,
    force_show: bool,
    visible: bool,
    initial_snapshot_consumed: bool,
    last_track_key: Option<String>,
    pending_art_url: Option<String>,
    failed_art_url: Option<String>,
    art_request_id: u64,
    system_dark: bool,
    system_reduces_motion: bool,
    high_contrast: bool,
    settings_open: bool,
    last_error: Option<String>,
}

impl UiState {
    fn new(
        config: Config,
        options: UiOptions,
        system_dark: bool,
        system_reduces_motion: bool,
    ) -> Self {
        Self {
            config,
            snapshot: None,
            players: Vec::new(),
            morph: Morph::new_compact(),
            artwork: ArtworkVisual::new(),
            hover_control: None,
            focused_control: None,
            focus_visible: false,
            pointer_inside: false,
            suppress_hover_until_leave: false,
            seeking_seconds: None,
            seek_dragging: false,
            auto_expanded: false,
            hover_expanded: false,
            manual_expanded: false,
            user_hidden: options.start_hidden,
            force_show: false,
            visible: false,
            initial_snapshot_consumed: false,
            last_track_key: None,
            pending_art_url: None,
            failed_art_url: None,
            art_request_id: 0,
            system_dark,
            system_reduces_motion,
            high_contrast: false,
            settings_open: false,
            last_error: None,
        }
    }

    fn wants_expanded(&self) -> bool {
        self.config.behavior.pin_expanded
            || self.manual_expanded
            || self.hover_expanded
            || self.auto_expanded
    }

    fn reduced_motion(&self) -> bool {
        self.config
            .effective_reduced_motion(self.system_reduces_motion)
    }

    fn layout(&self, kind: WidgetKind) -> FrameLayout {
        layout::compute(
            kind,
            self.morph.value(),
            self.config.appearance.corner_radius,
            &self.config.widgets,
        )
    }

    fn should_be_visible(&self) -> bool {
        if self.settings_open {
            return false;
        }
        visible_for(
            &self.config,
            self.snapshot.as_ref(),
            self.user_hidden,
            self.force_show,
        )
    }

    fn clear_transient_expansion(&mut self) {
        self.auto_expanded = false;
        self.hover_expanded = false;
        self.manual_expanded = false;
    }
}

fn visible_for(
    config: &Config,
    snapshot: Option<&PlayerSnapshot>,
    user_hidden: bool,
    force_show: bool,
) -> bool {
    if user_hidden {
        return false;
    }
    if force_show {
        return true;
    }
    let Some(snapshot) = snapshot else {
        return !config.behavior.hide_when_idle;
    };
    match snapshot.status {
        PlaybackStatus::Playing => true,
        PlaybackStatus::Paused => config.behavior.show_when_paused,
        PlaybackStatus::Stopped => !config.behavior.hide_when_idle,
    }
}

fn track_key(snapshot: &PlayerSnapshot) -> String {
    format!("{}|{}", snapshot.bus_name, snapshot.track.identity())
}

impl UiController {
    pub fn build(app: &gtk::Application, config: Config, options: UiOptions) -> Result<Rc<Self>> {
        let (system_dark, system_reduces_motion, high_contrast) = system_ui_preferences();
        let mut state = UiState::new(config, options, system_dark, system_reduces_motion);
        state.high_contrast = high_contrast;
        let state = Rc::new(RefCell::new(state));

        let controller = Rc::new_cyclic(|weak| UiController {
            windows: WidgetWindows::new(),
            state: Rc::clone(&state),
            app: app.clone(),
            self_ref: RefCell::new(weak.clone()),
            media: RefCell::new(None),
            artwork_loader: RefCell::new(None),
            settings: RefCell::new(None),
            frame_source: RefCell::new(None),
            progress_source: RefCell::new(None),
            auto_source: RefCell::new(None),
            clock_source: RefCell::new(None),
            save_source: RefCell::new(None),
            popover: RefCell::new(None),
            quitting: Cell::new(false),
        });

        controller.sync_windows()?;
        controller.install_system_settings_watch();
        controller.install_monitor_watch();
        controller.start_services();
        controller.sync_visibility();
        controller.refresh_view();
        controller.start_clock_timer();
        Ok(controller)
    }

    /// Create windows for newly enabled widgets and wire their events.
    fn sync_windows(&self) -> Result<()> {
        let config = self.state.borrow().config.clone();
        let added = self.windows.sync(&self.app, &config)?;
        for kind in added {
            if let Some(window) = self.windows.get(kind) {
                self.install_window_events(&window);
            }
        }
        if self.windows.is_empty() {
            anyhow::bail!("no widget windows could be created");
        }
        Ok(())
    }

    fn reapply_all(&self, config: &Config) {
        for window in self.windows.all() {
            window.reapply(config);
        }
    }

    fn rc(&self) -> Rc<Self> {
        self.self_ref
            .borrow()
            .upgrade()
            .expect("controller is alive")
    }

    fn install_window_events(&self, window: &Rc<WidgetWindow>) {
        let kind = window.kind;
        let state = Rc::clone(&self.state);
        window.canvas.set_draw_func(move |_, cr, width, height| {
            let state = state.borrow();
            let view = ViewData {
                config: &state.config,
                snapshot: state.snapshot.as_ref(),
                artwork: &state.artwork,
                morph: state.morph.value(),
                hover_control: state.hover_control,
                focused_control: state.focused_control,
                seeking_seconds: state.seeking_seconds,
                now: Instant::now(),
                system_dark: state.system_dark,
                high_contrast: state.high_contrast,
            };
            draw::draw(kind, cr, f64::from(width), f64::from(height), &view);
        });

        if let Some(buttons) = &window.buttons {
            let weak = Rc::downgrade(&self.rc());
            buttons.play.connect_clicked(move |_| {
                if let Some(controller) = weak.upgrade() {
                    controller.clear_focus_visible();
                    controller.send_media(MediaCommand::PlayPause);
                    controller.note_interaction();
                }
            });
            let weak = Rc::downgrade(&self.rc());
            buttons.previous.connect_clicked(move |_| {
                if let Some(controller) = weak.upgrade() {
                    controller.clear_focus_visible();
                    controller.send_media(MediaCommand::Previous);
                    controller.note_interaction();
                }
            });
            let weak = Rc::downgrade(&self.rc());
            buttons.next.connect_clicked(move |_| {
                if let Some(controller) = weak.upgrade() {
                    controller.clear_focus_visible();
                    controller.send_media(MediaCommand::Next);
                    controller.note_interaction();
                }
            });
            for button in [&buttons.previous, &buttons.play, &buttons.next] {
                let weak = Rc::downgrade(&self.rc());
                button.connect_notify_local(Some("has-focus"), move |_, _| {
                    if let Some(controller) = weak.upgrade() {
                        controller.refresh_focus();
                    }
                });
                button.set_cursor_from_name(Some("pointer"));
            }
        }

        let motion = gtk::EventControllerMotion::new();
        let weak = Rc::downgrade(&self.rc());
        motion.connect_enter(move |_, x, y| {
            if let Some(controller) = weak.upgrade() {
                controller.on_pointer_enter(kind, x, y);
            }
        });
        let weak = Rc::downgrade(&self.rc());
        motion.connect_motion(move |_, x, y| {
            if let Some(controller) = weak.upgrade() {
                controller.on_pointer_motion(kind, x, y);
            }
        });
        let weak = Rc::downgrade(&self.rc());
        motion.connect_leave(move |_| {
            if let Some(controller) = weak.upgrade() {
                controller.on_pointer_leave(kind);
            }
        });
        window.fixed.add_controller(motion);

        let click = gtk::GestureClick::new();
        click.set_button(1);
        let weak = Rc::downgrade(&self.rc());
        click.connect_pressed(move |_, _presses, x, y| {
            if let Some(controller) = weak.upgrade() {
                controller.on_primary_press(kind, x, y);
            }
        });
        let weak = Rc::downgrade(&self.rc());
        click.connect_released(move |_, _presses, x, y| {
            if let Some(controller) = weak.upgrade() {
                controller.on_primary_release(kind, x, y);
            }
        });
        window.canvas.add_controller(click);

        let right_click = gtk::GestureClick::new();
        right_click.set_button(3);
        right_click.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(&self.rc());
        right_click.connect_pressed(move |gesture, _, x, y| {
            gesture.set_state(gtk::EventSequenceState::Claimed);
            if let Some(controller) = weak.upgrade() {
                controller.show_context_menu(x, y);
            }
        });
        window.fixed.add_controller(right_click);

        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(&self.rc());
        keys.connect_key_pressed(move |_, key, _code, modifiers| {
            if let Some(controller) = weak.upgrade() {
                controller.on_key(key, modifiers)
            } else {
                glib::Propagation::Proceed
            }
        });
        window.window.add_controller(keys);

        let anchor = Rc::clone(window);
        let weak = Rc::downgrade(&self.rc());
        window.window.connect_map(move |gtk_window| {
            if let Some(controller) = weak.upgrade() {
                let config = controller.state.borrow().config.clone();
                anchor
                    .backend
                    .borrow_mut()
                    .after_map(gtk_window, &anchor.placement(&config));
                controller.refresh_view();
            }
        });

        let weak = Rc::downgrade(&self.rc());
        window.window.connect_close_request(move |_| {
            if let Some(controller) = weak.upgrade() {
                controller.handle_action(Action::Hide);
            }
            glib::Propagation::Stop
        });

        window.set_cursor("grab");
    }

    fn install_system_settings_watch(self: &Rc<Self>) {
        if let Some(settings) = gtk::Settings::default() {
            for property in [
                "gtk-theme-name",
                "gtk-application-prefer-dark-theme",
                "gtk-enable-animations",
            ] {
                let weak = Rc::downgrade(self);
                settings.connect_notify_local(Some(property), move |_, _| {
                    if let Some(controller) = weak.upgrade() {
                        controller.refresh_system_preferences();
                    }
                });
            }
        }
    }

    fn install_monitor_watch(self: &Rc<Self>) {
        let Some(display) = gtk::gdk::Display::default() else {
            return;
        };
        let monitors = display.monitors();
        let weak = Rc::downgrade(self);
        monitors.connect_items_changed(move |_, _, _, _| {
            if let Some(controller) = weak.upgrade() {
                controller.on_display_changed();
            }
        });
    }

    fn start_services(self: &Rc<Self>) {
        let (art_tx, art_rx) = async_channel::unbounded();
        let artwork_callback: ArtworkCallback = Arc::new(move |response| {
            let _ = art_tx.try_send(response);
        });
        *self.artwork_loader.borrow_mut() = Some(ArtworkLoader::spawn(artwork_callback));
        let weak = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            while let Ok(response) = art_rx.recv().await {
                let Some(controller) = weak.upgrade() else {
                    break;
                };
                controller.on_artwork_response(response);
            }
        });

        let (media_tx, media_rx) = async_channel::unbounded();
        let media_callback: MediaCallback = Arc::new(move |event| {
            let _ = media_tx.try_send(event);
        });
        let weak = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            while let Ok(event) = media_rx.recv().await {
                let Some(controller) = weak.upgrade() else {
                    break;
                };
                controller.on_media_event(event);
            }
        });
        let media_config = self.state.borrow().config.media.clone();
        match MediaService::spawn(media_config, media_callback) {
            Ok(service) => {
                *self.media.borrow_mut() = Some(service);
            }
            Err(error) => {
                warn!(%error, "could not start MPRIS service");
                self.state.borrow_mut().last_error = Some(format!("{error:#}"));
            }
        }
        self.update_accessibility();
    }

    fn reset_window_focus(&self) {
        for window in self.windows.all() {
            gtk::prelude::GtkWindowExt::set_focus(&window.window, None::<&gtk::Widget>);
        }
        self.clear_focus_visible();
    }

    fn refresh_system_preferences(&self) {
        let (dark, reduced, high_contrast) = system_ui_preferences();
        let mut state = self.state.borrow_mut();
        state.system_dark = dark;
        state.system_reduces_motion = reduced;
        state.high_contrast = high_contrast;
        let should_snap = state.reduced_motion() && state.morph.is_active();
        if should_snap {
            let expanded = state.wants_expanded();
            state.morph.snap(expanded);
        }
        drop(state);
        if should_snap {
            self.stop_frame_loop();
        }
        self.refresh_view();
    }

    fn on_display_changed(&self) {
        let config = self.state.borrow().config.clone();
        self.reapply_all(&config);
        self.refresh_view();
        let weak = Rc::downgrade(&self.rc());
        glib::timeout_add_local_once(Duration::from_millis(120), move || {
            if let Some(controller) = weak.upgrade() {
                controller.refresh_view();
            }
        });
    }

    pub fn update_players(&self, players: Vec<PlayerInfo>) {
        let changed = {
            let mut state = self.state.borrow_mut();
            let changed = state.players != players;
            state.players = players.clone();
            changed
        };
        if changed && let Some(settings) = self.settings.borrow().as_ref() {
            settings.update_players(&players);
        }
    }

    fn on_media_event(&self, event: MediaEvent) {
        match event {
            MediaEvent::Snapshot(snapshot) => self.apply_snapshot(snapshot),
            MediaEvent::Players(players) => self.update_players(players),
            MediaEvent::Error(error) => {
                warn!(%error, "media service error");
                self.state.borrow_mut().last_error = Some(error);
            }
        }
    }

    fn apply_snapshot(&self, snapshot: Option<PlayerSnapshot>) {
        let now = Instant::now();
        let new_key = snapshot.as_ref().map(track_key);
        let (new_track, first_snapshot, previous) = {
            let state = self.state.borrow();
            let old_key = state.last_track_key.clone();
            let first = !state.initial_snapshot_consumed && snapshot.is_some();
            (
                new_key.is_some() && new_key != old_key,
                first,
                state.snapshot.as_ref().map(|snap| snap.status),
            )
        };

        {
            let mut state = self.state.borrow_mut();
            if new_key.is_none() {
                state.seeking_seconds = None;
                state.seek_dragging = false;
            }
            state.snapshot = snapshot.clone();
            state.last_track_key = new_key;
            if snapshot.is_some() {
                state.initial_snapshot_consumed = true;
            }
        }

        if let Some(snapshot) = snapshot.as_ref() {
            self.request_artwork(snapshot);
            if new_track || !matches!(previous, Some(PlaybackStatus::Playing)) {
                self.update_accessibility();
            }
        } else {
            {
                let mut state = self.state.borrow_mut();
                state.pending_art_url = None;
                state.failed_art_url = None;
                state.artwork.show_fallback(now);
            }
            self.start_frame_loop();
        }

        self.sync_visibility();

        if new_track && !first_snapshot {
            let auto_expand = {
                let state = self.state.borrow();
                state.config.behavior.auto_expand_on_new_track
                    && snapshot
                        .as_ref()
                        .is_some_and(|snap| snap.status == PlaybackStatus::Playing)
                    && !state.config.behavior.pin_expanded
                    && !state.user_hidden
            };
            if auto_expand {
                {
                    let mut state = self.state.borrow_mut();
                    state.auto_expanded = true;
                    state.manual_expanded = false;
                    if state.pointer_inside {
                        state.hover_expanded = false;
                    }
                }
                self.apply_expansion_target();
                self.schedule_auto_collapse();
                info!(
                    track = ?snapshot.as_ref().map(|snap| snap.track.title.clone()),
                    "new track; expanding overlay"
                );
            }
        } else if first_snapshot && self.state.borrow().config.behavior.pin_expanded {
            self.apply_expansion_target();
        }

        self.sync_visibility();
        self.refresh_view();
        self.update_progress_timer();
        self.schedule_auto_collapse_if_needed();
    }

    fn request_artwork(&self, snapshot: &PlayerSnapshot) {
        let url = snapshot.track.art_url.as_deref();
        let allow_remote = self.state.borrow().config.media.allow_remote_artwork;
        let (should_request, request_url, failed_same) = {
            let state = self.state.borrow();
            match url {
                None => (false, None, false),
                Some(url) => {
                    let already_loaded = state.artwork.url.as_deref() == Some(url);
                    let already_pending = state.pending_art_url.as_deref() == Some(url);
                    let failed_same = state.failed_art_url.as_deref() == Some(url);
                    (
                        !already_loaded && !already_pending && !failed_same,
                        Some(url.to_owned()),
                        failed_same,
                    )
                }
            }
        };

        if let Some(url) = request_url {
            if should_request {
                let request = self
                    .artwork_loader
                    .borrow()
                    .as_ref()
                    .and_then(|loader| loader.request(&url, allow_remote));
                let mut state = self.state.borrow_mut();
                state.failed_art_url = None;
                if let Some(id) = request {
                    state.art_request_id = id;
                    state.pending_art_url = Some(url);
                } else {
                    state.pending_art_url = None;
                    state.failed_art_url = Some(url);
                    state.artwork.show_fallback(Instant::now());
                    drop(state);
                    self.start_frame_loop();
                    return;
                }
                drop(state);
            }
            return;
        }

        if failed_same {
            return;
        }
        let has_art = {
            let state = self.state.borrow();
            state.artwork.url.is_some() || state.pending_art_url.is_some()
        };
        if has_art {
            {
                let mut state = self.state.borrow_mut();
                state.pending_art_url = None;
                state.artwork.show_fallback(Instant::now());
            }
            self.start_frame_loop();
        }
    }

    fn on_artwork_response(&self, response: ArtworkResponse) {
        match response {
            ArtworkResponse::Loaded { id, artwork } => {
                let loaded = {
                    let mut state = self.state.borrow_mut();
                    if id != state.art_request_id {
                        return;
                    }
                    let url = state.pending_art_url.clone();
                    state.pending_art_url = None;
                    state.failed_art_url = None;
                    if let Some(url) = url.as_deref() {
                        state.artwork.update(&artwork, url, Instant::now());
                    }
                    url.is_some()
                };
                if loaded {
                    debug!("artwork loaded");
                    self.start_frame_loop();
                    self.refresh_view();
                }
            }
            ArtworkResponse::Failed { id, error } => {
                let failed_url = {
                    let mut state = self.state.borrow_mut();
                    if id != state.art_request_id {
                        return;
                    }
                    let failed_url = state.pending_art_url.take();
                    state.failed_art_url = failed_url.clone();
                    state.artwork.show_fallback(Instant::now());
                    failed_url
                };
                if let Some(url) = failed_url {
                    debug!(%error, %url, "artwork unavailable; using fallback");
                } else {
                    debug!(%error, "artwork unavailable; using fallback");
                }
                self.start_frame_loop();
                self.refresh_view();
            }
        }
    }

    fn update_accessibility(&self) {
        let (title, artist) = {
            let state = self.state.borrow();
            match state.snapshot.as_ref() {
                Some(snapshot) => (snapshot.track.title.clone(), snapshot.track.artist_line()),
                None => (
                    "Nothing playing".to_owned(),
                    "Start a media player".to_owned(),
                ),
            }
        };
        let Some(media) = self.windows.media() else {
            return;
        };
        let window_title = format!("KOKO WIDGET — {title}");
        media.window.set_title(Some(&window_title));
        media.canvas.update_property(&[
            gtk::accessible::Property::Label(&format!("{title}, {artist}")),
            gtk::accessible::Property::Description(
                "Floating music controls. Space play or pause, arrow keys seek, N next, P previous, E expand, M pin.",
            ),
            gtk::accessible::Property::KeyShortcuts("Space;Left;Right;N;P;E;M;Escape"),
        ]);
        let playing = self
            .state
            .borrow()
            .snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.status == PlaybackStatus::Playing);
        let play_label = if playing { "Pause" } else { "Play" };
        if let Some(buttons) = &media.buttons {
            buttons
                .play
                .update_property(&[gtk::accessible::Property::Label(play_label)]);
            buttons
                .previous
                .update_property(&[gtk::accessible::Property::Label("Previous track")]);
            buttons
                .next
                .update_property(&[gtk::accessible::Property::Label("Next track")]);
            buttons.play.set_tooltip_text(Some(play_label));
        }
    }

    fn clear_focus_visible(&self) {
        {
            let mut state = self.state.borrow_mut();
            if !state.focus_visible {
                state.focused_control = None;
                return;
            }
            state.focus_visible = false;
            state.focused_control = None;
        }
        self.windows.queue_draw_all();
    }

    fn refresh_focus(&self) {
        let show = self.state.borrow().focus_visible;
        if !show {
            let mut state = self.state.borrow_mut();
            if state.focused_control.is_some() {
                state.focused_control = None;
                drop(state);
                self.windows.queue_draw_all();
            }
            return;
        }
        let Some(media) = self.windows.media() else {
            return;
        };
        let Some(buttons) = &media.buttons else {
            return;
        };
        let focused = if buttons.previous.has_focus() {
            Some(Control::Previous)
        } else if buttons.play.has_focus() {
            Some(Control::PlayPause)
        } else if buttons.next.has_focus() {
            Some(Control::Next)
        } else {
            None
        };
        let mut state = self.state.borrow_mut();
        if state.focused_control != focused {
            state.focused_control = focused;
            drop(state);
            self.windows.queue_draw_all();
        }
    }

    fn refresh_view(&self) {
        let (visible, enabled, play_label) = {
            let state = self.state.borrow();
            let capabilities = state
                .snapshot
                .as_ref()
                .map(|snapshot| snapshot.capabilities);
            let previous_enabled = capabilities.is_some_and(|caps| caps.can_go_previous);
            let next_enabled = capabilities.is_some_and(|caps| caps.can_go_next);
            let play_enabled = capabilities
                .is_some_and(|caps| caps.can_control && (caps.can_play || caps.can_pause));
            let playing = state
                .snapshot
                .as_ref()
                .is_some_and(|snapshot| snapshot.status == PlaybackStatus::Playing);
            (
                state.visible,
                (previous_enabled, play_enabled, next_enabled),
                if playing { "Pause" } else { "Play" }.to_owned(),
            )
        };

        for window in self.windows.all() {
            let layout = self.state.borrow().layout(window.kind);
            if let Some(buttons) = &window.buttons {
                let move_button =
                    |button: &gtk::Button, rect: Rect, show: bool, sensitive: bool| {
                        button.set_visible(show);
                        button.set_sensitive(sensitive);
                        window.fixed.move_(button, rect.x, rect.y);
                        button.set_size_request(
                            layout::CONTROL_SIZE as i32,
                            layout::CONTROL_SIZE as i32,
                        );
                    };
                move_button(
                    &buttons.previous,
                    layout.previous_hit,
                    visible && layout.previous_alpha > 0.03,
                    visible && enabled.0,
                );
                move_button(&buttons.play, layout.play_hit, visible, visible && enabled.1);
                move_button(&buttons.next, layout.next_hit, visible, visible && enabled.2);
                if !buttons.play.has_focus() {
                    buttons
                        .play
                        .update_property(&[gtk::accessible::Property::Label(&play_label)]);
                }
            }
            window.queue_draw();
            window.update_input(&layout);
        }
    }

    fn sync_visibility(&self) {
        let should_show = self.state.borrow().should_be_visible();
        let currently_visible = self.state.borrow().visible;
        if should_show && !currently_visible {
            {
                let mut state = self.state.borrow_mut();
                state.visible = true;
                state.pointer_inside = false;
            }
            for window in self.windows.all() {
                window.show();
            }
            self.reset_window_focus();
            self.refresh_view();
            self.update_progress_timer();
            self.start_clock_timer();
            let weak = Rc::downgrade(&self.rc());
            glib::idle_add_local_once(move || {
                if let Some(controller) = weak.upgrade() {
                    controller.refresh_view();
                }
            });
            self.schedule_auto_collapse_if_needed();
            if self.state.borrow().config.behavior.pin_expanded {
                self.apply_expansion_target();
            }
        } else if !should_show && currently_visible {
            self.hide_overlay();
        }
    }

    fn hide_overlay(&self) {
        self.cancel_auto_collapse();
        self.stop_frame_loop();
        self.stop_progress_timer();
        self.stop_clock_timer();
        {
            let mut state = self.state.borrow_mut();
            if !state.visible {
                return;
            }
            state.visible = false;
            state.pointer_inside = false;
            state.hover_control = None;
            state.focused_control = None;
            state.seeking_seconds = None;
            state.seek_dragging = false;
            state.clear_transient_expansion();
            state.morph.snap(false);
        }
        for window in self.windows.all() {
            window.cancel_drag();
            window.hide();
        }
    }

    fn apply_expansion_target(&self) {
        let (expanded, reduced, was_active) = {
            let state = self.state.borrow();
            (
                state.wants_expanded(),
                state.reduced_motion(),
                state.morph.is_active(),
            )
        };
        if reduced {
            self.stop_frame_loop();
            {
                let mut state = self.state.borrow_mut();
                state.morph.snap(expanded);
            }
            self.refresh_view();
            self.update_progress_timer();
            return;
        }
        debug!(expanded, was_active, "applying expansion target");
        let now = Instant::now();
        let speed = self.state.borrow().config.animation_scale();
        {
            let mut state = self.state.borrow_mut();
            state.morph.retarget(expanded, now, speed, reduced);
        }
        if was_active || self.state.borrow().morph.is_active() {
            self.start_frame_loop();
        }
        self.update_progress_timer();
    }

    /// Keep the clock and date widgets current. One timer aligned to the next
    /// whole second is cheaper and smoother than polling at 60 Hz.
    fn start_clock_timer(&self) {
        if self.clock_source.borrow().is_some() {
            return;
        }
        let weak = Rc::downgrade(&self.rc());
        let source = glib::timeout_add_local(Duration::from_millis(500), move || {
            let Some(controller) = weak.upgrade() else {
                return ControlFlow::Break;
            };
            if controller.clock_should_continue() {
                controller.windows.queue_draw_all();
                ControlFlow::Continue
            } else {
                *controller.clock_source.borrow_mut() = None;
                ControlFlow::Break
            }
        });
        *self.clock_source.borrow_mut() = Some(source);
    }

    fn clock_should_continue(&self) -> bool {
        let state = self.state.borrow();
        state.visible && (state.config.widgets.clock.enabled || state.config.widgets.date.enabled)
    }

    fn stop_clock_timer(&self) {
        if let Some(source) = self.clock_source.borrow_mut().take() {
            source.remove();
        }
    }

    fn start_frame_loop(&self) {
        if self.frame_source.borrow().is_some() {
            return;
        }
        let weak = Rc::downgrade(&self.rc());
        let source = glib::timeout_add_local(Duration::from_millis(16), move || {
            let Some(controller) = weak.upgrade() else {
                return ControlFlow::Break;
            };
            if controller.tick_frame() {
                ControlFlow::Continue
            } else {
                *controller.frame_source.borrow_mut() = None;
                ControlFlow::Break
            }
        });
        *self.frame_source.borrow_mut() = Some(source);
    }

    fn stop_frame_loop(&self) {
        if let Some(source) = self.frame_source.borrow_mut().take() {
            source.remove();
        }
    }

    fn tick_frame(&self) -> bool {
        let now = Instant::now();
        let mut state = self.state.borrow_mut();
        let was_active = state.morph.is_active();
        state.morph.tick(now);
        state.artwork.invalidate_transition_if_complete(now);
        let still_animating = state.morph.is_active() || state.artwork.has_pending_transition();
        let completed = was_active && !state.morph.is_active();
        drop(state);
        self.refresh_view();
        if completed {
            self.update_progress_timer();
            self.schedule_auto_collapse_if_needed();
        }
        still_animating
    }

    fn update_progress_timer(&self) {
        let should_run = {
            let state = self.state.borrow();
            state.visible
                && state.morph.phase() == Phase::Expanded
                && state.snapshot.as_ref().is_some_and(|snapshot| {
                    snapshot.status == PlaybackStatus::Playing
                        && snapshot.duration_seconds().is_some()
                })
                && state.seeking_seconds.is_none()
        };
        if should_run && self.progress_source.borrow().is_none() {
            let weak = Rc::downgrade(&self.rc());
            let source = glib::timeout_add_local(Duration::from_millis(250), move || {
                let Some(controller) = weak.upgrade() else {
                    return ControlFlow::Break;
                };
                if controller.progress_should_continue() {
                    controller.windows.queue_draw_all();
                    ControlFlow::Continue
                } else {
                    *controller.progress_source.borrow_mut() = None;
                    ControlFlow::Break
                }
            });
            *self.progress_source.borrow_mut() = Some(source);
        } else if !should_run {
            self.stop_progress_timer();
        }
    }

    fn progress_should_continue(&self) -> bool {
        let state = self.state.borrow();
        state.visible
            && state.morph.phase() == Phase::Expanded
            && state.snapshot.as_ref().is_some_and(|snapshot| {
                snapshot.status == PlaybackStatus::Playing && snapshot.duration_seconds().is_some()
            })
            && state.seeking_seconds.is_none()
    }

    fn stop_progress_timer(&self) {
        if let Some(source) = self.progress_source.borrow_mut().take() {
            source.remove();
        }
    }

    fn schedule_auto_collapse_if_needed(&self) {
        self.cancel_auto_collapse();
        let eligible = {
            let state = self.state.borrow();
            state.auto_expanded
                && !state.pointer_inside
                && !state.manual_expanded
                && !state.config.behavior.pin_expanded
                && state.visible
        };
        if !eligible {
            return;
        }
        let timeout = self.state.borrow().config.behavior.auto_collapse_seconds;
        let weak = Rc::downgrade(&self.rc());
        let source =
            glib::timeout_add_local_once(Duration::from_secs_f64(timeout.max(0.5)), move || {
                if let Some(controller) = weak.upgrade() {
                    *controller.auto_source.borrow_mut() = None;
                    controller.on_auto_collapse_timeout();
                }
            });
        *self.auto_source.borrow_mut() = Some(source);
    }

    fn schedule_auto_collapse(&self) {
        self.schedule_auto_collapse_if_needed();
    }

    fn cancel_auto_collapse(&self) {
        if let Some(source) = self.auto_source.borrow_mut().take() {
            source.remove();
        }
    }

    fn on_auto_collapse_timeout(&self) {
        let collapse = {
            let state = self.state.borrow();
            state.auto_expanded
                && !state.pointer_inside
                && !state.manual_expanded
                && !state.config.behavior.pin_expanded
        };
        if !collapse {
            return;
        }
        info!("auto-collapse timeout elapsed");
        {
            let mut state = self.state.borrow_mut();
            state.auto_expanded = false;
            state.hover_expanded = false;
        }
        self.apply_expansion_target();
    }

    fn note_interaction(&self) {
        self.cancel_auto_collapse();
    }

    pub fn apply_config(&self, mut config: Config, save: bool) {
        let old = self.state.borrow().config.clone();
        let old_pin = old.behavior.pin_expanded;
        let old_position = old.position.clone();
        let old_media = old.media.clone();
        let old_widgets = old.widgets.clone();
        let old_visible_rules = (old.behavior.hide_when_idle, old.behavior.show_when_paused);
        for warning in config.sanitize() {
            warn!("{warning}");
        }
        let new_pin = config.behavior.pin_expanded;
        self.state.borrow_mut().config = config.clone();
        if old_widgets != config.widgets {
            if let Err(error) = self.sync_windows() {
                warn!(%error, "could not create widget windows");
            }
        }
        if old_position != config.position || old_widgets != config.widgets {
            self.resize_canvas(&config);
            self.reapply_all(&config);
            if self.state.borrow().visible {
                for window in self.windows.all() {
                    window.show();
                }
            }
        }
        if old_media != config.media
            && let Some(service) = self.media.borrow().as_ref()
        {
            service.configure(config.media.clone());
        }
        if old_pin != new_pin {
            let mut state = self.state.borrow_mut();
            state.auto_expanded = false;
            state.hover_expanded = false;
            state.manual_expanded = false;
        }
        let new_rules = (
            config.behavior.hide_when_idle,
            config.behavior.show_when_paused,
        );
        if old_visible_rules != new_rules {
            self.sync_visibility();
        }
        let snap_animation = {
            let state = self.state.borrow();
            state.reduced_motion() && state.morph.is_active()
        };
        if snap_animation {
            self.stop_frame_loop();
            let mut state = self.state.borrow_mut();
            let expanded = state.wants_expanded();
            state.morph.snap(expanded);
        }
        if old_pin != new_pin {
            self.apply_expansion_target();
        }
        self.schedule_auto_collapse_if_needed();
        self.refresh_view();
        self.update_progress_timer();
        if save {
            self.debounce_save_config();
        }
    }

    /// Match the transparent canvas to the configured widget row.
    /// Match every transparent canvas to its configured card size.
    fn resize_canvas(&self, config: &Config) {
        for window in self.windows.all() {
            window.resize_canvas(config);
        }
    }

    fn debounce_save_config(&self) {
        self.cancel_save();
        let weak = Rc::downgrade(&self.rc());
        let source = glib::timeout_add_local_once(Duration::from_millis(400), move || {
            if let Some(controller) = weak.upgrade() {
                *controller.save_source.borrow_mut() = None;
                controller.save_now();
            }
        });
        *self.save_source.borrow_mut() = Some(source);
    }

    fn cancel_save(&self) {
        if let Some(source) = self.save_source.borrow_mut().take() {
            source.remove();
        }
    }

    pub fn save_now(&self) {
        self.cancel_save();
        let config = self.state.borrow().config.clone();
        if let Err(error) = config.save() {
            warn!(%error, "could not save configuration");
        }
        if let Err(error) = config.sync_autostart() {
            warn!(%error, "could not update XDG autostart entry");
        }
    }

    fn toggle_pin(&self) {
        let pinned = {
            let mut state = self.state.borrow_mut();
            state.config.behavior.pin_expanded = !state.config.behavior.pin_expanded;
            state.auto_expanded = false;
            state.hover_expanded = false;
            state.manual_expanded = false;
            state.config.behavior.pin_expanded
        };
        self.apply_expansion_target();
        self.schedule_auto_collapse_if_needed();
        self.refresh_view();
        self.debounce_save_config();
        info!(pinned, "expanded state pin toggled");
    }

    fn show_context_menu(&self, x: f64, y: f64) {
        let Some(anchor) = self
            .windows
            .media()
            .or_else(|| self.windows.all().first().cloned())
        else {
            return;
        };
        if let Some(existing) = self.popover.borrow_mut().take() {
            existing.popdown();
            existing.unparent();
        }
        let popover = gtk::Popover::new();
        popover.set_autohide(true);
        popover.set_has_arrow(false);
        popover.set_position(gtk::PositionType::Bottom);
        popover.add_css_class("koko-menu");
        popover.set_parent(&anchor.fixed);

        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.add_css_class("koko-menu-content");
        content.set_size_request(236, -1);

        // A short header gives the menu a subject instead of being a bare list.
        let header = gtk::Box::new(gtk::Orientation::Vertical, 1);
        header.add_css_class("koko-menu-header");
        let heading = gtk::Label::new(Some(crate::APP_NAME));
        heading.add_css_class("koko-menu-title");
        heading.set_xalign(0.0);
        let (title, artist) = {
            let state = self.state.borrow();
            match state.snapshot.as_ref() {
                Some(snapshot) => (snapshot.track.title.clone(), snapshot.track.artist_line()),
                None => (
                    "Nothing playing".to_owned(),
                    "No player connected".to_owned(),
                ),
            }
        };
        let subtitle = gtk::Label::new(Some(if artist.is_empty() { &title } else { &artist }));
        subtitle.add_css_class("koko-menu-subtitle");
        subtitle.set_xalign(0.0);
        subtitle.set_ellipsize(pango::EllipsizeMode::End);
        header.append(&heading);
        header.append(&subtitle);
        content.append(&header);

        let group = |title: &str| {
            let label = gtk::Label::new(Some(title));
            label.add_css_class("koko-menu-group");
            label.set_xalign(0.0);
            label
        };

        content.append(&group("Window"));

        let pinned = self.state.borrow().config.behavior.pin_expanded;
        let expanded = self.state.borrow().wants_expanded();
        for (label, action) in [
            (
                if pinned {
                    "Unpin expanded"
                } else {
                    "Pin expanded"
                },
                MenuAction::Pin,
            ),
            (
                if expanded { "Collapse" } else { "Expand" },
                MenuAction::ToggleExpanded,
            ),
            ("Center on screen", MenuAction::Center),
        ] {
            content.append(&self.menu_item(label, action, &popover, None));
        }

        content.append(&group("Configure"));

        let toggles: Vec<(WidgetKind, &str)> = WidgetKind::ALL
            .iter()
            .map(|kind| (*kind, kind.label()))
            .collect();
        for (kind, label) in toggles {
            let enabled = self.state.borrow().config.widgets.is_enabled(kind);
            content.append(&self.menu_item(
                label,
                MenuAction::ToggleWidget(kind),
                &popover,
                Some(enabled),
            ));
        }
        content.append(&self.menu_item("All settings…", MenuAction::Settings, &popover, None));

        let footer = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        footer.add_css_class("koko-menu-footer");
        let hide = self.menu_item("Hide", MenuAction::Hide, &popover, None);
        hide.set_hexpand(true);
        footer.append(&hide);
        let quit = self.menu_item("Quit", MenuAction::Quit, &popover, None);
        quit.add_css_class("koko-menu-danger");
        quit.set_hexpand(true);
        footer.append(&quit);
        content.append(&footer);

        popover.set_child(Some(&content));
        popover.set_pointing_to(Some(&gdk::Rectangle::new(
            x.round() as i32,
            y.round() as i32,
            1,
            1,
        )));
        popover.connect_closed({
            let popover = popover.clone();
            move |_| popover.unparent()
        });
        popover.popup();
        *self.popover.borrow_mut() = Some(popover);
    }

    /// One menu row: label on the left, optional state on the right.
    fn menu_item(
        &self,
        label: &str,
        action: MenuAction,
        popover: &gtk::Popover,
        checked: Option<bool>,
    ) -> gtk::Button {
        let button = gtk::Button::new();
        button.add_css_class("koko-menu-item");
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        let text = gtk::Label::new(Some(label));
        text.set_xalign(0.0);
        text.set_hexpand(true);
        row.append(&text);
        if let Some(checked) = checked {
            let state = gtk::Label::new(Some(if checked { "On" } else { "Off" }));
            state.add_css_class(if checked {
                "koko-menu-on"
            } else {
                "koko-menu-off"
            });
            row.append(&state);
        }
        button.set_child(Some(&row));
        button.set_halign(gtk::Align::Fill);
        button.set_hexpand(true);
        button.update_property(&[gtk::accessible::Property::Label(label)]);
        let weak = Rc::downgrade(&self.rc());
        let popover = popover.clone();
        button.connect_clicked(move |_| {
            popover.popdown();
            if let Some(controller) = weak.upgrade() {
                controller.perform_menu_action(action);
            }
        });
        button
    }

    fn perform_menu_action(&self, action: MenuAction) {
        match action {
            MenuAction::Pin => self.toggle_pin(),
            MenuAction::ToggleExpanded => self.toggle_expanded_from_click(),
            MenuAction::Center => self.reset_position(),
            MenuAction::ToggleWidget(kind) => {
                let enabled = self.state.borrow().config.widgets.is_enabled(kind);
                let mut config = self.state.borrow().config.clone();
                config.widgets.set_enabled(kind, !enabled);
                self.apply_config(config, true);
            }
            MenuAction::Settings => self.open_settings(),
            MenuAction::Hide => self.handle_action(Action::Hide),
            MenuAction::Quit => self.handle_action(Action::Quit),
        }
    }

    fn reset_position(&self) {
        {
            let mut state = self.state.borrow_mut();
            state.config.position.mode = crate::config::PositionMode::Center;
            state.config.widgets.positions.clear();
        }
        let config = self.state.borrow().config.clone();
        self.reapply_all(&config);
        self.refresh_view();
        self.debounce_save_config();
        info!("widget positions reset to the screen centre");
    }

    fn on_pointer_enter(&self, kind: WidgetKind, x: f64, y: f64) {
        debug!(?kind, x, y, "pointer entered widget");
        let mut expanded_by_hover = false;
        {
            let mut state = self.state.borrow_mut();
            if !state.visible {
                return;
            }
            state.pointer_inside = true;
            self.cancel_auto_collapse();
            if kind == WidgetKind::Media
                && state.config.behavior.expand_on_hover
                && !state.wants_expanded()
                && !state.suppress_hover_until_leave
            {
                state.hover_expanded = true;
                expanded_by_hover = true;
            }
        }
        if expanded_by_hover {
            self.apply_expansion_target();
        }
        self.on_pointer_motion(kind, x, y);
    }

    fn on_pointer_motion(&self, kind: WidgetKind, x: f64, y: f64) {
        let Some(window) = self.windows.get(kind) else {
            return;
        };
        let seeking = {
            let state = self.state.borrow();
            kind == WidgetKind::Media && state.seek_dragging && state.seeking_seconds.is_some()
        };
        if seeking {
            self.update_seek_from_pointer(kind, x);
            return;
        }
        if let Some((origin_x, origin_y)) = window.press_origin() {
            let dx = x - origin_x;
            let dy = y - origin_y;
            if !window.is_dragging() && dx.hypot(dy) > 5.0 {
                let config = self.state.borrow().config.clone();
                if window.begin_drag(&config) {
                    self.state.borrow_mut().hover_control = None;
                } else {
                    window.set_press_origin(None);
                }
            }
            if window.is_dragging() {
                self.cancel_auto_collapse();
                window.drag_by(dx, dy);
                window.set_cursor("grabbing");
                return;
            }
        }
        let (control, cursor) = {
            let state = self.state.borrow();
            if !state.visible {
                return;
            }
            let layout = state.layout(kind);
            let capabilities = state.snapshot.as_ref().map(|snap| snap.capabilities);
            let control = if kind == WidgetKind::Media {
                control_at(&layout, capabilities, x, y)
            } else {
                None
            };
            let over_progress = kind == WidgetKind::Media
                && layout.progress_hit.contains(x, y)
                && state
                    .snapshot
                    .as_ref()
                    .is_some_and(|snap| snap.capabilities.can_seek)
                && layout.progress_alpha > 0.4;
            let inside = layout.card.rect.contains(x, y);
            let cursor = if over_progress {
                "ew-resize"
            } else if inside {
                "grab"
            } else {
                "default"
            };
            (control, cursor)
        };
        let mut state = self.state.borrow_mut();
        let changed = state.hover_control != control;
        state.hover_control = control;
        drop(state);
        window.set_cursor(cursor);
        if changed {
            window.queue_draw();
        }
    }

    fn on_pointer_leave(&self, kind: WidgetKind) {
        debug!(?kind, "pointer left widget");
        if let Some(window) = self.windows.get(kind) {
            window.set_press_origin(None);
            if window.is_dragging() {
                if let Some(placement) = window.end_drag() {
                    self.persist_drag(kind, placement);
                }
            }
            window.set_cursor("grab");
            window.queue_draw();
        }
        let (was_hover_expanded, auto_expanded, manual, pinned) = {
            let mut state = self.state.borrow_mut();
            state.pointer_inside = false;
            state.hover_control = None;
            state.suppress_hover_until_leave = false;
            (
                state.hover_expanded,
                state.auto_expanded,
                state.manual_expanded,
                state.config.behavior.pin_expanded,
            )
        };
        if was_hover_expanded && !manual && !auto_expanded && !pinned {
            let mut state = self.state.borrow_mut();
            state.hover_expanded = false;
            drop(state);
            self.apply_expansion_target();
        } else if auto_expanded && !manual && !pinned {
            self.schedule_auto_collapse();
        }
    }

    fn on_primary_press(&self, kind: WidgetKind, x: f64, y: f64) {
        self.clear_focus_visible();
        let Some(window) = self.windows.get(kind) else {
            return;
        };
        let seek = {
            let state = self.state.borrow();
            if !state.visible {
                return;
            }
            if kind != WidgetKind::Media {
                None
            } else {
                let layout = state.layout(kind);
                let can_seek = state
                    .snapshot
                    .as_ref()
                    .is_some_and(|snap| snap.capabilities.can_seek);
                let duration = state
                    .snapshot
                    .as_ref()
                    .and_then(|snap| snap.duration_seconds());
                if can_seek
                    && layout.progress_alpha > 0.4
                    && layout.progress_hit.contains(x, y)
                    && duration.is_some()
                {
                    let fraction = ((x - layout.progress_track.x) / layout.progress_track.width)
                        .clamp(0.0, 1.0);
                    Some((fraction, duration.unwrap_or(0.0)))
                } else {
                    None
                }
            }
        };
        if let Some((fraction, duration)) = seek {
            self.cancel_auto_collapse();
            window.set_press_origin(None);
            {
                let mut state = self.state.borrow_mut();
                state.seeking_seconds = Some(fraction * duration);
                state.seek_dragging = true;
            }
            self.stop_progress_timer();
            window.queue_draw();
        } else {
            // A press can become a click (toggle or menu) or a drag (move this
            // window). Nothing is toggled until release, so dragging does not
            // flicker the expanded state.
            window.set_press_origin(Some((x, y)));
            self.cancel_auto_collapse();
        }
    }

    fn on_primary_release(&self, kind: WidgetKind, x: f64, y: f64) {
        let Some(window) = self.windows.get(kind) else {
            return;
        };
        if self.state.borrow().seek_dragging {
            self.finish_seek(kind, x);
            return;
        }
        if window.is_dragging() {
            window.set_press_origin(None);
            let placement = window.end_drag();
            window.set_cursor("grab");
            if let Some(placement) = placement {
                self.persist_drag(kind, placement);
            }
            self.schedule_auto_collapse_if_needed();
            window.queue_draw();
            return;
        }
        let Some((origin_x, origin_y)) = window.press_origin() else {
            return;
        };
        window.set_press_origin(None);
        // A large movement without a supported drag backend is not a click.
        if (x - origin_x).hypot(y - origin_y) <= 6.0 {
            if kind == WidgetKind::Media {
                self.toggle_expanded_from_click();
            } else {
                self.show_context_menu(x, y);
            }
        }
    }

    fn persist_drag(&self, kind: WidgetKind, placement: crate::window::DragPlacement) {
        info!(
            ?kind,
            display = ?placement.display,
            x = placement.x,
            y = placement.y,
            "widget moved"
        );
        {
            let mut state = self.state.borrow_mut();
            let mut spot = crate::config::WidgetSpot {
                display: placement.display.clone(),
                x: placement.x,
                y: placement.y,
            };
            if spot.display.is_none() {
                spot.display = state.config.position.display.clone();
            }
            state.config.widgets.positions.set(kind, Some(spot));
        }
        let config = self.state.borrow().config.clone();
        self.reapply_all(&config);
        self.refresh_view();
        self.debounce_save_config();
    }

    fn finish_seek(&self, kind: WidgetKind, x: f64) {
        let target = {
            let state = self.state.borrow();
            let duration = state
                .snapshot
                .as_ref()
                .and_then(|snap| snap.duration_seconds());
            duration.map(|duration| {
                let layout = state.layout(kind);
                let fraction = ((x - layout.progress_track.x) / layout.progress_track.width)
                    .clamp(0.0, 1.0);
                fraction * duration
            })
        };
        {
            let mut state = self.state.borrow_mut();
            state.seek_dragging = false;
            state.seeking_seconds = None;
            if let (Some(snapshot), Some(target)) = (state.snapshot.as_mut(), target) {
                snapshot.position_us = (target * 1_000_000.0) as i64;
                snapshot.captured_at = Instant::now();
            }
        }
        if let Some(target) = target {
            info!(target_seconds = target, "seek committed");
            self.send_media(MediaCommand::SeekTo((target * 1_000_000.0) as i64));
        }
        self.update_progress_timer();
        self.schedule_auto_collapse_if_needed();
        if let Some(window) = self.windows.get(kind) {
            window.queue_draw();
        }
    }

    fn update_seek_from_pointer(&self, kind: WidgetKind, x: f64) {
        let (duration, fraction) = {
            let state = self.state.borrow();
            let duration = state
                .snapshot
                .as_ref()
                .and_then(|snap| snap.duration_seconds());
            let layout = state.layout(kind);
            let fraction = ((x - layout.progress_track.x) / layout.progress_track.width)
                .clamp(0.0, 1.0);
            (duration, fraction)
        };
        if let Some(duration) = duration {
            let mut state = self.state.borrow_mut();
            if state.seek_dragging {
                state.seeking_seconds = Some(fraction * duration);
                drop(state);
                if let Some(window) = self.windows.get(kind) {
                    window.queue_draw();
                }
            }
        }
    }

    fn toggle_expanded_from_click(&self) {
        let was_expanded = self.state.borrow().wants_expanded();
        if was_expanded {
            let unpin = {
                let mut state = self.state.borrow_mut();
                let unpin = state.config.behavior.pin_expanded;
                state.config.behavior.pin_expanded = false;
                state.auto_expanded = false;
                state.hover_expanded = false;
                state.manual_expanded = false;
                state.suppress_hover_until_leave = state.pointer_inside;
                unpin
            };
            if unpin {
                self.debounce_save_config();
            }
            self.cancel_auto_collapse();
            self.apply_expansion_target();
        } else {
            {
                let mut state = self.state.borrow_mut();
                state.manual_expanded = true;
                state.auto_expanded = false;
                state.hover_expanded = false;
            }
            self.cancel_auto_collapse();
            self.apply_expansion_target();
        }
        self.refresh_view();
    }

    fn on_key(&self, key: gdk::Key, modifiers: gdk::ModifierType) -> glib::Propagation {
        if !self.state.borrow().visible {
            return glib::Propagation::Proceed;
        }
        let control_focused = self.windows.media().is_some_and(|window| {
            window.buttons.as_ref().is_some_and(|buttons| {
                buttons.play.has_focus()
                    || buttons.previous.has_focus()
                    || buttons.next.has_focus()
            })
        });
        let only_shift = modifiers
            .difference(gdk::ModifierType::SHIFT_MASK)
            .is_empty();
        if key == gdk::Key::Escape {
            self.handle_action(Action::Collapse);
            return glib::Propagation::Stop;
        }
        if key == gdk::Key::Tab {
            self.state.borrow_mut().focus_visible = true;
            return glib::Propagation::Proceed;
        }
        if control_focused {
            return glib::Propagation::Proceed;
        }
        match key {
            gdk::Key::space if only_shift => {
                self.send_media(MediaCommand::PlayPause);
                glib::Propagation::Stop
            }
            gdk::Key::Left if only_shift => {
                self.seek_relative(-5.0);
                glib::Propagation::Stop
            }
            gdk::Key::Right if only_shift => {
                self.seek_relative(5.0);
                glib::Propagation::Stop
            }
            gdk::Key::n if only_shift => {
                self.send_media(MediaCommand::Next);
                glib::Propagation::Stop
            }
            gdk::Key::p if only_shift => {
                self.send_media(MediaCommand::Previous);
                glib::Propagation::Stop
            }
            gdk::Key::e if only_shift => {
                self.toggle_expanded_from_click();
                glib::Propagation::Stop
            }
            gdk::Key::m if only_shift => {
                self.toggle_pin();
                glib::Propagation::Stop
            }
            gdk::Key::q if modifiers.contains(gdk::ModifierType::CONTROL_MASK) => {
                self.handle_action(Action::Quit);
                glib::Propagation::Stop
            }
            _ => glib::Propagation::Proceed,
        }
    }

    fn seek_relative(&self, delta_seconds: f64) {
        let position = {
            let state = self.state.borrow();
            let Some(snapshot) = state.snapshot.as_ref() else {
                return;
            };
            if !snapshot.capabilities.can_seek {
                return;
            }
            snapshot.position_seconds_at(Instant::now())
        };
        let target = (position + delta_seconds).max(0.0);
        self.send_media(MediaCommand::SeekTo((target * 1_000_000.0) as i64));
    }

    fn send_media(&self, command: MediaCommand) {
        if let Some(service) = self.media.borrow().as_ref() {
            service.send(command);
        }
    }

    pub fn handle_action(&self, action: Action) {
        info!(?action, "UI action");
        match action {
            Action::Show => {
                {
                    let mut state = self.state.borrow_mut();
                    state.user_hidden = false;
                    state.force_show = true;
                }
                self.sync_visibility();
                self.refresh_view();
            }
            Action::Hide => {
                {
                    let mut state = self.state.borrow_mut();
                    state.user_hidden = true;
                    state.force_show = false;
                }
                self.hide_overlay();
            }
            Action::ToggleVisibility => {
                if self.state.borrow().visible {
                    self.handle_action(Action::Hide);
                } else {
                    self.handle_action(Action::Show);
                }
            }
            Action::Expand => {
                self.cancel_auto_collapse();
                {
                    let mut state = self.state.borrow_mut();
                    state.user_hidden = false;
                    state.force_show = true;
                    state.auto_expanded = false;
                    state.hover_expanded = false;
                    state.manual_expanded = true;
                }
                self.sync_visibility();
                self.apply_expansion_target();
                self.refresh_view();
            }
            Action::Collapse => {
                self.cancel_auto_collapse();
                {
                    let mut state = self.state.borrow_mut();
                    state.config.behavior.pin_expanded = false;
                    state.auto_expanded = false;
                    state.hover_expanded = false;
                    state.manual_expanded = false;
                }
                self.apply_expansion_target();
                self.debounce_save_config();
            }
            Action::ToggleExpanded => self.toggle_expanded_from_click(),
            Action::Next => self.send_media(MediaCommand::Next),
            Action::Previous => self.send_media(MediaCommand::Previous),
            Action::PlayPause => self.send_media(MediaCommand::PlayPause),
            Action::TogglePin => self.toggle_pin(),
            Action::Settings => self.open_settings(),
            Action::Menu => self.open_main_menu(),
            Action::Quit => {
                if self.quitting.replace(true) {
                    return;
                }
                self.save_now();
                self.app.quit();
            }
        }
    }

    /// Open the launcher menu. This is what a desktop entry or a bare launch
    /// should show: a single place to reach every widget and setting.
    pub fn open_main_menu(&self) {
        {
            let mut state = self.state.borrow_mut();
            state.user_hidden = false;
            state.force_show = true;
        }
        self.sync_visibility();
        self.refresh_view();
        let Some(window) = self
            .windows
            .media()
            .or_else(|| self.windows.all().first().cloned())
        else {
            return;
        };
        let layout = self.state.borrow().layout(window.kind);
        let (x, y) = layout.card.rect.center();
        self.show_context_menu(x, y);
    }

    fn open_settings(&self) {
        if let Some(existing) = self.settings.borrow().as_ref() {
            existing.present();
            return;
        }
        let Some(anchor) = self
            .windows
            .media()
            .or_else(|| self.windows.all().first().cloned())
        else {
            return;
        };
        self.state.borrow_mut().settings_open = true;
        info!("opening settings window");
        self.sync_visibility();
        let config = self.state.borrow().config.clone();
        let players = self.state.borrow().players.clone();
        let weak = Rc::downgrade(&self.rc());
        let on_change: Rc<dyn Fn(Config)> = Rc::new(move |config| {
            if let Some(controller) = weak.upgrade() {
                controller.apply_config(config, true);
            }
        });
        let weak = Rc::downgrade(&self.rc());
        let on_close: Rc<dyn Fn()> = Rc::new(move || {
            if let Some(controller) = weak.upgrade() {
                controller.restore_after_settings();
            }
        });
        let handle = settings::open(
            &anchor.window,
            &self.app,
            config,
            players,
            on_change,
            on_close,
        );
        let weak = Rc::downgrade(&self.rc());
        handle.window.connect_close_request(move |_| {
            if let Some(controller) = weak.upgrade() {
                controller.restore_after_settings();
            }
            glib::Propagation::Proceed
        });
        let weak = Rc::downgrade(&self.rc());
        handle.window.connect_destroy(move |_| {
            if let Some(controller) = weak.upgrade() {
                controller.restore_after_settings();
            }
        });
        *self.settings.borrow_mut() = Some(handle);
    }

    fn restore_after_settings(&self) {
        let was_open = {
            let mut state = self.state.borrow_mut();
            let was_open = state.settings_open;
            state.settings_open = false;
            was_open
        };
        self.settings.borrow_mut().take();
        if was_open {
            info!("settings window closed");
            self.sync_visibility();
            self.refresh_view();
        }
    }

    pub fn players_snapshot(&self) -> Vec<PlayerInfo> {
        self.state.borrow().players.clone()
    }

    pub fn config_snapshot(&self) -> Config {
        self.state.borrow().config.clone()
    }

    pub fn backend_kind(&self) -> crate::window::BackendKind {
        self.windows
            .media()
            .or_else(|| self.windows.all().first().cloned())
            .map(|window| window.backend.borrow().kind)
            .unwrap_or(crate::window::BackendKind::Other)
    }

    pub fn backend_label(&self) -> &'static str {
        self.backend_kind().label()
    }

    pub fn is_visible(&self) -> bool {
        self.state.borrow().visible
    }

    pub fn flush_config(&self) {
        self.save_now();
    }
}

fn control_at(
    layout: &FrameLayout,
    capabilities: Option<crate::media::metadata::Capabilities>,
    x: f64,
    y: f64,
) -> Option<Control> {
    let caps = capabilities?;
    let checks = [
        (
            Control::Previous,
            caps.can_go_previous,
            layout.previous_alpha > 0.15,
        ),
        (
            Control::PlayPause,
            caps.can_control && (caps.can_play || caps.can_pause),
            true,
        ),
        (Control::Next, caps.can_go_next, true),
    ];
    for (control, enabled, visible) in checks {
        if enabled && visible && layout.control_hit(control).contains(x, y) {
            return Some(control);
        }
    }
    None
}

fn system_ui_preferences() -> (bool, bool, bool) {
    let Some(settings) = gtk::Settings::default() else {
        return (true, false, false);
    };
    let theme = settings
        .gtk_theme_name()
        .map(|name| name.to_string())
        .unwrap_or_default()
        .to_lowercase();
    let dark = settings.is_gtk_application_prefer_dark_theme() || theme.contains("dark");
    let animations = settings.property::<bool>("gtk-enable-animations");
    let high_contrast = theme.contains("highcontrast") || theme.contains("high-contrast");
    (dark, !animations, high_contrast)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::metadata::{Capabilities, Track};

    fn snapshot(status: PlaybackStatus) -> PlayerSnapshot {
        PlayerSnapshot {
            bus_name: "org.mpris.MediaPlayer2.mock".into(),
            identity: "Mock".into(),
            track: Track {
                id: Some("/track/1".into()),
                title: "A track".into(),
                artists: vec!["An artist".into()],
                album: None,
                art_url: None,
                length_us: Some(120_000_000),
            },
            status,
            capabilities: Capabilities {
                can_control: true,
                can_play: true,
                can_pause: true,
                can_go_next: true,
                can_go_previous: true,
                can_seek: true,
            },
            position_us: 0,
            captured_at: Instant::now(),
            rate: 1.0,
        }
    }

    #[test]
    fn visibility_honours_idle_and_paused_preferences() {
        let mut config = Config::default();
        assert!(!visible_for(&config, None, false, false));
        assert!(visible_for(&config, None, false, true));
        assert!(!visible_for(&config, None, true, true));
        let playing = snapshot(PlaybackStatus::Playing);
        assert!(visible_for(&config, Some(&playing), false, false));
        let paused = snapshot(PlaybackStatus::Paused);
        assert!(visible_for(&config, Some(&paused), false, false));
        config.behavior.show_when_paused = false;
        assert!(!visible_for(&config, Some(&paused), false, false));
        config.behavior.hide_when_idle = false;
        let stopped = snapshot(PlaybackStatus::Stopped);
        assert!(visible_for(&config, Some(&stopped), false, false));
        assert!(visible_for(&config, None, false, false));
    }

    #[test]
    fn track_identity_includes_player_and_track() {
        let mut one = snapshot(PlaybackStatus::Playing);
        let key = track_key(&one);
        one.track.id = Some("/track/2".into());
        assert_ne!(key, track_key(&one));
        one.bus_name = "org.mpris.MediaPlayer2.other".into();
        one.track.id = Some("/track/1".into());
        assert_ne!(key, track_key(&one));
    }

    #[test]
    fn transient_expansion_flags_clear_together() {
        let mut config = Config::default();
        config.behavior.pin_expanded = false;
        let mut state = UiState::new(config, UiOptions::default(), true, false);
        state.auto_expanded = true;
        state.hover_expanded = true;
        state.manual_expanded = true;
        assert!(state.wants_expanded());
        state.clear_transient_expansion();
        assert!(!state.wants_expanded());
    }
}
