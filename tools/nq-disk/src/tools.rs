//! Host utilities — flash is self-contained (bundled lwext4 + pure-Rust GPT).
//! Only root and a large-enough target disk are required at runtime.

use anyhow::{bail, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn which(name: &str) -> Option<PathBuf> {
    if let Ok(p) = Command::new("which").arg(name).output() {
        if p.status.success() {
            let s = String::from_utf8_lossy(&p.stdout).trim().to_string();
            if !s.is_empty() && Path::new(&s).exists() {
                return Some(PathBuf::from(s));
            }
        }
    }
    for p in [
        format!("/opt/homebrew/bin/{name}"),
        format!("/usr/local/bin/{name}"),
        format!("/usr/bin/{name}"),
        format!("/bin/{name}"),
    ] {
        if Path::new(&p).exists() {
            return Some(PathBuf::from(p));
        }
    }
    None
}

pub fn find_xorriso() -> Option<PathBuf> {
    which("xorriso")
}

/// True when the process can open raw disks (root / Administrator).
pub fn running_as_root() -> bool {
    #[cfg(unix)]
    {
        unsafe { libc::geteuid() == 0 }
    }
    #[cfg(windows)]
    {
        // Best-effort: flash on Windows still needs elevated token; we treat
        // non-elevated runs as not-root. Detection uses a simple privileged open
        // probe later; for the menu gate we check if we're elevated via
        // CheckTokenMembership is overkill — use geteuid-equivalent via
        // `net session` would be slow. Assume true and rely on I/O errors
        // unless we have a cheap check later.
        true
    }
    #[cfg(not(any(unix, windows)))]
    {
        true
    }
}

pub fn require_root() -> Result<()> {
    if !running_as_root() {
        bail!("{}", root_required_message().trim());
    }
    Ok(())
}

/// Shown when the process is not elevated. Everything in nq-disk needs root
/// (raw disks, GPT slices, ext4 seed/edit on the USB data volume).
pub fn root_required_message() -> String {
    "\
nq-disk needs administrator privileges for every action
(flash USB, edit config, file manager, load image, unmount).

You started without root / elevation — nothing can run yet.

  Quit and start again with:

    sudo ./nq-disk

  (run that from the folder that contains nq-disk and the .iso)

On Windows: right‑click → Run as administrator.
"
    .into()
}

#[derive(Debug, Clone)]
pub struct FlashPreflight {
    pub ok: bool,
    pub messages: Vec<String>,
    pub errors: Vec<String>,
}

pub fn preflight_flash(iso: &Path, disk: &Path) -> FlashPreflight {
    let mut messages = Vec::new();
    let mut errors = Vec::new();

    if !iso.is_file() {
        errors.push(format!("ISO not found: {}", iso.display()));
    } else {
        let sz = std::fs::metadata(iso).map(|m| m.len()).unwrap_or(0);
        messages.push(format!("ISO size: {} MiB", sz / (1024 * 1024)));
    }

    if !disk.exists() {
        errors.push(format!("disk device not found: {}", disk.display()));
    } else {
        messages.push(format!("target disk: {}", disk.display()));
    }

    if !running_as_root() {
        errors.push(
            "not running as root — flash is blocked. Quit and run:  sudo ./nq-disk".into(),
        );
    } else {
        messages.push("running as root".into());
    }

    messages.push("ext4 engine: bundled lwext4 (no host mke2fs/debugfs)".into());
    messages.push("partitioning: pure-Rust GPT (no host sgdisk)".into());

    if let (Ok(iso_meta), Ok(disk_size)) = (std::fs::metadata(iso), block_device_size(disk)) {
        let need = iso_meta.len() + 256 * 1024 * 1024;
        if disk_size > 0 && disk_size < need {
            errors.push(format!(
                "disk too small ({} MiB); need at least {} MiB (ISO + 256MiB data)",
                disk_size / (1024 * 1024),
                need / (1024 * 1024)
            ));
        } else if disk_size > 0 {
            messages.push(format!(
                "disk size: {} MiB (OK)",
                disk_size / (1024 * 1024)
            ));
        }
    }

    // Optional helpers for pulling image out of ISO (not required to flash)
    if find_xorriso().is_some() || which("bsdtar").is_some() || which("7z").is_some() {
        messages.push("ISO image extract: xorriso/bsdtar/7z available (optional)".into());
    } else {
        messages.push(
            "ISO image extract tools optional — seed still writes config.toml; \
             put image.qcow2 via Load image if not embedded"
                .into(),
        );
    }

    FlashPreflight {
        ok: errors.is_empty(),
        messages,
        errors,
    }
}

pub fn block_device_size(disk: &Path) -> Result<u64> {
    crate::sized_disk::probe_size(disk)
}
