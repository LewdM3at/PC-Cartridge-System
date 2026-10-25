#!/bin/bash
#
# Undoes linux/install-user.sh. Touches nothing outside $HOME, and nothing the
# system-wide installer put there.

set -uo pipefail

if [ "$EUID" -eq 0 ]; then
    echo "Do not run this one with sudo — it only removes files in your home."
    exit 1
fi

BIN_DIR="$HOME/.local/bin"
UNIT_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"

echo "Removing the user-level PC GamePak install..."

systemctl --user disable --now pc-gamepak-watcher.service 2>/dev/null || true
rm -f "$UNIT_DIR/pc-gamepak-watcher.service"
systemctl --user daemon-reload 2>/dev/null || true

rm -f "$BIN_DIR/pc-gamepak" "$BIN_DIR/pc-gamepak-watcher"

APPS_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
ICON_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor/128x128/apps"
rm -f "$APPS_DIR/pc-gamepak.desktop" "$ICON_DIR/pc-gamepak.png"
update-desktop-database "$APPS_DIR" >/dev/null 2>&1 || true
gtk-update-icon-cache "${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor" >/dev/null 2>&1 || true

echo "Done. Settings and the artwork cache are left alone:"
echo "  ${XDG_STATE_HOME:-$HOME/.local/state}/pc-gamepak"
