#!/bin/sh
# Write a native-qemu hybrid ISO to a whole removable device and verify the
# exact ISO byte range afterwards. This script deliberately requires both an
# explicit acknowledgement and a whole-disk path: it must never be convenient
# to overwrite a partition or the macOS system disk by accident.
set -eu

usage() {
	cat <<'EOF'
Usage: sudo scripts/write-usb.sh --yes-really-write IMAGE.iso /dev/DEVICE

Examples:
  macOS: sudo scripts/write-usb.sh --yes-really-write native-qemu-x86_64.iso /dev/rdisk4
  Linux: sudo scripts/write-usb.sh --yes-really-write native-qemu-x86_64.iso /dev/sdb

The target must be a whole disk, not a partition. All data on it is destroyed.
EOF
}

if [ "$#" -ne 3 ] || [ "$1" != "--yes-really-write" ]; then
	usage >&2
	exit 64
fi

image=$2
target=$3

if [ "$(id -u)" -ne 0 ]; then
	echo "write-usb: run this command through sudo (the target device needs root access)" >&2
	exit 1
fi
if [ ! -f "$image" ]; then
	echo "write-usb: ISO is not a regular file: $image" >&2
	exit 1
fi
if [ ! -b "$target" ] && [ ! -c "$target" ]; then
	echo "write-usb: target is not a block or raw device: $target" >&2
	exit 1
fi

# Accept only unpartitioned device names. macOS disk numbers need a separate
# digits-only check so /dev/disk10 works while /dev/disk4s1 is still rejected.
case "$target" in
	/dev/disk*|/dev/rdisk*)
		disk_number=${target#/dev/disk}
		if [ "$disk_number" = "$target" ]; then
			disk_number=${target#/dev/rdisk}
		fi
		case "$disk_number" in
			''|0|*[!0-9]*)
				echo "write-usb: refuse $target; provide a whole non-system disk (for example /dev/rdisk4)" >&2
				exit 1
				;;
		esac
		;;
	/dev/sd[a-z]|/dev/vd[a-z]|/dev/nvme[0-9]n[0-9]|/dev/mmcblk[0-9]) ;;
	*)
		echo "write-usb: refuse $target; provide a whole non-system disk (for example /dev/sdb or /dev/rdisk4)" >&2
		exit 1
		;;
esac

hash_command() {
	if command -v shasum >/dev/null 2>&1; then
		shasum -a 256
	else
		sha256sum
	fi
}

if [ "$(uname -s)" = "Darwin" ]; then
	logical_target=${target#/dev/r}
	echo "write-usb: unmounting $logical_target"
	diskutil unmountDisk "$logical_target"
	echo "write-usb: writing $image to $target (macOS: press Ctrl-T for progress)"
	dd if="$image" of="$target" bs=4m
	else
	if command -v lsblk >/dev/null 2>&1 \
		&& lsblk -nrpo MOUNTPOINT "$target" | awk 'NF { found = 1 } END { exit !found }'; then
		echo "write-usb: refuse $target because it or one of its partitions is mounted; unmount it first" >&2
		exit 1
	fi
	echo "write-usb: writing $image to $target"
	dd if="$image" of="$target" bs=4M conv=fsync status=progress
fi
sync

# Hash only the leading byte range occupied by the ISO. A USB stick is usually
# larger than the image, so hashing the whole device would be both wrong and
# needlessly slow.
image_bytes=$(wc -c < "$image" | tr -d '[:space:]')
block_bytes=1048576
block_count=$(( (image_bytes + block_bytes - 1) / block_bytes ))
expected_hash=$(hash_command < "$image" | awk '{print $1}')
actual_hash=$(dd if="$target" bs="$block_bytes" count="$block_count" 2>/dev/null \
	| head -c "$image_bytes" | hash_command | awk '{print $1}')

if [ "$expected_hash" != "$actual_hash" ]; then
	echo "write-usb: verification FAILED; do not boot this device" >&2
	exit 1
fi
echo "write-usb: verified SHA-256 $actual_hash"

if [ "$(uname -s)" = "Darwin" ]; then
	diskutil eject "$logical_target"
fi
