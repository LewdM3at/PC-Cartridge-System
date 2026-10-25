#!/bin/bash
#
# Called by pc-gamepak-remove@.service when a partition is unplugged.
#
# Closes the launcher window if it was showing the cartridge that just left. It
# does not touch the game: pulling a cartridge while the game is running is the
# user's business, and killing their session would be a worse surprise than a
# stale window.
#
# The old version pgrep'd for "launch.sh" and killed anything matching. Nothing
# is called launch.sh any more, and matching on a name that loose could have hit
# an unrelated process.

set -uo pipefail

DEVICE="${1:-}"

if [ -z "$DEVICE" ]; then
    echo "usage: $0 <kernel device name, e.g. sdb1>" >&2
    exit 2
fi

STATE_DIR="${XDG_STATE_HOME:-$HOME/.local/state}/pc-gamepak"
mkdir -p "$STATE_DIR"
exec >>"$STATE_DIR/helper.log" 2>&1

echo "==== $(date -Is) cartridge removed: $DEVICE ===="

# Match only launcher processes, and only ones whose --drive argument is a mount
# point that no longer exists. A second cartridge still plugged in keeps its own
# window.
CLOSED=0

for PID in $(pgrep -x pc-gamepak 2>/dev/null || true); do
    # argv is NUL-separated; --drive is followed by the mount point.
    mapfile -d '' -t ARGS < "/proc/$PID/cmdline" 2>/dev/null || continue

    MOUNT=""
    for i in "${!ARGS[@]}"; do
        if [ "${ARGS[$i]}" = "--drive" ]; then
            MOUNT="${ARGS[$((i + 1))]:-}"
            break
        fi
    done

    [ -z "$MOUNT" ] && continue

    # Still mounted means this launcher belongs to a cartridge that is present.
    if mountpoint -q "$MOUNT" 2>/dev/null; then
        continue
    fi

    echo "closing launcher $PID (was showing $MOUNT)"
    kill -TERM "$PID" 2>/dev/null || true
    CLOSED=$((CLOSED + 1))
done

if [ "$CLOSED" -eq 0 ]; then
    echo "no launcher window to close"
fi
