//! Flash hybrid ISO to a whole disk, create ext4 data partition, seed files.
//!
//! Partitioning: pure-Rust GPT (`gpt` crate).
//! Filesystem: bundled lwext4 (no host mke2fs/debugfs).

use crate::partition;
use crate::rawdisk;
use crate::tools::{self, find_xorriso, preflight_flash, require_root};
use crate::volume;
use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct DiskInfo {
    pub path: PathBuf,
    /// Logical path for display (diskN not rdiskN on macOS).
    pub display_path: PathBuf,
    pub size_bytes: u64,
    pub model: String,
    pub is_system: bool,
    pub is_external: bool,
}

/// List whole disks; mark system disk so UI can hide/exclude it.
pub fn list_disks() -> Result<Vec<DiskInfo>> {
    #[cfg(target_os = "macos")]
    {
        return list_disks_macos();
    }
    #[cfg(target_os = "linux")]
    {
        return list_disks_linux();
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        bail!("disk listing not implemented on this platform yet");
    }
}

#[cfg(target_os = "macos")]
fn list_disks_macos() -> Result<Vec<DiskInfo>> {
    let list = Command::new("diskutil").arg("list").output()?;
    let text = String::from_utf8_lossy(&list.stdout);
    let system = system_disk_macos();
    let mut disks = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for line in text.lines() {
        let line = line.trim();
        // "/dev/disk4 (external, physical):" or "/dev/disk0 (internal, physical):"
        if !line.starts_with("/dev/disk") {
            continue;
        }
        let num: String = line
            .trim_start_matches("/dev/disk")
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        if num.is_empty() || !seen.insert(num.clone()) {
            continue;
        }
        let logical = PathBuf::from(format!("/dev/disk{num}"));
        let raw = PathBuf::from(format!("/dev/rdisk{num}"));
        let info = Command::new("diskutil")
            .args(["info", logical.to_str().unwrap()])
            .output()
            .ok();
        let info_txt = info
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default();

        // Skip non-whole / virtual disk images when possible
        let is_virtual = info_txt.contains("Virtual:")
            && info_txt
                .lines()
                .any(|l| l.contains("Virtual:") && l.contains("Yes"));
        let protocol = info_txt
            .lines()
            .find(|l| l.contains("Protocol:"))
            .map(|l| l.to_string())
            .unwrap_or_default();
        if is_virtual || protocol.contains("Disk Image") {
            continue;
        }

        let size = parse_size_from_diskutil(&info_txt);
        let model = info_txt
            .lines()
            .find(|l| l.contains("Device / Media Name:") || l.contains("Media Name:"))
            .map(|l| l.split(':').nth(1).unwrap_or("").trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "disk".into());
        let is_internal = info_txt.lines().any(|l| {
            let t = l.trim();
            t.starts_with("Internal:") && t.contains("Yes")
        });
        let is_usb = protocol.to_lowercase().contains("usb")
            || info_txt.lines().any(|l| {
                l.contains("Protocol:") && l.to_lowercase().contains("usb")
            });
        // disk0 is almost always the boot/system disk on macOS.
        let is_system = num == "0"
            || system
                .as_ref()
                .map(|s| s == &logical || s == &raw || s.ends_with(num.as_str()))
                .unwrap_or(false)
            || (is_internal && !is_usb && size > 64 * 1024 * 1024 * 1024);

        disks.push(DiskInfo {
            path: if raw.exists() {
                raw
            } else {
                logical.clone()
            },
            display_path: logical,
            size_bytes: size,
            model,
            is_system,
            is_external: is_usb || (!is_internal && !is_system),
        });
    }
    disks.sort_by(|a, b| a.display_path.cmp(&b.display_path));
    Ok(disks)
}

#[cfg(target_os = "macos")]
fn system_disk_macos() -> Option<PathBuf> {
    let out = Command::new("diskutil").args(["info", "/"]).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    for line in text.lines() {
        if line.contains("Part of Whole:") {
            let id = line.split(':').nth(1)?.trim();
            return Some(PathBuf::from(format!("/dev/{id}")));
        }
    }
    Some(PathBuf::from("/dev/disk0"))
}

#[cfg(target_os = "macos")]
fn parse_size_from_diskutil(info: &str) -> u64 {
    for line in info.lines() {
        if line.contains("Disk Size:") || line.contains("Total Size:") {
            if let Some(start) = line.find('(') {
                let rest = &line[start + 1..];
                if let Some(num) = rest.split_whitespace().next() {
                    if let Ok(n) = num.parse::<u64>() {
                        return n;
                    }
                }
            }
        }
    }
    0
}

