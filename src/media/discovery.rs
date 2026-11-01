//! MPRIS service discovery.
//!
//! Discovery is event-driven: `NameOwnerChanged` wakes the media worker only
//! when an MPRIS name appears or disappears. The app does not poll the bus.

use anyhow::{Context, Result};
use zbus::Connection;

use crate::media::metadata::MPRIS_PREFIX;

pub fn is_mpris_name(name: &str) -> bool {
    let Some(suffix) = name.strip_prefix(MPRIS_PREFIX) else {
        return false;
    };
    !suffix.is_empty() && !suffix.starts_with(':') && !name.starts_with(':')
}

pub async fn list_players(connection: &Connection) -> Result<Vec<String>> {
    let proxy = zbus::fdo::DBusProxy::new(connection)
        .await
        .context("could not connect to the session bus")?;
    let mut names = proxy
        .list_names()
        .await
        .context("could not list session bus names")?
        .into_iter()
        .map(|name| name.to_string())
        .filter(|name| is_mpris_name(name))
        .collect::<Vec<_>>();
    names.sort();
    names.dedup();
    Ok(names)
}

/// A player removal is delivered as an event rather than discovered by polling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnerChange {
    pub name: String,
    pub old_owner: String,
    pub new_owner: String,
}

impl OwnerChange {
    pub fn is_removal(&self) -> bool {
        !self.old_owner.is_empty() && self.new_owner.is_empty()
    }

    pub fn is_addition(&self) -> bool {
        self.old_owner.is_empty() && !self.new_owner.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_mpris_names_but_not_unique_connections() {
        assert!(is_mpris_name("org.mpris.MediaPlayer2.spotify"));
        assert!(is_mpris_name("org.mpris.MediaPlayer2.playerctld"));
        assert!(!is_mpris_name("org.mpris.MediaPlayer2"));
        assert!(!is_mpris_name("org.mpris.MediaPlayer2."));
        assert!(!is_mpris_name(":1.42"));
        assert!(!is_mpris_name("org.example.SomethingElse"));
    }

    #[test]
    fn owner_change_classifies_additions_and_removals() {
        let added = OwnerChange {
            name: "org.mpris.MediaPlayer2.vlc".into(),
            old_owner: String::new(),
            new_owner: ":1.66".into(),
        };
        assert!(added.is_addition());
        assert!(!added.is_removal());
        let removed = OwnerChange {
            name: "org.mpris.MediaPlayer2.vlc".into(),
            old_owner: ":1.66".into(),
            new_owner: String::new(),
        };
        assert!(removed.is_removal());
    }
}
