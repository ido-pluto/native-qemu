#!/usr/bin/env bash
# Structural checks: workflow + docs match the shipping product shape.
# Exit 0 only if all assertions hold. Used as a committed gate for "docs/CI agree".
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
WF="$ROOT/.github/workflows/build.yml"
README="$ROOT/README.md"
fail=0

need() {
	local file="$1" pattern="$2" label="$3"
	if ! grep -Eiq -- "$pattern" "$file"; then
		echo "FAIL: $label (pattern not found in $(basename "$file")): $pattern" >&2
		fail=1
	else
		echo "ok: $label"
	fi
}

need "$WF" 'build-qemu-3dfx' 'CI job build-qemu-3dfx'
need "$WF" 'qemu-3dfx-x86_64\.tar\.gz' 'CI uploads qemu-3dfx-x86_64.tar.gz'
need "$WF" 'native-qemu-x86_64\.iso' 'CI packages native-qemu-x86_64.iso'
need "$WF" 'nq-disk' 'CI builds nq-disk'
need "$WF" 'QEMU_VERSION: "9\.2\.2"|QEMU_VERSION: .9\.2\.2' 'QEMU 9.2.2 pin'
# aarch64 Alpine product ISO must not remain in matrix
if grep -E 'arch: aarch64' "$WF" | grep -vq '^[[:space:]]*#'; then
	# allow comments only
	if grep -nE '^\s+- arch: aarch64' "$WF"; then
		echo "FAIL: product CI still builds aarch64 ISO matrix entry" >&2
		fail=1
	fi
else
	echo "ok: no aarch64 ISO matrix entry"
fi

need "$README" 'nq-disk' 'README documents nq-disk'
need "$README" 'sudo \./nq-disk' 'README requires sudo ./nq-disk'
need "$README" 'qemu-3dfx-x86_64\.tar\.gz' 'README documents qemu-3dfx tarball'
need "$README" 'native-qemu-x86_64\.iso' 'README documents x86_64 ISO'
need "$README" 'Void' 'README mentions Void migration'
need "$README" '9\.2' 'README mentions QEMU 9.2'

test -x "$ROOT/build/qemu-3dfx.sh" || {
	echo "FAIL: build/qemu-3dfx.sh missing or not executable" >&2
	fail=1
}
test -x "$ROOT/build/void-iso.sh" || {
	echo "FAIL: build/void-iso.sh missing or not executable" >&2
	fail=1
}
bash -n "$ROOT/build/qemu-3dfx.sh"
bash -n "$ROOT/build/void-iso.sh"
echo "ok: build scripts executable + bash -n"

if [ "$fail" -ne 0 ]; then
	echo "check-ci-product-shape: FAILED" >&2
	exit 1
fi
echo "check-ci-product-shape: all checks passed"
