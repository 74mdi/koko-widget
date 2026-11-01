//! Event-driven MPRIS media worker.
//!
//! The worker owns one D-Bus connection, watches `NameOwnerChanged` to discover
//! players, and watches `PropertiesChanged`/`Seeked` per active player. It never
//! polls the bus for metadata. The only local timer is the UI's progress clock.

use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};

use anyhow::{Context, Result};
use futures_util::StreamExt;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tracing::{debug, info, warn};
use zbus::Connection;

use crate::{
    config::Media as MediaConfig,
    media::{
        discovery,
        metadata::{MPRIS_PREFIX, PlaybackStatus, PlayerInfo, PlayerSnapshot},
        mpris::{self, PLAYER_INTERFACE, ParsedPlayerProperties},
        player_manager::{SelectionCandidate, choose_bus_name},
    },
};

#[derive(Debug, Clone)]
pub enum MediaCommand {
    PlayPause,
    Play,
    Pause,
    Next,
    Previous,
    SeekTo(i64),
    SeekBy(i64),
    Configure(MediaConfig),
    Refresh,
}

#[derive(Debug, Clone)]
pub enum MediaEvent {
    /// Currently selected player, or `None` when no usable player exists.
    Snapshot(Option<PlayerSnapshot>),
    /// All discovered players, for the settings window.
    Players(Vec<PlayerInfo>),
    /// A transient error that made an operation fail. Never used to fake state.
    Error(String),
}

pub type MediaCallback = Arc<dyn Fn(MediaEvent) + Send + Sync + 'static>;

/// Handle used by the GTK thread to send commands to the worker.
#[derive(Clone)]
pub struct MediaService {
    command_tx: UnboundedSender<MediaCommand>,
}

impl MediaService {
    pub fn spawn(config: MediaConfig, callback: MediaCallback) -> Result<Self> {
        let (command_tx, command_rx) = unbounded_channel();
        std::thread::Builder::new()
            .name("koko-media".to_owned())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        callback(MediaEvent::Error(format!(
                            "Could not start the media runtime: {error}"
                        )));
                        return;
                    }
                };
                runtime.block_on(async move {
                    if let Err(error) = run_media_worker(config, command_rx, callback.clone()).await
                    {
                        warn!(%error, "MPRIS worker stopped");
                        callback(MediaEvent::Error(format!(
                            "Media service unavailable: {error:#}"
                        )));
                        callback(MediaEvent::Snapshot(None));
                    }
                });
            })
            .context("could not spawn MPRIS worker thread")?;
        Ok(Self { command_tx })
    }

    pub fn send(&self, command: MediaCommand) {
        if let Err(error) = self.command_tx.send(command) {
            tracing::warn!(%error, "media command dropped");
        }
    }

    pub fn play_pause(&self) {
        self.send(MediaCommand::PlayPause);
    }

    pub fn next(&self) {
        self.send(MediaCommand::Next);
    }

    pub fn previous(&self) {
        self.send(MediaCommand::Previous);
    }

    pub fn seek_to(&self, position_us: i64) {
        self.send(MediaCommand::SeekTo(position_us.max(0)));
    }

    pub fn configure(&self, config: MediaConfig) {
        self.send(MediaCommand::Configure(config));
    }
}

#[derive(Debug, Clone)]
struct PlayerRecord {
    snapshot: PlayerSnapshot,
    last_activity_seq: u64,
    became_playing_seq: u64,
}

impl PlayerRecord {
    fn info(&self) -> PlayerInfo {
        PlayerInfo {
            bus_name: self.snapshot.bus_name.clone(),
            identity: self.snapshot.identity.clone(),
            status: self.snapshot.status,
            track_title: self.snapshot.track.title.clone(),
            artist: self.snapshot.track.artist_line(),
        }
    }
}

#[derive(Debug)]
enum InternalEvent {
    Update { record: PlayerRecord },
    TaskEnded { bus_name: String, reason: String },
    Refresh { forced: Option<String> },
}

#[derive(Default)]
struct WorkerState {
    records: HashMap<String, PlayerRecord>,
    tasks: HashMap<String, tokio::task::JoinHandle<()>>,
    selected: Option<String>,
    last_players: Vec<PlayerInfo>,
}

