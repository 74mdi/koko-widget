//! `koko-widget --diagnostics`: an honest report of runtime capabilities and
//! discovered media players.

use crate::{
    config::Config,
    media::{
        discovery,
        metadata::PlaybackStatus,
        mpris::{self, ParsedPlayerProperties},
    },
    platform::{self, Capabilities, DisplayBackend},
    utils::{cache_dir, config_file, state_dir},
};
use anyhow::Result;

pub fn run() -> Result<()> {
    println!("KOKO WIDGET {}", crate::VERSION);
    println!("Native GTK4 / MPRIS overlay for Linux and FreeBSD");
    println!();

    println!("Platform");
    println!("  OS:              {}", std::env::consts::OS);
    if let Some(distro) = platform::distribution_name() {
        println!("  Distribution:    {distro}");
    }
    println!(
        "  Session type:    {}",
        std::env::var("XDG_SESSION_TYPE")
            .ok()
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "unknown".to_owned())
    );
    let niri = std::env::var("NIRI_SOCKET")
        .ok()
        .filter(|value| !value.is_empty())
        .is_some();
    if niri {
        println!("  Niri session:    yes (NIRI_SOCKET is set)");
    }
    println!();

    println!("Display and window backend");
    if gtk::init().is_ok() {
        let capabilities = platform::detect();
        print_capabilities(&capabilities);
    } else {
        println!("  GTK display:     unavailable (headless or GDK backend not running)");
        println!("  Window backend:  unavailable");
    }
    println!();

    println!("Media");
    print_mpris_report();
    println!();

    println!("Paths");
    println!("  Configuration:   {}", config_file().display());
    let outcome = Config::load();
    println!(
        "  Config status:   {}",
        if outcome.warnings.is_empty() {
            "valid (or defaults in use)"
        } else {
            "recovered with warnings"
        }
    );
    for warning in &outcome.warnings {
        println!("    ! {warning}");
    }
    println!(
        "  Artwork cache:   {}",
        cache_dir().join("artwork").display()
    );
    println!("  State directory: {}", state_dir().display());
    println!();

    println!("Glass / compositing");
    println!("  Real background blur is compositor-provided. KOKO WIDGET never captures");
    println!("  the screen or pretends blur exists. Layer namespace: `koko-widget`.");
    if niri {
        println!("  Niri: add a layer rule to your config if you want real blur:");
        println!("    layer-rule {{");
        println!("        match namespace=\"^koko-widget$\"");
        println!("        background-effect {{ blur true }}");
        println!("    }}");
        println!("  KOKO WIDGET does not modify your compositor configuration.");
    } else {
        println!("  Use a compositor rule for the layer namespace if supported; the layered");
        println!("  gradient and highlight fallback remains legible without blur.");
    }
    println!();

    println!("Limitations are documented in docs/compatibility.md.");
    Ok(())
}

fn print_capabilities(capabilities: &Capabilities) {
    println!("  GDK backend:     {}", capabilities.gdk_backend_name);
    println!("  Backend:         {}", capabilities.backend.label());
    if let Some(compositor) = &capabilities.compositor_hint {
        println!("  Compositor hint: {compositor}");
    }
    println!(
        "  Layer-shell:     {} ({})",
        if capabilities.layer_shell {
            "supported"
        } else {
            "unavailable"
        },
        if capabilities.layer_shell_compiled {
            "compiled in"
        } else {
            "build feature disabled"
        }
    );
    println!(
        "  Input shaping:   {}",
        if capabilities.input_shapes {
            "supported"
        } else {
            "unavailable (transparent margins may capture clicks)"
        }
    );
    println!("  GTK runtime:     {}", capabilities.gtk_runtime);
    println!(
        "  Reduced motion:  {}",
        if capabilities.reduced_motion {
            "yes"
        } else {
            "no"
        }
    );
    println!(
        "  Dark theme:      {}",
        if capabilities.dark_theme { "yes" } else { "no" }
    );
    if capabilities.monitors.is_empty() {
        println!("  Monitors:        none reported");
    } else {
        for (index, monitor) in capabilities.monitors.iter().enumerate() {
            println!(
                "  Monitor {}:      {} — {}x{} at {},{} @ {}x",
                index + 1,
                monitor.label(index),
                monitor.width,
                monitor.height,
                monitor.x,
                monitor.y,
                monitor.scale
            );
        }
    }
    if capabilities.backend == DisplayBackend::Wayland && !capabilities.layer_shell {
        println!("  Note: the interface works as a normal Wayland toplevel, but global");
        println!("        always-on-top and exact positioning are compositor decisions.");
    }
    if capabilities.backend == DisplayBackend::X11 {
        println!("  Note: always-on-top uses EWMH hints; the window manager may override them.");
        println!("        Transparency requires an active X11 compositor.");
    }
}

fn print_mpris_report() {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            println!("  Session bus:     unavailable ({error})");
            return;
        }
    };
    runtime.block_on(async {
        let connection = match zbus::Connection::session().await {
            Ok(connection) => connection,
            Err(error) => {
                println!("  Session bus:     unavailable ({error})");
                return;
            }
        };
        let names = match discovery::list_players(&connection).await {
            Ok(names) => names,
            Err(error) => {
                println!("  Session bus:     discovery failed ({error:#})");
                return;
            }
        };
        if names.is_empty() {
            println!("  Session bus:     connected");
            println!("  MPRIS players:   none discovered");
            return;
        }
        println!("  Session bus:     connected");
        println!("  MPRIS players:   {}", names.len());
        for name in names {
            let result = async {
                let (identity, _root, properties) = mpris::fetch_all(&connection, &name).await?;
                let parsed = ParsedPlayerProperties::from_properties(&properties);
                let position = if parsed.has_position {
                    parsed.position_us
                } else {
                    mpris::fetch_position(&connection, &name).await.unwrap_or(0)
                };
                Ok::<_, anyhow::Error>((identity, parsed, position))
            }
            .await;
            match result {
                Ok((identity, parsed, position)) => {
                    let _ = position;
                    println!(
                        "    {identity} ({}) — {} — {}{}",
                        name.trim_start_matches("org.mpris.MediaPlayer2."),
                        status_label(parsed.status),
                        parsed.track.title,
                        if parsed.track.artists.is_empty() {
                            String::new()
                        } else {
                            format!(" — {}", parsed.track.artist_line())
                        }
                    );
                    println!(
                        "      control: {} · next: {} · previous: {} · seek: {}",
                        yes_no(parsed.capabilities.can_control),
                        yes_no(parsed.capabilities.can_go_next),
                        yes_no(parsed.capabilities.can_go_previous),
                        yes_no(parsed.capabilities.can_seek)
                    );
                    if let Some(url) = parsed.track.art_url.as_deref() {
                        println!("      artwork: {}", short(url, 96));
                    }
                }
                Err(error) => {
                    println!("    {} — could not read properties: {error:#}", name);
                }
            }
        }
    });
}

fn status_label(status: PlaybackStatus) -> &'static str {
    match status {
        PlaybackStatus::Playing => "Playing",
        PlaybackStatus::Paused => "Paused",
        PlaybackStatus::Stopped => "Stopped",
    }
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

fn short(input: &str, max: usize) -> String {
    if input.chars().count() <= max {
        return input.to_owned();
    }
    let mut result = input
        .chars()
        .take(max.saturating_sub(1))
        .collect::<String>();
    result.push('…');
    result
}
