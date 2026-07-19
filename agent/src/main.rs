mod config;
mod hooks;
mod logging;
mod qemu;
mod qmp;
mod services;
mod storage;
mod usb;

use signal_hook::consts::{SIGINT, SIGTERM};
use signal_hook::iterator::Signals;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex};

const CONFIG_PATH: &str = "/etc/native-qemu/config.toml";
const CONFIG_EXAMPLE_PATH: &str = "/etc/native-qemu/config.toml.example";

enum VmResult {
    CleanShutdown,
    Crashed(String),
}

fn main() {
    if !Path::new(CONFIG_PATH).exists() && Path::new(CONFIG_EXAMPLE_PATH).exists() {
        let _ = std::fs::copy(CONFIG_EXAMPLE_PATH, CONFIG_PATH);
    }

    let cfg = match config::load(Path::new(CONFIG_PATH)) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("native-qemu: fatal: could not load {CONFIG_PATH}: {e}");
            drop_to_shell(&format!("failed to load config: {e}"));
            return;
        }
    };

    if cfg.version != 1 {
        drop_to_shell(&format!(
            "unsupported config version {} (this agent supports version = 1)",
            cfg.version
        ));
        return;
    }
    if let Err(error) = config::validate(&cfg) {
        drop_to_shell(&format!("invalid config.toml: {error}"));
        return;
    }

    let log_path = logging::init(&cfg.logging);
    println!(
        "native-qemu-agent starting (arch={}), logging to {log_path:?}",
        cfg.vm.arch
    );

    if cfg.vm.arch != std::env::consts::ARCH {
        drop_to_shell(&format!(
            "config.toml vm.arch=\"{}\" does not match this host's architecture (\"{}\") — \
             native-qemu does not cross-arch emulate",
            cfg.vm.arch,
            std::env::consts::ARCH
        ));
        return;
    }
    if let hooks::HookOutcome::Abort(msg) = hooks::run(&cfg.startup, "startup") {
        drop_to_shell(&msg);
        return;
    }

    if cfg.system.ssh_enabled {
        start_ssh(&cfg);
    }

    let mut attempts: u32 = 0;
    loop {
        match run_one_vm_lifecycle(&cfg) {
            Ok(VmResult::CleanShutdown) => match cfg.lifecycle.on_guest_shutdown.as_str() {
                "restart_vm" => {
                    attempts = 0;
                    continue;
                }
                "drop_to_shell" => {
                    drop_to_shell("guest shut down (lifecycle.on_guest_shutdown = drop_to_shell)");
                    break;
                }
                _ => {
                    println!("native-qemu: guest shut down cleanly, powering off host");
                    poweroff();
                    break;
                }
            },
            Ok(VmResult::Crashed(reason)) => {
                attempts += 1;
                eprintln!("native-qemu: VM exited abnormally (attempt {attempts}): {reason}");
                if attempts > cfg.lifecycle.max_restart_attempts {
                    drop_to_shell(&format!(
                        "VM crashed {attempts} times, exceeding max_restart_attempts={}",
                        cfg.lifecycle.max_restart_attempts
                    ));
                    break;
                }
                match cfg.lifecycle.on_guest_crash.as_str() {
                    "poweroff_host" => {
                        poweroff();
                        break;
                    }
                    "restart_vm" => continue,
                    _ => {
                        drop_to_shell(&reason);
                        break;
                    }
                }
            }
            Err(fatal) => {
                drop_to_shell(&fatal);
                break;
            }
        }
    }
}

