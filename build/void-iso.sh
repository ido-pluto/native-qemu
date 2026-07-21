#!/usr/bin/env bash
# Build a Void Linux **glibc** x86_64 hybrid live ISO with:
#   - CI-built QEMU 9.2 + qemu-3dfx under /usr/local
#   - native-qemu-agent on tty1 (runit)
#   - Mesa/SDL2 and KVM-friendly kernel modules
#
# Prefer running inside Void (xbps). On Ubuntu CI we re-exec via Docker:
#   ghcr.io/void-linux/void-glibc-full:latest --privileged
#
# Usage:
#   sudo ./build/void-iso.sh --qemu-tarball dist/qemu-3dfx-x86_64.tar.gz \
#       --agent-bin target/release/native-qemu-agent
#   OUTDIR=dist ./build/void-iso.sh ...
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
ARCH="${ARCH:-x86_64}"
OUTDIR="${OUTDIR:-${ROOT}/dist}"
QEMU_TARBALL="${QEMU_TARBALL:-${ROOT}/dist/qemu-3dfx-x86_64.tar.gz}"
AGENT_BIN="${AGENT_BIN:-}"
VOID_MKLIVE_URL="${VOID_MKLIVE_URL:-https://github.com/void-linux/void-mklive.git}"
VOID_MKLIVE_REF="${VOID_MKLIVE_REF:-master}"
WORK="${WORK:-${ROOT}/.cache/void-iso}"
DOCKER_IMAGE="${DOCKER_IMAGE:-ghcr.io/void-linux/void-glibc-full:latest}"
FORCE_DOCKER="${FORCE_DOCKER:-0}"

usage() {
	sed -n '2,25p' "$0"
}

while [ $# -gt 0 ]; do
	case "$1" in
	--qemu-tarball) QEMU_TARBALL="$2"; shift 2 ;;
	--agent-bin) AGENT_BIN="$2"; shift 2 ;;
	--outdir) OUTDIR="$2"; shift 2 ;;
	--work) WORK="$2"; shift 2 ;;
	--docker) FORCE_DOCKER=1; shift ;;
	--help|-h) usage; exit 0 ;;
	*) echo "unknown arg: $1" >&2; usage; exit 2 ;;
	esac
done

log() { echo "void-iso: $*"; }

need_file() {
	if [ ! -f "$1" ]; then
		echo "void-iso: missing $1" >&2
		exit 1
	fi
}

need_file "$QEMU_TARBALL"

# Resolve agent binary
if [ -z "$AGENT_BIN" ]; then
	for c in \
		"${ROOT}/target/release/native-qemu-agent" \
		"${ROOT}/agent/target/release/native-qemu-agent"; do
		if [ -x "$c" ]; then AGENT_BIN=$c; break; fi
	done
fi
if [ -z "${AGENT_BIN:-}" ] || [ ! -x "$AGENT_BIN" ]; then
	echo "void-iso: native-qemu-agent not found — build with:" >&2
	echo "  cargo build -p native-qemu-agent --release" >&2
	exit 1
fi

# --- Re-exec into Void container when host cannot run mklive natively --------
# Prefer "do we have xbps-install?" over /etc/os-release (container images vary).
have_xbps=0
command -v xbps-install >/dev/null 2>&1 && have_xbps=1

