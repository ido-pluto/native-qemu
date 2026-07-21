# native-qemu — Plan

A tiny, USB-bootable Linux appliance whose entire job is: boot, load a config file, launch QEMU with a
specific VM (raw `.img` or `.qcow2`) with configured device passthrough (USB, sound, network, camera,
keyboard/mouse), and when the guest OS shuts down, power off the physical machine. No desktop, no
window manager, no interactive host UI — a direct proxy between hardware and a single VM.

## Goals

- Boot from a USB stick on real hardware (BIOS + UEFI) in seconds.
- Load a single human-editable config file describing the VM and its devices.
- Pass through keyboard, mouse, camera, sound, network, and arbitrary USB devices into the guest.
- Guest shutdown → host `poweroff`. This is an appliance, not a hypervisor management box.
- Lightweight: minimal package set, minimal moving parts, no unnecessary daemons.
- Support both x86_64 and aarch64.
- Persistent logs, a tiny in-guest docs site, and a safe shared folder for moving files between
  guest and host.

## Non-goals / constraints

- **No cross-architecture emulation.** The x86_64 build only runs (via KVM) on x86_64 hosts; the
  aarch64 build only runs on aarch64 hosts. TCG software emulation across architectures is far too
  slow for "lightweight, direct passthrough" and is explicitly out of scope.
- **No GUI, no display manager, no desktop environment on the host.** The only host-side surfaces
  are: serial/console boot log, the docs webserver, and SSH.
- Once a keyboard/mouse is passed through via `usb-host`, **the host has zero input devices left.**
  All host administration after that point happens over the network (SSH) or via the physical power
  button. This is a deliberate tradeoff, not an oversight — see "Admin / escape hatch" below.

## Key decisions made

