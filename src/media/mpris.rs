//! Low-level MPRIS D-Bus operations.
//!
//! This module knows about the MPRIS wire protocol, but not about selection or
//! the UI. That keeps the media backend replaceable and testable.

use std::collections::HashMap;

use anyhow::{Context, Result, bail};
use zbus::{Connection, OwnedObjectPath, OwnedVariant, Variant, names::InterfaceName};

use crate::media::metadata::{Capabilities, MPRIS_PATH, PlaybackStatus, PlayerSnapshot, Track};

pub const PLAYER_INTERFACE: &str = "org.mpris.MediaPlayer2.Player";
pub const ROOT_INTERFACE: &str = "org.mpris.MediaPlayer2";
pub const PROPERTIES_INTERFACE: &str = "org.freedesktop.DBus.Properties";

/// A player property state assembled from `GetAll` and incremental updates.
#[derive(Debug, Clone)]
pub struct ParsedPlayerProperties {
    pub track: Track,
    pub status: PlaybackStatus,
    pub capabilities: Capabilities,
    pub position_us: i64,
    pub rate: f64,
    pub has_position: bool,
}

impl Default for ParsedPlayerProperties {
    fn default() -> Self {
        Self {
            track: Track::default(),
            status: PlaybackStatus::Stopped,
            capabilities: Capabilities::default(),
            position_us: 0,
            rate: 1.0,
            has_position: false,
        }
    }
}

impl ParsedPlayerProperties {
    pub fn from_properties(properties: &HashMap<String, OwnedVariant>) -> Self {
        let mut parsed = Self::default();
        parsed.apply(properties);
        parsed
    }

    /// Merge a `PropertiesChanged` map or an invalidated-property refresh.
    pub fn apply(&mut self, properties: &HashMap<String, OwnedVariant>) {
        if let Some(metadata) = properties
            .get("Metadata")
            .and_then(crate::media::metadata::metadata_map)
        {
            self.track = Track::from_metadata(&metadata);
        }

        if let Some(status) = properties
            .get("PlaybackStatus")
            .and_then(|value| match &**value {
                Variant::Str(status) => Some(PlaybackStatus::parse(status.as_str())),
                _ => None,
            })
        {
            self.status = status;
        }

        self.capabilities.merge(properties);

        if let Some(position) = properties.get("Position").and_then(integer_value) {
            self.position_us = position.max(0);
            self.has_position = true;
        }
        if let Some(rate) = properties.get("Rate").and_then(number_value)
            && rate.is_finite()
            && rate > 0.0
        {
            self.rate = rate;
        }
    }

    /// Apply the borrowed property map delivered by `PropertiesChanged`.
    pub fn apply_changed(&mut self, properties: &HashMap<&str, Variant<'_>>) {
        let owned = properties
            .iter()
            .filter_map(|(key, value)| {
                OwnedVariant::try_from(value)
                    .ok()
                    .map(|value| ((*key).to_owned(), value))
            })
            .collect::<HashMap<_, _>>();
        self.apply(&owned);
    }

    pub fn to_snapshot(
        &self,
        bus_name: &str,
        identity: &str,
        captured_at: std::time::Instant,
    ) -> PlayerSnapshot {
        PlayerSnapshot {
            bus_name: bus_name.to_owned(),
            identity: identity.to_owned(),
            track: self.track.clone(),
            status: self.status,
            capabilities: self.capabilities,
            position_us: self.position_us,
            captured_at,
            rate: self.rate,
        }
    }
}

/// Fetch the two MPRIS interfaces for a player.
pub async fn fetch_all(
    connection: &Connection,
    bus_name: &str,
) -> Result<(
    String,
    HashMap<String, OwnedVariant>,
    HashMap<String, OwnedVariant>,
)> {
    let root = get_all(connection, bus_name, ROOT_INTERFACE).await?;
    let player = get_all(connection, bus_name, PLAYER_INTERFACE)
        .await
        .with_context(|| format!("{bus_name} does not expose an MPRIS player interface"))?;
    let identity = root
        .get("Identity")
        .and_then(|value| match &**value {
            Variant::Str(identity) => Some(identity.as_str().to_owned()),
            _ => None,
        })
        .or_else(|| {
            bus_name
                .strip_prefix(crate::media::metadata::MPRIS_PREFIX)
                .map(|suffix| suffix.to_owned())
        })
        .unwrap_or_else(|| bus_name.to_owned());
    Ok((identity, root, player))
}

