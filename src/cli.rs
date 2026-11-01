//! Command-line interface.

use clap::{ArgGroup, Parser};

use crate::ipc::Action;

#[derive(Debug, Parser, Clone)]
#[command(
    name = "koko-widget",
    version,
    about = "A native Liquid Glass music overlay for Linux and FreeBSD",
    long_about = "KOKO WIDGET is a slim, floating MPRIS music overlay. It works with \
                  any player that exposes the standard MPRIS D-Bus interface."
)]
#[command(group(
    ArgGroup::new("action")
        .multiple(false)
        .args([
            "show", "hide", "toggle", "expand", "collapse", "toggle_expanded",
            "next", "previous", "play_pause", "toggle_pin", "settings",
            "menu", "diagnostics", "quit"
        ])
))]
pub struct Cli {
    /// Start without showing the overlay until playback begins.
    #[arg(long)]
    pub start_hidden: bool,

    /// Show the existing overlay (or start it if needed).
    #[arg(long)]
    pub show: bool,

    /// Hide the overlay without quitting.
    #[arg(long)]
    pub hide: bool,

    /// Toggle overlay visibility.
    #[arg(long)]
    pub toggle: bool,

    /// Expand the overlay and pin it open.
    #[arg(long)]
    pub expand: bool,

    /// Collapse the overlay and clear the pin.
    #[arg(long)]
    pub collapse: bool,

    /// Toggle the expanded/compact presentation.
    #[arg(long)]
    pub toggle_expanded: bool,

    /// Skip to the next track in the selected player.
    #[arg(long)]
    pub next: bool,

    /// Return to the previous track in the selected player.
    #[arg(long)]
    pub previous: bool,

    /// Toggle play/pause in the selected player.
    #[arg(long)]
    pub play_pause: bool,

    /// Pin or unpin the expanded state.
    #[arg(long)]
    pub toggle_pin: bool,

    /// Open the settings window.
    #[arg(long)]
    pub settings: bool,

    /// Open the main menu (the launcher action).
    #[arg(long)]
    pub menu: bool,

    /// Print a compatibility and media diagnostics report.
    #[arg(long)]
    pub diagnostics: bool,

    /// Ask the running instance to quit.
    #[arg(long)]
    pub quit: bool,
}

impl Cli {
    pub fn action(&self) -> Option<Action> {
        if self.show {
            Some(Action::Show)
        } else if self.hide {
            Some(Action::Hide)
        } else if self.toggle {
            Some(Action::ToggleVisibility)
        } else if self.expand {
            Some(Action::Expand)
        } else if self.collapse {
            Some(Action::Collapse)
        } else if self.toggle_expanded {
            Some(Action::ToggleExpanded)
        } else if self.next {
            Some(Action::Next)
        } else if self.previous {
            Some(Action::Previous)
        } else if self.play_pause {
            Some(Action::PlayPause)
        } else if self.toggle_pin {
            Some(Action::TogglePin)
        } else if self.settings {
            Some(Action::Settings)
        } else if self.menu {
            Some(Action::Menu)
        } else if self.diagnostics {
            None
        } else if self.quit {
            Some(Action::Quit)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_flags_are_exclusive() {
        assert!(Cli::try_parse_from(["koko-widget", "--next", "--previous"]).is_err());
    }

    #[test]
    fn parses_action_flags() {
        let cli = Cli::try_parse_from(["koko-widget", "--play-pause"]).unwrap();
        assert_eq!(cli.action(), Some(Action::PlayPause));
        let cli = Cli::try_parse_from(["koko-widget", "--start-hidden"]).unwrap();
        assert_eq!(cli.action(), None);
        let cli = Cli::try_parse_from(["koko-widget", "--toggle"]).unwrap();
        assert_eq!(cli.action(), Some(Action::ToggleVisibility));
        let cli = Cli::try_parse_from(["koko-widget", "--menu"]).unwrap();
        assert_eq!(cli.action(), Some(Action::Menu));
    }
}