async fn run_media_worker(
    initial_config: MediaConfig,
    mut commands: UnboundedReceiver<MediaCommand>,
    emit: MediaCallback,
) -> Result<()> {
    let connection = Connection::session()
        .await
        .context("could not connect to the session D-Bus")?;
    info!("connected to session bus for MPRIS");
    let dbus = zbus::fdo::DBusProxy::new(&connection)
        .await
        .context("could not create the D-Bus proxy")?;
    let mut owner_changes = dbus
        .receive_name_owner_changed()
        .await
        .context("could not subscribe to NameOwnerChanged")?;

    let (internal_tx, mut internal_rx) = unbounded_channel();
    let sequence = Arc::new(AtomicU64::new(1));
    let mut state = WorkerState::default();
    let mut config = initial_config.clone();

    refresh_players(
        &connection,
        &mut state,
        &internal_tx,
        &sequence,
        &emit,
        None,
    )
    .await;
    reselect(&mut state, &config, &emit);

    loop {
        tokio::select! {
            command = commands.recv() => {
                match command {
                    Some(MediaCommand::Configure(media)) => {
                        config = media;
                        reselect(&mut state, &config, &emit);
                    }
                    Some(command) => {
                        if let Err(error) = handle_command(
                            &connection,
                            &mut state,
                            command,
                            &emit,
                        ).await {
                            warn!(%error, "MPRIS command failed");
                            emit(MediaEvent::Error(format!("{error:#}")));
                        }
                    }
                    None => break,
                }
            }
            event = internal_rx.recv() => {
                match event {
                    Some(InternalEvent::Update { record }) => {
                        let bus = record.snapshot.bus_name.clone();
                        state.records.insert(bus, record);
                        // Selection depends on every player, not just the
                        // currently selected one: a previously paused player
                        // may have just started playing and must take over.
                        reselect(&mut state, &config, &emit);
                    }
                    Some(InternalEvent::TaskEnded { bus_name, reason }) => {
                        warn!(%bus_name, %reason, "MPRIS player task ended");
                        state.records.remove(&bus_name);
                        state.tasks.remove(&bus_name);
                        if state.selected.as_deref() == Some(bus_name.as_str()) {
                            state.selected = None;
                        }
                        reselect(&mut state, &config, &emit);
                    }
                    Some(InternalEvent::Refresh { forced }) => {
                        refresh_players(
                            &connection,
                            &mut state,
                            &internal_tx,
                            &sequence,
                            &emit,
                            forced.as_deref(),
                        ).await;
                        reselect(&mut state, &config, &emit);
                    }
                    None => break,
                }
            }
            change = owner_changes.next() => {
                match change {
                    Some(change) => {
                        let Ok(args) = change.args() else { continue };
                        let owner = args.name();
                        let Some(name) = mpris_well_known_name(owner.as_str()) else { continue };
                        debug!(name = %owner, "MPRIS NameOwnerChanged");
                        let _ = internal_tx.send(InternalEvent::Refresh {
                            forced: Some(format!("{MPRIS_PREFIX}{name}")),
                        });
                    }
                    None => break,
                }
            }
        }
    }
    for (_, task) in state.tasks.drain() {
        task.abort();
    }
    Ok(())
}

fn mpris_well_known_name(name: &str) -> Option<&str> {
    if !discovery::is_mpris_name(name) {
        return None;
    }
    name.strip_prefix(MPRIS_PREFIX)
}

async fn refresh_players(
    connection: &Connection,
    state: &mut WorkerState,
    internal_tx: &UnboundedSender<InternalEvent>,
    sequence: &Arc<AtomicU64>,
    emit: &MediaCallback,
    force_restart: Option<&str>,
) {
    let names = match discovery::list_players(connection).await {
        Ok(names) => {
            debug!(?names, "MPRIS discovery refresh");
            names
        }
        Err(error) => {
            warn!(%error, "could not discover MPRIS players");
            emit(MediaEvent::Error(format!(
                "Player discovery failed: {error:#}"
            )));
            return;
        }
    };

    if let Some(forced) = force_restart
        && names.iter().any(|name| name == forced)
    {
        if let Some(handle) = state.tasks.remove(forced) {
            handle.abort();
        }
        state.records.remove(forced);
        if state.selected.as_deref() == Some(forced) {
            state.selected = None;
        }
    }

    for name in &names {
        if state.tasks.contains_key(name) {
            continue;
        }
        info!(%name, "starting MPRIS player watcher");
        let connection = connection.clone();
        let internal_tx = internal_tx.clone();
        let sequence = Arc::clone(sequence);
        let bus_name = name.clone();
        let task = tokio::spawn(async move {
            if let Err(error) =
                run_player_task(connection, bus_name.clone(), internal_tx.clone(), sequence).await
            {
                let _ = internal_tx.send(InternalEvent::TaskEnded {
                    bus_name,
                    reason: format!("{error:#}"),
                });
            }
        });
        state.tasks.insert(name.clone(), task);
    }

    let stale = state
        .tasks
        .keys()
        .filter(|name| !names.contains(name))
        .cloned()
        .collect::<Vec<_>>();
    for name in stale {
        if let Some(task) = state.tasks.remove(&name) {
            task.abort();
        }
        state.records.remove(&name);
        if state.selected.as_deref() == Some(name.as_str()) {
            state.selected = None;
        }
    }
    emit_players_if_changed(state, emit);
}

