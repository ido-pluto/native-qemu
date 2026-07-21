#!/usr/bin/env bash
# Build QEMU 9.2.x + kjliew/qemu-3dfx (MESA GL / 3Dfx Glide pass-through).
#
# Outputs a relocatable prefix tree under $DESTDIR (default: dist/qemu-3dfx-prefix)
# and optionally packs dist/qemu-3dfx-x86_64.tar.gz.
#
# Usage (Ubuntu/Debian or Void glibc builder):
#   sudo apt-get install -y ...   # see CI workflow
#   ./build/qemu-3dfx.sh
#   DESTDIR=/tmp/q ./build/qemu-3dfx.sh --pack
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
QEMU_VERSION="${QEMU_VERSION:-9.2.2}"
QEMU_3DFX_REPO="${QEMU_3DFX_REPO:-https://github.com/kjliew/qemu-3dfx.git}"
# Pin when QEMU_3DFX_REF is set; default: master tip at build time.
QEMU_3DFX_REF="${QEMU_3DFX_REF:-eefd567ddc7d718ab5848f9d04df0dfd40776cf1}"
QEMU_URL="${QEMU_URL:-https://download.qemu.org/qemu-${QEMU_VERSION}.tar.xz}"
WORK="${WORK:-${ROOT}/.cache/qemu-3dfx-build}"
DESTDIR="${DESTDIR:-${ROOT}/dist/qemu-3dfx-prefix}"
PREFIX="${PREFIX:-/usr/local}"
JOBS="${JOBS:-$(nproc 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo 4)}"
PACK=0

for arg in "$@"; do
	case "$arg" in
	--pack) PACK=1 ;;
	--help|-h)
		sed -n '2,20p' "$0"
		exit 0
		;;
	esac
done

echo "qemu-3dfx: version=${QEMU_VERSION} ref=${QEMU_3DFX_REF} jobs=${JOBS}"
echo "qemu-3dfx: work=${WORK} destdir=${DESTDIR}"

# Optional: install deps when running on Void (CI container).
if command -v xbps-install >/dev/null 2>&1; then
	echo "qemu-3dfx: ensuring Void build dependencies"
	xbps-install -Syu xbps || true
	xbps-install -Sy git curl tar xz rsync patch bash flex bison \
		gcc make pkg-config python3 python3-pip ninja || true
	xbps-install -Sy glib-devel pixman-devel SDL2-devel libepoxy-devel \
		libslirp-devel dtc zlib-devel libzstd-devel || true
	xbps-install -Sy libX11-devel libXext-devel libXxf86vm-devel libXi-devel \
		libXrandr-devel libXrender-devel || true
	xbps-install -Sy MesaLib-devel 2>/dev/null \
		|| xbps-install -Sy libglvnd-devel 2>/dev/null \
		|| true
	# QEMU's mkvenv needs distlib (not always packaged on Void)
	python3 -m pip install --break-system-packages -q distlib setuptools wheel 2>/dev/null \
		|| pip3 install -q distlib setuptools wheel 2>/dev/null \
		|| true
fi

mkdir -p "$WORK" "$DESTDIR" "${ROOT}/dist"
cd "$WORK"

if [ ! -d qemu-3dfx/.git ]; then
	rm -rf qemu-3dfx
	git clone --filter=blob:none "$QEMU_3DFX_REPO" qemu-3dfx
fi
git -C qemu-3dfx fetch --depth 1 origin "$QEMU_3DFX_REF" 2>/dev/null \
	|| git -C qemu-3dfx fetch origin "$QEMU_3DFX_REF"
git -C qemu-3dfx checkout -f "$QEMU_3DFX_REF"

TARBALL="qemu-${QEMU_VERSION}.tar.xz"
if [ ! -f "$TARBALL" ]; then
	echo "qemu-3dfx: downloading ${QEMU_URL}"
	curl -fL --retry 3 -o "$TARBALL" "$QEMU_URL"
fi

SRC="qemu-${QEMU_VERSION}"
rm -rf "$SRC"
tar xf "$TARBALL"
cd "$SRC"

echo "qemu-3dfx: applying hw overlays + patch"
rsync -a "${WORK}/qemu-3dfx/qemu-0/hw/3dfx" ./hw/
rsync -a "${WORK}/qemu-3dfx/qemu-1/hw/mesa" ./hw/
patch -p0 -i "${WORK}/qemu-3dfx/00-qemu92x-mesa-glide.patch"
# sign_commit stamps a version string via git; ignore ownership/safe.directory noise in Docker
git config --global --add safe.directory "$ROOT" 2>/dev/null || true
git config --global --add safe.directory "$WORK" 2>/dev/null || true
git config --global --add safe.directory "$(pwd)" 2>/dev/null || true
bash "${WORK}/qemu-3dfx/scripts/sign_commit" || echo "qemu-3dfx: sign_commit skipped (non-fatal)"

BUILD="${WORK}/build-${QEMU_VERSION}"
rm -rf "$BUILD"
mkdir -p "$BUILD"
cd "$BUILD"

# Narrow target list: appliance is x86_64 KVM + SDL/OpenGL for 3dfx/MESA.
# qemu-img is kept via --enable-tools for image ops on the stick.
echo "qemu-3dfx: configure"
"${WORK}/${SRC}/configure" \
	--prefix="$PREFIX" \
	--target-list=x86_64-softmmu \
	--enable-system \
	--enable-tools \
	--disable-user \
	--enable-kvm \
	--enable-sdl \
	--enable-opengl \
	--enable-slirp \
	--enable-vhost-net \
	--enable-vhost-user \
	--disable-docs \
	--disable-gtk \
	--disable-vnc \
	--disable-spice \
	--disable-xen \
	--disable-smartcard \
	--disable-libnfs \
	--disable-libiscsi \
	--disable-rbd \
	--disable-glusterfs \
	--disable-curl \
	--disable-brlapi \
	--disable-vde \
	--disable-netmap \
	--disable-capstone \
	--disable-werror

echo "qemu-3dfx: make -j${JOBS}"
make -j"$JOBS"

echo "qemu-3dfx: install DESTDIR=${DESTDIR}"
rm -rf "$DESTDIR"
mkdir -p "$DESTDIR"
make install DESTDIR="$DESTDIR"

BIN="${DESTDIR}${PREFIX}/bin/qemu-system-x86_64"
test -x "$BIN"
echo "qemu-3dfx: smoke version"
"$BIN" --version || true
echo "qemu-3dfx: smoke 3dfx/mesa devices (if QEMU can run without KVM here)"
set +e
HELP="$("$BIN" -device help 2>&1)"
set -e
if echo "$HELP" | grep -Eiq '3dfx|glide|mesa'; then
	echo "qemu-3dfx: device help mentions 3dfx/mesa/glide ✓"
	echo "$HELP" | grep -Eiq '3dfx|glide|mesa' || true
	echo "$HELP" | grep -Ei '3dfx|glide|mesa' | head -20
else
	echo "qemu-3dfx: WARNING — could not confirm 3dfx/mesa in -device help (may need full run)"
	echo "$HELP" | head -40
fi

if [ "$PACK" -eq 1 ]; then
	OUT="${ROOT}/dist/qemu-3dfx-x86_64.tar.gz"
	echo "qemu-3dfx: packing ${OUT}"
	tar -C "$DESTDIR" -czf "$OUT" .
	ls -lh "$OUT"
fi

echo "qemu-3dfx: done"
