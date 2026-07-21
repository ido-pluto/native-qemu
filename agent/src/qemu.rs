use crate::config::Config;
use crate::services::{MacvtapRuntime, VirtiofsRuntime};
use crate::usb::UsbEntry;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

pub const QMP_SOCKET: &str = "/run/native-qemu-qmp.sock";

fn binary_for_arch(arch: &str) -> &'static str {
    match arch {
        "aarch64" => "qemu-system-aarch64",
        _ => "qemu-system-x86_64",
    }
}

/// Maps config `display.vga` names to the QEMU `-device` name.
fn qemu_vga_device(vga: &str) -> &str {
    match vga {
        "cirrus" => "cirrus-vga",
        "std" | "VGA" => "VGA",
        "virtio" | "virtio-gpu-pci" => "virtio-gpu-pci",
        other => other,
    }
}

/// Builds the `-smp` value: full topology when sockets/cores/threads are all
/// set, otherwise just the vcpu count.
fn smp_arg(cfg: &Config) -> String {
    let s = cfg.vm.sockets;
    let c = cfg.vm.cores;
    let t = cfg.vm.threads;
    if s > 0 && c > 0 && t > 0 {
        let total = s.saturating_mul(c).saturating_mul(t);
        format!("{total},sockets={s},cores={c},threads={t}")
    } else {
        cfg.vm.vcpus.to_string()
    }
}

/// Ensures a writable per-VM copy of the AAVMF UEFI variable store exists
/// (aarch64's "virt" machine always needs UEFI; the template file is
/// read-only and must not be written to directly or every boot would race
/// on the same shared vars file).
fn ensure_uefi_vars(disk_path: &Path, extension: &str, template: &str) -> std::io::Result<PathBuf> {
    let vars_path = disk_path.with_extension(extension);
    if !vars_path.exists() {
        std::fs::copy(template, &vars_path)?;
    }
    Ok(vars_path)
}

