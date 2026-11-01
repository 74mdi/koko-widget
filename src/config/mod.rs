//! Persistent application configuration.
//!
//! The configuration is deliberately small and human-editable. Everything is
//! validated before use so a malformed file cannot put the overlay into an
//! unusable state.

use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::utils::{self, parse_hex_color, write_atomic};

pub const DEFAULT_CONFIG_HEADER: &str = "\
# KOKO WIDGET configuration\n\
# This file is managed by KOKO WIDGET's settings window as well as by hand.\n\
# Edit it while the app is closed, or changes will be overwritten on save.\n\n";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
#[derive(Default)]
pub struct Config {
    pub appearance: Appearance,
    pub position: Position,
    pub behavior: Behavior,
    pub media: Media,
    pub animation: Animation,
    pub widgets: Widgets,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Appearance {
    /// Draw the layered glass surface. When disabled the widget uses an opaque
    /// charcoal surface with the same geometry.
    pub glass: bool,
    /// Overall opacity of the glass base, from 0.35 to 1.0.
    pub opacity: f64,
    /// Prefer compositor-side blur. This has no effect unless the compositor
    /// offers a matching blur rule for the `koko-widget` layer namespace.
    pub blur: bool,
    pub theme: Theme,
    /// Accent used when artwork adaptation is disabled.
    pub accent: String,
    /// Derive a restrained tint from the current artwork.
    pub adaptive_accent: bool,
    pub corner_radius: f64,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            glass: true,
            opacity: 0.84,
            blur: true,
            theme: Theme::Dark,
            accent: "#8b7cf6".to_owned(),
            adaptive_accent: true,
            corner_radius: 34.0,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Theme {
    Dark,
    Light,
    Auto,
}

impl Theme {
    pub const ALL: [Theme; 3] = [Theme::Dark, Theme::Light, Theme::Auto];
    pub fn label(self) -> &'static str {
        match self {
            Self::Dark => "Dark",
            Self::Light => "Light",
            Self::Auto => "Follow system",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Position {
    pub mode: PositionMode,
    /// Connector name (`eDP-1`, `HDMI-2`, …), `primary`, or an index such as `1`.
    pub display: Option<String>,
    /// Distance from the relevant screen edge for top/bottom modes.
    pub margin: i32,
    /// Custom left offset for [`PositionMode::Custom`].
    pub custom_x: i32,
    /// Custom top offset for [`PositionMode::Custom`].
    pub custom_y: i32,
    pub layer: Layer,
}

impl Default for Position {
    fn default() -> Self {
        Self {
            mode: PositionMode::Center,
            display: None,
            margin: 28,
            custom_x: 40,
            custom_y: 40,
            layer: Layer::Top,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum PositionMode {
    Center,
    TopCenter,
    BottomCenter,
    Custom,
}

impl PositionMode {
    pub const ALL: [PositionMode; 4] = [
        PositionMode::Center,
        PositionMode::TopCenter,
        PositionMode::BottomCenter,
        PositionMode::Custom,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Center => "Screen center",
            Self::TopCenter => "Top center",
            Self::BottomCenter => "Bottom center",
            Self::Custom => "Custom position",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Layer {
    Top,
    Overlay,
}

impl Layer {
    pub const ALL: [Layer; 2] = [Layer::Top, Layer::Overlay];
    pub fn label(self) -> &'static str {
        match self {
            Self::Top => "Top layer",
            Self::Overlay => "Overlay (above fullscreen)",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Behavior {
    pub auto_expand_on_new_track: bool,
    pub auto_collapse_seconds: f64,
    pub expand_on_hover: bool,
    pub pin_expanded: bool,
    pub show_when_paused: bool,
    pub hide_when_idle: bool,
    pub autostart: bool,
}

impl Default for Behavior {
    fn default() -> Self {
        Self {
            auto_expand_on_new_track: true,
            auto_collapse_seconds: 3.5,
            expand_on_hover: true,
            pin_expanded: false,
            show_when_paused: true,
            hide_when_idle: true,
            autostart: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Media {
    /// Exact MPRIS bus suffix, identity, or unique bus name.
    pub preferred_player: Option<String>,
    /// Ordered fallback list. Entries match identity or bus suffix.
    pub player_priority: Vec<String>,
    pub auto_select: bool,
    pub allow_remote_artwork: bool,
}

impl Media {
    pub fn with_defaults() -> Self {
        Self {
            preferred_player: None,
            player_priority: Vec::new(),
            auto_select: true,
            allow_remote_artwork: true,
        }
    }
}

impl Default for Media {
    fn default() -> Self {
        Self::with_defaults()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Animation {
    pub enabled: bool,
    /// Multiplier for animation durations. 1.0 is the reference speed.
    pub speed: f64,
    pub reduced_motion: ReducedMotion,
}

impl Default for Animation {
    fn default() -> Self {
        Self {
            enabled: true,
            speed: 1.0,
            reduced_motion: ReducedMotion::Auto,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ReducedMotion {
    Auto,
    Always,
    Never,
}

impl ReducedMotion {
    pub const ALL: [ReducedMotion; 3] = [
        ReducedMotion::Auto,
        ReducedMotion::Always,
        ReducedMotion::Never,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "Follow system setting",
            Self::Always => "Always reduce motion",
            Self::Never => "Never reduce motion",
        }
    }
}

/// Which widget kinds can be placed in the overlay, in display order.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum WidgetKind {
    Media,
    Clock,
    Date,
    Text,
}

impl WidgetKind {
    pub const ALL: [WidgetKind; 4] = [
        WidgetKind::Media,
        WidgetKind::Clock,
        WidgetKind::Date,
        WidgetKind::Text,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Media => "Now playing",
            Self::Clock => "Clock",
            Self::Date => "Date",
            Self::Text => "Text",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Media => "Album art, track details and transport controls",
            Self::Clock => "Live time with an optional seconds display",
            Self::Date => "Today's date in a format you choose",
            Self::Text => "Any text you want, such as a greeting or note",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Widgets {
    /// Left-to-right order of every enabled widget.
    pub order: Vec<WidgetKind>,
    pub media: MediaWidget,
    pub clock: ClockWidget,
    pub date: DateWidget,
    pub text: TextWidget,
    /// Gap between adjacent cards when a widget has no explicit position.
    pub gap: f64,
    /// Per-widget saved positions. A widget without a spot is auto-placed
    /// relative to the media pill in `order`.
    pub positions: WidgetPositions,
    /// Vertical alignment of shorter cards inside the row.
    pub align: RowAlign,
}

impl Default for Widgets {
    fn default() -> Self {
        Self {
            order: vec![WidgetKind::Media, WidgetKind::Clock, WidgetKind::Date],
            media: MediaWidget::default(),
            clock: ClockWidget::default(),
            date: DateWidget::default(),
            text: TextWidget::default(),
            gap: 12.0,
            positions: WidgetPositions::default(),
            align: RowAlign::Center,
        }
    }
}

/// Where one widget sits on its monitor.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(default)]
pub struct WidgetSpot {
    /// Connector name, `primary`, or an index. `None` follows the global one.
    pub display: Option<String>,
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(default)]
pub struct WidgetPositions {
    pub media: Option<WidgetSpot>,
    pub clock: Option<WidgetSpot>,
    pub date: Option<WidgetSpot>,
    pub text: Option<WidgetSpot>,
}

impl WidgetPositions {
    pub fn get(&self, kind: WidgetKind) -> Option<&WidgetSpot> {
        match kind {
            WidgetKind::Media => self.media.as_ref(),
            WidgetKind::Clock => self.clock.as_ref(),
            WidgetKind::Date => self.date.as_ref(),
            WidgetKind::Text => self.text.as_ref(),
        }
    }

    pub fn set(&mut self, kind: WidgetKind, spot: Option<WidgetSpot>) {
        match kind {
            WidgetKind::Media => self.media = spot,
            WidgetKind::Clock => self.clock = spot,
            WidgetKind::Date => self.date = spot,
            WidgetKind::Text => self.text = spot,
        }
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn any(&self) -> bool {
        self.media.is_some() || self.clock.is_some() || self.date.is_some() || self.text.is_some()
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum RowAlign {
    Center,
    Top,
    Bottom,
}

impl RowAlign {
    pub const ALL: [RowAlign; 3] = [RowAlign::Center, RowAlign::Top, RowAlign::Bottom];
    pub fn label(self) -> &'static str {
        match self {
            Self::Center => "Centered",
            Self::Top => "Top aligned",
            Self::Bottom => "Bottom aligned",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct MediaWidget {
    pub enabled: bool,
    /// Draw the glass pill behind the media card.
    pub background: bool,
    /// Show the album artwork.
    pub artwork: bool,
    /// Corner radius of the artwork thumbnail.
    pub artwork_radius: f64,
    pub show_artist: bool,
    pub show_controls: bool,
    pub show_progress: bool,
    pub title_size: f64,
    pub artist_size: f64,
}

impl Default for MediaWidget {
    fn default() -> Self {
        Self {
            enabled: true,
            background: true,
            artwork: true,
            artwork_radius: 18.0,
            show_artist: true,
            show_controls: true,
            show_progress: true,
            title_size: 13.0,
            artist_size: 11.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ClockWidget {
    pub enabled: bool,
    pub show_seconds: bool,
    pub hour_format: HourFormat,
    /// Optional caption drawn above the time, for example "Local time".
    pub label: String,
    pub show_label: bool,
    pub font_size: f64,
    pub label_size: f64,
    pub weight: FontWeight,
    /// `None` uses the theme text colour.
    pub colour: Option<String>,
    /// Card width in logical pixels.
    pub width: f64,
    /// Card height in logical pixels. Each widget keeps its own pill shape.
    pub height: f64,
    /// Draw a glass pill behind this widget. When off, the content floats on
    /// the desktop with only a glass text treatment.
    pub background: bool,
}

impl Default for ClockWidget {
    fn default() -> Self {
        Self {
            enabled: true,
            show_seconds: false,
            hour_format: HourFormat::Auto,
            label: "Local time".to_owned(),
            show_label: false,
            font_size: 30.0,
            label_size: 10.0,
            weight: FontWeight::Semibold,
            colour: None,
            width: 172.0,
            height: 112.0,
            background: false,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum HourFormat {
    Auto,
    TwelveHour,
    TwentyFourHour,
}

impl HourFormat {
    pub const ALL: [HourFormat; 3] = [
        HourFormat::Auto,
        HourFormat::TwelveHour,
        HourFormat::TwentyFourHour,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "Follow locale",
            Self::TwelveHour => "12-hour (1:30 PM)",
            Self::TwentyFourHour => "24-hour (13:30)",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct DateWidget {
    pub enabled: bool,
    /// `strftime`-style pattern.
    pub format: String,
    pub show_label: bool,
    pub label: String,
    pub font_size: f64,
    pub label_size: f64,
    pub weight: FontWeight,
    pub colour: Option<String>,
    /// Split long dates onto two lines at the first space.
    pub wrap: bool,
    /// Card width in logical pixels.
    pub width: f64,
    /// Card height in logical pixels. Each widget keeps its own pill shape.
    pub height: f64,
    /// Draw a glass pill behind this widget. When off, the content floats on
    /// the desktop with only a glass text treatment.
    pub background: bool,
}

impl Default for DateWidget {
    fn default() -> Self {
        Self {
            enabled: true,
            format: "%A %-d %B".to_owned(),
            show_label: false,
            label: "Today".to_owned(),
            font_size: 18.0,
            label_size: 10.0,
            weight: FontWeight::Medium,
            colour: None,
            wrap: false,
            width: 210.0,
            height: 112.0,
            background: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct TextWidget {
    pub enabled: bool,
    pub text: String,
    pub font_size: f64,
    pub weight: FontWeight,
    pub colour: Option<String>,
    pub align: TextAlign,
    /// Draw a small caption above the text.
    pub show_label: bool,
    pub label: String,
    pub label_size: f64,
    /// Card width in logical pixels.
    pub width: f64,
    /// Card height in logical pixels. Each widget keeps its own pill shape.
    pub height: f64,
    /// Draw a glass pill behind this widget. When off, the content floats on
    /// the desktop with only a glass text treatment.
    pub background: bool,
}

impl Default for TextWidget {
    fn default() -> Self {
        Self {
            enabled: false,
            text: "Stay focused".to_owned(),
            font_size: 18.0,
            weight: FontWeight::Medium,
            colour: None,
            align: TextAlign::Center,
            show_label: false,
            label: "Note".to_owned(),
            label_size: 10.0,
            width: 200.0,
            height: 112.0,
            background: false,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum TextAlign {
    Left,
    Center,
    Right,
}

impl TextAlign {
    pub const ALL: [TextAlign; 3] = [TextAlign::Left, TextAlign::Center, TextAlign::Right];
    pub fn label(self) -> &'static str {
        match self {
            Self::Left => "Left",
            Self::Center => "Center",
            Self::Right => "Right",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum FontWeight {
    Light,
    Normal,
    Medium,
    Semibold,
    Bold,
}

impl FontWeight {
    pub const ALL: [FontWeight; 5] = [
        FontWeight::Light,
        FontWeight::Normal,
        FontWeight::Medium,
        FontWeight::Semibold,
        FontWeight::Bold,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Light => "Light",
            Self::Normal => "Regular",
            Self::Medium => "Medium",
            Self::Semibold => "Semibold",
            Self::Bold => "Bold",
        }
    }
}

impl Widgets {
    /// Enabled widgets in display order, always de-duplicated.
    pub fn active(&self) -> Vec<WidgetKind> {
        let mut seen = Vec::new();
        for kind in &self.order {
            if seen.contains(kind) {
                continue;
            }
            if self.is_enabled(*kind) {
                seen.push(*kind);
            }
        }
        // Any enabled widget missing from the order list is appended.
        for kind in WidgetKind::ALL {
            if self.is_enabled(kind) && !seen.contains(&kind) {
                seen.push(kind);
            }
        }
        seen
    }

    /// Whether a widget draws its own glass pill.
    pub fn has_background(&self, kind: WidgetKind) -> bool {
        match kind {
            WidgetKind::Media => self.media.background,
            WidgetKind::Clock => self.clock.background,
            WidgetKind::Date => self.date.background,
            WidgetKind::Text => self.text.background,
        }
    }

    pub fn is_enabled(&self, kind: WidgetKind) -> bool {
        match kind {
            WidgetKind::Media => self.media.enabled,
            WidgetKind::Clock => self.clock.enabled,
            WidgetKind::Date => self.date.enabled,
            WidgetKind::Text => self.text.enabled,
        }
    }

    pub fn set_enabled(&mut self, kind: WidgetKind, enabled: bool) {
        match kind {
            WidgetKind::Media => self.media.enabled = enabled,
            WidgetKind::Clock => self.clock.enabled = enabled,
            WidgetKind::Date => self.date.enabled = enabled,
            WidgetKind::Text => self.text.enabled = enabled,
        }
        if enabled && !self.order.contains(&kind) {
            self.order.push(kind);
        }
    }
}

/// Result of loading configuration, including non-fatal recovery notes.
#[derive(Debug, Clone)]
pub struct LoadOutcome {
    pub config: Config,
    pub warnings: Vec<String>,
    pub path: PathBuf,
}

impl Config {
    /// Load from the standard XDG location.
    pub fn load() -> LoadOutcome {
        Self::load_from(&utils::config_file())
    }

    /// Load an explicit path. This is public so tests and packagers can use it.
    pub fn load_from(path: &Path) -> LoadOutcome {
        let mut warnings = Vec::new();
        let mut config = match fs::read_to_string(path) {
            Ok(text) => match toml::from_str::<Self>(&text) {
                Ok(config) => config,
                Err(error) => {
                    let stamp = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map(|value| value.as_secs())
                        .unwrap_or(0);
                    let backup = path.with_extension(format!("toml.invalid-{stamp}"));
                    let message = format!(
                        "Configuration at {} is invalid ({error}). Using defaults; the broken file was kept at {}.",
                        path.display(),
                        backup.display()
                    );
                    warn!("{message}");
                    warnings.push(message);
                    if let Err(rename_error) = fs::rename(path, &backup) {
                        warnings.push(format!("Could not preserve invalid config: {rename_error}"));
                    }
                    Self::default()
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(error) => {
                let message = format!("Could not read {}: {error}", path.display());
                warn!("{message}");
                warnings.push(message);
                Self::default()
            }
        };

        config.behavior.autostart = autostart_is_installed();
        let sanitize_warnings = config.sanitize();
        warnings.extend(sanitize_warnings);
        LoadOutcome {
            config,
            warnings,
            path: path.to_owned(),
        }
    }

    /// Clamp values into safe ranges. Returns a note for every adjustment.
    pub fn sanitize(&mut self) -> Vec<String> {
        let mut notes = Vec::new();

        let opacity = self.appearance.opacity.clamp(0.35, 1.0);
        if (self.appearance.opacity - opacity).abs() > f64::EPSILON {
            notes.push(format!(
                "Appearance opacity adjusted from {} to {opacity}",
                self.appearance.opacity
            ));
        }
        self.appearance.opacity = opacity;

        let radius = self.appearance.corner_radius.clamp(18.0, 48.0);
        if (self.appearance.corner_radius - radius).abs() > f64::EPSILON {
            notes.push(format!(
                "Corner radius adjusted from {} to {radius}",
                self.appearance.corner_radius
            ));
        }
        self.appearance.corner_radius = radius;

        if parse_hex_color(&self.appearance.accent).is_none() {
            notes.push("Accent colour was invalid; reset to #8b7cf6".to_owned());
            self.appearance.accent = Appearance::default().accent;
        }

        let timeout = self.behavior.auto_collapse_seconds.clamp(1.0, 30.0);
        if (self.behavior.auto_collapse_seconds - timeout).abs() > f64::EPSILON {
            notes.push(format!(
                "Auto-collapse timeout adjusted from {} to {timeout}",
                self.behavior.auto_collapse_seconds
            ));
        }
        self.behavior.auto_collapse_seconds = timeout;

        let speed = self.animation.speed.clamp(0.25, 2.0);
        if (self.animation.speed - speed).abs() > f64::EPSILON {
            notes.push(format!(
                "Animation speed adjusted from {} to {speed}",
                self.animation.speed
            ));
        }
        self.animation.speed = speed;

        self.position.margin = self.position.margin.clamp(0, 2000);
        self.position.custom_x = self.position.custom_x.clamp(0, 20_000);
        self.position.custom_y = self.position.custom_y.clamp(0, 20_000);

        if let Some(player) = &self.media.preferred_player
            && player.trim().is_empty()
        {
            self.media.preferred_player = None;
        }
        self.media
            .player_priority
            .retain(|entry| !entry.trim().is_empty());

        self.sanitize_widgets(&mut notes);

        notes
    }

    fn sanitize_widgets(&mut self, notes: &mut Vec<String>) {
        self.widgets.order.retain(|kind| {
            let keep = WidgetKind::ALL.contains(kind);
            if !keep {
                notes.push(format!(
                    "Removed unknown widget {kind:?} from the widget order"
                ));
            }
            keep
        });
        // Every enabled widget must appear exactly once.
        let mut deduped: Vec<WidgetKind> = Vec::new();
        for kind in std::mem::take(&mut self.widgets.order) {
            if !deduped.contains(&kind) {
                deduped.push(kind);
            }
        }
        for kind in WidgetKind::ALL {
            if self.widgets.is_enabled(kind) && !deduped.contains(&kind) {
                deduped.push(kind);
            }
        }
        self.widgets.order = deduped;

        if self.widgets.active().is_empty() {
            notes.push("No widgets were enabled; turning the media widget back on".to_owned());
            self.widgets.media.enabled = true;
            if !self.widgets.order.contains(&WidgetKind::Media) {
                self.widgets.order.insert(0, WidgetKind::Media);
            }
        }

        self.widgets.gap = self.widgets.gap.clamp(0.0, 48.0);
        self.widgets.clock.width = self.widgets.clock.width.clamp(96.0, 420.0);
        self.widgets.date.width = self.widgets.date.width.clamp(96.0, 420.0);
        self.widgets.text.width = self.widgets.text.width.clamp(96.0, 420.0);
        for spot in [
            &mut self.widgets.positions.media,
            &mut self.widgets.positions.clock,
            &mut self.widgets.positions.date,
            &mut self.widgets.positions.text,
        ]
        .into_iter()
        .flatten()
        {
            spot.x = spot.x.clamp(-20_000, 20_000);
            spot.y = spot.y.clamp(-20_000, 20_000);
            if let Some(display) = spot.display.as_ref()
                && display.trim().is_empty()
            {
                spot.display = None;
            }
        }

        self.widgets.clock.height = self.widgets.clock.height.clamp(72.0, 320.0);
        self.widgets.date.height = self.widgets.date.height.clamp(72.0, 320.0);
        self.widgets.text.height = self.widgets.text.height.clamp(72.0, 320.0);

        let media = &mut self.widgets.media;
        media.title_size = media.title_size.clamp(8.0, 32.0);
        media.artist_size = media.artist_size.clamp(7.0, 26.0);
        media.artwork_radius = media.artwork_radius.clamp(0.0, 48.0);

        for (name, size, min, max) in [
            ("Clock font size", self.widgets.clock.font_size, 10.0, 96.0),
            ("Clock label size", self.widgets.clock.label_size, 7.0, 28.0),
            ("Date font size", self.widgets.date.font_size, 8.0, 72.0),
            ("Date label size", self.widgets.date.label_size, 7.0, 28.0),
            ("Text font size", self.widgets.text.font_size, 8.0, 72.0),
            ("Text label size", self.widgets.text.label_size, 7.0, 28.0),
        ] {
            if !(min..=max).contains(&size) {
                notes.push(format!("{name} adjusted into {min}–{max}"));
            }
        }
        self.widgets.clock.font_size = self.widgets.clock.font_size.clamp(10.0, 96.0);
        self.widgets.clock.label_size = self.widgets.clock.label_size.clamp(7.0, 28.0);
        self.widgets.date.font_size = self.widgets.date.font_size.clamp(8.0, 72.0);
        self.widgets.date.label_size = self.widgets.date.label_size.clamp(7.0, 28.0);
        self.widgets.text.font_size = self.widgets.text.font_size.clamp(8.0, 72.0);
        self.widgets.text.label_size = self.widgets.text.label_size.clamp(7.0, 28.0);

        for (name, colour) in [
            ("Clock colour", &mut self.widgets.clock.colour),
            ("Date colour", &mut self.widgets.date.colour),
            ("Text colour", &mut self.widgets.text.colour),
        ] {
            if let Some(value) = colour.as_ref()
                && parse_hex_color(value).is_none()
            {
                notes.push(format!("{name} was invalid; using the theme colour"));
                *colour = None;
            }
        }

        if self.widgets.date.format.trim().is_empty() {
            notes.push("Date format was empty; reset to the default".to_owned());
            self.widgets.date.format = DateWidget::default().format;
        }
        if self.widgets.date.format.len() > 64 {
            notes.push("Date format was too long; truncated to 64 characters".to_owned());
            self.widgets.date.format.truncate(64);
        }
        if self.widgets.text.text.chars().count() > 240 {
            notes.push("Text widget content was truncated to 240 characters".to_owned());
            self.widgets.text.text = self.widgets.text.text.chars().take(240).collect();
        }
        for label in [
            &mut self.widgets.clock.label,
            &mut self.widgets.date.label,
            &mut self.widgets.text.label,
        ] {
            if label.chars().count() > 40 {
                *label = label.chars().take(40).collect();
            }
        }
    }

    pub fn save(&self) -> Result<()> {
        self.save_to(&utils::config_file())
    }

    pub fn save_to(&self, path: &Path) -> Result<()> {
        let body = toml::to_string_pretty(self).context("could not serialise configuration")?;
        let content = format!("{DEFAULT_CONFIG_HEADER}{body}");
        write_atomic(path, content.as_bytes())?;
        info!(path = %path.display(), "configuration saved");
        Ok(())
    }

    /// Apply the `autostart` preference to the user's XDG autostart directory.
    /// Only the KOKO WIDGET-owned desktop file is ever touched.
    pub fn sync_autostart(&self) -> Result<()> {
        let path = autostart_file();
        if self.behavior.autostart {
            let executable =
                std::env::current_exe().context("could not find current executable")?;
            let escaped = desktop_exec_escape(&executable.to_string_lossy());
            let contents = format!(
                "[Desktop Entry]\n\
                 Type=Application\n\
                 Version=1.0\n\
                 Name=KOKO WIDGET\n\
                 Comment=Floating music overlay\n\
                 Exec={escaped}\n\
                 Icon=io.github.kokowidget.KokoWidget\n\
                 Terminal=false\n\
                 Categories=AudioVideo;Player;\n\
                 X-GNOME-Autostart-enabled=true\n\
                 X-KOKO-WIDGET-OWNED=true\n"
            );
            write_atomic(&path, contents.as_bytes())?;
        } else if path.exists() {
            // Only remove files that this application owns. This guards against a
            // user-created file with the same name.
            let owned = fs::read_to_string(&path)
                .map(|text| text.contains("X-KOKO-WIDGET-OWNED=true"))
                .unwrap_or(false);
            if owned {
                fs::remove_file(&path)
                    .with_context(|| format!("could not remove {}", path.display()))?;
            } else {
                warn!(
                    path = %path.display(),
                    "autostart file was not created by KOKO WIDGET; leaving it untouched"
                );
            }
        }
        Ok(())
    }

    /// Whether animation should be suppressed for this render.
    pub fn effective_reduced_motion(&self, system_reduces: bool) -> bool {
        !self.animation.enabled
            || matches!(self.animation.reduced_motion, ReducedMotion::Always)
            || (matches!(self.animation.reduced_motion, ReducedMotion::Auto) && system_reduces)
    }

    pub fn animation_scale(&self) -> f32 {
        if self.animation.enabled {
            self.animation.speed.clamp(0.25, 2.0) as f32
        } else {
            0.0
        }
    }
}

pub fn autostart_file() -> PathBuf {
    utils::autostart_dir().join("io.github.kokowidget.KokoWidget.desktop")
}

fn autostart_is_installed() -> bool {
    let path = autostart_file();
    fs::read_to_string(path)
        .map(|contents| contents.contains("X-KOKO-WIDGET-OWNED=true"))
        .unwrap_or(false)
}

fn desktop_exec_escape(input: &str) -> String {
    // The Exec key uses quoting rules close to shell quoting. Wrapping in double
    // quotes and escaping backslashes/quotes/backticks/dollar signs is sufficient
    // for local paths.
    let escaped = input
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('`', "\\`")
        .replace('$', "\\$");
    format!("\"{escaped}\"")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn defaults_are_sane() {
        let config = Config::default();
        assert!(config.appearance.glass);
        assert!((0.0..=1.0).contains(&config.appearance.opacity));
        assert!(config.behavior.auto_expand_on_new_track);
        assert_eq!(config.position.mode, PositionMode::Center);
    }

    #[test]
    fn invalid_values_are_clamped() {
        let mut config = Config::default();
        config.appearance.opacity = 9.0;
        config.appearance.corner_radius = -3.0;
        config.behavior.auto_collapse_seconds = 0.0;
        config.appearance.accent = "hot pink".into();
        let notes = config.sanitize();
        assert_eq!(config.appearance.opacity, 1.0);
        assert_eq!(config.appearance.corner_radius, 18.0);
        assert_eq!(config.behavior.auto_collapse_seconds, 1.0);
        assert!(!notes.is_empty());
        assert!(parse_hex_color(&config.appearance.accent).is_some());
    }

    #[test]
    fn malformed_config_falls_back_and_preserves_input() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut file = fs::File::create(&path).unwrap();
        file.write_all(b"this is [[[not toml").unwrap();
        drop(file);
        let outcome = Config::load_from(&path);
        assert_eq!(outcome.config, Config::default());
        assert!(outcome.warnings.iter().any(|note| note.contains("invalid")));
        assert!(!path.exists());
        assert!(
            fs::read_dir(dir.path())
                .unwrap()
                .flatten()
                .any(|entry| entry.file_name().to_string_lossy().contains("invalid-"))
        );
    }

    #[test]
    fn config_round_trips_through_toml() {
        let mut config = Config::default();
        config.appearance.opacity = 0.66;
        config.media.preferred_player = Some("spotify".into());
        config.position.mode = PositionMode::TopCenter;
        let text = toml::to_string_pretty(&config).unwrap();
        let parsed: Config = toml::from_str(&text).unwrap();
        assert_eq!(parsed, config);
    }

    #[test]
    fn widgets_default_to_media_clock_and_date() {
        let widgets = Widgets::default();
        assert_eq!(
            widgets.active(),
            vec![WidgetKind::Media, WidgetKind::Clock, WidgetKind::Date]
        );
        assert!(!widgets.text.enabled);
    }

    #[test]
    fn widget_order_is_preserved_and_deduplicated() {
        let widgets = Widgets {
            order: vec![
                WidgetKind::Date,
                WidgetKind::Media,
                WidgetKind::Date,
                WidgetKind::Clock,
            ],
            ..Widgets::default()
        };
        assert_eq!(
            widgets.active(),
            vec![WidgetKind::Date, WidgetKind::Media, WidgetKind::Clock]
        );
    }

    #[test]
    fn disabled_widgets_are_skipped_but_keep_their_place() {
        let mut widgets = Widgets::default();
        widgets.date.enabled = false;
        widgets.order = vec![WidgetKind::Clock, WidgetKind::Media, WidgetKind::Date];
        widgets.clock.enabled = false;
        assert_eq!(widgets.active(), vec![WidgetKind::Media]);
        widgets.clock.enabled = true;
        assert_eq!(widgets.active(), vec![WidgetKind::Clock, WidgetKind::Media]);
    }

    #[test]
    fn enabling_a_widget_appends_it_to_the_order() {
        let mut widgets = Widgets::default();
        widgets.set_enabled(WidgetKind::Text, true);
        assert!(widgets.order.contains(&WidgetKind::Text));
        assert_eq!(*widgets.order.last().unwrap(), WidgetKind::Text);
    }

    #[test]
    fn sanitize_recovers_from_an_empty_widget_row() {
        let mut config = Config::default();
        config.widgets.media.enabled = false;
        config.widgets.clock.enabled = false;
        config.widgets.date.enabled = false;
        config.widgets.text.enabled = false;
        config.widgets.order.clear();
        let notes = config.sanitize();
        assert!(config.widgets.media.enabled);
        assert!(!config.widgets.active().is_empty());
        assert!(notes.iter().any(|note| note.contains("widget")));
    }

    #[test]
    fn sanitize_clamps_widget_sizes_and_colours() {
        let mut config = Config::default();
        config.widgets.clock.font_size = 500.0;
        config.widgets.text.font_size = 1.0;
        config.widgets.date.width = 5000.0;
        config.widgets.clock.colour = Some("not a colour".into());
        config.widgets.text.text = "x".repeat(400);
        config.sanitize();
        assert_eq!(config.widgets.clock.font_size, 96.0);
        assert_eq!(config.widgets.text.font_size, 8.0);
        assert_eq!(config.widgets.date.width, 420.0);
        assert!(config.widgets.clock.colour.is_none());
        assert_eq!(config.widgets.text.text.chars().count(), 240);
    }

    #[test]
    fn widget_configuration_round_trips() {
        let mut config = Config::default();
        config.widgets.order = vec![
            WidgetKind::Clock,
            WidgetKind::Media,
            WidgetKind::Date,
            WidgetKind::Text,
        ];
        config.widgets.clock.show_seconds = true;
        config.widgets.clock.colour = Some("#ffcc00".into());
        config.widgets.text.text = "Hello".into();
        config.widgets.gap = 20.0;
        let text = toml::to_string_pretty(&config).unwrap();
        let parsed: Config = toml::from_str(&text).unwrap();
        assert_eq!(parsed.widgets, config.widgets);
    }

    #[test]
    fn old_configs_without_widgets_still_load() {
        let text = "[appearance]\nopacity = 0.7\n";
        let _ = &text;
        let config: Config = toml::from_str(text).unwrap();
        assert_eq!(config.widgets, Widgets::default());
        assert!(config.appearance.glass);
    }

    #[test]
    fn reduced_motion_preference_is_respected() {
        let mut config = Config::default();
        assert!(!config.effective_reduced_motion(false));
        assert!(config.effective_reduced_motion(true));
        config.animation.reduced_motion = ReducedMotion::Never;
        assert!(!config.effective_reduced_motion(true));
        config.animation.enabled = false;
        assert!(config.effective_reduced_motion(false));
    }
}
