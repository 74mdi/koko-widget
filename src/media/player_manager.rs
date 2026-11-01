//! Deterministic selection of one active MPRIS player.
//!
//! Commands are only ever addressed to the selected bus name, so choosing a
//! player cannot accidentally control several applications at once.

use crate::{
    config::Media as MediaConfig,
    media::metadata::{MPRIS_PREFIX, PlaybackStatus},
};

pub const PLAYERCTLD: &str = "org.mpris.MediaPlayer2.playerctld";

#[derive(Debug, Clone, PartialEq)]
pub struct SelectionCandidate {
    pub bus_name: String,
    pub identity: String,
    pub status: PlaybackStatus,
    /// Monotonic sequence number of the most recent transition to Playing.
    pub became_playing_seq: u64,
    /// Monotonic sequence number of the most recent metadata/capability change.
    pub last_activity_seq: u64,
    pub is_proxy: bool,
}

impl SelectionCandidate {
    pub fn new(bus_name: impl Into<String>, identity: impl Into<String>) -> Self {
        let bus_name = bus_name.into();
        Self {
            is_proxy: bus_name == PLAYERCTLD,
            bus_name,
            identity: identity.into(),
            status: PlaybackStatus::Stopped,
            became_playing_seq: 0,
            last_activity_seq: 0,
        }
    }

    pub fn suffix(&self) -> &str {
        self.bus_name
            .strip_prefix(MPRIS_PREFIX)
            .unwrap_or(&self.bus_name)
    }
}

/// Choose the player to display and control.
///
/// The policy is:
/// 1. A user-chosen player wins when it is playing.
/// 2. Otherwise the most recently started playing player wins.
/// 3. With nothing playing, the user's preferred player wins.
/// 4. Otherwise the configured priority list, playback state and activity time
///    break the tie.
///
/// `playerctld` is a compatibility proxy. It is ignored while a real player is
/// available so that commands do not go through two independent paths.
pub fn choose_bus_name(
    candidates: &[SelectionCandidate],
    current: Option<&str>,
    config: &MediaConfig,
) -> Option<String> {
    if candidates.is_empty() {
        return None;
    }

    if !config.auto_select {
        if let Some(current) = current
            && let Some(found) = candidates.iter().find(|c| c.bus_name == current)
            && !found.is_proxy
        {
            return Some(found.bus_name.clone());
        }
        if let Some(preferred) = preferred_candidate(candidates, config) {
            return Some(preferred.bus_name.clone());
        }
        return candidates
            .iter()
            .find(|candidate| !candidate.is_proxy)
            .map(|candidate| candidate.bus_name.clone());
    }

    let real: Vec<&SelectionCandidate> = candidates
        .iter()
        .filter(|candidate| !candidate.is_proxy)
        .collect();
    let pool = if real.is_empty() {
        candidates.iter().collect::<Vec<_>>()
    } else {
        real
    };

    let playing: Vec<&SelectionCandidate> = pool
        .iter()
        .copied()
        .filter(|candidate| candidate.status == PlaybackStatus::Playing)
        .collect();

    if !playing.is_empty() {
        if let Some(preferred) = preferred_from(&playing, config) {
            return Some(preferred.bus_name.clone());
        }
        let mut sorted = playing;
        sorted.sort_by(|a, b| {
            b.became_playing_seq
                .cmp(&a.became_playing_seq)
                .then_with(|| b.last_activity_seq.cmp(&a.last_activity_seq))
                .then_with(|| rank_for(a, config).cmp(&rank_for(b, config)))
                .then_with(|| a.bus_name.cmp(&b.bus_name))
        });
        return Some(sorted[0].bus_name.clone());
    }

    if let Some(preferred) = preferred_from(&pool, config) {
        return Some(preferred.bus_name.clone());
    }

    let mut sorted = pool;
    sorted.sort_by(|a, b| {
        let a_paused = a.status == PlaybackStatus::Paused;
        let b_paused = b.status == PlaybackStatus::Paused;
        b_paused
            .cmp(&a_paused)
            .then_with(|| rank_for(a, config).cmp(&rank_for(b, config)))
            .then_with(|| b.last_activity_seq.cmp(&a.last_activity_seq))
            .then_with(|| a.bus_name.cmp(&b.bus_name))
    });
    sorted.first().map(|candidate| candidate.bus_name.clone())
}

fn preferred_candidate<'a>(
    candidates: &'a [SelectionCandidate],
    config: &MediaConfig,
) -> Option<&'a SelectionCandidate> {
    preferred_from(&candidates.iter().collect::<Vec<_>>(), config)
}

fn preferred_from<'a>(
    candidates: &[&'a SelectionCandidate],
    config: &MediaConfig,
) -> Option<&'a SelectionCandidate> {
    let preferred = config.preferred_player.as_deref()?.trim();
    if preferred.is_empty() {
        return None;
    }
    candidates
        .iter()
        .copied()
        .find(|candidate| matches_entry(candidate, preferred))
}

pub fn matches_entry(candidate: &SelectionCandidate, entry: &str) -> bool {
    let entry = entry.trim();
    if entry.is_empty() {
        return false;
    }
    candidate.bus_name.eq_ignore_ascii_case(entry)
        || candidate.suffix().eq_ignore_ascii_case(entry)
        || candidate.identity.eq_ignore_ascii_case(entry)
}

/// Calculate a candidate's rank from the configured fallback priority list.
/// Players not in the list share the final rank.
fn rank_for(candidate: &SelectionCandidate, config: &MediaConfig) -> usize {
    priority_rank(
        &candidate.bus_name,
        &candidate.identity,
        &config.player_priority,
    )
}

