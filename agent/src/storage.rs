use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Filesystem label of the writable ext4 USB data volume used for config and
/// the default guest disk image.
pub const DATA_VOLUME_LABEL: &str = "native-qemu";
/// Where we mount the data volume when it is not already mounted elsewhere.
pub const DATA_VOLUME_MOUNT: &str = "/mnt/native-qemu-data";
/// Seed image path on boot media, relative to the media mountpoint.
const SEED_DISK_REL: &str = "images/image.qcow2";

/// Kernel block-device name prefixes that are never real storage (virtual
/// devices, not disks a user could point vm.disk.storage at).
const IGNORED_PREFIXES: &[&str] = &["loop", "ram", "zram", "dm-", "md"];

fn is_ignored(name: &str) -> bool {
    IGNORED_PREFIXES.iter().any(|p| name.starts_with(p))
}

/// True if this /sys/class/block entry is a whole disk (not a partition of
/// one) — partitions carry a "partition" attribute file, whole disks don't.
fn is_whole_disk(name: &str) -> bool {
    !Path::new("/sys/class/block")
        .join(name)
        .join("partition")
        .exists()
}

fn read_trimmed(path: &Path) -> Option<String> {
    fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

/// Resolves a partition's sysfs symlink to find which whole disk it belongs
/// to, e.g. "sda1" -> "sda", "mmcblk0p1" -> "mmcblk0", by reading the actual
/// kernel-reported parent directory rather than guessing from the name.
fn parent_disk_of(partition_name: &str) -> Option<String> {
    let link = fs::read_link(Path::new("/sys/class/block").join(partition_name)).ok()?;
    let comps: Vec<_> = link.components().collect();
    // .../devices/.../block/<disk>/<partition>
    if comps.len() >= 2 {
        comps[comps.len() - 2]
            .as_os_str()
            .to_str()
            .map(|s| s.to_string())
    } else {
        None
    }
}

/// Finds the whole-disk block device backing whatever is mounted under
/// /media/ (that's where Alpine's initramfs mounts the boot media), so we
/// can identify and exclude "the device we booted from" from the internal/
/// external storage lists.
fn boot_disk_name() -> Option<String> {
    let mounts = fs::read_to_string("/proc/mounts").ok()?;
    for line in mounts.lines() {
        let mut fields = line.split_whitespace();
        let device = fields.next()?;
        let mountpoint = fields.next().unwrap_or("");
        if !mountpoint.starts_with("/media/") {
            continue;
        }
        let dev_name = device.trim_start_matches("/dev/");
        if dev_name.is_empty() {
            continue;
        }
        if is_whole_disk(dev_name) {
            return Some(dev_name.to_string());
        }
        if let Some(parent) = parent_disk_of(dev_name) {
            return Some(parent);
        }
    }
    None
}

#[derive(Debug, Clone)]
pub struct Disk {
    pub name: String,
}

/// Enumerates real, non-virtual whole disks under /sys/class/block, split
/// into (boot_disk, internal_disks, external_disks) — internal/external are
/// each sorted by name for a stable, reboot-to-reboot consistent numbering.
pub fn enumerate() -> (Option<String>, Vec<Disk>, Vec<Disk>) {
    let boot = boot_disk_name();
    let mut internal = Vec::new();
    let mut external = Vec::new();

    let entries = match fs::read_dir("/sys/class/block") {
        Ok(e) => e,
        Err(_) => return (boot, internal, external),
    };

    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if is_ignored(&name) || !is_whole_disk(&name) {
            continue;
        }
        if boot.as_deref() == Some(name.as_str()) {
            continue;
        }
        let removable = read_trimmed(&Path::new("/sys/class/block").join(&name).join("removable"))
            .as_deref()
            == Some("1");
        let disk = Disk { name: name.clone() };
        if removable {
            external.push(disk);
        } else {
            internal.push(disk);
        }
    }

    internal.sort_by(|a, b| a.name.cmp(&b.name));
    external.sort_by(|a, b| a.name.cmp(&b.name));
    (boot, internal, external)
}

