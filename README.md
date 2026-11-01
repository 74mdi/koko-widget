# KOKO WIDGET

A native, Liquid Glass inspired floating widget overlay for Linux and FreeBSD.
It shows your music, the time, the date and any text you want — as separate
glass pills you can arrange, restyle and move anywhere on the desktop.

Written in Rust with GTK4 and Cairo. No Electron, no Tauri, no WebView.

## What it does

- **Now playing** — MPRIS discovery and control for any standards-compliant
  player, with album artwork, transport controls and a seekable progress bar.
- **Clock** — live time with optional seconds, 12/24-hour or locale format.
- **Date** — any `strftime` pattern, with optional two-line wrapping.
- **Text** — a short note, greeting or reminder.
- Drag the row anywhere; the position is remembered.
- Left click a pill to expand the media card. Right click for the menu.
- A real settings window with a sidebar, live preview and auto-save.

Every widget is an independent pill with its own width, height, colours,
typography and corner radius. Any widget can also drop its background and render
as liquid-glass lettering directly on the desktop.

## Requirements

| | |
|---|---|
| Rust | 1.92 or newer (edition 2024) |
| GTK | 4.18 or newer |
| Optional | `gtk4-layer-shell` for Wayland layer surfaces |

```sh
# Debian / Ubuntu
sudo apt install libgtk-4-dev libgtk4-layer-shell-dev libpango1.0-dev

# Fedora
sudo dnf install gtk4-devel gtk4-layer-shell-devel pango-devel

# Arch
sudo pacman -S gtk4 gtk4-layer-shell pango

# FreeBSD
pkg install gtk4 gtk4-layer-shell pango
```

`gtk4-layer-shell` is optional. Build with `--no-default-features` if your
system does not provide it; the overlay then uses a plain Wayland toplevel or
the X11 backend.

## Build and run

```sh
cargo build --release
./target/release/koko-widget
```

Install for the current user:

```sh
./packaging/install.sh
```

## Command line

```sh
koko-widget              # show the overlay and open the menu
koko-widget --show       # show without the menu
koko-widget --hide       # hide without quitting
koko-widget --toggle     # toggle visibility
koko-widget --settings   # open settings
koko-widget --menu       # open the main menu
koko-widget --next       # next track
koko-widget --previous   # previous track
koko-widget --play-pause # play or pause
koko-widget --expand     # expand and pin
koko-widget --collapse   # collapse and unpin
koko-widget --quit       # quit the running instance
koko-widget --diagnostics
```

The first instance owns a D-Bus control name; later invocations forward their
command and exit, so you can bind these to compositor shortcuts safely.

## Configuration

`~/.config/koko-widget/config.toml` (or `$XDG_CONFIG_HOME/koko-widget/`). The
settings window writes this file; hand editing works too.

```toml
[widgets]
order = ["media", "clock", "date"]
gap = 12.0
align = "center"

[widgets.media]
enabled = true
background = true
artwork = true
artwork_radius = 18.0
show_artist = true
show_controls = true
show_progress = true
title_size = 13.0
artist_size = 11.0

[widgets.clock]
enabled = true
background = false      # liquid-glass lettering, no pill
show_seconds = false
hour_format = "auto"    # auto | twelve-hour | twenty-four-hour
label = "Local time"
show_label = true
font_size = 30.0
label_size = 10.0
weight = "semibold"
colour = "#a9b6ff"
width = 172.0
height = 112.0

[widgets.date]
enabled = true
background = false
format = "%A %-d %B"
show_label = true
label = "Today"
font_size = 16.0
weight = "medium"
wrap = true
width = 200.0
height = 112.0

[widgets.text]
enabled = false
background = false
text = "Stay focused"
font_size = 18.0
weight = "semibold"
align = "left"
width = 150.0
height = 112.0
```

`order` controls left-to-right placement. `background = false` removes the pill
and draws the content as liquid-glass lettering; `background = true` gives that
widget its own glass pill. Every size is in logical pixels or points and is
clamped to a safe range on load.

## Liquid glass

There are two layers to the effect, and it is worth being precise about which
one does what.

**In-app (always available).** Every pill is rendered as layered glass in Cairo:
a graded translucent base, an accent wash, specular rim highlights, inset
shadows and a fine grain. Text without a background is drawn as glass glyphs —
a graded fill, a specular top edge and a soft contact shadow. This works on
every compositor, including X11.

**Compositor (optional).** True refraction — light bending through the surface,
sampling the desktop behind it — can only be done by the compositor, because
only the compositor owns the pixels behind your window. KOKO WIDGET exposes a
stable layer namespace, `koko-widget`, so a compositor rule can target it.

On niri with a liquid-glass build, add this to your config (the settings window
has a **Copy Niri rule** button):

```kdl
layer-rule {
    match namespace="^koko-widget$"
    background-effect {
        blur true
        xray true
        liquid-glass {
            refraction-strength 3.5
            power-factor 8.0
            refraction-power 1.2
            fringing 0.35
            edge-lighting 0.4
            glow-weight 0.001
            saturation 1.1
            vibrancy 0.3
        }
    }
}
```

Field names differ between compositor forks; check your build's documentation.

### About `liquid-glass-rs` and `iced_glass`

Both were requested and both were evaluated:

- [`liquid-glass-rs`](https://github.com/Stapxs/liquid-glass-rs) wraps macOS 26's
  `NSGlassEffectView`. It is macOS-only and cannot compile or render on Linux or
  FreeBSD.
- [`iced_glass`](https://github.com/maxbergmark/iced_glass) is an excellent glass
  crate for the **iced** GUI toolkit, not GTK. It depends on a custom iced fork
  for framebuffer readback and would mean rewriting the entire interface and
  losing GTK4 layer-shell integration.

KOKO WIDGET therefore implements the glass treatment natively in Cairo and
delegates real refraction to the compositor, which is the only place it can
physically happen on Linux.

## Desktop integration

The installer places:

- `~/.local/bin/koko-widget`
- `~/.local/share/applications/io.github.kokowidget.KokoWidget.desktop`
- hicolor icons from 16 px to 512 px

Optional autostart is an XDG entry the app creates and owns; toggling the
setting removes only that file.

## Wayland, X11 and fallbacks

| Session | Backend | Behaviour |
|---|---|---|
| Wayland + layer-shell | `zwlr_layer_shell_v1` | Anchored, click-through margins, drag via margins |
| Wayland without layer-shell | toplevel | Floating window, still fully interactive |
| X11 | EWMH dock window | `_NET_WM_STATE_ABOVE`, sticky, positioned via EWMH |

Drag support exists for layer-shell and X11. Input regions are restricted to the
visible pills, so clicks in the transparent gaps reach the desktop.

## Development

```sh
cargo fmt
cargo clippy --all-targets
cargo test
cargo run --example mock_player   # a fake MPRIS player with three tracks
```

The mock player exposes `org.mpris.MediaPlayer2.koko_mock` and supports
play/pause, next, previous and seeking, which is enough to exercise every code
path without a real media player.

## Licence

MIT.
