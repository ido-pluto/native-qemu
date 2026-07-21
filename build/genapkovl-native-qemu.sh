#!/bin/sh
# Generates <hostname>.apkovl.tar.gz — the overlay applied on top of the
# squashfs rootfs at boot. Invoked by aports/scripts/mkimg.base.sh's
# build_apkovl() as: fakeroot ./genapkovl-native-qemu.sh <hostname>
# (run with cwd = a scratch workdir; output file must land in that cwd).
#
# Copies the tracked overlay/ tree (config example, hook script examples,
# and the compiled native-qemu-agent binary — placed there by ci-build.sh
# before this runs) as-is, then adds the generated hostname/inittab/motd.

set -e

HOSTNAME="${1:?usage: $0 <hostname>}"
OVERLAY_SRC="${NATIVE_QEMU_OVERLAY_DIR:-/repo/overlay}"
REPO_ROOT="$(dirname "$OVERLAY_SRC")"

tmp="$(mktemp -d)"
cleanup() { rm -rf "$tmp"; }
trap cleanup EXIT

if [ -d "$OVERLAY_SRC" ]; then
	cp -a "$OVERLAY_SRC"/. "$tmp"/
fi

# Place direct root-level config.toml copies in the appliance image so operators
# can edit this file on writable boot media before first run.
# Prefer assets/default/config.toml (Win98 SE first / ReactOS, 3dfx-tuned) when
# present; fall back to the documented example otherwise.
# A duplicate uppercase 8.3-style name is intentionally included because older
# legacy media readers may fold filenames to case-insensitive/short-name style.
if [ -f "$REPO_ROOT/assets/default/config.toml" ]; then
	cp "$REPO_ROOT/assets/default/config.toml" "$tmp/config.toml"
else
	cp "$OVERLAY_SRC/etc/native-qemu/config.toml.example" "$tmp/config.toml"
fi
cat "$tmp/config.toml" > "$tmp/CONFIG.TOML"

# Keep the physical Windows XP VirtIO test profile available on the appliance
# itself, not merely in the source checkout used to build the ISO.
if [ -f "$REPO_ROOT/examples/winxp-virtio.toml" ]; then
	mkdir -p "$tmp/etc/native-qemu/examples"
	cp "$REPO_ROOT/examples/winxp-virtio.toml" "$tmp/etc/native-qemu/examples/winxp-virtio.toml"
fi

mkdir -p "$tmp"/etc
echo "$HOSTNAME" > "$tmp"/etc/hostname

# `apks` in the mkimage profile builds the ISO's local boot repository, but
# does not itself select packages for installation into the diskless root.
# The apkovl's world file is that selection. Without it the launcher starts
# but its QEMU/service dependencies are absent after the initramfs switch.
case "${NATIVE_QEMU_ARCH:-$(uname -m)}" in
	x86_64)
		native_qemu_system_package="qemu-system-x86_64"
		native_qemu_firmware_package="ovmf"
		;;
	aarch64)
		native_qemu_system_package="qemu-system-aarch64"
		native_qemu_firmware_package="aavmf"
		;;
	*)
		echo "native-qemu: unsupported appliance architecture: ${NATIVE_QEMU_ARCH:-$(uname -m)}" >&2
		exit 1
		;;
esac
mkdir -p "$tmp"/etc/apk
cat > "$tmp"/etc/apk/world <<EOF
alpine-base
busybox
openrc
doas
e2fsprogs
kbd-bkeymaps
tzdata
dropbear
usbutils
pciutils
openssl
libgcc
$native_qemu_system_package
$native_qemu_firmware_package
qemu-img
qemu-hw-usb-host
qemu-bridge-helper
qemu-audio-alsa
qemu-audio-pa
qemu-ui-sdl
pipewire
pipewire-pulse
virtiofsd
dnsmasq
samba
iproute2
EOF

# Tells mkinitfs's initramfs-init to add its standard default sysinit/boot/
# shutdown services (mdev, hwdrivers, modloop, hostname, syslog, etc.) —
# normally on by default, but suppressed whenever an apkovl is present unless
# this marker exists. Without it, hardware auto-detection (mdev/hwdrivers)
# and the modloop (which carries virtually the full kernel module set) never
# get enabled, and /etc/hostname above never actually gets applied either.
# initramfs-init deletes this file itself after reading it.
touch "$tmp"/etc/.default_boot_services

cat > "$tmp"/etc/motd <<'EOF'

  native-qemu — direct QEMU passthrough appliance
  Edit /etc/native-qemu/config.toml (copied from config.toml.example on
  first boot), then `lbu commit` and reboot to apply.

EOF

cat > "$tmp"/etc/inittab <<'EOF'
::sysinit:/sbin/openrc sysinit
::wait:/sbin/openrc boot
::wait:/sbin/openrc default
tty1::respawn:/usr/local/bin/native-qemu-agent
::ctrlaltdel:/sbin/reboot
::shutdown:/sbin/openrc shutdown
EOF

mkdir -p "$tmp"/usr/local/bin "$tmp"/etc/native-qemu
[ ! -f "$tmp"/usr/local/bin/native-qemu-agent ] || chmod +x "$tmp"/usr/local/bin/native-qemu-agent
for f in startup.sh.example shutdown.sh.example; do
	[ ! -f "$tmp"/etc/native-qemu/"$f" ] || chmod +x "$tmp"/etc/native-qemu/"$f"
done

tar -c -C "$tmp" etc usr config.toml CONFIG.TOML | gzip -9n > "$HOSTNAME.apkovl.tar.gz"
