# native-qemu appliance profile for Alpine's mkimage.sh
#
# Builds a minimal, no-GUI Alpine ISO whose only job is to boot directly into
# a configured QEMU VM. This file is copied into a
# checkout of https://gitlab.alpinelinux.org/alpine/aports "scripts/" directory
# before mkimage.sh runs — see .github/workflows/build.yml.
#
# The launcher is built during CI and placed in the apkovl overlay.

profile_native_qemu() {
	profile_base

	profile_abbrev="nqemu"
	image_name="native-qemu"
	image_ext="iso"
	output_format="iso"
	arch="x86_64 aarch64"
	# "lts" (not "virt") is the general-purpose kernel flavor with the widest
	# in-tree driver coverage for arbitrary physical hardware — this appliance
	# boots on real machines, not just VMs, so broad hardware support matters
	# more than the smaller/tuned "virt" kernel.
	kernel_flavors="lts"
	initfs_features="$initfs_features virtio"

	# Deliberately minimal — override profile_base's broader "standard" apks
	# list rather than extend it, to keep the appliance lightweight.
	apks="alpine-base busybox openrc doas e2fsprogs kbd-bkeymaps tzdata \
		dropbear usbutils pciutils openssl"

	case "$ARCH" in
	x86_64)  apks="$apks qemu-system-x86_64 ovmf" ;;
		aarch64) apks="$apks qemu-system-aarch64 aavmf" ;;  # aarch64's "virt" machine has no BIOS option — UEFI is mandatory
	esac
	# Alpine's qemu packaging is modular: device/backend support beyond the
	# base emulator is split into separate qemu-hw-*/qemu-audio-*/etc.
	# packages (the "qemu-modules" meta-package pulls in *all* of them,
	# including GTK and Spice we do not need — so these are picked individually
	# instead). qemu-ui-sdl is deliberately included: it renders the guest
	# directly on a physical KMS/DRM console, without a host desktop.
	# `pipewire-pulse` exposes Alpine's supported QEMU PulseAudio backend to
	# PipeWire when sound.backend = "pipewire".  virtiofsd, dnsmasq, Samba,
	# and iproute2 power the optional shared-folder, docs, SMB, and macvtap
	# services; all are inactive unless explicitly enabled in config.toml.
	apks="$apks qemu-img qemu-hw-usb-host qemu-bridge-helper qemu-audio-alsa \
		qemu-audio-pa qemu-ui-sdl pipewire pipewire-pulse virtiofsd dnsmasq samba iproute2"

	# Broad out-of-the-box hardware support (touchscreens, SD/MMC readers,
	# HDMI/DisplayPort, etc.) mostly comes for free, independent of this
	# package list:
	#   - kernel_flavors=lts above ships virtually the entire in-tree Linux
	#     driver set as loadable modules in the "modloop" (a squashfs baked
	#     onto the ISO) — not something picked per device.
	#   - every kernel build here automatically pulls in linux-firmware (see
	#     aports scripts/mkimg.base.sh's section_kernels), covering firmware
	#     blobs most GPU/wifi/etc. drivers need.
	#   - mdev + hwdrivers + modloop are enabled as default sysinit services
	#     unconditionally by Alpine's initramfs-init, so connected hardware
	#     is auto-detected and the matching module auto-loaded with zero
	#     config from us.
	# USB-attached peripherals (most touchscreens, external SD readers, HDMI
	# capture dongles) additionally get automatic VM passthrough for free via
	# config.toml's default USB passthrough policy (see plan.md). Built-in,
	# non-USB hardware (an internal I2C touchscreen, a soldered PCIe SD
	# controller, the machine's own GPU for real HDMI/DisplayPort output)
	# needs VFIO PCI passthrough of the whole controller instead — a
	# separate future extension noted in plan.md, not solved by packages.
	#
	# hardware_packages is the extension point for anything beyond that:
	# add one package name per line and it's baked into the next build.
	hardware_packages="
	"
	apks="$apks $hardware_packages"

	apkovl="genapkovl-native-qemu.sh"
	hostname="native-qemu"
}
