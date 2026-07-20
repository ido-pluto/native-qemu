# native-qemu

A tiny, USB-bootable Linux appliance whose entire job is to boot straight into a QEMU virtual
machine — no desktop, no window manager, no host-side UI. You configure a VM once (which disk image,
which devices get passed through — keyboard, mouse, sound, camera, USB, network), and every boot from
the stick goes: power on → load config → launch that VM with everything passed through → guest shuts
down → the physical machine powers off. A direct hardware-to-VM proxy.

Full architecture, configuration reference, and the phased build roadmap live in [plan.md](plan.md).
Use [RELEASE_CHECKLIST.md](RELEASE_CHECKLIST.md) before publishing a tagged release.

## Status

This project is under active development. The ISO now boots directly into `native-qemu-agent`, which
creates `/etc/native-qemu/config.toml` from its bundled example on first boot and launches a
config-driven QEMU VM. It supports a local raw/qcow2 disk, user/bridge/macvtap networking, ALSA or
PipeWire sound, direct physical guest display, USB passthrough, virtiofs and scoped SMB sharing, a
guest-bridge docs/DNS service, startup/shutdown hooks, persistent logging, SSH, and lifecycle
handling (power off, restart, or rescue shell).

The default config intentionally names a non-existent test disk, so a fresh stick stops at a rescue
shell rather than starting an arbitrary disk. Put a guest disk at the configured storage path, edit
the config, and run `lbu commit` before rebooting. Guest-bridge services require an existing bridge
with an IPv4 address; global SMB sharing additionally requires a LAN interface and a password file.
Check [plan.md](plan.md#phased-roadmap) for the remaining real-hardware release gates.

## Supported hardware

Two separate builds, each running **natively** via KVM — there is no cross-architecture emulation:

| ISO | Runs on |
|---|---|
| `native-qemu-x86_64.iso` | x86_64 (Intel/AMD) machines |
| `native-qemu-aarch64.iso` | aarch64 (ARM64) machines |

Use the ISO matching the physical machine you're booting it on, not the guest OS you intend to run.

## Getting the ISO

Download the latest release from this repository's [Releases page](../../releases) — each tagged
release includes both `native-qemu-x86_64.iso` and `native-qemu-aarch64.iso`, built automatically by
GitHub Actions (see `.github/workflows/build.yml`).

To build it yourself instead of downloading, push a tag (`git tag v0.1.0 && git push --tags`) or run
the "Build ISOs" workflow manually from the Actions tab — it uploads both ISOs as workflow artifacts
even without a tag.

For local builds with a visible loading spinner and progress output while the containerized
mkimage flow runs, use:

```sh
./scripts/build-native-qemu-iso.sh x86_64 dist
./scripts/build-native-qemu-iso.sh aarch64 dist
```

## Writing the ISO to a USB stick

These images are hybrid ISOs — the same file that boots as an optical image can be written directly
(raw) to a USB stick. **Writing to the wrong device will destroy whatever is on it — double-check
the device path before running anything.**

The supplied helper requires an explicit acknowledgement, rejects partitions and macOS's system disk,
unmounts the target where appropriate, and SHA-256-verifies the bytes it wrote:

```sh
# macOS example (identify the external disk first with: diskutil list):
sudo scripts/write-usb.sh --yes-really-write native-qemu-x86_64.iso /dev/rdisk4

# Linux example (identify the whole disk first with: lsblk):
sudo scripts/write-usb.sh --yes-really-write native-qemu-x86_64.iso /dev/sdb
```

It deliberately refuses partition paths such as `/dev/sdb1` and `/dev/disk4s1`. Read the platform
notes below before choosing the target.

### macOS

```sh
# 1. Plug in the USB stick, then list disks to find it (look for the matching size):
diskutil list

# 2. Use its raw whole-disk path (e.g. /dev/rdisk4 — NOT /dev/disk4s1):
sudo scripts/write-usb.sh --yes-really-write native-qemu-x86_64.iso /dev/rdiskN
```

### Linux

```sh
# 1. Find the device (look for the matching size — NOT a partition like /dev/sdb1):
lsblk

# 2. Unmount any auto-mounted partitions, then write and verify:
sudo scripts/write-usb.sh --yes-really-write native-qemu-x86_64.iso /dev/sdX
```

### Windows

Use [Rufus](https://rufus.ie/) or [balenaEtcher](https://etcher.balena.io/): select the downloaded
`.iso` as the source, select the USB stick as the target, and write in **DD/raw image mode** (Rufus
will prompt for this automatically when it detects a hybrid ISO — choose "Write in DD Image mode",
not the default ISO mode).

## Booting it

1. Plug the USB stick into the target machine and power it on.
2. Enter the boot menu (varies by machine — commonly `F12`, `F10`, `Esc`, or `Del` right after power
   on) and select the USB stick. Both BIOS (legacy/CSM) and UEFI boot are supported on x86_64;
   aarch64 machines boot it via UEFI.
3. It boots directly into `native-qemu-agent`. On a fresh stick it creates the example config, then
   opens a rescue shell because the example disk path does not exist.

## Configuration

On first boot the agent resolves configuration in this order:
`/media/<boot-device>/config.toml`, `/media/<boot-device>/CONFIG.TOML`, `/config.toml`,
`/CONFIG.TOML`, `/etc/native-qemu/config.toml`,
then `/etc/native-qemu/config.toml.example`. Edit the selected file and, for host-side
persistence, run `lbu commit` if it should survive reboot.

For example, to use a guest image at `vms/main.qcow2` on the first internal disk, set:

```toml
[vm]
arch = "x86_64"

[vm.disk]
format = "qcow2"
storage = 1
path = "vms/main.qcow2"
```

Then persist the edit from the appliance shell with `lbu commit` and reboot. Storage index `0` is
the boot media, `1` is the first fixed internal disk, and `2+` are removable disks in stable name
order. A missing disk or a required missing USB device enters the rescue shell by default.

### Optional guest services

- Set `sound.backend = "pipewire"` to start PipeWire and its PulseAudio compatibility service; QEMU
  uses its supported `pa` audio backend automatically.
- `[display] backend = "sdl"` (the default) renders the guest directly on the appliance's active
  KMS/DRM display, with no host desktop or display manager. `none` is only for a deliberately
  headless VM with another management console.
- Set `network.mode = "macvtap"` and set `network.bridge_iface` to a physical parent interface for a
  direct, dedicated VM network attachment. It is not a host-to-guest management link; keep SSH or
  another management path available.
- Enable `[shared_folder]` to expose a safe host directory over virtiofs. Linux guests mount it with
  `mount -t virtiofs native-qemu-share /mnt/shared`.
- Enable `[docs_server]` only on a guest-facing bridge with an IPv4 address. It serves the bundled
  docs and resolves `light-docs.local`; `dhcp_range` is opt-in to avoid becoming an unexpected DHCP
  authority on an existing network.
- `[[smb_share]]` supports `vm_only` shares bound only to that bridge and `global` shares bound only
  to `[smb].lan_iface`. Every share requires a password file; never put passwords in `config.toml`.

### Windows XP VirtIO test image

`/Users/ido/Downloads/WinXpAgent (1).qcow2` passed a read-only QCOW2 integrity check and is suitable
as the x86_64 test guest. Copy it to the appliance storage selected by `storage = 1`, for example
`vms/WinXpAgent.qcow2`, then copy the bundled
`/etc/native-qemu/examples/winxp-virtio.toml` to `/etc/native-qemu/config.toml` (the same source
profile is also at [`examples/winxp-virtio.toml`](examples/winxp-virtio.toml)).

Windows XP must already have the Red Hat `viostor` (VirtIO block) driver installed to boot from a
VirtIO disk. If it does not, install that driver while the image is still booted through its existing
controller before switching to this profile. The initial physical test should retain a keyboard and
mouse for guest passthrough and keep a separate SSH management path available.

## Contributing / building blocks

- `build/mkimg.native_qemu.sh` — the Alpine `mkimage.sh` profile (package list, kernel flavor, image
  naming) for this appliance.
- `agent/` — the Rust launcher built and copied into the ISO overlay during CI.
- `overlay/` — the default configuration and example lifecycle hooks baked into the ISO.
- `build/genapkovl-native-qemu.sh` — generates the Alpine overlay (agent, configuration, hostname,
  and init configuration) baked into the ISO at boot.
- `.github/workflows/build.yml` — CI: builds both architectures and, on a version tag, publishes a
  GitHub Release with both ISOs attached.