async fn run_player_task(
    connection: Connection,
    bus_name: String,
    internal_tx: UnboundedSender<InternalEvent>,
    sequence: Arc<AtomicU64>,
) -> Result<()> {
    // Subscribe before fetching so no metadata change can slip through the gap.
    let mut property_changes = mpris::subscribe_property_changes(&connection, &bus_name).await?;
    let mut seeked = mpris::subscribe_seeked(&connection, &bus_name).await?;

    let (identity, _root, initial_properties) = mpris::fetch_all(&connection, &bus_name).await?;
    info!(%bus_name, %identity, "MPRIS player watcher attached");
    let mut parsed = ParsedPlayerProperties::from_properties(&initial_properties);
    if !parsed.has_position
        && let Ok(position) = mpris::fetch_position(&connection, &bus_name).await
    {
        parsed.position_us = position;
        parsed.has_position = true;
    }
    let mut previous_status = parsed.status;
    let mut previous_track = parsed.track.identity();
    let initial_seq = sequence.fetch_add(1, Ordering::Relaxed);
    let mut playing_since = if parsed.status == PlaybackStatus::Playing {
        initial_seq
    } else {
        0
    };
    send_update(
        &internal_tx,
        &bus_name,
        &identity,
        &parsed,
        initial_seq,
        playing_since,
    );

    loop {
        tokio::select! {
            change = property_changes.next() => {
                let Some(change) = change else { break };
                let Ok(args) = change.args() else { continue };
                if args.interface_name().as_str() != PLAYER_INTERFACE {
                    continue;
                }
                let changed = args.changed_properties();
                let invalidated = args.invalidated_properties();
                if !invalidated.is_empty() {
                    match mpris::get_all(&connection, &bus_name, PLAYER_INTERFACE).await {
                        Ok(map) => parsed = ParsedPlayerProperties::from_properties(&map),
                        Err(error) => {
                            debug!(%error, "could not refresh invalidated MPRIS metadata");
                            continue;
                        }
                    }
                } else {
                    parsed.apply_changed(changed);
                }

                let track_changed = parsed.track.identity() != previous_track;
                let status_changed = parsed.status != previous_status;
                // Position is read-only and has no change signal. Re-read it only
                // for property updates (not on a timer), preserving an accurate
                // local projection without polling.
                if (track_changed || status_changed || !parsed.has_position)
                    && let Ok(position) = mpris::fetch_position(&connection, &bus_name).await {
                        parsed.position_us = position;
                        parsed.has_position = true;
                    }

                let now_seq = sequence.fetch_add(1, Ordering::Relaxed);
                if parsed.status == PlaybackStatus::Playing {
                    if previous_status != PlaybackStatus::Playing {
                        playing_since = now_seq;
                    }
                } else {
                    playing_since = 0;
                }
                previous_status = parsed.status;
                previous_track = parsed.track.identity();
                send_update(
                    &internal_tx,
                    &bus_name,
                    &identity,
                    &parsed,
                    now_seq,
                    playing_since,
                );
            }
            signal = seeked.next() => {
                let Some(signal) = signal else { break };
                if let Ok(position) = mpris::parse_seeked(&signal) {
                    parsed.position_us = position;
                    parsed.has_position = true;
                    let seq = sequence.fetch_add(1, Ordering::Relaxed);
                    if parsed.status != PlaybackStatus::Playing {
                        playing_since = 0;
                    }
                    send_update(
                        &internal_tx,
                        &bus_name,
                        &identity,
                        &parsed,
                        seq,
                        playing_since,
                    );
                }
            }
        }
    }
    Ok(())
}

fn send_update(
    internal_tx: &UnboundedSender<InternalEvent>,
    bus_name: &str,
    identity: &str,
    parsed: &ParsedPlayerProperties,
    activity_seq: u64,
    became_playing_seq: u64,
) {
    let snapshot = parsed.to_snapshot(bus_name, identity, Instant::now());
    let _ = internal_tx.send(InternalEvent::Update {
        record: PlayerRecord {
            snapshot,
            last_activity_seq: activity_seq,
            became_playing_seq,
        },
    });
}

fn candidate_for(record: &PlayerRecord, _config: &MediaConfig) -> SelectionCandidate {
    let mut candidate = SelectionCandidate::new(
        record.snapshot.bus_name.clone(),
        record.snapshot.identity.clone(),
    );
    candidate.status = record.snapshot.status;
    candidate.became_playing_seq = record.became_playing_seq;
    candidate.last_activity_seq = record.last_activity_seq;
    candidate
}

