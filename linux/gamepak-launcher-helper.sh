#!/bin/bash
#
# Started by pc-gamepak@.service when udev sees a partition appear.
#
# Its whole job is to wait for the desktop to mount the cartridge and then open
# the launcher on it. Nothing is executed from the cartridge here: the launcher
# shows the cover art and waits for the user to press Play.
#
# That is why there is no trust list any more. The old design auto-executed
# launch.sh on insert, so it needed a SHA-256 allowlist to be safe. Now a human
# clicks Play, which is a better gate than a hash of a file the same person
# could rewrite.

set -euo pipefail

DEVICE="${1:-}"

if [ -z "$DEVICE" ]; then
    echo "usage: $0 <kernel device name, e.g. sdb1>" >&2
    exit 2
fi

STATE_DIR="${XDG_STATE_HOME:-$HOME/.local/state}/pc-gamepak"
mkdir -p "$STATE_DIR"
LOG_FILE="$STATE_DIR/helper.log"

# Keep one short log rather than growing forever.
exec >>"$LOG_FILE" 2>&1
if [ "$(stat -c %s "$LOG_FILE" 2>/dev/null || echo 0)" -gt 262144 ]; then
    : >"$LOG_FILE"
fi

echo "==== $(date -Is) cartridge detected: $DEVICE ===="

# Wait for the desktop automounter. udev fires as soon as the kernel sees the
# partition, which is earlier than the mount we actually need.
MOUNT_POINT=""
for _ in $(seq 1 60); do
    MOUNT_POINT=$(findmnt -n -f -o TARGET "/dev/$DEVICE" 2>/dev/null || true)
    [ -n "$MOUNT_POINT" ] && break
    sleep 0.5
done

if [ -z "$MOUNT_POINT" ]; then
    echo "no mount point appeared for /dev/$DEVICE after 30s; giving up"
    exit 0
fi

echo "mounted at: $MOUNT_POINT"

# Only cartridges get a launcher. Without this every USB stick would pop a
# window.
if [ ! -f "$MOUNT_POINT/cartridge.conf" ] && [ ! -f "$MOUNT_POINT/memorycard.conf" ] && [ ! -f "$MOUNT_POINT/autorun.inf" ]; then
    echo "no cartridge.conf, memorycard.conf or autorun.inf at the root; not a cartridge"
    exit 0
fi

# Where the launcher ended up depends on how it was installed: install.sh puts
# it in /usr/local/bin, a package in /usr/bin. Searching both means this helper
# does not have to know which one ran. PC_GAMEPAK_LAUNCHER overrides all of it
# and is spelled the same way the watcher spells it; PC_CARTRIDGE_LAUNCHER is
# the old name, still honoured so existing installs keep working.
LAUNCHER="${PC_GAMEPAK_LAUNCHER:-${PC_CARTRIDGE_LAUNCHER:-}}"

if [ -z "$LAUNCHER" ]; then
    for candidate in /usr/local/bin/pc-gamepak /usr/bin/pc-gamepak; do
        if [ -x "$candidate" ]; then
            LAUNCHER="$candidate"
            break
        fi
    done
fi

if [ -z "$LAUNCHER" ] || [ ! -x "$LAUNCHER" ]; then
    echo "launcher not found in /usr/local/bin or /usr/bin"
    echo "install it with linux/install.sh, or set PC_GAMEPAK_LAUNCHER"
    exit 0
fi

echo "opening launcher for $MOUNT_POINT"

# Note that "opening the launcher" is not the same as "a window appears". The
# launcher reads `on_cartridge_insert` from the user's settings and may start the
# game, post a notification, or exit without drawing anything. That decision is
# deliberately inside the launcher and not here: this helper, the resident
# watcher and the tray menu all open a launcher, and a setting honoured by one of
# them and not the others would be worse than no setting at all.

# The launcher needs the user's session to put a window on screen. The systemd
# unit already runs as the desktop user; point it at their display.
export DISPLAY="${DISPLAY:-:0}"
export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
if [ -z "${WAYLAND_DISPLAY:-}" ] && [ -S "$XDG_RUNTIME_DIR/wayland-0" ]; then
    export WAYLAND_DISPLAY=wayland-0
fi

exec "$LAUNCHER" --drive "$MOUNT_POINT"