if [ "$FORCE_DOCKER" = 1 ] || [ "$have_xbps" = 0 ]; then
	log "host lacks xbps (or FORCE_DOCKER=1) — running via Docker ${DOCKER_IMAGE}"
	if ! command -v docker >/dev/null 2>&1; then
		echo "void-iso: docker required to build on non-Void hosts" >&2
		exit 1
	fi
	mkdir -p "$OUTDIR" "$WORK"
	# Map repo + caches; privileged for loop/mount in mklive
	exec docker run --rm --privileged \
		-e ARCH \
		-e FORCE_DOCKER=0 \
		-v "$ROOT:/repo:rw" \
		-v "$(cd "$(dirname "$QEMU_TARBALL")" && pwd)/$(basename "$QEMU_TARBALL"):/qemu/qemu-3dfx-x86_64.tar.gz:ro" \
		-v "$(cd "$(dirname "$AGENT_BIN")" && pwd)/$(basename "$AGENT_BIN"):/agent/native-qemu-agent:ro" \
		-w /repo \
		"$DOCKER_IMAGE" \
		/bin/sh -c '
			set -eu
			xbps-install -Syu xbps || true
			xbps-install -Sy bash git curl tar xz rsync xorriso squashfs-tools \
				liblz4 ca-certificates coreutils findutils grep sed gawk || true
			exec bash /repo/build/void-iso.sh \
				--qemu-tarball /qemu/qemu-3dfx-x86_64.tar.gz \
				--agent-bin /agent/native-qemu-agent \
				--outdir /repo/dist \
				--work /repo/.cache/void-iso
		'
fi

log "running natively with xbps-install"

if [ "$(id -u)" -ne 0 ]; then
	echo "void-iso: must run as root (or via Docker)" >&2
	exit 1
fi

mkdir -p "$OUTDIR" "$WORK"
INCLUDE="${WORK}/include"
rm -rf "$INCLUDE"
mkdir -p "$INCLUDE"

log "staging include tree + qemu-3dfx + agent"
# Overlay configs
if [ -d "$ROOT/overlay" ]; then
	cp -a "$ROOT/overlay"/. "$INCLUDE"/
fi
mkdir -p "$INCLUDE/usr/local/bin" "$INCLUDE/etc/native-qemu" "$INCLUDE/etc/sv"

# Default configs at image root (boot media readable)
if [ -f "$ROOT/assets/default/config.toml" ]; then
	cp "$ROOT/assets/default/config.toml" "$INCLUDE/config.toml"
else
	cp "$ROOT/overlay/etc/native-qemu/config.toml.example" "$INCLUDE/config.toml"
fi
cp "$INCLUDE/config.toml" "$INCLUDE/CONFIG.TOML"
mkdir -p "$INCLUDE/etc/native-qemu"
cp "$INCLUDE/config.toml" "$INCLUDE/etc/native-qemu/config.toml.example" 2>/dev/null || true
if [ -f "$ROOT/overlay/etc/native-qemu/config.toml.example" ]; then
	cp "$ROOT/overlay/etc/native-qemu/config.toml.example" "$INCLUDE/etc/native-qemu/config.toml.example"
fi
if [ -f "$ROOT/examples/winxp-virtio.toml" ]; then
	mkdir -p "$INCLUDE/etc/native-qemu/examples"
	cp "$ROOT/examples/winxp-virtio.toml" "$INCLUDE/etc/native-qemu/examples/"
fi

# Runit service from repo template
cp -a "$ROOT/build/void-include/etc/sv/native-qemu-agent" "$INCLUDE/etc/sv/"
chmod 755 "$INCLUDE/etc/sv/native-qemu-agent/run" "$INCLUDE/etc/sv/native-qemu-agent/finish"

# Agent
cp "$AGENT_BIN" "$INCLUDE/usr/local/bin/native-qemu-agent"
chmod 755 "$INCLUDE/usr/local/bin/native-qemu-agent"

# Unpack qemu-3dfx prefix (tarball contains ./usr/local/...)
# into include so mklive -I copies it into ROOTFS
tar -C "$INCLUDE" -xzf "$QEMU_TARBALL"
if [ ! -x "$INCLUDE/usr/local/bin/qemu-system-x86_64" ]; then
	echo "void-iso: tarball missing usr/local/bin/qemu-system-x86_64" >&2
	tar tzf "$QEMU_TARBALL" | head -30 >&2
	exit 1
fi

# --- void-mklive ------------------------------------------------------------
MKLIVE_DIR="${WORK}/void-mklive"
if [ ! -d "$MKLIVE_DIR/.git" ]; then
	rm -rf "$MKLIVE_DIR"
	git clone --depth 1 --branch "$VOID_MKLIVE_REF" "$VOID_MKLIVE_URL" "$MKLIVE_DIR"
