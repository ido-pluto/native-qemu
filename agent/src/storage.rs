use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

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

/// Resolves a config `storage` index to a real, mounted directory:
/// 0 = boot media (already mounted by the initramfs under /media/*),
/// 1 = first internal disk, 2.. = external disks in stable order.
/// Internal/external disks get mounted on demand under /mnt/native-qemu/.
pub fn resolve(index: u32) -> Result<PathBuf, StorageError> {
    if index == 0 {
        let mounts = fs::read_to_string("/proc/mounts").unwrap_or_default();
        for line in mounts.lines() {
            let mut fields = line.split_whitespace();
            let _device = fields.next();
            if let Some(mp) = fields.next() {
                if mp.starts_with("/media/") {
                    return Ok(PathBuf::from(mp));
                }
            }
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
    let mounts = fs::read_to_string("/proc/mounts").unwrap_or_default();
    let already = mounts
        .lines()
        .any(|l| l.split_whitespace().nth(1) == Some(mountpoint.to_str().unwrap_or("")));
    if already {
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
