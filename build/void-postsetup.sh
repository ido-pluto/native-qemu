#!/bin/sh
# Runs inside void-mklive as -x postsetup (chroot path = $1).
# Finalize native-qemu appliance on Void glibc live rootfs.
set -eu
ROOTFS="${1:?rootfs path required}"

echo "void-postsetup: configuring native-qemu in $ROOTFS"

# Ensure custom QEMU + agent are executable
if [ -x "$ROOTFS/usr/local/bin/native-qemu-agent" ]; then
	chmod 755 "$ROOTFS/usr/local/bin/native-qemu-agent"
else
	echo "void-postsetup: ERROR missing native-qemu-agent" >&2
	exit 1
fi
if [ -x "$ROOTFS/usr/local/bin/qemu-system-x86_64" ]; then
	chmod 755 "$ROOTFS/usr/local/bin/qemu-system-x86_64"
	# Show version for build logs (best-effort; needs matching libs in ROOTFS)
	if command -v chroot >/dev/null 2>&1; then
		chroot "$ROOTFS" /usr/local/bin/qemu-system-x86_64 --version 2>/dev/null \
			|| echo "void-postsetup: note: could not exec qemu --version in chroot (libs may land later)"
	fi
else
	echo "void-postsetup: ERROR missing /usr/local/bin/qemu-system-x86_64 (qemu-3dfx not baked in)" >&2
	exit 1
fi

# PATH for root login shells
if [ -f "$ROOTFS/root/.profile" ]; then
	grep -q '/usr/local/bin' "$ROOTFS/root/.profile" 2>/dev/null \
		|| echo 'export PATH="/usr/local/bin:$PATH"' >>"$ROOTFS/root/.profile"
else
	echo 'export PATH="/usr/local/bin:/usr/bin:/bin"' >"$ROOTFS/root/.profile"
fi

# runit: take over tty1 for the agent (drop agetty-tty1)
mkdir -p "$ROOTFS/etc/runit/runsvdir/default"
rm -f "$ROOTFS/etc/runit/runsvdir/default/agetty-tty1"
ln -sfn /etc/sv/native-qemu-agent "$ROOTFS/etc/runit/runsvdir/default/native-qemu-agent"
chmod 755 "$ROOTFS/etc/sv/native-qemu-agent/run" 2>/dev/null || true
chmod 755 "$ROOTFS/etc/sv/native-qemu-agent/finish" 2>/dev/null || true

# Keep udevd for device discovery; optional sshd for rescue
if [ -e "$ROOTFS/etc/sv/udevd" ]; then
	ln -sfn /etc/sv/udevd "$ROOTFS/etc/runit/runsvdir/default/udevd"
fi

# Hostname
echo native-qemu >"$ROOTFS/etc/hostname"

# Motd
cat >"$ROOTFS/etc/motd" <<'EOF'

  native-qemu — Void glibc appliance with QEMU 9.2 + 3dfx/MESA
  Agent runs on tty1. Data volume: LABEL=native-qemu (config.toml + image.qcow2).
  Custom QEMU: /usr/local/bin/qemu-system-x86_64

EOF

# Ensure e2fsprogs tools exist for optional host mounts of data volume
command -v chroot >/dev/null

echo "void-postsetup: done"
