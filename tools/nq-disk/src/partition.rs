//! Pure-Rust GPT data-partition creation (no sgdisk/parted).

use crate::sized_disk::{probe_size, SizedDisk};
use anyhow::{bail, Context, Result};
use gpt::disk::LogicalBlockSize;
use gpt::partition_types;
use gpt::GptConfig;
use std::path::Path;

/// Result of creating the data partition on a flashed stick.
#[derive(Debug, Clone)]
pub struct DataPartition {
    /// Absolute byte offset of the partition on the whole disk.
    pub start_bytes: u64,
    /// Size in bytes.
    pub size_bytes: u64,
    pub first_lba: u64,
    pub last_lba: u64,
    /// Path that was opened for raw I/O (prefer rdisk on macOS).
    pub raw_path: std::path::PathBuf,
}

const LB: u64 = 512;
const ALIGN_LBA: u64 = (1024 * 1024) / LB;

/// After writing a hybrid ISO, expand GPT to the full disk (via write which
/// relocates the backup header to the true disk end) and add a Linux data
/// partition in free space for the ext4 volume.
pub fn ensure_data_partition(disk: &Path, iso_size: u64) -> Result<DataPartition> {
    let disk_size = probe_size(disk).context("probe disk size for GPT")?;
    if disk_size <= iso_size + 64 * 1024 * 1024 {
        bail!(
            "disk too small for data partition (disk={} iso={})",
            disk_size,
            iso_size
        );
    }

    // SizedDisk makes SeekFrom::End work on macOS rdisk (avoids ENOTTY / os error 25).
    let sized = SizedDisk::open(disk).context("open disk for GPT")?;
    let raw_path = sized.path().to_path_buf();

    let mut gdisk = GptConfig::new()
        .writable(true)
        .logical_block_size(LogicalBlockSize::Lb512)
        // Allow primary-only hybrid tables; backup is rewritten on write().
        .open_from_device(sized)
        .with_context(|| {
            format!(
                "open GPT on {} (hybrid ISO should carry a primary GPT)",
                raw_path.display()
            )
        })?;

    // Remove our previous data partition if re-flashing.
    let stale: Vec<u32> = gdisk
        .partitions()
        .iter()
        .filter(|(_, p)| p.is_used() && p.name.eq_ignore_ascii_case("native-qemu"))
        .map(|(id, _)| *id)
        .collect();
    for id in stale {
        gdisk.remove_partition(id);
    }

    // Force header rebuild against full disk size: write() uses SeekFrom::End
    // (via SizedDisk) so backup LBA = last sector of the real USB stick, which
    // expands last_usable past the hybrid ISO footprint.
    gdisk
        .update_partitions(gdisk.partitions().clone())
        .map_err(|e| anyhow::anyhow!("expand GPT headers: {e}"))?;

    let iso_lba = (iso_size + LB - 1) / LB;
    let free = gdisk.find_free_sectors();
    let (start_lba, length_lba) = free
        .into_iter()
        .map(|(start, len)| {
            let end = start + len;
            if end <= iso_lba {
                return (start, 0u64);
            }
            let s = if start < iso_lba {
                ((iso_lba + ALIGN_LBA - 1) / ALIGN_LBA) * ALIGN_LBA
            } else {
                ((start + ALIGN_LBA - 1) / ALIGN_LBA) * ALIGN_LBA
            };
            if s >= end {
                (start, 0u64)
            } else {
                (s, end - s)
            }
        })
        .filter(|(_, len)| *len > ALIGN_LBA * 2)
        .max_by_key(|(_, len)| *len)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "no free GPT space after the ISO for a data partition \
                 (USB must be larger than the ISO by a few hundred MiB)"
            )
        })?;

    let id = gdisk.find_next_partition_id().unwrap_or(2);
    gdisk
        .add_partition_at(
            "native-qemu",
            id,
            start_lba,
            length_lba,
            partition_types::LINUX_FS,
            0,
        )
        .map_err(|e| anyhow::anyhow!("add GPT partition: {e}"))?;

    let mut device = gdisk
        .write()
        .map_err(|e| anyhow::anyhow!("write GPT: {e}"))?;
    // Flush
    use std::io::Write;
    let _ = device.flush();

    reread_pt(&raw_path);

    let last_lba = start_lba + length_lba - 1;
    Ok(DataPartition {
        start_bytes: start_lba * LB,
        size_bytes: length_lba * LB,
        first_lba: start_lba,
        last_lba,
        raw_path,
    })
}

fn reread_pt(disk: &Path) {
    use std::process::Command;
    #[cfg(target_os = "linux")]
    {
        let _ = Command::new("partprobe").arg(disk).status();
        let _ = Command::new("blockdev")
            .args(["--rereadpt", disk.to_str().unwrap_or("")])
            .status();
    }
    #[cfg(target_os = "macos")]
    {
        let logical = disk
            .to_string_lossy()
            .replace("/dev/rdisk", "/dev/disk");
        let _ = Command::new("diskutil").args(["list", &logical]).status();
    }
}

/// Locate a previously created `native-qemu` GPT data partition on one disk.
pub fn find_data_partition_on_disk(disk: &Path) -> Result<DataPartition> {
    let sized = SizedDisk::open(disk).context("open disk to scan GPT")?;
    let raw_path = sized.path().to_path_buf();
    let gdisk = GptConfig::new()
        .writable(false)
        .logical_block_size(LogicalBlockSize::Lb512)
        .open_from_device(sized)
        .with_context(|| format!("read GPT on {}", raw_path.display()))?;

    for (_id, part) in gdisk.partitions() {
        if !part.is_used() {
            continue;
        }
        if part.name.eq_ignore_ascii_case("native-qemu") {
            let first = part.first_lba;
            let last = part.last_lba;
            if last < first {
                continue;
            }
            let length = last - first + 1;
            return Ok(DataPartition {
                start_bytes: first * LB,
                size_bytes: length * LB,
                first_lba: first,
                last_lba: last,
                raw_path,
            });
        }
    }
    bail!(
        "no GPT partition named 'native-qemu' on {}",
        raw_path.display()
    )
}

/// Scan non-system whole disks for a `native-qemu` data partition.
pub fn find_any_data_partition() -> Result<DataPartition> {
    let disks = crate::flash::list_disks().unwrap_or_default();
    let mut errors = Vec::new();
    for d in disks {
        if d.is_system {
            continue;
        }
        match find_data_partition_on_disk(&d.path) {
            Ok(p) => return Ok(p),
            Err(e) => errors.push(format!("{}: {e}", d.display_path.display())),
        }
    }
    if errors.is_empty() {
        bail!("no removable disks found to scan for data volume");
    }
    bail!(
        "could not find GPT partition 'native-qemu' on any disk:\n  {}",
        errors.join("\n  ")
    )
}
