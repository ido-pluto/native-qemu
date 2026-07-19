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

# Keep the physical Windows XP VirtIO test profile available on the appliance
# itself, not merely in the source checkout used to build the ISO.
if [ -f "$REPO_ROOT/examples/winxp-virtio.toml" ]; then
	mkdir -p "$tmp/etc/native-qemu/examples"
	cp "$REPO_ROOT/examples/winxp-virtio.toml" "$tmp/etc/native-qemu/examples/winxp-virtio.toml"
fi

mkdir -p "$tmp"/etc
echo "$HOSTNAME" > "$tmp"/etc/hostname

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

tar -c -C "$tmp" etc usr | gzip -9n > "$HOSTNAME.apkovl.tar.gz"