/// Builds the full qemu-system-* argv (excluding argv[0], the binary
/// itself) from the resolved config, disk path, and USB devices to attach.
pub fn build_args(
    cfg: &Config,
    disk_path: &Path,
    usb_devices: &[UsbEntry],
    virtiofs: Option<&VirtiofsRuntime>,
    macvtap: Option<&MacvtapRuntime>,
) -> std::io::Result<Vec<String>> {
    let mut args: Vec<String> = Vec::new();
    let push = |args: &mut Vec<String>, s: &str| args.push(s.to_string());

    // KVM needs hardware virtualization actually exposed to this machine
    // (enabled in firmware, not itself running nested without passthrough).
    // Falling back to software emulation (tcg) when /dev/kvm is absent means
    // the appliance still boots — slowly — instead of hard-failing, which
    // matters both for real machines with virtualization disabled/missing
    // and for testing this agent inside a VM without nested KVM.
    let have_kvm = Path::new("/dev/kvm").exists();
    let accel = if have_kvm { "kvm" } else { "tcg" };
    if !have_kvm {
        eprintln!(
            "native-qemu: warning: /dev/kvm not available — falling back to software \
             emulation (tcg), which is drastically slower. Check that virtualization \
             is enabled in firmware."
        );
    }
    // aarch64 always uses the "virt" machine; x86 uses cfg.vm.machine (pc/q35).
    let machine_type = if cfg.vm.arch == "aarch64" {
        "virt"
    } else {
        cfg.vm.machine.as_str()
    };
    push(&mut args, "-machine");
    let memory_backend = if virtiofs.is_some() {
        ",memory-backend=mem"
    } else {
        ""
    };
    args.push(format!("{machine_type},accel={accel}{memory_backend}"));
    push(&mut args, "-cpu");
    if have_kvm {
        push(&mut args, &cfg.vm.cpu);
    } else {
        // "host" CPU passthrough only makes sense with KVM; substitute a
        // generic TCG-compatible model instead of failing outright.
        // Legacy models like pentium3 remain usable under TCG.
        let tcg_cpu = if cfg.vm.arch == "aarch64" {
            "max"
        } else if cfg.vm.cpu == "host" {
            "qemu64"
        } else {
            cfg.vm.cpu.as_str()
        };
        push(&mut args, tcg_cpu);
    }
    push(&mut args, "-m");
    push(&mut args, &cfg.vm.memory);
    push(&mut args, "-smp");
    args.push(smp_arg(cfg));
    push(&mut args, "-nodefaults");
    push(&mut args, "-no-user-config");
    // This is an appliance, not a host desktop: SDL talks directly to the
    // active KMS/DRM console and renders the *guest* display there.  Keeping
    // a standard VGA device on x86 makes Windows XP usable without needing a
    // guest virtio-gpu driver. Operators who use an alternative remote
    // console can explicitly choose display.backend = "none".
    push(&mut args, "-display");
    if cfg.display.backend == "sdl" {
        push(&mut args, "sdl,gl=off");
        push(&mut args, "-device");
        // aarch64 defaults to virtio-gpu unless the operator explicitly set
        // a non-default display.vga value.
        let vga_device = if cfg.vm.arch == "aarch64" && cfg.display.vga == "VGA" {
            "virtio-gpu-pci"
        } else {
            qemu_vga_device(&cfg.display.vga)
        };
        push(&mut args, vga_device);
    } else {
        push(&mut args, "none");
    }
    push(&mut args, "-no-reboot");

    if cfg.vm.arch == "aarch64" {
        // "virt" has no BIOS option at all — UEFI is mandatory.
        let vars = ensure_uefi_vars(disk_path, "aavmf-vars.fd", "/usr/share/AAVMF/AAVMF_VARS.fd")?;
        push(&mut args, "-drive");
        args.push("if=pflash,format=raw,readonly=on,file=/usr/share/AAVMF/AAVMF_CODE.fd".into());
        push(&mut args, "-drive");
        args.push(format!(
            "if=pflash,format=raw,file={}",
            vars.to_string_lossy()
        ));
    } else if cfg.vm.firmware == "uefi" {
        let vars = ensure_uefi_vars(disk_path, "ovmf-vars.fd", "/usr/share/OVMF/OVMF_VARS.fd")?;
        push(&mut args, "-drive");
        args.push("if=pflash,format=raw,readonly=on,file=/usr/share/OVMF/OVMF_CODE.fd".into());
        push(&mut args, "-drive");
        args.push(format!(
            "if=pflash,format=raw,file={}",
            vars.to_string_lossy()
        ));
    }

    // Disk
    push(&mut args, "-drive");
    args.push(format!(
        "file={},if=none,id=drive0,format={},cache={},discard={}",
        disk_path.to_string_lossy(),
        cfg.vm.disk.format,
        cfg.vm.disk.cache,
        cfg.vm.disk.discard
    ));
    match cfg.vm.disk.bus.as_str() {
        "scsi" => {
            push(&mut args, "-device");
            push(&mut args, "virtio-scsi-pci,id=scsi0");
            push(&mut args, "-device");
            push(&mut args, "scsi-hd,drive=drive0,bus=scsi0.0");
        }
        "ide" => {
            push(&mut args, "-device");
            push(&mut args, "ide-hd,drive=drive0,bus=ide.0");
        }
        _ => {
            push(&mut args, "-device");
            push(&mut args, "virtio-blk-pci,drive=drive0");
        }
    }

    if let Some(shared) = virtiofs {
        // vhost-user-fs requires guest memory backed by a shared memfd.
        push(&mut args, "-object");
        args.push(format!(
            "memory-backend-memfd,id=mem,size={},share=on",
            cfg.vm.memory
        ));
        push(&mut args, "-chardev");
        args.push(format!(
            "socket,id=charfs0,path={}",
            shared.socket.to_string_lossy()
        ));
        push(&mut args, "-device");
        args.push(format!(
            "vhost-user-fs-pci,chardev=charfs0,tag={}",
            shared.guest_tag
        ));
    }

    // Network
    match cfg.network.mode.as_str() {
        "bridge" => {
            let br = cfg.network.bridge_iface.as_deref().unwrap_or("br0");
            push(&mut args, "-netdev");
            args.push(format!("bridge,id=net0,br={br}"));
        }
        "macvtap" => {
            let macvtap =
                macvtap.ok_or_else(|| std::io::Error::other("macvtap runtime was not prepared"))?;
            push(&mut args, "-netdev");
            args.push(format!("tap,id=net0,fd={}", macvtap.raw_fd()));
        }
        _ => {
            push(&mut args, "-netdev");
            push(&mut args, "user,id=net0");
        }
    }
    push(&mut args, "-device");
    args.push(format!("{},netdev=net0", cfg.network.model));

    // Sound
    if cfg.sound.enabled {
        push(&mut args, "-audiodev");
        let audio = if cfg.sound.backend == "pipewire" {
            // Alpine's QEMU 9.0 package exposes PipeWire through its
            // PulseAudio-compatible backend, not a native "pipewire"
            // audiodev. RuntimeServices starts pipewire-pulse at this path.
            "pa,id=snd0,server=/run/native-qemu-pipewire/pulse/native".into()
        } else {
            format!("{},id=snd0", cfg.sound.backend)
        };
        args.push(audio);
        push(&mut args, "-device");
        // sb16 (and most other models) take audiodev= as a property.
        args.push(format!("{},audiodev=snd0", cfg.sound.model));
    }

    // USB controller + passthrough devices
    push(&mut args, "-device");
    push(&mut args, "qemu-xhci,id=usb-bus0");
    for (i, dev) in usb_devices.iter().enumerate() {
        push(&mut args, "-device");
        args.push(format!(
            "usb-host,bus=usb-bus0.0,hostbus={},hostaddr={},id=usbdev{}",
            dev.bus_num, dev.device_num, i
        ));
    }

    // QMP control socket
    push(&mut args, "-qmp");
    args.push(format!("unix:{QMP_SOCKET},server,nowait"));

    Ok(args)
}

