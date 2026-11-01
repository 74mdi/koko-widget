//! Single-instance control channel and CLI command transport.
//!
//! The overlay owns a well-known D-Bus name. A second process sends its command
//! to the primary instance and exits instead of creating a second widget.

use std::{future::pending, sync::Arc, time::Duration};

use anyhow::{Context, Result};

pub const SERVICE_NAME: &str = "io.github.kokowidget.KokoWidget.Control";
pub const OBJECT_PATH: &str = "/io/github/kokowidget/KokoWidget";
pub const INTERFACE: &str = "io.github.kokowidget.KokoWidget.Control";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Show,
    Hide,
    ToggleVisibility,
    Expand,
    Collapse,
    ToggleExpanded,
    Next,
    Previous,
    PlayPause,
    TogglePin,
    Settings,
    Menu,
    Quit,
}

impl Action {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Show => "show",
            Self::Hide => "hide",
            Self::ToggleVisibility => "toggle-visibility",
            Self::Expand => "expand",
            Self::Collapse => "collapse",
            Self::ToggleExpanded => "toggle-expanded",
            Self::Next => "next",
            Self::Previous => "previous",
            Self::PlayPause => "play-pause",
            Self::TogglePin => "toggle-pin",
            Self::Settings => "settings",
            Self::Menu => "menu",
            Self::Quit => "quit",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value.trim() {
            "show" => Self::Show,
            "hide" => Self::Hide,
            "toggle-visibility" | "toggle" => Self::ToggleVisibility,
            "expand" => Self::Expand,
            "collapse" => Self::Collapse,
            "toggle-expanded" => Self::ToggleExpanded,
            "next" => Self::Next,
            "previous" => Self::Previous,
            "play-pause" => Self::PlayPause,
            "toggle-pin" => Self::TogglePin,
            "settings" => Self::Settings,
            "menu" => Self::Menu,
            "quit" => Self::Quit,
            _ => return None,
        })
    }
}

pub type ActionCallback = Arc<dyn Fn(Action) + Send + Sync + 'static>;

struct ControlInterface {
    callback: ActionCallback,
}

#[zbus::interface(name = "io.github.kokowidget.KokoWidget.Control")]
impl ControlInterface {
    async fn perform(&self, action: &str) -> zbus::fdo::Result<()> {
        let action = Action::parse(action)
            .ok_or_else(|| zbus::fdo::Error::InvalidArgs(format!("unknown action: {action}")))?;
        tracing::debug!(?action, "control action received");
        (self.callback)(action);
        Ok(())
    }

    async fn ping(&self) -> zbus::fdo::Result<String> {
        Ok("pong".to_owned())
    }

    async fn version(&self) -> zbus::fdo::Result<String> {
        Ok(crate::VERSION.to_owned())
    }
}

/// Spawn the primary-instance D-Bus service. Returns immediately; if another
/// primary already owns the name an error is returned back to the caller.
pub fn spawn_server(callback: ActionCallback) -> Result<()> {
    let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("koko-ipc".to_owned())
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(error) => {
                    let _ = ready_tx.send(Err(format!("could not start IPC runtime: {error}")));
                    return;
                }
            };
            runtime.block_on(async move {
                let result = async {
                    let connection = zbus::Connection::session()
                        .await
                        .context("could not connect to session D-Bus")?;
                    connection
                        .request_name(SERVICE_NAME)
                        .await
                        .context("could not own single-instance D-Bus name")?;
                    connection
                        .object_server()
                        .at(OBJECT_PATH, ControlInterface { callback })
                        .await
                        .context("could not publish control interface")?;
                    let _ = ready_tx.send(Ok(()));
                    pending::<()>().await;
                    Ok::<(), anyhow::Error>(())
                }
                .await;
                if let Err(error) = result {
                    tracing::warn!(%error, "single-instance control service stopped");
                }
            });
        })
        .context("could not spawn IPC thread")?;

    match ready_rx.recv_timeout(Duration::from_secs(3)) {
        Ok(Ok(())) => Ok(()),
        Ok(Err(message)) => anyhow::bail!(message),
        Err(error) => anyhow::bail!("IPC service did not become ready: {error}"),
    }
}

/// Send an action to the primary instance. Returns false if no primary exists
/// (or the bus is unavailable), so the caller can fall back to starting one.
pub fn try_send_action(action: Action) -> bool {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            tracing::debug!(%error, "could not create IPC client runtime");
            return false;
        }
    };
    runtime.block_on(async move {
        let fut = async {
            let connection = zbus::Connection::session().await?;
            connection
                .call_method(
                    Some(SERVICE_NAME),
                    OBJECT_PATH,
                    Some(INTERFACE),
                    "Perform",
                    &(action.as_str(),),
                )
                .await?;
            Ok::<(), zbus::Error>(())
        };
        matches!(
            tokio::time::timeout(Duration::from_secs(2), fut).await,
            Ok(Ok(()))
        )
    })
}

pub fn try_quit_existing() -> bool {
    try_send_action(Action::Quit)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_strings_round_trip() {
        for action in [
            Action::Show,
            Action::Hide,
            Action::ToggleVisibility,
            Action::Expand,
            Action::Collapse,
            Action::ToggleExpanded,
            Action::Next,
            Action::Previous,
            Action::PlayPause,
            Action::TogglePin,
            Action::Settings,
            Action::Quit,
        ] {
            assert_eq!(Action::parse(action.as_str()), Some(action));
        }
        assert_eq!(Action::parse("toggle"), Some(Action::ToggleVisibility));
        assert_eq!(Action::parse("nonsense"), None);
    }
}