pub fn priority_rank(bus_name: &str, identity: &str, priority: &[String]) -> usize {
    let suffix = bus_name.strip_prefix(MPRIS_PREFIX).unwrap_or(bus_name);
    priority
        .iter()
        .position(|entry| {
            entry.eq_ignore_ascii_case(bus_name)
                || entry.eq_ignore_ascii_case(suffix)
                || entry.eq_ignore_ascii_case(identity)
        })
        .unwrap_or(usize::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(
        name: &str,
        identity: &str,
        status: PlaybackStatus,
        playing_seq: u64,
        activity_seq: u64,
    ) -> SelectionCandidate {
        let mut candidate =
            SelectionCandidate::new(format!("org.mpris.MediaPlayer2.{name}"), identity);
        candidate.status = status;
        candidate.became_playing_seq = playing_seq;
        candidate.last_activity_seq = activity_seq;
        candidate
    }

    #[test]
    fn newest_playing_player_wins() {
        let candidates = vec![
            candidate("vlc", "VLC", PlaybackStatus::Playing, 1, 10),
            candidate("spotify", "Spotify", PlaybackStatus::Playing, 2, 7),
        ];
        let chosen = choose_bus_name(&candidates, None, &MediaConfig::default()).unwrap();
        assert_eq!(chosen, "org.mpris.MediaPlayer2.spotify");
    }

    #[test]
    fn preferred_player_wins_among_playing_players() {
        let mut config = MediaConfig::default();
        config.preferred_player = Some("vlc".into());
        let candidates = vec![
            candidate("vlc", "VLC", PlaybackStatus::Playing, 1, 10),
            candidate("spotify", "Spotify", PlaybackStatus::Playing, 2, 7),
        ];
        let chosen = choose_bus_name(&candidates, None, &config).unwrap();
        assert_eq!(chosen, "org.mpris.MediaPlayer2.vlc");
    }

    #[test]
    fn preferred_player_is_used_when_paused() {
        let mut config = MediaConfig::default();
        config.preferred_player = Some("Strawberry".into());
        let candidates = vec![
            candidate("vlc", "VLC", PlaybackStatus::Stopped, 0, 10),
            candidate("strawberry", "Strawberry", PlaybackStatus::Paused, 0, 2),
        ];
        let chosen = choose_bus_name(&candidates, None, &config).unwrap();
        assert_eq!(chosen, "org.mpris.MediaPlayer2.strawberry");
    }

    #[test]
    fn priority_list_breaks_ties() {
        let mut config = MediaConfig::default();
        config.player_priority = vec!["vlc".into(), "spotify".into()];
        let candidates = vec![
            candidate("spotify", "Spotify", PlaybackStatus::Paused, 0, 50),
            candidate("vlc", "VLC", PlaybackStatus::Paused, 0, 10),
        ];
        let chosen = choose_bus_name(&candidates, None, &config).unwrap();
        assert_eq!(chosen, "org.mpris.MediaPlayer2.vlc");
    }

    #[test]
    fn paused_beats_stopped() {
        let candidates = vec![
            candidate("stopped", "Stopped", PlaybackStatus::Stopped, 0, 50),
            candidate("paused", "Paused", PlaybackStatus::Paused, 0, 2),
        ];
        let chosen = choose_bus_name(&candidates, None, &MediaConfig::default()).unwrap();
        assert_eq!(chosen, "org.mpris.MediaPlayer2.paused");
    }

    #[test]
    fn playerctld_is_ignored_when_a_real_player_exists() {
        let proxy = candidate("playerctld", "playerctld", PlaybackStatus::Playing, 99, 99);
        let real = candidate("vlc", "VLC", PlaybackStatus::Paused, 0, 1);
        let chosen = choose_bus_name(&[proxy, real], None, &MediaConfig::default()).unwrap();
        assert_eq!(chosen, "org.mpris.MediaPlayer2.vlc");
    }

    #[test]
    fn playerctld_is_used_as_a_last_resort() {
        let proxy = candidate("playerctld", "playerctld", PlaybackStatus::Playing, 99, 99);
        let chosen = choose_bus_name(&[proxy], None, &MediaConfig::default()).unwrap();
        assert_eq!(chosen, PLAYERCTLD);
    }

    #[test]
    fn auto_select_off_keeps_current_player() {
        let mut config = MediaConfig::default();
        config.auto_select = false;
        config.preferred_player = Some("vlc".into());
        let candidates = vec![
            candidate("current", "Current", PlaybackStatus::Paused, 0, 1),
            candidate("vlc", "VLC", PlaybackStatus::Playing, 5, 5),
        ];
        let chosen =
            choose_bus_name(&candidates, Some("org.mpris.MediaPlayer2.current"), &config).unwrap();
        assert_eq!(chosen, "org.mpris.MediaPlayer2.current");
    }

    #[test]
    fn matches_full_name_suffix_and_identity() {
        let candidate = candidate("spotify", "Spotify", PlaybackStatus::Paused, 0, 0);
        assert!(matches_entry(&candidate, "spotify"));
        assert!(matches_entry(&candidate, "Spotify"));
        assert!(matches_entry(&candidate, "org.mpris.MediaPlayer2.spotify"));
        assert!(!matches_entry(&candidate, "vlc"));
    }

    #[test]
    fn priority_rank_uses_case_insensitive_match() {
        let priority = vec!["VLC".to_string(), "spotify".to_string()];
        assert_eq!(
            priority_rank("org.mpris.MediaPlayer2.vlc", "VLC", &priority),
            0
        );
        assert_eq!(
            priority_rank("org.mpris.MediaPlayer2.spotify", "Spotify", &priority),
            1
        );
        assert_eq!(
            priority_rank("org.mpris.MediaPlayer2.other", "Other", &priority),
            usize::MAX
        );
    }
}
