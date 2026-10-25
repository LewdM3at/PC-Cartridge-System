#!/bin/bash

set -e

echo "Uninstalling Steam Game Cartridge launcher..."

########################################
# Check root
########################################

if [ "$EUID" -ne 0 ]; then
    echo "Please run this script with sudo."
    exit 1
fi


########################################
# Stop running services
########################################

echo "Stopping cartridge services..."

systemctl stop 'pc-gamepak@*' 2>/dev/null || true
systemctl stop 'pc-gamepak-remove@*' 2>/dev/null || true


########################################
# Remove launcher helper
########################################

echo "Removing launcher helper..."

# The first two are names this project used before it was called PC GamePak,
# left here so an upgrade in place does not strand a stale helper.
rm -f /usr/local/bin/cartridge-launcher-helper
rm -f /usr/local/bin/pc-cartridge-helper
rm -f /usr/local/bin/pc-gamepak-helper
rm -f /usr/local/bin/pc-cartridge-remove
rm -f /usr/local/bin/pc-cartridge-eject
rm -f /usr/local/bin/pc-cartridge-launcher
rm -f /usr/local/bin/pc-gamepak-remove
rm -f /usr/local/bin/pc-gamepak-eject
rm -f /usr/local/bin/pc-gamepak


########################################
# Remove systemd service
########################################

echo "Removing systemd service..."

rm -f /etc/systemd/system/game-cartridge@.service
rm -f /etc/systemd/system/game-cartridge-remove@.service
rm -f /etc/systemd/system/pc-gamepak@.service
rm -f /etc/systemd/system/pc-gamepak-remove@.service


########################################
# Remove udev rule
########################################

echo "Removing udev rule..."

rm -f /etc/udev/rules.d/99-steam-game-cartridge.rules
rm -f /etc/udev/rules.d/99-game-cartridge.rules
rm -f /etc/udev/rules.d/99-pc-gamepak.rules
rm -f /etc/udev/rules.d/70-pc-gamepak-barcode.rules


########################################
# Remove desktop entry and icon
########################################

echo "Removing desktop entry and icon..."

rm -f /usr/share/applications/pc-gamepak.desktop
rm -f /usr/share/icons/hicolor/128x128/apps/pc-gamepak.png

update-desktop-database /usr/share/applications >/dev/null 2>&1 || true
gtk-update-icon-cache /usr/share/icons/hicolor >/dev/null 2>&1 || true

#######################################################################
# Remove steam-games-cartridges config directory in User's HOME
#######################################################################

if [ -n "$SUDO_USER" ]; then
    USERNAME="$SUDO_USER"
else
    USERNAME="$USER"
fi

USER_HOME=$(eval echo "~$USERNAME")

echo "Removing config directory..."

rm -rf "$USER_HOME/.config/steam-games-cartridges"
rm -rf "$USER_HOME/.config/pc-gamepak"


########################################
# Reload services
########################################

echo "Reloading system services..."

systemctl daemon-reload
systemctl reset-failed

udevadm control --reload-rules
udevadm trigger


########################################
# Done
########################################

echo ""
echo "=========================================="
echo " Steam Game Cartridge removed"
echo "=========================================="
echo ""