#[cfg(target_os = "linux")]
fn list_disks_linux() -> Result<Vec<DiskInfo>> {
    let mut disks = Vec::new();
    let system = system_disk_linux();
    // NAME SIZE TYPE TRAN RM MODEL — use NUL-friendly pairs via separate calls
    let out = Command::new("lsblk")
        .args(["-b", "-n", "-d", "-o", "NAME,SIZE,TYPE,TRAN,RM,MODEL"])
        .output()
        .context("lsblk")?;
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let mut parts = line.split_whitespace();
        let Some(name) = parts.next() else { continue };
        let Some(size_s) = parts.next() else { continue };
        let Some(type_) = parts.next() else { continue };
        if type_ != "disk" {
            continue;
        }
        let size: u64 = size_s.parse().unwrap_or(0);
        let mut rest: Vec<&str> = parts.collect();
        // TRAN RM MODEL...  — TRAN may be empty shown as blank; lsblk still emits fields
        // When TRAN empty, columns shift. Prefer reading via /sys.
        let path = PathBuf::from(format!("/dev/{name}"));
        let rm = std::fs::read_to_string(format!("/sys/block/{name}/removable"))
            .map(|s| s.trim() == "1")
            .unwrap_or(false);
        let tran = std::fs::read_to_string(format!("/sys/block/{name}/device/../../../transport"))
            .or_else(|_| {
                // usb path heuristic
                std::fs::read_link(format!("/sys/block/{name}"))
                    .map(|p| {
                        if p.to_string_lossy().contains("usb") {
                            "usb".into()
                        } else {
                            String::new()
                        }
                    })
            })
            .unwrap_or_default();
        let model = if rest.len() >= 2 {
            // skip possible TRAN and RM tokens if present as words
            rest.join(" ")
        } else {
            rest.join(" ")
        };
        let model = model
            .replace("usb", "")
            .replace(" 0 ", " ")
            .replace(" 1 ", " ")
            .trim()
            .to_string();
        let model = if model.is_empty() {
            name.to_string()
        } else {
            model
        };
        let is_system = system.as_ref().map(|s| s == &path).unwrap_or(false);
        let _ = rest;
        disks.push(DiskInfo {
            path: path.clone(),
            display_path: path,
            size_bytes: size,
            model,
            is_system,
            is_external: rm || tran.contains("usb"),
        });
    }
    Ok(disks)
}

#[cfg(target_os = "linux")]
fn system_disk_linux() -> Option<PathBuf> {
    let out = Command::new("findmnt")
        .args(["-n", "-o", "SOURCE", "/"])
        .output()
        .ok()?;
    let src = String::from_utf8_lossy(&out.stdout).trim().to_string();
    // strip partition → whole disk
    let name = Path::new(&src).file_name()?.to_str()?.to_string();
    let whole = if name.starts_with("nvme") || name.starts_with("mmcblk") {
        // nvme0n1p2 → nvme0n1
        name.rsplit_once('p').map(|(a, _)| a.to_string()).unwrap_or(name)
    } else {
        name.trim_end_matches(|c: char| c.is_ascii_digit()).to_string()
    };
    Some(PathBuf::from(format!("/dev/{whole}")))
}

pub fn find_iso_near_cwd() -> Option<PathBuf> {
    let mut dirs = vec![std::env::current_dir().ok()?];
    if let Ok(exe) = std::env::current_exe() {
        if let Some(p) = exe.parent() {
            dirs.push(p.to_path_buf());
        }
    }
    let mut candidates = Vec::new();
    for cwd in dirs {
        if let Ok(rd) = fs::read_dir(&cwd) {
            for ent in rd.flatten() {
                let p = ent.path();
                if p.extension().and_then(|e| e.to_str()) == Some("iso") {
                    candidates.push(p);
                }
            }
        }
    }
    candidates.sort_by(|a, b| {
        let score = |p: &Path| {
            let n = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if n.contains("native-qemu") {
                0
            } else {
                1
            }
        };
        score(a).cmp(&score(b)).then(a.cmp(b))
    });
    candidates.into_iter().next()
}

pub struct FlashProgress {
    pub phase: String,
    pub bytes_done: u64,
    pub bytes_total: u64,
}