pub async fn get_all(
    connection: &Connection,
    bus_name: &str,
    interface: &str,
) -> Result<HashMap<String, OwnedVariant>> {
    let reply = connection
        .call_method(
            Some(bus_name),
            MPRIS_PATH,
            Some(PROPERTIES_INTERFACE),
            "GetAll",
            &(interface,),
        )
        .await
        .with_context(|| format!("GetAll {interface} on {bus_name} failed"))?;
    reply
        .body()
        .deserialize()
        .context("could not decode MPRIS properties")
}

pub async fn fetch_position(connection: &Connection, bus_name: &str) -> Result<i64> {
    let reply = connection
        .call_method(
            Some(bus_name),
            MPRIS_PATH,
            Some(PROPERTIES_INTERFACE),
            "Get",
            &(PLAYER_INTERFACE, "Position"),
        )
        .await
        .with_context(|| format!("Get Position on {bus_name} failed"))?;
    let value: OwnedVariant = reply
        .body()
        .deserialize()
        .context("could not decode MPRIS position")?;
    integer_value(&value).context("Position was not an integer")
}

pub async fn call_player(
    connection: &Connection,
    bus_name: &str,
    method: &str,
    body: &(impl serde::Serialize + zbus::DynamicType),
) -> Result<()> {
    connection
        .call_method(
            Some(bus_name),
            MPRIS_PATH,
            Some(PLAYER_INTERFACE),
            method,
            body,
        )
        .await
        .with_context(|| format!("{method} failed for {bus_name}"))?;
    Ok(())
}

pub async fn play_pause(connection: &Connection, bus_name: &str) -> Result<()> {
    call_player(connection, bus_name, "PlayPause", &()).await
}

pub async fn next(connection: &Connection, bus_name: &str) -> Result<()> {
    call_player(connection, bus_name, "Next", &()).await
}

pub async fn previous(connection: &Connection, bus_name: &str) -> Result<()> {
    call_player(connection, bus_name, "Previous", &()).await
}

pub async fn play(connection: &Connection, bus_name: &str) -> Result<()> {
    call_player(connection, bus_name, "Play", &()).await
}

pub async fn pause(connection: &Connection, bus_name: &str) -> Result<()> {
    call_player(connection, bus_name, "Pause", &()).await
}

pub async fn set_position(
    connection: &Connection,
    bus_name: &str,
    track_id: &str,
    position_us: i64,
) -> Result<()> {
    let path = OwnedObjectPath::try_from(track_id)
        .with_context(|| format!("invalid MPRIS track id {track_id}"))?;
    connection
        .call_method(
            Some(bus_name),
            MPRIS_PATH,
            Some(PLAYER_INTERFACE),
            "SetPosition",
            &(path, position_us.max(0)),
        )
        .await
        .with_context(|| format!("SetPosition failed for {bus_name}"))?;
    Ok(())
}

pub async fn seek(connection: &Connection, bus_name: &str, offset_us: i64) -> Result<()> {
    call_player(connection, bus_name, "Seek", &(offset_us,)).await
}

pub async fn subscribe_property_changes(
    connection: &Connection,
    bus_name: &str,
) -> Result<zbus::fdo::PropertiesChangedStream> {
    let proxy = zbus::fdo::PropertiesProxy::builder(connection)
        .destination(bus_name.to_owned())
        .path(MPRIS_PATH)
        .build()
        .await
        .context("could not subscribe to player properties")?;
    proxy
        .receive_properties_changed()
        .await
        .context("could not receive property changes")
}

pub async fn subscribe_seeked(
    connection: &Connection,
    bus_name: &str,
) -> Result<zbus::proxy::SignalStream<'static>> {
    let proxy = zbus::Proxy::new(
        connection,
        bus_name.to_owned(),
        MPRIS_PATH.to_owned(),
        PLAYER_INTERFACE.to_owned(),
    )
    .await
    .context("could not create MPRIS player proxy")?;
    proxy
        .receive_signal("Seeked")
        .await
        .context("could not subscribe to Seeked")
}

