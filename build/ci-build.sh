#!/bin/sh
# Runs inside an `alpine:$ALPINE_VERSION` container (via `docker run`, not
# GitHub's job-level `container:` — that feature can't run JS actions like
# checkout/upload-artifact on arm64 runners, so the Alpine work happens as a
# single containerized step instead). Expects the repo bind-mounted at /repo,
# and ALPINE_VERSION / MATRIX_ARCH / GITHUB_REF_NAME in the environment.
set -eu

apk add --no-cache \
	abuild apk-tools alpine-conf busybox fakeroot \
	xorriso squashfs-tools mtools \
	grub grub-efi git \
	rust cargo

# Build native-qemu-agent (the Rust binary that does all the runtime work —
# config parsing, storage/USB resolution, talking to qemu over QMP) natively
# in this same container. Since this container's arch always matches the
# target ISO's arch (see build.yml's per-arch runners), this produces a
# proper musl binary with no cross-compilation needed at all. Alpine 3.20
# ships rustc 1.78, which predates a couple of crates' newer minimum
# versions — see agent/Cargo.toml's indexmap pin.
( cd /repo/agent && cargo test --locked && cargo build --locked --release )
mkdir -p /repo/overlay/usr/local/bin
cp /repo/agent/target/release/native-qemu-agent /repo/overlay/usr/local/bin/native-qemu-agent

# syslinux (BIOS/legacy bootloader) only exists as an x86/x86_64 package —
# there's no aarch64 build of it at all, so only install it when we're
# actually assembling an x86_64 image (mkimg.base.sh's section_syslinux only
# runs for that arch anyway; UEFI via grub covers both arches).
if [ "$MATRIX_ARCH" = "x86_64" ]; then
	apk add --no-cache syslinux
fi

# Use the aports branch matching ALPINE_VERSION, not the default (edge/master)
# branch — mkimage.sh/mkimg.base.sh call CLI tools (e.g. update-kernel) whose
# flags vary between alpine-conf versions, so the scripts must match the
# alpine-conf actually installed from the v$ALPINE_VERSION repos above.
#
# mkimage.sh also asserts main/build-base exists as a sanity check that it's
# running inside a real aports checkout, so that path needs to be pulled in
# alongside scripts/.
git clone --depth 1 --filter=blob:none --sparse --branch "${ALPINE_VERSION}-stable" \
	https://gitlab.alpinelinux.org/alpine/aports.git /tmp/aports
git -C /tmp/aports sparse-checkout set scripts main/build-base

cp /repo/build/mkimg.native_qemu.sh /repo/build/genapkovl-native-qemu.sh /tmp/aports/scripts/
chmod +x /tmp/aports/scripts/genapkovl-native-qemu.sh

# The ISO embeds a local apk repository (the "boot repository") that the
# initramfs installs from at boot time via `apk add --no-network`, which
# refuses anything from an untrusted (unsigned) index. So we need our own
# signing keypair — not Alpine's real one, just something apk considers
# trusted — and that public key baked into the initramfs via --hostkeys.
# Without this, the boot-time apk add silently installs nothing and the new
# root ends up empty ("/sbin/init not found in new root").
abuild-keygen -a -n
. /root/.abuild/abuild.conf
cp "${PACKAGER_PRIVKEY}.pub" /etc/apk/keys/

mkdir -p /repo/dist
cd /tmp/aports/scripts
./mkimage.sh \
	--tag "${GITHUB_REF_NAME:-dev}" \
	--outdir /repo/dist \
	--arch "$MATRIX_ARCH" \
	--hostkeys \
	--repository "https://dl-cdn.alpinelinux.org/alpine/v${ALPINE_VERSION}/main" \
	--repository "https://dl-cdn.alpinelinux.org/alpine/v${ALPINE_VERSION}/community" \
	--profile native_qemu

# A green mkimage invocation alone does not prove that our custom apkovl was
# embedded in the ISO. Inspect the produced artifact itself before CI uploads
# it: the launcher, config, hooks, and guest docs must all be present, and
# the selected architecture must carry the matching UEFI firmware package.
iso="$(find /repo/dist -maxdepth 1 -type f -name '*.iso' -print | head -n 1)"
test -n "$iso"
echo "native-qemu: inspecting ISO artifact $iso"
verify_dir="$(mktemp -d)"
cleanup_verify() { rm -rf "$verify_dir"; }
trap cleanup_verify EXIT
# `hostname="native-qemu"` in the image profile determines this stable path.
# Keep it explicit: besides avoiding a fragile filesystem search, this makes a
# profile/overlay naming regression fail with a useful, local error.
apkovl_path="/native-qemu.apkovl.tar.gz"
echo "native-qemu: extracting apkovl $apkovl_path"
xorriso -osirrox on -indev "$iso" -extract "$apkovl_path" "$verify_dir/apkovl.tar.gz" >/dev/null
tar -tzf "$verify_dir/apkovl.tar.gz" > "$verify_dir/apkovl.contents"
for required in \
	usr/local/bin/native-qemu-agent \
	etc/native-qemu/config.toml.example \
	etc/native-qemu/startup.sh.example \
	etc/native-qemu/shutdown.sh.example \
	etc/native-qemu/examples/winxp-virtio.toml \
	etc/native-qemu/docs/index.html \
	etc/native-qemu/docs/configuration.html; do
	echo "native-qemu: checking apkovl member $required"
	grep -qx "$required" "$verify_dir/apkovl.contents"
done
case "$MATRIX_ARCH" in
	x86_64) firmware_package='ovmf-*.apk' ;;
	aarch64) firmware_package='aavmf-*.apk' ;;
	*) exit 1 ;;
esac
echo "native-qemu: checking embedded firmware package $firmware_package"
# Alpine's xorriso uses its own `-find` action syntax; `-exec echo` emits
# every matching ISO pathname, whereas GNU find's `-print` is not valid.
xorriso -indev "$iso" -find / -type f -name "$firmware_package" -exec echo 2>/dev/null | grep -q .