/// qemu-bridge-helper (the suid binary that lets an unprivileged qemu
/// attach a tap device to a bridge) refuses to touch any bridge that isn't
/// explicitly allow-listed in /etc/qemu/bridge.conf. Since the bridge name
/// here comes from our own trusted config.toml, allow-listing it
/// automatically is reasonable for a single-purpose appliance.
pub fn ensure_bridge_allowed(bridge: &str) -> std::io::Result<()> {
    if !Path::new("/sys/class/net")
        .join(bridge)
        .join("bridge")
        .is_dir()
    {
        return Err(std::io::Error::other(format!(
            "{bridge} is not an existing Linux bridge"
        )));
    }
    std::fs::create_dir_all("/etc/qemu")?;
    std::fs::write("/etc/qemu/bridge.conf", format!("allow {bridge}\n"))
}

/// Spawns qemu-system-<arch> with the given args, inheriting stdio so its
/// own logs flow into our log file (main.rs redirects our own stdout/stderr
/// there before calling this).
pub fn spawn(
    cfg: &Config,
    args: &[String],
    macvtap: Option<&MacvtapRuntime>,
) -> std::io::Result<Child> {
    let binary = binary_for_arch(&cfg.vm.arch);
    let mut command = Command::new(binary);
    command.args(args).stdin(Stdio::null());
    if cfg.display.backend == "sdl" {
        // The appliance runs on a Linux console without X11 or Wayland. SDL's
        // KMSDRM driver presents the guest straight on the physical display.
        command.env("SDL_VIDEODRIVER", "kmsdrm");
    }
    if let Some(macvtap) = macvtap {
        let fd = macvtap.raw_fd();
        // Files opened by Rust have FD_CLOEXEC set.  QEMU must inherit this
        // single descriptor so its `-netdev tap,fd=N` can use the macvtap.
        unsafe {
            command.pre_exec(move || {
                let flags = libc::fcntl(fd, libc::F_GETFD);
                if flags == -1 || libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    command.spawn()
}

#[cfg(test)]
mod tests {
    use super::build_args;
    use crate::config::Config;
    use std::path::Path;

    #[test]
    fn default_x86_config_exposes_an_xp_compatible_vga_display() {
        let cfg: Config = toml::from_str(include_str!(
            "../../overlay/etc/native-qemu/config.toml.example"
        ))
        .unwrap();
        let args = build_args(&cfg, Path::new("/tmp/xp.qcow2"), &[], None, None).unwrap();
        assert!(args
            .windows(2)
            .any(|pair| pair == ["-display", "sdl,gl=off"]));
        assert!(args.windows(2).any(|pair| pair == ["-device", "VGA"]));
        // Default machine is q35 when not overridden.
        assert!(args.iter().any(|a| a.starts_with("q35,accel=")));
    }

    #[test]
    fn reactos_default_config_builds_legacy_pc_args() {
        let cfg: Config = toml::from_str(include_str!("../../assets/default/config.toml")).unwrap();
        let args = build_args(&cfg, Path::new("/tmp/image.qcow2"), &[], None, None).unwrap();

        assert!(
            args.iter().any(|a| a.starts_with("pc,accel=")),
            "expected -machine pc, got: {args:?}"
        );
        assert!(
            args.windows(2).any(|pair| pair == ["-cpu", "pentium3"]
                || (pair[0] == "-cpu" && pair[1] == "pentium3")),
            "expected -cpu pentium3 (or TCG-safe equivalent), got: {args:?}"
        );
        // Under TCG we still pass through non-host CPU models.
        let cpu_idx = args.iter().position(|a| a == "-cpu").unwrap();
        assert_eq!(args[cpu_idx + 1], "pentium3");

        assert!(args.windows(2).any(|pair| pair == ["-m", "512M"]));
        assert!(args
            .windows(2)
            .any(|pair| pair == ["-smp", "1,sockets=1,cores=1,threads=1"]));
        assert!(args
            .iter()
            .any(|a| a == "rtl8139,netdev=net0" || a.starts_with("rtl8139,")));
        assert!(
            args.windows(2)
                .any(|pair| pair == ["-device", "cirrus-vga"]),
            "expected -device cirrus-vga, got: {args:?}"
        );
        assert!(args
            .windows(2)
            .any(|pair| pair == ["-device", "sb16,audiodev=snd0"]));
        assert!(args
            .windows(2)
            .any(|pair| pair == ["-device", "ide-hd,drive=drive0,bus=ide.0"]));
    }
}