/// Flash ISO and return the created data partition (for immediate volume open).
pub fn flash_iso(
    iso: &Path,
    disk: &Path,
    image_override: Option<&Path>,
    mut progress: impl FnMut(FlashProgress),
) -> Result<partition::DataPartition> {
    require_root()?;

    let pre = preflight_flash(iso, disk);
    if !pre.ok {
        bail!("preflight failed:\n  {}", pre.errors.join("\n  "));
    }

    let iso_size = fs::metadata(iso)?.len();
    let disk_size = tools::block_device_size(disk).unwrap_or(0);
    if disk_size > 0 && disk_size < iso_size + 64 * 1024 * 1024 {
        bail!(
            "disk is smaller than ISO + 64MiB (disk={} ISO={})",
            disk_size,
            iso_size
        );
    }

    progress(FlashProgress {
        phase: "unmounting".into(),
        bytes_done: 0,
        bytes_total: 1,
    });
    rawdisk::release_disk(disk).context("unmount target disk")?;

    progress(FlashProgress {
        phase: "writing ISO".into(),
        bytes_done: 0,
        bytes_total: iso_size,
    });
    write_iso_raw(iso, disk, iso_size, &mut progress).context("write ISO to disk")?;

    // After the write handle closes, macOS Disk Arbitration remounts the
    // hybrid ISO partitions (EFI/alpine/…). That makes /dev/rdiskN EBUSY
    // (os error 16) until we force-unmount again — before every later step.
    progress(FlashProgress {
        phase: "unmounting (macOS remounts after write)".into(),
        bytes_done: 0,
        bytes_total: 1,
    });
    rawdisk::release_disk(disk).context("unmount after ISO write")?;

    progress(FlashProgress {
        phase: "verifying ISO".into(),
        bytes_done: 0,
        bytes_total: iso_size,
    });
    verify_iso_prefix(iso, disk, iso_size, &mut progress).context("verify ISO on disk")?;

    rawdisk::release_disk(disk).context("unmount after verify")?;

    progress(FlashProgress {
        phase: "creating GPT data partition".into(),
        bytes_done: 0,
        bytes_total: 1,
    });
    let part = with_busy_retry(disk, "create data partition", || {
        partition::ensure_data_partition(disk, iso_size)
    })?;

    progress(FlashProgress {
        phase: "extracting default image from ISO".into(),
        bytes_done: 0,
        bytes_total: 1,
    });
    let img = image_override
        .map(|p| p.to_path_buf())
        .or_else(|| extract_image_from_iso(iso).ok().flatten());

    progress(FlashProgress {
        phase: "mkfs.ext4 + seed (bundled lwext4)".into(),
        bytes_done: 0,
        bytes_total: 1,
    });
    // GPT write closed the handle → macOS remounts again. Seed opens rdisk
    // via rawdisk::open_raw (unmount + retry); also pre-release here.
    rawdisk::release_disk(disk).context("unmount before mkfs/seed")?;
    with_busy_retry(disk, "format/seed ext4", || {
        volume::seed_partition_slice(
            &part.raw_path,
            part.start_bytes,
            part.size_bytes,
            img.as_deref(),
        )
    })
    .with_context(|| {
        format!(
            "format/seed ext4 at LBA {} ({} MiB) on {}",
            part.first_lba,
            part.size_bytes / (1024 * 1024),
            part.raw_path.display()
        )
    })?;

    progress(FlashProgress {
        phase: "done".into(),
        bytes_done: 1,
        bytes_total: 1,
    });
    Ok(part)
}

/// Run `op` after force-unmount; retry on EBUSY (macOS remount races).
fn with_busy_retry<T, F>(disk: &Path, what: &str, mut op: F) -> Result<T>
where
    F: FnMut() -> Result<T>,
{
    let mut last = None;
    for attempt in 0..10 {
        rawdisk::release_disk(disk)?;
        match op() {
            Ok(v) => return Ok(v),
            Err(e) => {
                let busy = rawdisk::is_busy_anyhow(&e);
                last = Some(e);
                if !busy {
                    break;
                }
                thread::sleep(Duration::from_millis(300 + attempt as u64 * 200));
            }
        }
    }
    Err(last.unwrap_or_else(|| anyhow::anyhow!("{what} failed"))).with_context(|| {
        format!(
            "{what} on {} after unmount retries — try:\n  diskutil unmountDisk force {}",
            disk.display(),
            rawdisk::logical_disk(disk).display()
        )
    })
}

fn write_iso_raw(
    iso: &Path,
    disk: &Path,
    iso_size: u64,
    progress: &mut impl FnMut(FlashProgress),
) -> Result<()> {
    use crate::sized_disk::{SizedDisk, SECTOR};

    let mut src = File::open(iso).with_context(|| format!("open ISO {}", iso.display()))?;
    // SizedDisk: sector-aligned writes on macOS rdisk (avoids EINVAL).
    let mut dst = SizedDisk::open(disk).context("open target disk for ISO write")?;

    // Use a sector-multiple buffer (4 MiB).
    let mut buf = vec![0u8; 4 * 1024 * 1024];
    let mut done = 0u64;
    loop {
        let n = src.read(&mut buf)?;
        if n == 0 {
            break;
        }
        // Pad last chunk to a full sector so raw devices accept the write.
        let mut write_len = n;
        if (write_len as u64) % SECTOR != 0 {
            let pad_to = ((write_len as u64 + SECTOR - 1) / SECTOR) * SECTOR;
            buf[write_len..pad_to as usize].fill(0);
            write_len = pad_to as usize;
        }
        let written = dst
            .write(&buf[..write_len])
            .context("sector-aligned write to disk")?;
        if written < n {
            // We may have written padding past ISO size into device — count only ISO bytes.
        }
        done += n as u64;
        // Keep stream position consistent: we asked to write write_len but ISO only had n.
        // SizedDisk advanced by written; if we padded, pos is ahead of pure ISO bytes — OK.
        if done % (16 * 1024 * 1024) < buf.len() as u64 || done >= iso_size {
            progress(FlashProgress {
                phase: "writing ISO".into(),
                bytes_done: done.min(iso_size),
                bytes_total: iso_size,
            });
        }
        if n < buf.len() {
            break;
        }
    }
    dst.flush().context("flush written ISO to device")?;
    let _ = Command::new("sync").status();
    Ok(())
}

