#!/bin/sh

# Builds both x86_64/aarch64 native-qemu ISOs with a visible spinner while the
# heavy containerized mkimage step runs. The actual build process remains in the
# Docker worker; this shell thread only reports progress so the caller thread does
# not appear frozen during long image generations.

set -eu

usage() {
	cat <<'EOF'
Usage: ./scripts/build-native-qemu-iso.sh <x86_64|aarch64> <output-dir>

This wrapper runs the existing mkimage pipeline and prints progress while it is
running in the background.

Example:
  ./scripts/build-native-qemu-iso.sh x86_64 dist
EOF
}

if [ "$#" -ne 2 ]; then
	usage >&2
	exit 64
fi

ARCH=$1
OUTDIR=$2

if [ "$ARCH" != "x86_64" ] && [ "$ARCH" != "aarch64" ]; then
	echo "build-native-qemu-iso: unsupported architecture '$ARCH'" >&2
	usage >&2
	exit 64
fi

mkdir -p "$OUTDIR"

log_file="$(mktemp -t native-qemu-build-XXXXXX.log)"
trap 'rm -f "$log_file"' EXIT

(
	docker run --rm \
		-e ALPINE_VERSION="${ALPINE_VERSION:-3.20}" \
		-e MATRIX_ARCH="$ARCH" \
		-e GITHUB_REF_NAME="${GITHUB_REF_NAME:-dev}" \
		-v "$PWD":/repo -w /repo \
		"alpine:${ALPINE_VERSION:-3.20}" \
		sh /repo/build/ci-build.sh >"$log_file" 2>&1
) &

build_pid=$!

(
	i=0
	spin='|/-\\'
	while kill -0 "$build_pid" 2>/dev/null; do
		i=$(((i + 1) % 4))
		done_char=$(printf "%s" "$spin" | cut -c "$((i + 1))")
		status_line=""
		if [ -s "$log_file" ]; then
			status_line=$(tail -n 1 "$log_file" | sed 's/[[:space:]]\+/ /g')
		fi
		printf '\rnative-qemu: building image [%s] %s' "$done_char" "$status_line"
		sleep 0.25
	done
	echo
) &

spinner_pid=$!

if ! wait "$build_pid"; then
	echo
	cat "$log_file" >&2
	exit 1
fi

kill "$spinner_pid" 2>/dev/null || true
echo

generated_iso=$(find dist -maxdepth 1 -type f -name "native-qemu-$ARCH*.iso" 2>/dev/null | head -n 1)
if [ -z "$generated_iso" ]; then
	echo "build-native-qemu-iso: build finished but no ISO was produced" >&2
	exit 1
fi

cp "$generated_iso" "$OUTDIR/"
echo "build-native-qemu-iso: copied $(basename "$generated_iso") into $OUTDIR"
