# native-qemu release checklist

Do not call a release production-ready merely because the Rust checks or the
ISO build pass. The appliance touches boot firmware, storage, host networking,
and guest hardware, so each release needs the following gates.

## Build and artifact gates

- [x] GitHub Actions builds and uploads both `native-qemu-x86_64.iso` and
  `native-qemu-aarch64.iso` from the candidate commit.
- [x] Inspect each ISO artifact: it contains `native-qemu-agent`, the
  configuration example, lifecycle-hook examples, and both static docs pages.
- [x] Verify the x86_64 ISO includes OVMF and the aarch64 ISO includes AAVMF.
- [ ] Confirm the release tag contains the same commit that passed the matrix.

Evidence: GitHub Actions run `29704982314` passed both native builds and
performs the ISO-payload and architecture-specific firmware checks on commit
`44cb13d`. A release tag has not yet been created.

## Emulated firmware smoke gates

- [x] The downloaded x86_64 artifact reaches a running QEMU state using its
  BIOS boot path with the ISO mounted.
- [x] The downloaded x86_64 artifact reaches a running QEMU state using OVMF
  UEFI with the ISO mounted.
- [x] The downloaded aarch64 artifact reaches a running QEMU state using
  AAVMF/EDK2 UEFI with the ISO mounted.

Evidence: artifacts from Actions run `29705107679` were checked on 2026-07-20.
Their SHA-256 digests are x86_64
`9aae7e8c06cc35f4bf67c5c02c341106a222fcd5fab0f5c663658477e79ca617` and
aarch64 `9b18439045dc9e7ae27893703af71289f1bc0b6f450b529277cff940e991d961`.
These are firmware/startup smoke tests only; they do not replace the physical
boot and guest-device gates below.

## Physical boot gates

- [ ] Boot the x86_64 ISO from a USB stick using legacy BIOS/CSM.
- [ ] Boot the x86_64 ISO from a USB stick using UEFI with both
  `firmware = "bios"` and `firmware = "uefi"` guest profiles.
- [ ] Boot the aarch64 ISO from real UEFI hardware.
- [ ] Confirm a fresh stick enters the rescue shell safely when the example
  disk is absent, and that `lbu commit` persists a corrected configuration.

## Guest lifecycle and device gates

- [ ] Start raw and qcow2 guests from each supported storage index; verify a
  normal guest shutdown triggers the configured host lifecycle action.
- [ ] Boot the documented `examples/winxp-virtio.toml` profile on x86_64 with
  its `viostor` driver preinstalled; verify direct SDL/KMSDRM display, guest
  keyboard/mouse input, and a normal XP shutdown.
- [ ] Verify required-USB failure, explicit USB selection, default USB policy,
  duplicate USB IDs, and hotplug on real devices.
- [ ] Verify user networking, an existing Linux bridge, and macvtap on real
  network hardware. Preserve an SSH management route during passthrough.
- [ ] Verify ALSA and PipeWire audio output on physical audio hardware.
- [ ] Verify virtiofs read/write behavior from Linux and a Windows guest with
  the required driver.
- [ ] Verify docs/DNS/DHCP only bind to the configured guest bridge.
- [ ] Verify `vm_only` SMB shares are unreachable from the LAN and `global`
  shares require authentication and only bind to the configured LAN interface.

## Operational gates

- [ ] Verify log rotation, startup/shutdown hook timeouts, crash-loop limits,
  SSH key-only access, and rescue-shell recovery.
- [ ] Confirm the generated log and shared directories are on the intended
  writable storage, never the read-only boot ISO.
- [ ] Record the tested hardware, firmware revision, guest OS, and any device
  compatibility exceptions in the release notes.
