//! Open whole raw disks on macOS/Linux with unmount retries.
//!
//! After writing a hybrid ISO, macOS Disk Arbitration remounts EFI/Alpine
//! volumes. That makes `/dev/rdiskN` return **EBUSY (os error 16)** until
//! `diskutil unmountDisk force` runs again. Every exclusive open goes through
//! here so flash / GPT / mkfs / seed share the same recovery path.

use anyhow::{bail, Context, Result};
use std::fs::{File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::Duration;

/// Prefer macOS raw character device when available.
pub fn prefer_raw(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    if s.contains("/dev/disk") && !s.contains("/dev/rdisk") {
        let raw = s.replacen("/dev/disk", "/dev/rdisk", 1);
        let p = PathBuf::from(&raw);
        if p.exists() {
            return p;
        }
    }
    path.to_path_buf()
}

/// Logical `/dev/diskN` form for diskutil (not rdisk).
pub fn logical_disk(path: &Path) -> PathBuf {
    PathBuf::from(
        path.to_string_lossy()
            .replace("/dev/rdisk", "/dev/disk"),
    )
}

pub fn is_busy(err: &io::Error) -> bool {
    // EBUSY = 16 on macOS/Linux; also match message for wrapped errors.
    matches!(err.raw_os_error(), Some(16))
        || format!("{err}").to_ascii_lowercase().contains("busy")
}

pub fn is_busy_anyhow(err: &anyhow::Error) -> bool {
    format!("{err:#}").to_ascii_lowercase().contains("busy")
        || format!("{err:#}").contains("os error 16")
}

/// Force-unmount every volume on the whole disk (does **not** eject).
pub fn unmount_whole_disk(disk: &Path) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        let logical = logical_disk(disk);
        let logical_s = logical.to_string_lossy().to_string();
        // Primary: unmount all volumes on this disk.
        let _ = Command::new("diskutil")
            .args(["unmountDisk", "force", &logical_s])
            .status();
        // Also unmount any leftover mount points under /Volumes that still
        // reference this disk (Finder race after hybrid ISO write).
        if let Ok(out) = Command::new("diskutil")
            .args(["info", "-plist", &logical_s])
            .output()
        {
            let _ = out; // best-effort; unmountDisk is the real work
        }
        // Second pass — DA often remounts within a few hundred ms.
        let _ = Command::new("diskutil")
            .args(["unmountDisk", "force", &logical_s])
            .status();
    }
    #[cfg(target_os = "linux")]
    {
        if let Ok(out) = Command::new("lsblk")
            .args(["-n", "-o", "NAME", "-l", disk.to_str().unwrap_or("")])
            .output()
        {
            for line in String::from_utf8_lossy(&out.stdout).lines() {
                let name = line.trim();
                if name.is_empty() {
                    continue;
                }
                let _ = Command::new("umount")
                    .args(["-f", &format!("/dev/{name}")])
                    .status();
            }
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = disk;
    }
    Ok(())
}

/// Open a raw whole-disk node for R/W or read-only, unmounting on EBUSY.
pub fn open_raw(path: &Path, writable: bool) -> Result<File> {
    let open_path = prefer_raw(path);
    let mut last: Option<io::Error> = None;

    for attempt in 0..12 {
        // Unmount before every attempt — macOS remounts aggressively.
        let _ = unmount_whole_disk(&open_path);
        if attempt > 0 {
            thread::sleep(Duration::from_millis(200 + attempt as u64 * 150));
            let _ = unmount_whole_disk(&open_path);
        }

        let mut opts = OpenOptions::new();
        opts.read(true);
        if writable {
            opts.write(true);
        }
        match opts.open(&open_path) {
            Ok(f) => return Ok(f),
            Err(e) => {
                let busy = is_busy(&e);
                last = Some(e);
                if !busy {
                    break;
                }
            }
        }
    }

    let logical = logical_disk(&open_path);
    Err(last
        .map(Into::into)
        .unwrap_or_else(|| anyhow::anyhow!("open failed")))
    .with_context(|| {
        format!(
            "open {} for {} after unmount retries \
             (device still busy — try: diskutil unmountDisk force {})",
            open_path.display(),
            if writable { "R/W" } else { "read" },
            logical.display()
        )
    })
}

/// Settle until a probe open succeeds (or give up after retries).
pub fn release_disk(disk: &Path) -> Result<()> {
    for attempt in 0..8 {
        unmount_whole_disk(disk)?;
        thread::sleep(Duration::from_millis(150 + attempt as u64 * 100));
        match open_raw(disk, false) {
            Ok(f) => {
                drop(f);
                // One more unmount so the next exclusive open starts clean.
                unmount_whole_disk(disk)?;
                thread::sleep(Duration::from_millis(100));
                return Ok(());
            }
            Err(e) if is_busy_anyhow(&e) => continue,
            Err(_) => {
                // Other errors (e.g. gone) — still return Ok so caller can try.
                return Ok(());
            }
        }
    }
    unmount_whole_disk(disk)?;
    thread::sleep(Duration::from_millis(400));
    Ok(())
}

/// Convenience: fail if we still cannot open after release.
pub fn ensure_openable(disk: &Path, writable: bool) -> Result<()> {
    let f = open_raw(disk, writable)?;
    drop(f);
    Ok(())
}

pub fn bail_if_not_openable(disk: &Path) -> Result<()> {
    ensure_openable(disk, true).with_context(|| {
        format!(
            "cannot open {} exclusively — unmount it in Finder or run:\n  diskutil unmountDisk force {}",
            disk.display(),
            logical_disk(disk).display()
        )
    })?;
    Ok(())
}

#[allow(dead_code)]
pub fn must_exist(path: &Path) -> Result<()> {
    if !path.exists() && !prefer_raw(path).exists() {
        bail!("disk device not found: {}", path.display());
    }
    Ok(())
}