#[derive(Debug)]
pub enum StorageError {
    NotFound(u32),
    MountFailed(String, String),
}
impl std::fmt::Display for StorageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StorageError::NotFound(i) => {
                write!(f, "storage index {i} does not resolve to any present disk")
            }
            StorageError::MountFailed(d, e) => write!(f, "failed to mount {d}: {e}"),
        }
    }
}

/// First partition of `disk` (e.g. "sda" -> "sda1"), or the disk itself if
/// it has no partitions (a filesystem directly on the whole device).
fn first_partition_or_self(disk: &str) -> String {
    if let Ok(entries) = fs::read_dir("/sys/class/block") {
        let mut parts: Vec<String> = entries
            .flatten()
            .filter_map(|e| e.file_name().to_str().map(str::to_string))
            .filter(|n| n.starts_with(disk) && n != disk && !is_whole_disk(n))
            .collect();
        parts.sort();
        if let Some(first) = parts.into_iter().next() {
            return first;
        }
    }
    disk.to_string()
}

/// Mountpoints currently under /media/* (boot ISO / Alpine media).
pub fn media_mounts() -> Vec<PathBuf> {
    let mounts = fs::read_to_string("/proc/mounts").unwrap_or_default();
    let mut out = Vec::new();
    for line in mounts.lines() {
        let mut fields = line.split_whitespace();
        let _device = fields.next();
        if let Some(mp) = fields.next() {
            if mp.starts_with("/media/") {
                out.push(PathBuf::from(mp));
            }
        }
    }
    out
}

/// Resolves the block device for LABEL=native-qemu, if present.
fn data_volume_device() -> Option<PathBuf> {
    let by_label = Path::new("/dev/disk/by-label").join(DATA_VOLUME_LABEL);
    if by_label.exists() {
        return fs::canonicalize(&by_label)
            .ok()
            .or_else(|| Some(by_label));
    }

    // findfs is part of util-linux / busybox on Alpine.
    if let Ok(output) = Command::new("findfs")
        .arg(format!("LABEL={DATA_VOLUME_LABEL}"))
        .output()
    {
        if output.status.success() {
            let dev = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !dev.is_empty() {
                return Some(PathBuf::from(dev));
            }
        }
    }
    None
}

fn devices_match(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(ca), Ok(cb)) => ca == cb,
        _ => a.to_string_lossy() == b.to_string_lossy(),
    }
}

/// Returns the mountpoint of `device` if it is already mounted.
fn mountpoint_for_device(device: &Path) -> Option<PathBuf> {
    let mounts = fs::read_to_string("/proc/mounts").ok()?;
    for line in mounts.lines() {
        let mut fields = line.split_whitespace();
        let mounted_dev = fields.next()?;
        let mp = fields.next()?;
        if devices_match(Path::new(mounted_dev), device) {
            return Some(PathBuf::from(mp));
        }
    }
    None
}

fn is_mounted_at(mountpoint: &Path) -> bool {
    let mounts = fs::read_to_string("/proc/mounts").unwrap_or_default();
    let want = mountpoint.to_string_lossy();
    mounts
        .lines()
        .any(|l| l.split_whitespace().nth(1) == Some(want.as_ref()))
}