| Decision | Choice | Why |
|---|---|---|
| Base distro | **Void Linux glibc x86_64** live ISO (`build/void-iso.sh` + void-mklive) | glibc + Mesa for **QEMU 9.2 + [qemu-3dfx](https://github.com/kjliew/qemu-3dfx)**; hybrid USB. **aarch64 ISO out of product scope.** |
| Custom QEMU | Built on Void in CI (`build/qemu-3dfx.sh`), **baked into ISO** at `/usr/local/bin/qemu-system-x86_64` | Not distro modular QEMU; tarball also attached for rebuilds. |
| Host USB tool | **`nq-disk`** (multi-platform) | Flash ISO + GPT/ext4 data volume + config editor; **requires sudo**. |
| USB passthrough mechanism | **QEMU `usb-host`** (libusb, by vendor:product ID) | Flexible, hot-pluggable, no IOMMU/VT-d requirement — unlike VFIO PCI passthrough of a whole controller, which is rigid and hardware-dependent. |
| VM disk location | **Configurable**, selected by a numbered storage index in `config.toml` | `0` = boot USB, `1` = internal (first fixed) disk, `2..N` = external disks in stable order. Avoids hardcoding where images live. |
| Unknown/unlisted USB devices | **Default to passthrough** | Matches "just connect everything to the VM" — explicit device entries become optional overrides (naming, `required` pinning), with an `exclude` denylist for the rare device that must stay on the host. |
| Host access to VM's own files while running | **virtiofs shared folder**, not a live/NBD mount of the disk image | Bidirectional and safe. Reaching into a `.qcow2`/`.img` that QEMU has open for writing (live or read-only) risks stale/inconsistent data or corruption; virtiofs sidesteps that entirely by being a separate real filesystem shared into the guest. |

## Architecture overview

```
 USB stick (Alpine, overlay/diskless boot)
   │
   ├─ GRUB (UEFI, both arches) / isolinux (x86_64 BIOS)
   ├─ Alpine initramfs → squashfs rootfs + writable overlay (persists via `lbu commit`)
   └─ `native-qemu-agent` on tty1 (launched by inittab after OpenRC; QEMU's
      SDL/KMSDRM backend renders the guest directly on the physical display):
        1. Run startup.sh hook (blocking/non-blocking per config)
        2. Enumerate block devices → resolve config's storage indices to real paths
        3. Parse /etc/native-qemu/config.toml
        4. Resolve configured USB devices to live bus:port; apply default policy to the rest
        5. Start dnsmasq (DHCP+DNS) on the guest-facing bridge, with a static
           light-docs.local → bridge-IP record
        6. Start busybox httpd serving /etc/native-qemu/docs on that bridge only
        7. Start virtiofsd for the shared folder
        8. Build and exec qemu-system-{x86_64,aarch64} (KVM, virtio-blk/scsi,
           virtio-net, virtio-sound, VGA/virtio-gpu, virtiofs, usb-host per resolved device)
        9. On QEMU exit: run shutdown.sh hook, then act per [lifecycle] policy
           (poweroff host / restart VM / drop to rescue shell)
```

## Full `config.toml` reference

```toml
version = 1                      # config schema version, for future migrations

[vm]
arch     = "x86_64"               # x86_64 | aarch64 — must match the host it boots on
firmware = "uefi"                 # uefi | bios (x86_64 only; aarch64 is uefi-only)
memory   = "8G"
vcpus    = 4
cpu      = "host"

[vm.disk]
format  = "qcow2"                 # raw | qcow2
storage = 2                       # 0=boot USB, 1=internal, 2=external#1, 3=external#2, ...
path    = "vms/main.qcow2"        # path inside the resolved storage's mountpoint
bus     = "virtio"                # virtio | scsi | ide
cache   = "none"
discard = "unmap"

[network]
mode         = "bridge"           # user | bridge | macvtap
bridge_iface = "br0"              # existing host Linux bridge connected to the physical NIC
model        = "virtio-net-pci"

[sound]
enabled = true
backend = "pipewire"              # pipewire | alsa
model   = "virtio-sound"

[display]
backend     = "sdl"                # sdl = direct guest output on host KMS/DRM | none = headless
# passthrough = "both"             # none|glide|mesa|both — qemu-3dfx on machine=pc (Win98 default)

[usb]
default = "passthrough"           # passthrough | host-only — policy for devices NOT listed below
exclude = ["0781:5581"]           # vendor:product always kept on the host, even under "passthrough"
hotplug = true                    # device plugged in after VM start is attached live (monitor device_add)

[[usb.device]]                    # optional: naming / pinning specific devices
name       = "keyboard"
vendor_id  = "046d"
product_id = "c31c"
required   = true                 # boot aborts to rescue shell if missing

[[usb.device]]
name       = "mouse"
vendor_id  = "046d"
product_id = "c08b"

[[usb.device]]
name       = "webcam"
vendor_id  = "1bcf"
product_id = "2b98"

[startup]
script     = "/etc/native-qemu/startup.sh"
blocking   = true                 # true: wait (or timeout) before launching QEMU; false: fire-and-forget
timeout    = "30s"
on_failure = "continue"           # continue | abort_to_rescue

[shutdown]
script  = "/etc/native-qemu/shutdown.sh"   # runs after guest exits, before poweroff
timeout = "15s"

[lifecycle]
on_guest_shutdown    = "poweroff_host"  # poweroff_host | restart_vm | drop_to_shell
on_guest_crash       = "drop_to_shell"
on_missing_resource  = "rescue_shell"   # rescue_shell | boot_anyway — missing disk/required USB device
max_restart_attempts = 3                # crash-loop protection before parking in rescue shell

[logging]
enabled  = true
storage  = 1                      # same numbering scheme as vm.disk.storage
path     = "native-qemu/logs"
max_size = "50M"
rotate   = 5

[docs_server]
enabled    = true
domain     = "light-docs.local"
port       = 80
bind_iface = "br0"                # only reachable from the guest-facing bridge, never the internet
docs_dir   = "/etc/native-qemu/docs"   # static HTML4, bundled but user-editable

[shared_folder]
enabled   = true
storage   = 1
host_path = "native-qemu/shared"      # a real directory on the chosen storage
guest_tag = "native-qemu-share"       # guest mounts with: mount -t virtiofs native-qemu-share /mnt/shared

[[smb_share]]                     # any number of independent SMB exports, each separately scoped
name          = "docs"
enabled       = true
storage       = 1
host_path     = "native-qemu/shared"  # can point at the same directory as [shared_folder], or a different one
share_name    = "native-qemu-docs"
scope         = "vm_only"         # vm_only | global (see below)
username      = "vmuser"
password_file = "/etc/native-qemu/smb-docs.secret"   # never store the password in config.toml itself
read_only     = false

[[smb_share]]
name          = "media"
enabled       = true
storage       = 2
host_path     = "external/media"
share_name    = "native-qemu-media"
scope         = "global"
username      = "vmuser"
password_file = "/etc/native-qemu/smb-media.secret"
read_only     = true

[smb]
lan_iface = "eth0"                # physical host NIC used for shares with scope = "global"

[system]
hostname           = "native-qemu"
timezone           = "auto"       # auto | America/Chicago (Texas) | IANA; applied on host before QEMU
rtc_base           = "localtime"  # localtime (Win9x) | utc
ssh_enabled        = true
ssh_authorized_key = "ssh-ed25519 AAAA..."
```

## Device passthrough details

- **Keyboard / mouse**: `usb-host` by vendor:product ID (or matched via the `default = "passthrough"`
  policy). Exclusive to the guest once attached — see "Admin / escape hatch" below.
- **Camera**: always a USB device in practice → same `usb-host` mechanism, no special case needed.
- **USB audio interfaces**: same `usb-host` mechanism.
- **Onboard sound**: `virtio-sound` device in the guest, backed by an `audiodev` pointing at the
  host's real audio stack (`pipewire` or `alsa`) — the guest gets a virtio sound card, the host
  routes it to physical hardware.
- **Network**: `virtio-net-pci` in the guest. Bridge mode uses an existing host Linux bridge
  (`bridge_iface`, for example `br0`) which the operator connects to a physical NIC; `macvtap` and
  plain `user`-mode (SLIRP, no bridging needed) are also selectable for
  simpler/no-privilege setups.
- **Unknown/unlisted USB devices**: passed through automatically under the default policy, so
  plugging in something not in the config still reaches the guest — matches "just connect other
  devices to the VM."

## Hardware / driver support

Broad out-of-the-box driver support (touchscreens, SD/MMC readers, HDMI/DisplayPort, and most other
peripherals) is mostly free, not something maintained as a hand-picked package list:

- The `lts` kernel flavor (not `virt`) ships virtually the entire in-tree Linux driver set as loadable
  modules in the "modloop" — a squashfs baked onto the ISO — because this appliance boots on arbitrary
  physical hardware, not just VMs.
- Every kernel build automatically pulls in `linux-firmware`, covering the firmware blobs most
  GPU/wifi/etc. drivers need.
- `mdev` + `hwdrivers` + `modloop` run as default sysinit services, so connected hardware is
  auto-detected and the matching module auto-loaded with zero config. (These are only enabled when the
  apkovl includes an `etc/.default_boot_services` marker — easy to silently lose if the overlay
  generator changes, since Alpine otherwise skips its whole default-services block whenever any apkovl
  is present at all.)

USB-attached peripherals (most touchscreens, external SD readers, HDMI capture dongles) additionally
get automatic VM passthrough for free via the `[usb] default = "passthrough"` policy — nothing extra to
build. Built-in, non-USB hardware — an internal I2C touchscreen, a soldered PCIe SD controller, or the
machine's own GPU for real HDMI/DisplayPort output — needs VFIO PCI passthrough of the whole controller
instead, which is architecturally different from `usb-host` and remains a future extension (see below),
not something more driver packages would fix.

`build/mkimg.native_qemu.sh` has a `hardware_packages` variable as the extension point for baking in
anything beyond this default coverage — add a package name per line.

## Admin / escape hatch

Once the keyboard and mouse are hard-assigned to the guest, the host has no local input. This is
solved by, in order of use:

1. **SSH into the host** (`[system] ssh_enabled` + `ssh_authorized_key`) — the primary way to check
   logs, edit `config.toml`, or issue a manual `poweroff`.
2. **`light-docs.local`** — a static docs site (served from the guest-facing bridge only, never
   exposed to the internet) explaining exactly this: how to SSH into the host from inside the guest,
   where the shared folder lives, where logs are, how to edit the config for next boot.
3. **Physical power button** (ACPI event) — always works as a hard "shut everything down now",
   independent of guest state.

## Shared folder (virtiofs) and SMB shares

**virtiofs** (`[shared_folder]`): the host runs `virtiofsd` pointed at `host_path`; QEMU is given a
`vhost-user-fs-pci` device plus a `memory-backend-memfd` shared-memory backend (required for
virtiofs). Inside the guest, `mount -t virtiofs native-qemu-share /mnt/shared` gives bidirectional,
always-safe read/write access — files dropped on either side appear on the other immediately, without
ever touching the live disk image directly. Requires a virtio-fs guest driver (built into Linux;
Windows needs the `virtio-win` driver package installed). This remains the single primary shared
folder between the host and its one VM.

**SMB shares** (`[[smb_share]]`, repeatable): any number of independent SMB exports, each with its
own `host_path`/storage and its own `scope`:

- `scope = "vm_only"` — bound only to the guest-facing bridge, same trust boundary as virtiofs and
  the docs server. Good for a guest OS (e.g. Windows) where mounting a network drive is easier than
  installing a virtio-fs driver.
- `scope = "global"` — also bound on the host's real LAN NIC (`[smb].lan_iface`), so other devices on
  the network (phone, laptop, other machines) can mount it directly, not just the guest. Bigger
  attack surface, so this tier always requires authentication (never anonymous/guest SMB) and should
  be paired with a firewall rule scoping which hosts can connect.

Every share (either scope) needs its own `username`/`password_file` — passwords are never stored in
`config.toml` itself. Prefer the Linux kernel's in-tree `ksmbd` server (minimal userspace footprint,
matches the "lightweight" goal) if Alpine's kernel has `CONFIG_SMB_SERVER` enabled; fall back to
`samba`'s `smbd` (heavier but universally available) if not — needs a quick check during Phase 7
implementation. A share can point at the same directory as `[shared_folder]` (dual access: virtiofs
for the primary guest, SMB for occasional access from elsewhere) or at an entirely different one.