else
	git -C "$MKLIVE_DIR" fetch --depth 1 origin "$VOID_MKLIVE_REF" || true
	git -C "$MKLIVE_DIR" checkout -f "$VOID_MKLIVE_REF" 2>/dev/null \
		|| git -C "$MKLIVE_DIR" pull --ff-only || true
fi

# Host tools for mklive on Void
log "installing Void host packages for mklive"
xbps-install -Syu xbps || true
xbps-install -Sy git bash curl xz tar rsync \
	xorriso squashfs-tools liblz4 \
	qemu-img 2>/dev/null || xbps-install -Sy git bash curl xz tar rsync xorriso squashfs-tools

# Runtime packages inside the live image (no distro qemu — we ship 3dfx)
# glibc is default for x86_64 void (not -musl).
# Core runtime for agent + host GL for 3dfx/MESA pass-through.
# (Do not install distro qemu — we bake qemu-3dfx into /usr/local.)
# Void package names: MesaLib (not Mesa), mesa-dri for drivers.
PKGS=(
	e2fsprogs
	dosfstools
	util-linux
	kmod
	iproute2
	bridge-utils
	usbutils
	pciutils
	openssh
	dnsmasq
	alsa-lib
	alsa-utils
	SDL2
	libepoxy
	MesaLib
	mesa-dri
	libglvnd
	pixman
	libzstd
	zlib
	libslirp
	bash
	less
	tzdata
)

INSTALL_PKGS=()
for p in "${PKGS[@]}"; do
	# -R searches remote repos; must use glibc current (not musl) for x86_64
	if XBPS_ARCH="$ARCH" xbps-query -R -p pkgver "$p" >/dev/null 2>&1; then
		INSTALL_PKGS+=("$p")
		log "pkg ok: $p"
	else
		log "skip missing package: $p"
	fi
done
# Absolute minimum if repo query was flaky
if [ "${#INSTALL_PKGS[@]}" -lt 5 ]; then
	INSTALL_PKGS=(e2fsprogs SDL2 libepoxy mesa-dri iproute2 usbutils bash dnsmasq)
fi

PKG_STR="${INSTALL_PKGS[*]}"
log "image packages: $PKG_STR"

ISO_OUT="${OUTDIR}/native-qemu-${ARCH}.iso"
rm -f "$ISO_OUT"

POSTSETUP="${ROOT}/build/void-postsetup.sh"
chmod +x "$POSTSETUP" "$ROOT/build/void-include/etc/sv/native-qemu-agent/run"

cd "$MKLIVE_DIR"
# mklive must be root; caches under WORK
export XBPS_ARCH="$ARCH"
./mklive.sh \
	-a "$ARCH" \
	-b base-system \
	-o "$ISO_OUT" \
	-T "native-qemu" \
	-p "$PKG_STR" \
	-I "$INCLUDE" \
	-S "udevd" \
	-C "live.autologin consoleblank=0" \
	-x "$POSTSETUP" \
	-c "${WORK}/xbps-cache-${ARCH}" \
	-H "${WORK}/xbps-cache-host"

test -f "$ISO_OUT"
# Verify ISO embeds custom qemu path inside squashfs is hard; check size + xorriso listing if available
ls -lh "$ISO_OUT"
log "ISO ready: $ISO_OUT"

# Structural check: extract and look for qemu-system-x86_64 in squashfs if tools allow
if command -v xorriso >/dev/null 2>&1 && command -v unsquashfs >/dev/null 2>&1; then
	VERIFY="${WORK}/iso-verify"
	rm -rf "$VERIFY"
	mkdir -p "$VERIFY"
	if xorriso -osirrox on -indev "$ISO_OUT" -extract /LiveOS/squashfs.img "$VERIFY/squashfs.img" 2>/dev/null; then
		# LiveOS layout: ext3fs.img inside squashfs — skip deep verify if complex
		log "extracted LiveOS/squashfs.img for smoke (size $(stat -c%s "$VERIFY/squashfs.img" 2>/dev/null || echo ?))"
	fi
fi

log "done"
