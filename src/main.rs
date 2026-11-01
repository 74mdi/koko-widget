use std::process::ExitCode;

use clap::Parser;
use tracing_subscriber::EnvFilter;

use koko_widget::{app, cli::Cli, diagnostics, ipc, ipc::Action};

fn main() -> ExitCode {
    init_logging();
    let cli = Cli::parse();

    if cli.diagnostics {
        return match diagnostics::run() {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("KOKO WIDGET diagnostics failed: {error:#}");
                ExitCode::FAILURE
            }
        };
    }

    let action = cli.action();

    if let Some(action) = action {
        if ipc::try_send_action(action) {
            return ExitCode::SUCCESS;
        }
        match action {
            Action::Hide | Action::Collapse | Action::Quit => {
                // There is no running instance to modify.
                return ExitCode::SUCCESS;
            }
            Action::Next | Action::Previous | Action::PlayPause => {
                eprintln!("KOKO WIDGET is not running; open it first or use --show.");
                return ExitCode::FAILURE;
            }
            _ => {}
        }
    } else if ipc::try_send_action(Action::Show) {
        // A second bare launch reveals the existing overlay and exits.
        return ExitCode::SUCCESS;
    }

    match app::run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("KOKO WIDGET: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn init_logging() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("koko_widget=info,warn"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .compact()
        .try_init();
}