/// Finds or mounts the ext4 data volume labeled `native-qemu`.
///
/// Preference order:
/// 1. Already mounted at `/mnt/native-qemu-data`
/// 2. Device for LABEL=native-qemu already mounted elsewhere
/// 3. Mount LABEL=native-qemu (ext4) at `/mnt/native-qemu-data`
pub fn ensure_data_volume() -> Option<PathBuf> {
    let mountpoint = PathBuf::from(DATA_VOLUME_MOUNT);
    if is_mounted_at(&mountpoint) {
        return Some(mountpoint);
    }

    let device = data_volume_device()?;
    if let Some(existing) = mountpoint_for_device(&device) {
        return Some(existing);
    }

    if let Err(e) = fs::create_dir_all(&mountpoint) {
        eprintln!(
            "native-qemu: warning: could not create {}: {e}",
            mountpoint.display()
        );
        return None;
    }

    // Prefer LABEL= so the call works even when we only know the label.
    let label_spec = format!("LABEL={DATA_VOLUME_LABEL}");
    let status = Command::new("mount")
        .args(["-t", "ext4", &label_spec])
        .arg(&mountpoint)
        .status();
    match status {
        Ok(s) if s.success() => {
            println!(
                "native-qemu: mounted data volume {label_spec} at {}",
                mountpoint.display()
            );
            return Some(mountpoint);
        }
        Ok(s) => {
            // Fall back to the resolved device path.
            let status2 = Command::new("mount")
                .args(["-t", "ext4"])
                .arg(&device)
                .arg(&mountpoint)
                .status();
            match status2 {
                Ok(s2) if s2.success() => {
                    println!(
                        "native-qemu: mounted data volume {} at {}",
                        device.display(),
                        mountpoint.display()
                    );
                    return Some(mountpoint);
                }
                Ok(s2) => eprintln!(
                    "native-qemu: warning: could not mount data volume {} (LABEL exit {s}, device exit {s2})",
                    device.display()
                ),
                Err(e) => eprintln!(
                    "native-qemu: warning: mount {} failed: {e}",
                    device.display()
                ),
            }
        }
        Err(e) => eprintln!("native-qemu: warning: mount {label_spec} failed: {e}"),
    }
    None
}

/// If `dest` is missing, copy the seed disk from boot media
/// (`images/image.qcow2`) onto it once. Returns true when a copy was made.
pub fn seed_disk_from_boot_media(dest: &Path) -> std::io::Result<bool> {
    if dest.exists() {
        return Ok(false);
    }
    for media in media_mounts() {
        let seed = media.join(SEED_DISK_REL);
        if !seed.is_file() {
            continue;
        }
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        println!(
            "native-qemu: seeding disk {} from {}",
            dest.display(),
            seed.display()
        );
        fs::copy(&seed, dest)?;
        return Ok(true);
    }
    Ok(false)
}

/// Resolves a config `storage` index to a real, mounted directory:
/// 0 = data volume labeled `native-qemu` when present, else boot media under
///     /media/* (already mounted by the initramfs),
/// 1 = first internal disk, 2.. = external disks in stable order.
/// Internal/external disks get mounted on demand under /mnt/native-qemu/.
pub fn resolve(index: u32) -> Result<PathBuf, StorageError> {
    if index == 0 {
        // Prefer the writable ext4 data volume over read-only ISO boot media.
        if let Some(data) = ensure_data_volume() {
            return Ok(data);
        }
        if let Some(media) = media_mounts().into_iter().next() {
            return Ok(media);
        }
        return Err(StorageError::NotFound(0));
    }

    let (_boot, internal, external) = enumerate();
    let disk = if index == 1 {
        internal.first()
    } else {
        external.get((index - 2) as usize)
    }
    .ok_or(StorageError::NotFound(index))?;

    let target = first_partition_or_self(&disk.name);
    let mountpoint = PathBuf::from(format!("/mnt/native-qemu/storage{index}"));

    // already mounted?
    if is_mounted_at(&mountpoint) {
        return Ok(mountpoint);
    }

    fs::create_dir_all(&mountpoint).ok();
    let status = Command::new("mount")
        .arg(format!("/dev/{target}"))
        .arg(&mountpoint)
        .status();
    match status {
        Ok(s) if s.success() => Ok(mountpoint),
        Ok(s) => Err(StorageError::MountFailed(
            target,
            format!("mount exited with {s}"),
        )),
        Err(e) => Err(StorageError::MountFailed(target, e.to_string())),
    }
}
