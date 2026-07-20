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
#
# Build via the workspace root so the binary lands at a predictable path
# (/repo/target/release/...), not /repo/agent/target (workspace members
# share the root target dir).
( cd /repo && cargo test -p native-qemu-agent --locked \
	&& cargo build -p native-qemu-agent --locked --release )
mkdir -p /repo/overlay/usr/local/bin
agent_bin=""
for candidate in \
	/repo/target/release/native-qemu-agent \
	/repo/agent/target/release/native-qemu-agent; do
	if [ -x "$candidate" ]; then
		agent_bin=$candidate
		break
	fi
done
if [ -z "$agent_bin" ]; then
	echo "native-qemu: agent binary not found after cargo build" >&2
	find /repo -name 'native-qemu-agent' -type f 2>/dev/null | head -20 >&2 || true
	exit 1
fi
echo "native-qemu: installing agent from $agent_bin"
cp "$agent_bin" /repo/overlay/usr/local/bin/native-qemu-agent

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
NATIVE_QEMU_ARCH="$MATRIX_ARCH" ./mkimage.sh \
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

# x86_64 only: unpack multi-part 7z guest image and inject as /images/image.qcow2
# so flash tools can seed the ext4 data volume with a default ReactOS/Win98 disk.
if [ "$MATRIX_ARCH" = "x86_64" ] && [ -f /repo/assets/image/image.7z.001 ]; then
	echo "native-qemu: injecting default images/image.qcow2 into ISO"
	apk add --no-cache p7zip >/dev/null
	img_tmp="$(mktemp -d)"
	7z x /repo/assets/image/image.7z.001 -o"$img_tmp" -y >/dev/null
	# Accept either image.qcow2 or nested path
	img_file="$(find "$img_tmp" -type f -name 'image.qcow2' | head -n 1)"
	test -n "$img_file"
	xorriso -dev "$iso" -boot_image any keep \
		-map "$img_file" /images/image.qcow2 \
		-chmod 0444 /images/image.qcow2 -- \
		>/dev/null
	rm -rf "$img_tmp"
fi

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
	etc/apk/world \
	config.toml \
	CONFIG.TOML \
	etc/native-qemu/config.toml.example \
	etc/native-qemu/startup.sh.example \
	etc/native-qemu/shutdown.sh.example \
	etc/native-qemu/examples/winxp-virtio.toml \
	etc/native-qemu/docs/index.html \
	etc/native-qemu/docs/configuration.html; do
	echo "native-qemu: checking apkovl member $required"
	grep -qx "$required" "$verify_dir/apkovl.contents"
done
tar -xOf "$verify_dir/apkovl.tar.gz" etc/apk/world > "$verify_dir/apkovl.world"
grep -qx 'libgcc' "$verify_dir/apkovl.world"
grep -qx "qemu-system-$MATRIX_ARCH" "$verify_dir/apkovl.world"
case "$MATRIX_ARCH" in
	x86_64)
		firmware_package='ovmf-*.apk'
		grep -qx 'ovmf' "$verify_dir/apkovl.world"
		;;
	aarch64)
		firmware_package='aavmf-*.apk'
		grep -qx 'aavmf' "$verify_dir/apkovl.world"
		;;
	*) exit 1 ;;
esac
echo "native-qemu: checking embedded firmware package $firmware_package"
# Alpine's xorriso uses its own `-find` action syntax; `-exec echo` emits
# every matching ISO pathname, whereas GNU find's `-print` is not valid.
xorriso -indev "$iso" -find / -type f -name "$firmware_package" -exec echo 2>/dev/null | grep -q .
if [ "$MATRIX_ARCH" = "x86_64" ] && [ -f /repo/assets/image/image.7z.001 ]; then
	echo "native-qemu: checking ISO embeds images/image.qcow2"
	xorriso -indev "$iso" -find /images -type f -name 'image.qcow2' -exec echo 2>/dev/null | grep -q .
fi