fn run_one_vm_lifecycle(cfg: &config::Config) -> Result<VmResult, String> {
    let missing = usb::missing_required(&cfg.usb);
    if !missing.is_empty() {
        let msg = format!("required USB device(s) not present: {}", missing.join(", "));
        if cfg.lifecycle.on_missing_resource == "rescue_shell" {
            return Err(msg);
        }
        eprintln!("native-qemu: warning: {msg} (continuing, on_missing_resource=boot_anyway)");
    }

    let storage_dir = storage::resolve(cfg.vm.disk.storage).map_err(|e| {
        format!(
            "could not resolve vm.disk.storage={}: {e}",
            cfg.vm.disk.storage
        )
    })?;
    let disk_path = storage_dir.join(&cfg.vm.disk.path);
    if !disk_path.exists() {
        return Err(format!(
            "configured disk does not exist: {}",
            disk_path.display()
        ));
    }

    let usb_devices = usb::resolve(&cfg.usb);
    println!(
        "native-qemu: launching VM, disk={}, {} USB device(s) attached",
        disk_path.display(),
        usb_devices.len()
    );

    if cfg.network.mode == "bridge" {
        let bridge = cfg.network.bridge_iface.as_deref().unwrap_or("br0");
        qemu::ensure_bridge_allowed(bridge)
            .map_err(|e| format!("could not prepare network bridge {bridge}: {e}"))?;
    }

    let services = services::RuntimeServices::start(cfg)
        .map_err(|e| format!("could not start host services: {e}"))?;
    let args = qemu::build_args(
        cfg,
        &disk_path,
        &usb_devices,
        services.virtiofs(),
        services.macvtap(),
    )
    .map_err(|e| format!("failed to prepare qemu arguments: {e}"))?;
    println!(
        "native-qemu: qemu-system-{} {}",
        cfg.vm.arch,
        args.join(" ")
    );

    let mut child = qemu::spawn(cfg, &args, services.macvtap())
        .map_err(|e| format!("failed to spawn qemu: {e}"))?;

    let qmp_handle: Arc<Mutex<Option<qmp::Qmp>>> = Arc::new(Mutex::new(None));
    {
        let qmp_handle = qmp_handle.clone();
        std::thread::spawn(move || {
            // qemu needs a moment to create the QMP socket after spawn.
            for _ in 0..50 {
                if let Ok(q) = qmp::Qmp::connect(qemu::QMP_SOCKET) {
                    *qmp_handle.lock().unwrap() = Some(q);
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            eprintln!("native-qemu: warning: could not connect to QMP socket");
        });
    }

    if let Ok(mut signals) = Signals::new([SIGTERM, SIGINT]) {
        let qmp_handle = qmp_handle.clone();
        std::thread::spawn(move || {
            if signals.forever().next().is_some() {
                println!(
                    "native-qemu: received shutdown signal, requesting graceful guest shutdown"
                );
                if let Some(qmp) = qmp_handle.lock().unwrap().as_mut() {
                    let _ = qmp.system_powerdown();
                }
            }
        });
    }

    if cfg.usb.hotplug {
        spawn_hotplug_watcher(cfg.usb.clone(), qmp_handle.clone(), usb_devices.clone());
    }

    let status = child
        .wait()
        .map_err(|e| format!("error waiting for qemu: {e}"))?;

    if let hooks::HookOutcome::Abort(msg) = hooks::run(&cfg.shutdown, "shutdown") {
        return Err(msg);
    }

    if status.success() {
        Ok(VmResult::CleanShutdown)
    } else {
        Ok(VmResult::Crashed(format!("qemu exited with {status}")))
    }
}

/// Polls for newly plugged-in USB devices matching the passthrough policy
/// and hot-adds them to the running guest over QMP. Polling (not netlink
/// uevents) keeps this simple; a couple of seconds of latency is an
/// acceptable trade-off for v1.
fn spawn_hotplug_watcher(
    usb_cfg: config::UsbConfig,
    qmp_handle: Arc<Mutex<Option<qmp::Qmp>>>,
    initial: Vec<usb::UsbEntry>,
) {
    std::thread::spawn(move || {
        let mut attached: std::collections::HashSet<usb::UsbEntry> = initial.into_iter().collect();
        let mut next_id = attached.len();
        loop {
            std::thread::sleep(std::time::Duration::from_secs(2));
            let current = usb::resolve(&usb_cfg);
            for dev in &current {
                if attached.contains(dev) {
                    continue;
                }
                if let Some(qmp) = qmp_handle.lock().unwrap().as_mut() {
                    let id = format!("usbhotplug{next_id}");
                    next_id += 1;
                    match qmp.device_add("usb-host", dev.bus_num, dev.device_num, "usb-bus0.0", &id)
                    {
                        Ok(()) => {
                            println!(
                                "native-qemu: hotplugged USB device {}:{}",
                                dev.vendor_id, dev.product_id
                            );
                            attached.insert(dev.clone());
                        }
                        Err(e) => eprintln!("native-qemu: hotplug device_add failed: {e}"),
                    }
                }
            }
        }
    });
}

fn start_ssh(cfg: &config::Config) {
    if let Some(key) = &cfg.system.ssh_authorized_key {
        let _ = std::fs::create_dir_all("/root/.ssh");
        let _ = std::fs::write("/root/.ssh/authorized_keys", format!("{key}\n"));
        let _ = Command::new("chmod").args(["700", "/root/.ssh"]).status();
        let _ = Command::new("chmod")
            .args(["600", "/root/.ssh/authorized_keys"])
            .status();
    }
    match Command::new("/usr/sbin/dropbear")
        .args(["-R", "-p", "22"])
        .spawn()
    {
        Ok(_) => println!("native-qemu: dropbear sshd started"),
        Err(e) => eprintln!("native-qemu: warning: could not start dropbear: {e}"),
    }
}

fn poweroff() {
    println!("native-qemu: powering off host");
    let _ = Command::new("poweroff").arg("-f").status();
}

/// Prints `reason` and replaces this process with an interactive shell on
/// whatever tty we're attached to. Since inittab respawns this binary on
/// tty1, exiting that shell (`exit`) causes init to relaunch the agent,
/// matching Alpine's own "type exit to continue boot" convention.
fn drop_to_shell(reason: &str) {
    eprintln!("native-qemu: {reason}");
    eprintln!("native-qemu: dropping to a rescue shell — type 'exit' to retry");
    let err = Command::new("/bin/sh").exec();
    eprintln!("native-qemu: could not exec /bin/sh: {err}");
}
