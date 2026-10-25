#!/bin/bash
# PC GamePak - Safe Eject Helper
# Usage: eject.sh <device>
# Example: eject.sh sdb
#
# Safely unmounts a block device and powers it off before physical removal.
# Requires udisks2 (udisksctl).

set -e

DEVICE="$1"

if [ -z "$DEVICE" ]; then
    echo "Usage: eject.sh <device>"
    echo "Example: eject.sh sdb"
    exit 1
fi

# Strip leading /dev/ if the user passed a full path
DEVICE="${DEVICE#/dev/}"

BLOCK_DEVICE="/dev/$DEVICE"

if [ ! -b "$BLOCK_DEVICE" ]; then
    echo "Device $BLOCK_DEVICE not found or is not a block device."
    exit 1
fi

echo "Ejecting $BLOCK_DEVICE ..."

# Unmount all partitions of the device
for PART in $(lsblk -ln -o NAME "$BLOCK_DEVICE" | tail -n +2); do
    MOUNT_POINT=$(findmnt -n -o TARGET "/dev/$PART" 2>/dev/null || true)
    if [ -n "$MOUNT_POINT" ]; then
        echo "Unmounting /dev/$PART from $MOUNT_POINT ..."
        udisksctl unmount -b "/dev/$PART" --no-user-interaction || true
    fi
done

# Power off the drive (spins down / safe to remove)
echo "Powering off $BLOCK_DEVICE ..."
udisksctl power-off -b "$BLOCK_DEVICE" --no-user-interaction

echo "Done. $BLOCK_DEVICE can now be safely removed."
