//! A controllable mock MPRIS player for local integration tests.
//!
//! This binary is intentionally a test tool, not part of the widget UI:
//!
//! ```text
//! cargo run --example mock_player -- --player-name koko_mock --paused
//! ```
//!
//! It exposes the standard MPRIS interfaces over the session bus and implements
//! play/pause, next/previous, seek and metadata changes.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Instant,
};

use anyhow::{Context, Result};
use clap::Parser;
use zbus::{
    Array, Connection, ObjectPath, OwnedObjectPath, OwnedVariant, Variant, interface,
    names::InterfaceName, object_server::SignalEmitter,
};

const PLAYER_IFACE: &str = "org.mpris.MediaPlayer2.Player";

#[derive(Debug, Parser)]
#[command(
    name = "koko-mock-player",
    about = "Mock MPRIS player for KOKO WIDGET tests"
)]
struct Args {
    /// MPRIS bus suffix, for example `koko_mock`.
    #[arg(long, default_value = "koko_mock")]
    player_name: String,
    /// Human-readable player identity.
    #[arg(long, default_value = "KOKO Mock Player")]
    identity: String,
    /// Replace the demo track list with one track using this title.
    #[arg(long)]
    title: Option<String>,
    #[arg(long, default_value = "Mock Artist")]
    artist: String,
    #[arg(long, default_value_t = 180)]
    duration_secs: u64,
    /// Local artwork path exposed through `mpris:artUrl`.
    #[arg(long)]
    art: Option<std::path::PathBuf>,
    /// Start paused instead of playing.
    #[arg(long)]
    paused: bool,
    /// Advertise CanGoNext = false.
    #[arg(long)]
    no_next: bool,
    /// Advertise CanSeek = false.
    #[arg(long)]
    no_seek: bool,
}

#[derive(Debug, Clone)]
struct MockTrack {
    title: String,
    artist: String,
    album: String,
    length_us: i64,
    art_url: Option<String>,
}

#[derive(Debug)]
struct MockState {
    tracks: Vec<MockTrack>,
    index: usize,
    playing: bool,
    position_base_us: i64,
    started_at: Instant,
    can_next: bool,
    can_seek: bool,
}

impl MockState {
    fn position_us(&self) -> i64 {
        if self.playing {
            self.position_base_us + self.started_at.elapsed().as_micros() as i64
        } else {
            self.position_base_us
        }
    }

    fn current_track(&self) -> &MockTrack {
        &self.tracks[self.index]
    }

    fn seek_to(&mut self, position_us: i64) {
        let duration = self.current_track().length_us;
        self.position_base_us = position_us.clamp(0, duration);
        self.started_at = Instant::now();
    }

    fn set_playing(&mut self, playing: bool) {
        let now_position = self.position_us();
        self.position_base_us = now_position;
        self.started_at = Instant::now();
        self.playing = playing;
    }

    fn advance(&mut self, direction: i32) {
        let len = self.tracks.len();
        self.index = ((self.index as i32 + direction).rem_euclid(len as i32)) as usize;
        self.seek_to(0);
    }
}

struct MockRoot {
    identity: String,
}

#[interface(name = "org.mpris.MediaPlayer2")]
impl MockRoot {
    #[zbus(property)]
    fn identity(&self) -> String {
        self.identity.clone()
    }