## Typical day-to-day flow

1. Boot the stick → `startup.sh` runs (bring up VPN, wait for network, mount a network share, sync
   time, whatever's needed) → QEMU launches the configured `.img`/`.qcow2` with all configured
   devices attached.
2. Drop files into `native-qemu-share` from either the guest or the host — they show up on both sides.
3. From inside the guest, SSH to the host to check `/var/log/native-qemu/` or tweak `config.toml` for
   next boot.
4. Guest browser → `light-docs.local` for a refresher on how any of this works.
5. Shut down the guest OS normally → `shutdown.sh` runs → physical machine powers off.

## Repo layout

```
native-qemu/
  agent/                          # Rust launcher, config preflight, QMP, services and tests
  build/
    mkimg.native_qemu.sh            # Alpine mkimg profile (x86_64 + aarch64 variants)
    genapkovl-native-qemu.sh        # generates the apkovl baked into the ISO
  overlay/
    etc/
      native-qemu/
        config.toml.example         # default appliance configuration
        examples/winxp-virtio.toml  # bundled x86_64 Windows XP VirtIO test profile
        docs/                       # guest-bridge static documentation
        startup.sh.example         # optional lifecycle hook
        shutdown.sh.example        # optional lifecycle hook
  .github/workflows/build.yml    # matrix build of both ISOs
  scripts/write-usb.sh            # guarded raw writer with post-write verification
  README.md                       # USB-writing and appliance-use instructions
  RELEASE_CHECKLIST.md            # CI, emulated, and physical release gates
```

## Build tooling

- Two Alpine `mkimg` profiles (`native-qemu-x86_64`, `native-qemu-aarch64`) built from the same overlay
  source, differing only in package architecture and arch-specific QEMU flags (`-machine q35` vs
  `-machine virt`).
- Both ISOs are hybrid (BIOS+UEFI bootable, dd-able straight to a USB stick); aarch64 targets UEFI
  only (matches real aarch64 hardware/firmware norms).
- Optional GitHub Actions matrix build producing both ISOs as release artifacts.

## Phased roadmap

| Phase | Goal |
|---|---|
| 1 | **Implemented:** bootable Alpine ISO and rescue-shell fallback; real-hardware validation remains a release gate. |
| 2 | **Implemented:** QEMU launch with virtio disk/network and guest-exit lifecycle handling; requires real-hardware guest validation. |
| 3 | **Implemented:** storage enumeration and config-driven VM/disk settings. |
| 4 | **Implemented:** default/explicit USB passthrough, optional hotplug, and Dropbear SSH escape hatch. |
| 5 | **Implemented:** ALSA or PipeWire sound and user/bridge/macvtap networking. Real-hardware networking/audio validation remains a release gate. |
| 6 | **Implemented:** startup/shutdown hooks, bounded persistent log rotation, crash-loop protection, and missing-resource policy. |
| 7 | **Implemented:** bridge-bound docs/DNS service, virtiofs shared folder, and scoped authenticated Samba shares. Real guest interoperability testing remains a release gate. |
| 8 | **Build support implemented:** aarch64 ISO with AAVMF/UEFI and `virt` machine, plus x86_64 BIOS/OVMF UEFI selection; real arm64 hardware testing remains. |
| 9 | **Implemented in CI:** rescue fallback, bundled docs, guarded USB-writing helper, direct physical guest display, a two-architecture matrix build, and ISO-payload verification exist. Physical-media boot and full guest-device validation remain release gates. |

## Future extensions (not in scope for v1)

- VFIO PCI passthrough (e.g. for a dedicated GPU) as an optional advanced mode alongside `usb-host`.
- Multiple VM profiles selectable at boot (currently one active config per stick).
- Remote syslog destination for logging (currently local file only).
- `qemu-guest-agent` integration in the guest for cleaner shutdown/crash detection signaling.
