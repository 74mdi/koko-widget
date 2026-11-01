//! Player-agnostic media metadata and capability types.

use std::collections::HashMap;

use zbus::{OwnedVariant, Variant};

pub const MPRIS_PATH: &str = "/org/mpris/MediaPlayer2";
pub const MPRIS_PREFIX: &str = "org.mpris.MediaPlayer2.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackStatus {
    Playing,
    Paused,
    Stopped,
}

impl PlaybackStatus {
    pub fn parse(value: &str) -> Self {
        match value {
            "Playing" => Self::Playing,
            "Paused" => Self::Paused,
            _ => Self::Stopped,
        }
    }

    pub fn is_playing(self) -> bool {
        self == Self::Playing
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Playing => "Playing",
            Self::Paused => "Paused",
            Self::Stopped => "Stopped",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Capabilities {
    pub can_control: bool,
    pub can_play: bool,
    pub can_pause: bool,
    pub can_go_next: bool,
    pub can_go_previous: bool,
    pub can_seek: bool,
}

impl Capabilities {
    pub fn from_properties(properties: &HashMap<String, OwnedVariant>) -> Self {
        Self {
            can_control: bool_property(properties, "CanControl"),
            can_play: bool_property(properties, "CanPlay"),
            can_pause: bool_property(properties, "CanPause"),
            can_go_next: bool_property(properties, "CanGoNext"),
            can_go_previous: bool_property(properties, "CanGoPrevious"),
            can_seek: bool_property(properties, "CanSeek"),
        }
    }

    pub fn merge(&mut self, properties: &HashMap<String, OwnedVariant>) {
        for (key, value) in properties {
            if let Some(flag) = bool_value(value) {
                match key.as_str() {
                    "CanControl" => self.can_control = flag,
                    "CanPlay" => self.can_play = flag,
                    "CanPause" => self.can_pause = flag,
                    "CanGoNext" => self.can_go_next = flag,
                    "CanGoPrevious" => self.can_go_previous = flag,
                    "CanSeek" => self.can_seek = flag,
                    _ => {}
                }
            }
        }
    }

    pub fn toggle_allowed(self, _playing: bool) -> bool {
        // Play and pause share a single toggle, so either capability is enough
        // regardless of the current state.
        self.can_control && (self.can_play || self.can_pause)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Track {
    pub id: Option<String>,
    pub title: String,
    pub artists: Vec<String>,
    pub album: Option<String>,
    pub art_url: Option<String>,
    pub length_us: Option<i64>,
}

impl Default for Track {
    fn default() -> Self {
        Self {
            id: None,
            title: "Nothing playing".to_owned(),
            artists: Vec::new(),
            album: None,
            art_url: None,
            length_us: None,
        }
    }
}

impl Track {
    pub fn from_metadata(metadata: &HashMap<String, OwnedVariant>) -> Self {
        let title = string_property(metadata, "xesam:title")
            .map(|value| sanitize_text(&value))
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "Unknown title".to_owned());

        let artists = metadata
            .get("xesam:artist")
            .map(string_list)
            .unwrap_or_default()
            .into_iter()
            .map(|value| sanitize_text(&value))
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>();

        let album = string_property(metadata, "xesam:album")
            .map(|value| sanitize_text(&value))
            .filter(|value| !value.is_empty());

        let art_url = string_property(metadata, "mpris:artUrl")
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty() && value.len() < 8192);

        let length_us = metadata
            .get("mpris:length")
            .and_then(integer_value)
            .filter(|value| *value > 0);

        let id = metadata
            .get("mpris:trackid")
            .and_then(|value| match &**value {
                Variant::ObjectPath(path) => Some(path.as_str().to_owned()),
                Variant::Str(path) => Some(path.as_str().to_owned()),
                _ => None,
            });

        Self {
            id,
            title,
            artists,
            album,
            art_url,
            length_us,
        }
    }

    /// Stable identity used to detect real track changes. Artwork URL and
    /// duration changes are deliberately excluded so a player refreshing
    /// inessential metadata cannot retrigger the expansion animation.
    pub fn identity(&self) -> String {
        if let Some(id) = &self.id {
            return format!("id:{id}");
        }
        format!(
            "meta:{}|{}|{}",
            self.title,
            self.artists.join("\u{1f}"),
            self.album.as_deref().unwrap_or("")
        )
    }

    pub fn artist_line(&self) -> String {
        if self.artists.is_empty() {
            "Unknown artist".to_owned()
        } else {
            self.artists.join(", ")
        }
    }

    pub fn duration_seconds(&self) -> Option<f64> {
        self.length_us
            .filter(|length| *length > 0)
            .map(|length| length as f64 / 1_000_000.0)
    }

    pub fn is_empty(&self) -> bool {
        self.id.is_none()
            && self.title == "Nothing playing"
            && self.artists.is_empty()
            && self.art_url.is_none()
    }
}

#[derive(Debug, Clone)]
pub struct PlayerSnapshot {
    pub bus_name: String,
    pub identity: String,
    pub track: Track,
    pub status: PlaybackStatus,
    pub capabilities: Capabilities,
    /// Position at `captured_at`, in microseconds.
    pub position_us: i64,
    pub captured_at: std::time::Instant,
    pub rate: f64,
}

impl PlayerSnapshot {
    pub fn position_seconds_at(&self, now: std::time::Instant) -> f64 {
        let base = self.position_us.max(0) as f64 / 1_000_000.0;
        if self.status == PlaybackStatus::Playing {
            let elapsed = now
                .saturating_duration_since(self.captured_at)
                .as_secs_f64();
            base + elapsed * self.rate.clamp(0.0, 8.0)
        } else {
            base
        }
    }

    pub fn duration_seconds(&self) -> Option<f64> {
        self.track.duration_seconds()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerInfo {
    pub bus_name: String,
    pub identity: String,
    pub status: PlaybackStatus,
    pub track_title: String,
    pub artist: String,
}

impl PlayerInfo {
    pub fn suffix(&self) -> &str {
        self.bus_name
            .strip_prefix(MPRIS_PREFIX)
            .unwrap_or(&self.bus_name)
    }
}

fn sanitize_text(value: &str) -> String {
    // Strip control characters and collapse newlines. MPRIS metadata is used
    // only for display, never for markup.
    let clean: String = value
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect();
    clean.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn string_property(properties: &HashMap<String, OwnedVariant>, key: &str) -> Option<String> {
    properties.get(key).and_then(string_value)
}

fn string_value(value: &OwnedVariant) -> Option<String> {
    match &**value {
        Variant::Str(text) => Some(text.as_str().to_owned()),
        Variant::Variant(inner) => variant_string(inner),
        _ => None,
    }
}

fn string_list(value: &OwnedVariant) -> Vec<String> {
    match &**value {
        Variant::Str(text) => vec![text.as_str().to_owned()],
        Variant::Array(items) => items
            .iter()
            .filter_map(|item| match item {
                Variant::Str(text) => Some(text.as_str().to_owned()),
                Variant::Variant(inner) => variant_string(inner),
                _ => None,
            })
            .collect(),
        Variant::Variant(inner) => variant_string_list(inner),
        _ => Vec::new(),
    }
}

fn variant_string(value: &Variant<'_>) -> Option<String> {
    match value {
        Variant::Str(text) => Some(text.as_str().to_owned()),
        Variant::Variant(inner) => variant_string(inner),
        _ => None,
    }
}

fn variant_string_list(value: &Variant<'_>) -> Vec<String> {
    match value {
        Variant::Str(text) => vec![text.as_str().to_owned()],
        Variant::Array(items) => items.iter().filter_map(variant_string).collect(),
        Variant::Variant(inner) => variant_string_list(inner),
        _ => Vec::new(),
    }
}

fn integer_value(value: &OwnedVariant) -> Option<i64> {
    match &**value {
        Variant::I16(value) => Some(i64::from(*value)),
        Variant::I32(value) => Some(i64::from(*value)),
        Variant::I64(value) => Some(*value),
        Variant::U16(value) => Some(i64::from(*value)),
        Variant::U32(value) => Some(i64::from(*value)),
        Variant::U64(value) => i64::try_from(*value).ok(),
        Variant::Variant(inner) => variant_integer(inner),
        _ => None,
    }
}

fn variant_integer(value: &Variant<'_>) -> Option<i64> {
    match value {
        Variant::I16(value) => Some(i64::from(*value)),
        Variant::I32(value) => Some(i64::from(*value)),
        Variant::I64(value) => Some(*value),
        Variant::U16(value) => Some(i64::from(*value)),
        Variant::U32(value) => Some(i64::from(*value)),
        Variant::U64(value) => i64::try_from(*value).ok(),
        Variant::Variant(inner) => variant_integer(inner),
        _ => None,
    }
}

fn bool_property(properties: &HashMap<String, OwnedVariant>, key: &str) -> bool {
    properties.get(key).and_then(bool_value).unwrap_or(false)
}

fn bool_value(value: &OwnedVariant) -> Option<bool> {
    match &**value {
        Variant::Bool(value) => Some(*value),
        Variant::Variant(inner) => match &**inner {
            Variant::Bool(value) => Some(*value),
            _ => None,
        },
        _ => None,
    }
}

/// Convert the `a{sv}` value returned by `org.freedesktop.DBus.Properties`
/// into the metadata map used by MPRIS.
pub fn metadata_map(value: &OwnedVariant) -> Option<HashMap<String, OwnedVariant>> {
    let owned = value.try_clone().ok()?;
    let dict = zbus::Dict::try_from(owned).ok()?;
    let mut map = HashMap::new();
    for (key, value) in dict.iter() {
        let key = match key {
            Variant::Str(key) => key.as_str(),
            _ => continue,
        };
        let value = match value {
            Variant::Variant(inner) => inner.as_ref(),
            other => other,
        };
        if let Ok(owned) = OwnedVariant::try_from(value) {
            map.insert(key.to_owned(), owned);
        }
    }
    Some(map)
}

pub fn mpris_name(bus_name: &str) -> Option<&str> {
    bus_name
        .strip_prefix(MPRIS_PREFIX)
        .filter(|suffix| !suffix.is_empty() && !suffix.contains('.'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use zbus::ObjectPath;

    fn owned(value: Variant<'_>) -> OwnedVariant {
        OwnedVariant::try_from(value).unwrap()
    }

    fn metadata(pairs: Vec<(&str, OwnedVariant)>) -> HashMap<String, OwnedVariant> {
        pairs
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect()
    }

    #[test]
    fn parses_a_full_metadata_map() {
        let artists =
            zbus::Array::from(vec![Variant::from("Aurora"), Variant::from("The Beloved")]);
        let map = metadata(vec![
            (
                "mpris:trackid",
                owned(Variant::ObjectPath(
                    ObjectPath::try_from("/track/1").unwrap(),
                )),
            ),
            ("xesam:title", owned(Variant::from("The Seed"))),
            ("xesam:artist", owned(Variant::Array(artists))),
            (
                "xesam:album",
                owned(Variant::from("A Different Kind of Human")),
            ),
            (
                "mpris:artUrl",
                owned(Variant::from("file:///tmp/cover.png")),
            ),
            ("mpris:length", owned(Variant::from(198_000_000_i64))),
        ]);
        let track = Track::from_metadata(&map);
        assert_eq!(track.title, "The Seed");
        assert_eq!(track.artist_line(), "Aurora, The Beloved");
        assert_eq!(track.album.as_deref(), Some("A Different Kind of Human"));
        assert_eq!(track.duration_seconds(), Some(198.0));
        assert_eq!(track.identity(), "id:/track/1");
    }

    #[test]
    fn collapses_whitespace_and_control_characters() {
        let map = metadata(vec![
            ("xesam:title", owned(Variant::from("  Hello\nWorld\u{0} "))),
            ("xesam:artist", owned(Variant::from("  The   Artist "))),
        ]);
        let track = Track::from_metadata(&map);
        assert_eq!(track.title, "Hello World");
        assert_eq!(track.artist_line(), "The Artist");
    }

    #[test]
    fn missing_metadata_has_honest_placeholders() {
        let track = Track::from_metadata(&HashMap::new());
        assert_eq!(track.title, "Unknown title");
        assert_eq!(track.artist_line(), "Unknown artist");
        assert!(track.length_us.is_none());
        assert!(track.art_url.is_none());
    }

    #[test]
    fn capabilities_merge_partial_updates() {
        let mut properties = HashMap::new();
        properties.insert("CanPlay".into(), owned(Variant::from(true)));
        properties.insert("CanGoNext".into(), owned(Variant::from(true)));
        properties.insert("CanControl".into(), owned(Variant::from(true)));
        let mut caps = Capabilities::from_properties(&properties);
        assert!(caps.can_play && caps.can_go_next && caps.can_control);
        assert!(!caps.can_go_previous);
        let mut update = HashMap::new();
        update.insert("CanGoPrevious".into(), owned(Variant::from(true)));
        caps.merge(&update);
        assert!(caps.can_go_previous);
    }

    #[test]
    fn playback_rate_projects_position() {
        use std::time::{Duration, Instant};
        let track = Track::default();
        let snapshot = PlayerSnapshot {
            bus_name: "org.mpris.MediaPlayer2.test".into(),
            identity: "Test".into(),
            track,
            status: PlaybackStatus::Playing,
            capabilities: Capabilities::default(),
            position_us: 10_000_000,
            captured_at: Instant::now(),
            rate: 1.0,
        };
        let future = snapshot.captured_at + Duration::from_secs(5);
        assert!((snapshot.position_seconds_at(future) - 15.0).abs() < 0.001);
    }

    #[test]
    fn mpris_name_extraction_is_strict() {
        assert_eq!(
            mpris_name("org.mpris.MediaPlayer2.spotify"),
            Some("spotify")
        );
        assert_eq!(mpris_name("org.mpris.MediaPlayer2"), None);
        assert_eq!(mpris_name("org.example.Player"), None);
    }

    #[test]
    fn artist_arrays_are_not_rendered_with_markup() {
        let map = metadata(vec![
            (
                "xesam:title",
                owned(Variant::from("<b>Do not render me</b>")),
            ),
            ("xesam:artist", owned(Variant::from("A & B"))),
        ]);
        let track = Track::from_metadata(&map);
        assert_eq!(track.title, "<b>Do not render me</b>");
        assert_eq!(track.artist_line(), "A & B");
    }
}