pub fn parse_seeked(message: &zbus::Message) -> Result<i64> {
    let body = message.body();
    let (position,): (i64,) = body.deserialize().context("could not decode Seeked")?;
    Ok(position.max(0))
}

/// Build a typed interface name once for code that needs to filter signal args.
pub fn player_interface_name() -> InterfaceName<'static> {
    InterfaceName::from_static_str_unchecked(PLAYER_INTERFACE)
}

fn integer_value(value: &OwnedVariant) -> Option<i64> {
    match &**value {
        Variant::I16(value) => Some(i64::from(*value)),
        Variant::I32(value) => Some(i64::from(*value)),
        Variant::I64(value) => Some(*value),
        Variant::U16(value) => Some(i64::from(*value)),
        Variant::U32(value) => Some(i64::from(*value)),
        Variant::U64(value) => i64::try_from(*value).ok(),
        Variant::Variant(inner) => match &**inner {
            Variant::I16(value) => Some(i64::from(*value)),
            Variant::I32(value) => Some(i64::from(*value)),
            Variant::I64(value) => Some(*value),
            Variant::U16(value) => Some(i64::from(*value)),
            Variant::U32(value) => Some(i64::from(*value)),
            Variant::U64(value) => i64::try_from(*value).ok(),
            _ => None,
        },
        _ => None,
    }
}

fn number_value(value: &OwnedVariant) -> Option<f64> {
    match &**value {
        Variant::F64(value) => Some(*value),
        Variant::I32(value) => Some(f64::from(*value)),
        Variant::I64(value) => Some(*value as f64),
        Variant::U32(value) => Some(f64::from(*value)),
        _ => None,
    }
}

pub fn call_error_is_unsupported(error: &anyhow::Error) -> bool {
    let Some(error) = error.downcast_ref::<zbus::Error>() else {
        return false;
    };
    matches!(
        error,
        zbus::Error::MethodError(name, _, _)
            if name.as_str() == "org.freedesktop.DBus.Error.UnknownMethod"
                || name.as_str() == "org.freedesktop.DBus.Error.NotSupported"
    )
}

pub fn ensure_seekable(track_id: Option<&str>, can_seek: bool) -> Result<()> {
    if !can_seek {
        bail!("the selected player does not support seeking");
    }
    if track_id.is_none() {
        // Seek-by-offset is still available without a track id, so this is not
        // an error. The caller decides which MPRIS primitive to use.
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_incremental_properties() {
        let mut map = HashMap::new();
        map.insert(
            "PlaybackStatus".into(),
            OwnedVariant::try_from(Variant::from("Playing")).unwrap(),
        );
        map.insert("CanControl".into(), OwnedVariant::from(true));
        map.insert("CanGoNext".into(), OwnedVariant::from(true));
        map.insert("Position".into(), OwnedVariant::from(42_000_i64));
        map.insert("Rate".into(), OwnedVariant::from(1.0_f64));
        let parsed = ParsedPlayerProperties::from_properties(&map);
        assert_eq!(parsed.status, PlaybackStatus::Playing);
        assert!(parsed.capabilities.can_control);
        assert!(parsed.capabilities.can_go_next);
        assert_eq!(parsed.position_us, 42_000);
        assert_eq!(parsed.rate, 1.0);
    }

    #[test]
    fn metadata_property_is_parsed_without_markup() {
        let mut metadata = std::collections::HashMap::new();
        metadata.insert("xesam:title".to_owned(), "A < B".to_owned());
        metadata.insert("xesam:artist".to_owned(), "C & D".to_owned());
        let metadata_value = OwnedVariant::from(metadata);
        let mut map = HashMap::new();
        map.insert("Metadata".into(), metadata_value);
        let parsed = ParsedPlayerProperties::from_properties(&map);
        assert_eq!(parsed.track.title, "A < B");
        assert_eq!(parsed.track.artist_line(), "C & D");
    }

    #[test]
    fn invalid_track_id_is_rejected() {
        let result = OwnedObjectPath::try_from("not a path");
        assert!(result.is_err());
    }

    #[test]
    fn parse_seeked_requires_an_integer() {
        let message = zbus::Message::signal("/org/mpris/MediaPlayer2", PLAYER_INTERFACE, "Seeked")
            .build(&(123_i64,))
            .unwrap();
        assert_eq!(parse_seeked(&message).unwrap(), 123);
    }
}