fn verify_iso_prefix(
    iso: &Path,
    disk: &Path,
    iso_size: u64,
    progress: &mut impl FnMut(FlashProgress),
) -> Result<()> {
    let mut expected = Sha256::new();
    let mut actual = Sha256::new();
    let mut src = File::open(iso)?;
    // SizedDisk::open_ro → rawdisk::open_raw (unmount + EBUSY retry).
    let mut dev =
        crate::sized_disk::SizedDisk::open_ro(disk).context("open disk for verify")?;
    // 1 MiB is sector-aligned
    let mut buf_a = vec![0u8; 1024 * 1024];
    let mut buf_b = vec![0u8; 1024 * 1024];
    let mut left = iso_size;
    let mut done = 0u64;
    while left > 0 {
        let chunk = (buf_a.len() as u64).min(left) as usize;
        let na = src.read(&mut buf_a[..chunk])?;
        // Device read via SizedDisk (sector-aligned under the hood)
        let nb = dev.read(&mut buf_b[..chunk])?;
        if na == 0 || nb == 0 || na != nb {
            bail!("short read during verification at offset {done} (iso={na} disk={nb})");
        }
        expected.update(&buf_a[..na]);
        actual.update(&buf_b[..nb]);
        left -= na as u64;
        done += na as u64;
        if done % (32 * 1024 * 1024) < chunk as u64 {
            progress(FlashProgress {
                phase: "verifying ISO".into(),
                bytes_done: done,
                bytes_total: iso_size,
            });
        }
    }
    // Drop device handle before returning so later GPT write can open R/W.
    drop(dev);
    if expected.finalize() != actual.finalize() {
        bail!("ISO verification failed — device contents do not match the image");
    }
    Ok(())
}

/// Extract images/image.qcow2 from ISO to a temp file.
pub fn extract_image_from_iso(iso: &Path) -> Result<Option<PathBuf>> {
    let dest_dir = std::env::temp_dir().join(format!("nq-iso-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dest_dir);
    fs::create_dir_all(&dest_dir)?;
    let dest = dest_dir.join("image.qcow2");

    if let Some(xorriso) = find_xorriso() {
        let st = Command::new(xorriso)
            .args([
                "-osirrox",
                "on",
                "-indev",
                iso.to_str().unwrap(),
                "-extract",
                "/images/image.qcow2",
                dest.to_str().unwrap(),
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()?;
        if st.success() && dest.is_file() && fs::metadata(&dest)?.len() > 0 {
            return Ok(Some(dest));
        }
    }

    for (bin, args_prefix) in [
        (
            "bsdtar",
            vec![
                "-xf".into(),
                iso.to_string_lossy().into_owned(),
                "-C".into(),
                dest_dir.to_string_lossy().into_owned(),
                "images/image.qcow2".into(),
            ],
        ),
        (
            "tar",
            vec![
                "-xf".into(),
                iso.to_string_lossy().into_owned(),
                "-C".into(),
                dest_dir.to_string_lossy().into_owned(),
                "images/image.qcow2".into(),
            ],
        ),
    ] {
        if tools::which(bin).is_none() {
            continue;
        }
        let st = Command::new(bin).args(&args_prefix).status();
        if st.map(|s| s.success()).unwrap_or(false) {
            let p = dest_dir.join("images/image.qcow2");
            if p.is_file() {
                return Ok(Some(p));
            }
            if dest.is_file() {
                return Ok(Some(dest));
            }
        }
    }

    // 7z
    if let Some(z) = tools::which("7z").or_else(|| tools::which("7zz")) {
        let st = Command::new(z)
            .args([
                "x",
                iso.to_str().unwrap(),
                &format!("-o{}", dest_dir.display()),
                "images/image.qcow2",
                "-y",
            ])
            .status();
        if st.map(|s| s.success()).unwrap_or(false) {
            let p = dest_dir.join("images/image.qcow2");
            if p.is_file() {
                return Ok(Some(p));
            }
        }
    }

    Ok(None)
}
