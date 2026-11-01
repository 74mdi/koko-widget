#!/bin/sh
# Install KOKO WIDGET for the current user.
#
#   ./packaging/install.sh            install (builds release if needed)
#   ./packaging/install.sh --uninstall
#   PREFIX=/usr/local ./packaging/install.sh   (needs write access)
#
# Everything lands under $PREFIX, default ~/.local, so no root is required.

set -eu

APP_ID=io.github.kokowidget.KokoWidget
BIN=koko-widget
HERE=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
PREFIX=${PREFIX:-$HOME/.local}

BIN_DIR=$PREFIX/bin
APP_DIR=$PREFIX/share/applications
ICON_DIR=$PREFIX/share/icons/hicolor

say() { printf '%s\n' "$*"; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }

uninstall() {
    say "Removing $APP_ID from $PREFIX"
    rm -f "$BIN_DIR/$BIN"
    rm -f "$APP_DIR/$APP_ID.desktop"
    for size in 16 24 32 48 64 128 256 512; do
        rm -f "$ICON_DIR/${size}x${size}/apps/$APP_ID.png"
    done
    # Autostart is owned by the app's settings toggle, but remove a stale copy
    # so an uninstall does not leave a launcher pointing at nothing.
    rm -f "${XDG_CONFIG_HOME:-$HOME/.config}/autostart/$APP_ID.desktop"
    refresh_caches
    say "Done."
}

refresh_caches() {
    command -v update-desktop-database >/dev/null 2>&1 &&
        update-desktop-database "$APP_DIR" >/dev/null 2>&1 || true
    command -v gtk-update-icon-cache >/dev/null 2>&1 &&
        gtk-update-icon-cache -qtf "$ICON_DIR" >/dev/null 2>&1 || true
}

case ${1:-} in
--uninstall | -u)
    uninstall
    exit 0
    ;;
-h | --help)
    sed -n '2,9p' "$0" | sed 's/^# \{0,1\}//'
    exit 0
    ;;
"") ;;
*)
    die "unknown argument: $1"
    ;;
esac

[ -f "$HERE/Cargo.toml" ] || die "$HERE does not look like the KOKO WIDGET source tree"

BINARY=$HERE/target/release/$BIN
if [ ! -x "$BINARY" ]; then
    command -v cargo >/dev/null 2>&1 || die "cargo is not installed"
    say "Building the release binary (this takes a few minutes)"
    # Build one job at a time: the release profile uses thin LTO, which is
    # memory hungry, and parallel codegen has OOM-killed small machines.
    (cd "$HERE" && CARGO_BUILD_JOBS=1 cargo build --release)
fi
[ -x "$BINARY" ] || die "build did not produce $BINARY"

say "Installing to $PREFIX"
install -d "$BIN_DIR" "$APP_DIR"
install -m 755 "$BINARY" "$BIN_DIR/$BIN"

# The desktop entry ships with Exec=koko-widget, which relies on ~/.local/bin
# being on PATH. Rewrite it to the absolute path so a launcher click works even
# when the session was started before the directory existed.
sed "s|^Exec=$BIN\$|Exec=$BIN_DIR/$BIN|; s|^TryExec=$BIN\$|TryExec=$BIN_DIR/$BIN|" \
    "$HERE/data/$APP_ID.desktop" >"$APP_DIR/$APP_ID.desktop"
chmod 644 "$APP_DIR/$APP_ID.desktop"

for size in 16 24 32 48 64 128 256 512; do
    source_icon=$HERE/assets/icons/hicolor/${size}x${size}/apps/$APP_ID.png
    [ -f "$source_icon" ] || continue
    install -d "$ICON_DIR/${size}x${size}/apps"
    install -m 644 "$source_icon" "$ICON_DIR/${size}x${size}/apps/$APP_ID.png"
done

refresh_caches

say ""
say "Installed."
say "  binary   $BIN_DIR/$BIN"
say "  launcher $APP_DIR/$APP_ID.desktop"
say ""
say "If the launcher does not appear, log out and back in, or run:"
say "  update-desktop-database $APP_DIR"
