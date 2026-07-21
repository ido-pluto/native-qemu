#!/usr/bin/env bash
# Build a Void Linux **glibc** x86_64 live/rootfs ISO for native-qemu + qemu-3dfx.
#
# Status: scaffold — Phase 1 of the Alpine → Void migration.
# Expects a qemu-3dfx prefix tarball from build/qemu-3dfx.sh --pack.
#
# High-level steps (implemented incrementally):
#   1. Bootstrap Void glibc rootfs (xbps)
#   2. Install runtime: linux, mesa, SDL2, KVM bits, base tools
#   3. Install native-qemu-agent (glibc) + qemu-3dfx under /usr/local
#   4. Enable agent at boot (runit service)
#   5. Produce bootable ISO (void-mklive or xorriso hybrid)
#
# Usage (on a Void or Ubuntu host with root/docker):
#   ./build/void-iso.sh --qemu-tarball dist/qemu-3dfx-x86_64.tar.gz
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
ARCH="${ARCH:-x86_64}"
OUTDIR="${OUTDIR:-${ROOT}/dist}"
QEMU_TARBALL="${QEMU_TARBALL:-${ROOT}/dist/qemu-3dfx-x86_64.tar.gz}"

usage() {
	sed -n '2,20p' "$0"
}

while [ $# -gt 0 ]; do
	case "$1" in
	--qemu-tarball)
		QEMU_TARBALL="$2"
		shift 2
		;;
	--outdir)
		OUTDIR="$2"
		shift 2
		;;
	--help|-h)
		usage
		exit 0
		;;
	*)
		echo "unknown arg: $1" >&2
		usage
		exit 2
		;;
	esac
done

echo "void-iso: arch=${ARCH} outdir=${OUTDIR}"
echo "void-iso: qemu_tarball=${QEMU_TARBALL}"

if [ ! -f "$QEMU_TARBALL" ]; then
	echo "void-iso: missing qemu tarball — build with:" >&2
	echo "  ./build/qemu-3dfx.sh --pack" >&2
	exit 1
fi

mkdir -p "$OUTDIR"

# --- Phase 1 placeholder ---------------------------------------------------
# Full void-mklive / xbps bootstrap will replace this once the qemu-3dfx CI
# artifact is green. For now we fail clearly so CI can gate on a future job.
cat >&2 <<EOF
void-iso: not fully implemented yet.

Next steps (see plan):
  1. bootstrap Void glibc ${ARCH} rootfs via xbps-install -r
  2. pkg: linux mesa SDL2 qemu-img deps libepoxy … 
  3. unpack ${QEMU_TARBALL} into rootfs
  4. install native-qemu-agent + runit service
  5. void-mklive / xorriso → ${OUTDIR}/native-qemu-${ARCH}.iso

EOF
exit 1