fn reselect(state: &mut WorkerState, config: &MediaConfig, emit: &MediaCallback) {
    let candidates = state
        .records
        .values()
        .map(|record| candidate_for(record, config))
        .collect::<Vec<_>>();
    let chosen = choose_bus_name(&candidates, state.selected.as_deref(), config);
    if chosen != state.selected {
        state.selected = chosen;
    }
    let snapshot = state
        .selected
        .as_ref()
        .and_then(|bus_name| state.records.get(bus_name))
        .map(|record| record.snapshot.clone());
    debug!(
        selected = ?state.selected,
        candidates = state.records.len(),
        status = ?snapshot.as_ref().map(|snapshot| snapshot.status),
        "MPRIS selection updated"
    );
    emit(MediaEvent::Snapshot(snapshot));
    emit_players_if_changed(state, emit);
}

fn emit_players_if_changed(state: &mut WorkerState, emit: &MediaCallback) {
    let mut players = state
        .records
        .values()
        .map(PlayerRecord::info)
        .collect::<Vec<_>>();
    players.sort_by(|a, b| {
        a.identity
            .to_lowercase()
            .cmp(&b.identity.to_lowercase())
            .then_with(|| a.bus_name.cmp(&b.bus_name))
    });
    if players != state.last_players {
        state.last_players = players.clone();
        emit(MediaEvent::Players(players));
    }
}

async fn handle_command(
    connection: &Connection,
    state: &mut WorkerState,
    command: MediaCommand,
    emit: &MediaCallback,
) -> Result<()> {
    match command {
        MediaCommand::Configure(_) => {
            // Configuration updates are handled by the main loop so that the
            // selection policy is re-applied once.
            Ok(())
        }
        MediaCommand::Refresh => {
            // Handled by the caller's internal refresh path.
            Ok(())
        }
        MediaCommand::PlayPause
        | MediaCommand::Play
        | MediaCommand::Pause
        | MediaCommand::Next
        | MediaCommand::Previous
        | MediaCommand::SeekTo(_)
        | MediaCommand::SeekBy(_) => {
            info!(?command, "executing MPRIS command");
            let Some(snapshot) = selected_snapshot(state) else {
                anyhow::bail!("no MPRIS player is selected");
            };
            let bus = snapshot.bus_name.clone();
            match command {
                MediaCommand::PlayPause => {
                    if snapshot.capabilities.can_control {
                        mpris::play_pause(connection, &bus).await?;
                    } else {
                        anyhow::bail!("the selected player cannot be controlled");
                    }
                }
                MediaCommand::Play => {
                    mpris::play(connection, &bus).await?;
                }
                MediaCommand::Pause => {
                    mpris::pause(connection, &bus).await?;
                }
                MediaCommand::Next => {
                    if !snapshot.capabilities.can_go_next {
                        anyhow::bail!("the selected player does not support next");
                    }
                    mpris::next(connection, &bus).await?;
                }
                MediaCommand::Previous => {
                    if !snapshot.capabilities.can_go_previous {
                        anyhow::bail!("the selected player does not support previous");
                    }
                    mpris::previous(connection, &bus).await?;
                }
                MediaCommand::SeekTo(target) => {
                    if !snapshot.capabilities.can_seek {
                        anyhow::bail!("the selected player does not support seeking");
                    }
                    let target = target.max(0);
                    let succeeded = if let Some(track_id) = snapshot.track.id.as_deref() {
                        match mpris::set_position(connection, &bus, track_id, target).await {
                            Ok(()) => true,
                            Err(error) if mpris::call_error_is_unsupported(&error) => false,
                            Err(error) => return Err(error),
                        }
                    } else {
                        false
                    };
                    if !succeeded {
                        let offset = target - snapshot.position_us;
                        mpris::seek(connection, &bus, offset).await?;
                    }
                    update_local_position(state, &bus, target, emit);
                }
                MediaCommand::SeekBy(offset) => {
                    if !snapshot.capabilities.can_seek {
                        anyhow::bail!("the selected player does not support seeking");
                    }
                    mpris::seek(connection, &bus, offset).await?;
                    let target = (snapshot.position_us + offset).max(0);
                    update_local_position(state, &bus, target, emit);
                }
                _ => unreachable!(),
            }
            Ok(())
        }
    }
}

fn selected_snapshot(state: &WorkerState) -> Option<&PlayerSnapshot> {
    state
        .selected
        .as_deref()
        .and_then(|name| state.records.get(name))
        .map(|record| &record.snapshot)
}

fn update_local_position(
    state: &mut WorkerState,
    bus_name: &str,
    position_us: i64,
    emit: &MediaCallback,
) {
    if let Some(record) = state.records.get_mut(bus_name) {
        record.snapshot.position_us = position_us.max(0);
        record.snapshot.captured_at = Instant::now();
        let snapshot = record.snapshot.clone();
        if state.selected.as_deref() == Some(bus_name) {
            emit(MediaEvent::Snapshot(Some(snapshot)));
        }
    }
}
