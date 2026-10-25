#!/bin/bash
#
# The rootless install.
#
# No root, no udev rule, no system units. Instead a small watcher runs as a
# systemd user service and blocks on the mount table — about 2 MB resident, no
# CPU while it waits. The trade against the system installer (linux/install.sh)
# is exactly that: one process, versus one password prompt.
#
# Everything lands under $HOME:
#   ~/.local/bin/pc-gamepak                  the launcher and wizard
#   ~/.local/bin/pc-gamepak-watcher          the watcher
#   ~/.config/systemd/user/…                 the service that starts it

set -euo pipefail

if [ "$EUID" -eq 0 ]; then
    echo "Do not run this one with sudo — it installs into your home directory."
    echo "For the system-wide install with a udev rule, use linux/install.sh."
    exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN_DIR="$HOME/.local/bin"
UNIT_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"

# Two layouts to cope with: a source checkout, where the binaries are under
# target/release, and an unpacked release tarball, where they sit beside this
# script's parent directory.
find_binary() {
    local name="$1" candidate
    for candidate in \
        "$SCRIPT_DIR/tauri-ui/src-tauri/target/release/$name" \
        "$SCRIPT_DIR/watcher/target/release/$name" \
        "$SCRIPT_DIR/$name"
    do
        if [ -x "$candidate" ]; then
            echo "$candidate"
            return 0
        fi
    done
    return 1
}

LAUNCHER_BUILD="$(find_binary pc-gamepak)" || {
    echo "Could not find the pc-gamepak binary."
    echo
    echo "From a source checkout, build it first:"
    echo "  cargo build --release --manifest-path tauri-ui/src-tauri/Cargo.toml"
    echo "  cargo build --release --manifest-path watcher/Cargo.toml"
    exit 1
}

WATCHER_BUILD="$(find_binary pc-gamepak-watcher)" || {
    echo "Could not find the pc-gamepak-watcher binary."
    echo "  cargo build --release --manifest-path watcher/Cargo.toml"
    exit 1
}

echo "Installing PC GamePak for $USER (no root)..."

mkdir -p "$BIN_DIR" "$UNIT_DIR"
install -m 755 "$LAUNCHER_BUILD" "$BIN_DIR/pc-gamepak"
install -m 755 "$WATCHER_BUILD" "$BIN_DIR/pc-gamepak-watcher"
UNIT_SOURCE="$SCRIPT_DIR/linux/pc-gamepak-watcher.service"
[ -f "$UNIT_SOURCE" ] || UNIT_SOURCE="$SCRIPT_DIR/pc-gamepak-watcher.service"
install -m 644 "$UNIT_SOURCE" "$UNIT_DIR/pc-gamepak-watcher.service"

systemctl --user daemon-reload
systemctl --user enable --now pc-gamepak-watcher.service

# Without this the window carries the desktop's generic fallback icon
# everywhere it appears (taskbar, Alt+Tab), because nothing here tells the
# desktop which app_id is ours or what it looks like.
DESKTOP_SOURCE="$SCRIPT_DIR/linux/pc-gamepak.desktop"
[ -f "$DESKTOP_SOURCE" ] || DESKTOP_SOURCE="$SCRIPT_DIR/pc-gamepak.desktop"
ICON_SOURCE="$SCRIPT_DIR/tauri-ui/src-tauri/icons/128x128.png"
[ -f "$ICON_SOURCE" ] || ICON_SOURCE="$SCRIPT_DIR/icons/128x128.png"
if [ -f "$DESKTOP_SOURCE" ] && [ -f "$ICON_SOURCE" ]; then
    APPS_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
    ICON_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor/128x128/apps"
    mkdir -p "$APPS_DIR" "$ICON_DIR"
    install -m 644 "$DESKTOP_SOURCE" "$APPS_DIR/pc-gamepak.desktop"
    install -m 644 "$ICON_SOURCE" "$ICON_DIR/pc-gamepak.png"
    update-desktop-database "$APPS_DIR" >/dev/null 2>&1 || true
    gtk-update-icon-cache "${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor" >/dev/null 2>&1 || true
fi

echo
echo "Installed."
echo "  launcher:  $BIN_DIR/pc-gamepak"
echo "  watcher:   $BIN_DIR/pc-gamepak-watcher (running now, and at every login)"
echo "  log:       ${XDG_STATE_HOME:-$HOME/.local/state}/pc-gamepak/watcher.log"
echo

if ! echo "$PATH" | tr ':' '\n' | grep -qx "$BIN_DIR"; then
    echo "Note: $BIN_DIR is not on your PATH, so 'pc-gamepak --create' will not"
    echo "      resolve. The watcher does not care — it uses the full path."
    echo
fi

# The one step this install cannot do without root: a barcode scanner is a
# keyboard, and only a udev rule can hand it to the user so the watcher may
# read it. Said rather than done, so this stays a rootless install.
if [ ! -f /etc/udev/rules.d/70-pc-gamepak-barcode.rules ]; then
    echo "For a USB barcode scanner, let the watcher read it (once, as root):"
    echo "  sudo install -m 644 $SCRIPT_DIR/linux/70-pc-gamepak-barcode.rules /etc/udev/rules.d/"
    echo "  sudo udevadm control --reload-rules && sudo udevadm trigger"
    echo
fi

echo "Plug a cartridge in to test it. To stop it watching:"
echo "  systemctl --user disable --now pc-gamepak-watcher.service"
