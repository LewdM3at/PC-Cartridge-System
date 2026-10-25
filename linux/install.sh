#!/bin/bash

set -e

if [ "$EUID" -ne 0 ]; then
    echo "Please run this installer with sudo."
    exit 1
fi

echo "Installing PC GamePak..."

# Everything below is relative to the directory holding `linux/`, found from
# this script rather than from the working directory — so `sudo ./linux/
# install.sh` and `sudo bash install.sh` from inside it both work, and so does
# the release tarball, whose layout is not a source checkout's.
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

# Check for important files. The icon is not among them: it is missing from an
# unpacked release tarball built before it was shipped, and a generic icon is a
# cosmetic loss, not a reason to refuse to install the thing that watches for
# cartridges.
for FILE in \
    "linux/gamepak-launcher-helper.sh" \
    "linux/gamepak-remove-helper.sh" \
    "linux/pc-gamepak@.service" \
    "linux/pc-gamepak-remove@.service" \
    "linux/99-pc-gamepak.rules" \
    "linux/pc-gamepak.desktop"
do
    if [ ! -f "$FILE" ]; then
        echo "Missing file: $FILE"
        exit 1
    fi
done

########################################
# Detect user
########################################

if [ -n "$SUDO_USER" ]; then
    USERNAME="$SUDO_USER"
else
    USERNAME="$USER"
fi

USER_HOME=$(eval echo "~$USERNAME")

echo "Installing for user: $USERNAME"
echo "Home directory: $USER_HOME"


########################################
# Install launcher helper
########################################

echo "Installing launcher helper..."

install -m 755 linux/gamepak-launcher-helper.sh /usr/local/bin/pc-gamepak-helper
install -m 755 linux/eject.sh /usr/local/bin/pc-gamepak-eject


########################################
# Install systemd template
########################################

echo "Installing systemd service..."

sed "s/__USERNAME__/$USERNAME/g" \
    "linux/pc-gamepak@.service" \
    > /etc/systemd/system/pc-gamepak@.service

sed "s/__USERNAME__/$USERNAME/g" \
    "linux/pc-gamepak-remove@.service" \
    > /etc/systemd/system/pc-gamepak-remove@.service


########################################
# Install removal helper
########################################

echo "Installing removal helper..."

install -m 755 linux/gamepak-remove-helper.sh /usr/local/bin/pc-gamepak-remove


########################################
# Install udev rule
########################################

echo "Installing udev rule..."

install -m 644 linux/99-pc-gamepak.rules /etc/udev/rules.d/99-pc-gamepak.rules


########################################
# Reload services
########################################

systemctl daemon-reload

udevadm control --reload-rules

udevadm trigger


########################################
# Done
########################################

########################################
# Install the launcher binary if it has been built
########################################

# Two layouts, as in install-user.sh: a source checkout builds into
# target/release, and an unpacked release tarball puts the binary beside
# `linux/`. Looking in only the first is how a tarball that ships the launcher
# came to report it as "not built yet" and install nothing.
LAUNCHER_BUILD=""
for CANDIDATE in \
    "tauri-ui/src-tauri/target/release/pc-gamepak" \
    "pc-gamepak"
do
    if [ -x "$CANDIDATE" ]; then
        LAUNCHER_BUILD="$CANDIDATE"
        break
    fi
done

if [ -n "$LAUNCHER_BUILD" ]; then
    echo "Installing launcher..."
    install -m 755 "$LAUNCHER_BUILD" /usr/local/bin/pc-gamepak
    LAUNCHER_STATE="installed"
else
    LAUNCHER_STATE="not built yet"
fi


########################################
# Desktop entry and icon
#
# Without these the window still runs — it just carries the desktop's
# generic fallback icon everywhere it appears (taskbar, Alt+Tab, the dock),
# because nothing tells the desktop which app_id is ours or what it looks
# like. StartupWMClass=pc-gamepak here has to match the app_id the window
# actually opens with, which is why it is taken from the wizard's own build
# output (below) rather than guessed at.
########################################

echo "Installing desktop entry and icon..."

install -m 644 linux/pc-gamepak.desktop /usr/share/applications/pc-gamepak.desktop

ICON_SOURCE="tauri-ui/src-tauri/icons/128x128.png"
[ -f "$ICON_SOURCE" ] || ICON_SOURCE="icons/128x128.png"
if [ -f "$ICON_SOURCE" ]; then
    install -m 644 "$ICON_SOURCE" \
        /usr/share/icons/hicolor/128x128/apps/pc-gamepak.png
else
    echo "  no icon found; the window will use the desktop's generic one."
fi

update-desktop-database /usr/share/applications >/dev/null 2>&1 || true
gtk-update-icon-cache /usr/share/icons/hicolor >/dev/null 2>&1 || true


########################################
# Done
########################################

echo ""
echo "=========================================="
echo " PC GamePak installed"
echo "=========================================="
echo ""
echo " Launcher: $LAUNCHER_STATE"

if [ "$LAUNCHER_STATE" != "installed" ]; then
    echo ""
    echo " Build it, then run this installer again:"
    echo ""
    echo "   cd tauri-ui && npm install && npm run build"
fi

echo ""
echo " Put a cartridge.conf at the root of the drive:"
echo ""
echo "   executable=steam://rungameid/12345"
echo "   title=My Game"
echo "   cover=cover.jpg"
echo ""
echo " Then plug the cartridge in. Nothing runs on its own:"
echo " the launcher opens and waits for you to press Play."
echo ""
echo " The drive must be automounted by your desktop."
echo " If your distro does not automount, install"
echo " something like udiskie."
echo ""