    #[zbus(property)]
    fn can_quit(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn can_raise(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn can_set_fullscreen(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn has_track_list(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn desktop_entry(&self) -> String {
        "koko-mock-player".to_owned()
    }

    #[zbus(property)]
    fn supported_uri_schemes(&self) -> Vec<String> {
        Vec::new()
    }

    #[zbus(property)]
    fn supported_mime_types(&self) -> Vec<String> {
        Vec::new()
    }
}

struct MockPlayer {
    state: Arc<Mutex<MockState>>,
}

#[interface(name = "org.mpris.MediaPlayer2.Player")]
impl MockPlayer {
    async fn play(
        &mut self,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        self.state.lock().unwrap().set_playing(true);
        self.emit_changes(&emitter, &["PlaybackStatus"]).await
    }

    async fn pause(
        &mut self,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        self.state.lock().unwrap().set_playing(false);
        self.emit_changes(&emitter, &["PlaybackStatus"]).await
    }

    async fn play_pause(
        &mut self,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        let playing = {
            let mut state = self.state.lock().unwrap();
            let playing = !state.playing;
            state.set_playing(playing);
            playing
        };
        eprintln!("mock: PlayPause -> playing={playing}");
        self.emit_changes(&emitter, &["PlaybackStatus"]).await
    }

    async fn stop(
        &mut self,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        {
            let mut state = self.state.lock().unwrap();
            state.set_playing(false);
            state.seek_to(0);
        }
        self.emit_changes(&emitter, &["PlaybackStatus"]).await?;
        Self::seeked(&emitter, 0)
            .await
            .map_err(|error| zbus::fdo::Error::Failed(error.to_string()))
    }

    async fn next(
        &mut self,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        let index = {
            let mut state = self.state.lock().unwrap();
            if !state.can_next {
                return Err(zbus::fdo::Error::NotSupported(
                    "CanGoNext is false".to_owned(),
                ));
            }
            state.advance(1);
            eprintln!("mock: Next -> track {}", state.index + 1);
            state.index
        };
        let _ = index;
        self.emit_changes(&emitter, &["Metadata", "PlaybackStatus"])
            .await?;
        Self::seeked(&emitter, 0)
            .await
            .map_err(|error| zbus::fdo::Error::Failed(error.to_string()))
    }

    async fn previous(
        &mut self,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        let index = {
            let mut state = self.state.lock().unwrap();
            state.advance(-1);
            state.index
        };
        eprintln!("mock: Previous -> track {}", index + 1);
        self.emit_changes(&emitter, &["Metadata", "PlaybackStatus"])
            .await?;
        Self::seeked(&emitter, 0)
            .await
            .map_err(|error| zbus::fdo::Error::Failed(error.to_string()))
    }

    async fn seek(
        &mut self,
        offset: i64,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        let position = {
            let mut state = self.state.lock().unwrap();
            if !state.can_seek {
                return Err(zbus::fdo::Error::NotSupported(
                    "CanSeek is false".to_owned(),
                ));
            }
            let position = state.position_us().saturating_add(offset);
            state.seek_to(position);
            state.position_base_us
        };
        Self::seeked(&emitter, position)
            .await
            .map_err(|error| zbus::fdo::Error::Failed(error.to_string()))
    }

    async fn set_position(
        &mut self,
        _track_id: OwnedObjectPath,
        position: i64,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        {
            let mut state = self.state.lock().unwrap();
            state.seek_to(position);
        }
        Self::seeked(&emitter, position)
            .await
            .map_err(|error| zbus::fdo::Error::Failed(error.to_string()))
    }

    async fn open_uri(&self, _uri: String) -> zbus::fdo::Result<()> {
        Ok(())
    }

    #[zbus(signal)]
    async fn seeked(emitter: &SignalEmitter<'_>, position: i64) -> zbus::Result<()>;

    #[zbus(property)]
    fn playback_status(&self) -> String {
        if self.state.lock().unwrap().playing {
            "Playing".to_owned()
        } else {
            "Paused".to_owned()
        }
    }

    #[zbus(property)]
    fn metadata(&self) -> HashMap<String, OwnedVariant> {
        let state = self.state.lock().unwrap();
        let track = state.current_track();
        let mut metadata = HashMap::new();
        metadata.insert(
            "mpris:trackid".to_owned(),
            owned(Variant::ObjectPath(
                ObjectPath::try_from(format!("/org/kokowidget/MockPlayer/Track/{}", state.index))
                    .expect("valid track path"),
            )),
        );
        metadata.insert(
            "mpris:length".to_owned(),
            OwnedVariant::from(track.length_us),
        );
        metadata.insert("xesam:title".to_owned(), owned_string(track.title.clone()));
        let artist = Array::from(vec![Variant::from(track.artist.clone())]);
        metadata.insert(
            "xesam:artist".to_owned(),
            OwnedVariant::try_from(Variant::Array(artist)).expect("valid artist array"),
        );
        if !track.album.is_empty() {
            metadata.insert("xesam:album".to_owned(), owned_string(track.album.clone()));
        }
        if let Some(art) = &track.art_url {
            metadata.insert("mpris:artUrl".to_owned(), owned_string(art.clone()));
        }
        metadata
    }

    #[zbus(property)]
    fn position(&self) -> i64 {
        self.state.lock().unwrap().position_us()
    }

    #[zbus(property(emits_changed_signal = "false"))]
    fn rate(&self) -> f64 {
        1.0
    }

    #[zbus(property)]
    fn minimum_rate(&self) -> f64 {
        1.0
    }

    #[zbus(property)]
    fn maximum_rate(&self) -> f64 {
        1.0
    }

    #[zbus(property)]
    fn can_control(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn can_play(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn can_pause(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn can_go_next(&self) -> bool {
        self.state.lock().unwrap().can_next
    }

    #[zbus(property)]
    fn can_go_previous(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn can_seek(&self) -> bool {
        self.state.lock().unwrap().can_seek
    }
}

impl MockPlayer {
    async fn emit_changes(
        &self,
        emitter: &SignalEmitter<'_>,
        properties: &[&str],
    ) -> zbus::fdo::Result<()> {
        for property in properties {
            let result = match *property {
                "PlaybackStatus" => self.playback_status_changed(emitter).await,
                "Metadata" => self.metadata_changed(emitter).await,
                _ => Ok(()),
            };
            result.map_err(|error| zbus::fdo::Error::Failed(error.to_string()))?;
        }
        Ok(())
    }
}

fn owned(value: Variant<'static>) -> OwnedVariant {
    OwnedVariant::try_from(value).expect("owned variant")
}

fn owned_string(value: String) -> OwnedVariant {
    owned(Variant::from(value))
}

fn demo_tracks(args: &Args) -> Vec<MockTrack> {
    let art = args.art.as_ref().map(|path| {
        path.canonicalize()
            .map(|path| format!("file://{}", path.display()))
            .unwrap_or_else(|_| format!("file://{}", path.display()))
    });
    if let Some(title) = &args.title {
        return vec![MockTrack {
            title: title.clone(),
            artist: args.artist.clone(),
            album: String::new(),
            length_us: (args.duration_secs * 1_000_000) as i64,
            art_url: art,
        }];
    }
    vec![
        MockTrack {
            title: "Test Track One".to_owned(),
            artist: "The Mock Band".to_owned(),
            album: "Mock Sessions".to_owned(),
            length_us: (args.duration_secs * 1_000_000) as i64,
            art_url: art.clone(),
        },
        MockTrack {
            title: "Test Track Two".to_owned(),
            artist: "The Mock Band".to_owned(),
            album: "Mock Sessions".to_owned(),
            length_us: 205_000_000,
            art_url: art.clone(),
        },
        MockTrack {
            title: "Test Track Three".to_owned(),
            artist: "A Different Artist".to_owned(),
            album: "Encore".to_owned(),
            length_us: 164_000_000,
            art_url: art,
        },
    ]
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let args = Args::parse();
    let suffix = args
        .player_name
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || *character == '_')
        .collect::<String>();
    anyhow::ensure!(!suffix.is_empty(), "player name is empty");
    let bus_name = format!("org.mpris.MediaPlayer2.{suffix}");

    let state = MockState {
        tracks: demo_tracks(&args),
        index: 0,
        playing: !args.paused,
        position_base_us: 0,
        started_at: Instant::now(),
        can_next: !args.no_next,
        can_seek: !args.no_seek,
    };
    let state = Arc::new(Mutex::new(state));

    let connection = Connection::session()
        .await
        .context("could not connect to the session bus")?;
    connection
        .request_name(bus_name.clone())
        .await
        .with_context(|| format!("could not own {bus_name}"))?;
    connection
        .object_server()
        .at(
            "/org/mpris/MediaPlayer2",
            MockRoot {
                identity: args.identity.clone(),
            },
        )
        .await
        .context("could not register the MPRIS root interface")?;
    connection
        .object_server()
        .at(
            "/org/mpris/MediaPlayer2",
            MockPlayer {
                state: Arc::clone(&state),
            },
        )
        .await
        .context("could not register the MPRIS player interface")?;

    eprintln!("mock MPRIS player ready: {bus_name}");
    eprintln!("  identity: {}", args.identity);
    for (index, track) in demo_tracks(&args).iter().enumerate() {
        eprintln!(
            "  {}. {} — {} ({:.0}s)",
            index + 1,
            track.title,
            track.artist,
            track.length_us as f64 / 1_000_000.0
        );
    }
    eprintln!("press Ctrl+C or kill this process to remove it from the bus");
    let _ = InterfaceName::from_static_str_unchecked(PLAYER_IFACE);
    std::future::pending::<()>().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_advances_and_clamps_seeks() {
        let mut state = MockState {
            tracks: vec![
                MockTrack {
                    title: "A".into(),
                    artist: "B".into(),
                    album: String::new(),
                    length_us: 10_000_000,
                    art_url: None,
                },
                MockTrack {
                    title: "B".into(),
                    artist: "B".into(),
                    album: String::new(),
                    length_us: 10_000_000,
                    art_url: None,
                },
            ],
            index: 0,
            playing: true,
            position_base_us: 0,
            started_at: Instant::now(),
            can_next: true,
            can_seek: true,
        };
        state.seek_to(12_000_000);
        assert_eq!(state.position_base_us, 10_000_000);
        state.advance(1);
        assert_eq!(state.index, 1);
        state.advance(1);
        assert_eq!(state.index, 0);
    }
}
