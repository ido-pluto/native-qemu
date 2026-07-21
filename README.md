# native-qemu

A tiny, USB-bootable Linux appliance whose entire job is to boot straight into a QEMU virtual
machine — no desktop, no window manager, no host-side UI. You configure a VM once (which disk image,
which devices get passed through — keyboard, mouse, sound, camera, USB, network), and every boot from
the stick goes: power on → load config → launch that VM with everything passed through → guest shuts
down → the physical machine powers off. A direct hardware-to-VM proxy.

Full architecture, configuration reference, and the phased build roadmap live in [plan.md](plan.md).
Use [RELEASE_CHECKLIST.md](RELEASE_CHECKLIST.md) before publishing a tagged release.

## Status

Active development. The appliance boots into `native-qemu-agent`, which loads `config.toml` from the
USB data volume (`LABEL=native-qemu`) and launches a config-driven QEMU VM (legacy PC profile for
ReactOS / Windows 98 SE class guests by default).

**Product track (current CI):**

| Piece | What ships today |
|---|---|
| Boot ISO | **`native-qemu-x86_64.iso`** (interim **Alpine** image) |
| Host tool | **`nq-disk`** for macOS (Apple Silicon), Linux (x86_64 + aarch64), Windows |
| Custom QEMU | **`qemu-3dfx-x86_64.tar.gz`** — QEMU **9.2.2** + [qemu-3dfx](https://github.com/kjliew/qemu-3dfx) (MESA GL / 3Dfx Glide), **glibc**, built in CI |
| Target arch | **x86_64** appliance only (no aarch64 ISO in product CI) |
| Next base OS | Migrating appliance rootfs to **Void Linux glibc** with this QEMU baked in (`build/void-iso.sh`) |

Guest data always lives on the USB **ext4** volume: `config.toml` + `image.qcow2` (path fixed so you
can swap guests without editing the config).

## Supported hardware

| Artifact | Runs on |
|---|---|
| `native-qemu-x86_64.iso` | x86_64 (Intel/AMD) machines (KVM) |

There is **no cross-architecture emulation**. Flash the stick on any host with `nq-disk`; **boot** it
on an x86_64 PC that matches the ISO.

## Getting a release kit

Tagged releases and workflow artifacts are built by [`.github/workflows/build.yml`](.github/workflows/build.yml).

Each platform zip (e.g. `native-qemu-macos-arm64.zip`, `native-qemu-linux-x86_64.zip`,
`native-qemu-windows-x86_64.zip`) contains:

1. **`nq-disk`** / **`nq-disk.exe`** — interactive flash / config / volume tool  
2. **`native-qemu-x86_64.iso`** — boot ISO for the target PC  
3. **`qemu-3dfx-x86_64.tar.gz`** — prefix tree with `usr/local/bin/qemu-system-x86_64` (and tools)  
4. **`README.txt`** — short flash instructions  

Download from the [Releases](../../releases) page or from a green **Build ISOs** Actions run
(artifact `native-qemu-release-packages`).

### Build yourself

```sh
# Host tool
cargo build -p nq-disk --release

# Interim Alpine x86_64 ISO (containerized mkimage)
./scripts/build-native-qemu-iso.sh x86_64 dist

# Custom QEMU 9.2 + 3dfx (glibc host; long build)
./build/qemu-3dfx.sh --pack   # → dist/qemu-3dfx-x86_64.tar.gz
```

## Writing the stick — use `nq-disk` (required: sudo / Administrator)

**Every `nq-disk` action needs root** (raw disks + ext4 data volume). Without elevation it exits
immediately and prints how to re-run.

```sh
cd /path/to/unzipped-kit   # or next to the .iso
sudo ./nq-disk             # macOS / Linux
# Windows: run nq-disk.exe as Administrator
```

Menu: **Flash ISO** → pick USB (system disks hidden) → **Edit config** → **Load image** → **Unmount**.

Details: [tools/nq-disk/README.md](tools/nq-disk/README.md).

Low-level ISO-only write (no data-volume seed):

```sh
sudo scripts/write-usb.sh --yes-really-write native-qemu-x86_64.iso /dev/rdiskN   # macOS
sudo scripts/write-usb.sh --yes-really-write native-qemu-x86_64.iso /dev/sdX      # Linux
```

## QEMU 9.2 + 3dfx tarball

CI builds QEMU from [download.qemu.org](https://download.qemu.org/) **9.2.2** plus
[kjliew/qemu-3dfx](https://github.com/kjliew/qemu-3dfx) (`00-qemu92x-mesa-glide.patch`). Script:
[`build/qemu-3dfx.sh`](build/qemu-3dfx.sh).

```sh
# Inspect
tar tzf qemu-3dfx-x86_64.tar.gz | head
# Install onto a glibc Linux root (e.g. future Void appliance)
sudo tar -C / -xzf qemu-3dfx-x86_64.tar.gz
# binary at /usr/local/bin/qemu-system-x86_64
```

The interim Alpine ISO still uses **distro QEMU**; baking this tarball into the live image is the
Void migration step (`build/void-iso.sh` — scaffold until Phase 1 lands).

Guest **Glide/OpenGL wrappers** (Windows DLLs from qemu-3dfx) are installed **inside the guest OS**,
not by the host flash tool. See the upstream qemu-3dfx README.

## Booting

1. Plug the stick into the **x86_64** target machine and open the firmware boot menu.  
2. Select the USB (BIOS or UEFI).  
3. `native-qemu-agent` starts; it uses the data volume `config.toml` and `image.qcow2` when present.

## Configuration

Preferred: edit on the host with `sudo ./nq-disk` → **Edit config** (completion, validate, undo).

On the appliance, the agent resolves config from the data volume / boot media / bundled defaults
(see agent docs and [plan.md](plan.md)). Persistence for the stick is the **ext4 data volume**, not
Alpine `lbu` (that path is interim-only).

Default profile targets **legacy PC** guests (e.g. `machine = "pc"`, IDE, Cirrus) suitable for
ReactOS / Win98 SE class systems; always keep the guest disk filename **`image.qcow2`**.

## Development

```sh
cargo test -p nq-disk
cargo test -p native-qemu-agent --locked
```

CI jobs (see `.github/workflows/build.yml`):

- `QEMU 9.2 + 3dfx (x86_64)` — custom QEMU tarball  
- `Build x86_64 ISO (Alpine interim)` — boot ISO  
- `nq-disk (*)` — host helpers  
- `Package release zips` — kits above  

## License

See repository license files and third-party notices for QEMU / qemu-3dfx / Alpine packaging as
applicable.
