//! High-level data volume API (bundled ext4 via GPT slice, or host directory).

use crate::ext4_io::{self, DirVolume, Ext4Volume};
use crate::partition::{self, DataPartition};
use anyhow::Result;
use std::path::{Path, PathBuf};

pub const VOLUME_LABEL: &str = "native-qemu";
pub const CONFIG_NAME: &str = "config.toml";
pub const IMAGE_NAME: &str = "image.qcow2";

enum Backend {
    Ext4(Ext4Volume),
    Dir(DirVolume),
}

pub struct Volume {
    backend: Backend,
}

#[derive(Debug, Clone)]
pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
}

impl Volume {
    pub fn open_path(root: impl Into<PathBuf>) -> Result<Self> {
        Ok(Self {
            backend: Backend::Dir(DirVolume::open(root)?),
        })
    }

    pub fn open_device(device: impl AsRef<Path>) -> Result<Self> {
        Ok(Self {
            backend: Backend::Ext4(Ext4Volume::open_device(device.as_ref())?),
        })
    }

    /// Open existing ext4 at a GPT partition slice (no mkfs). Primary path on macOS.
    pub fn open_existing_slice(disk: &Path, start: u64, size: u64) -> Result<Self> {
        Ok(Self {
            backend: Backend::Ext4(Ext4Volume::open_slice(disk, start, size)?),
        })
    }

    pub fn open_from_partition(part: &DataPartition) -> Result<Self> {
        Self::open_existing_slice(&part.raw_path, part.start_bytes, part.size_bytes)
    }

    /// Discover the native-qemu data volume without relying on OS mounts.
    ///
    /// Order:
    /// 1. Already-mounted directory (Linux `/media`, findmnt, …)
    /// 2. `blkid -L native-qemu` partition node (Linux)
    /// 3. **Scan GPT** for partition name `native-qemu` (works on macOS after flash)
    /// 4. Linux: mount by LABEL
    pub fn discover() -> Result<Self> {
        if let Some(mp) = find_mounted_path() {
            if let Ok(v) = Self::open_path(&mp) {
                return Ok(v);
            }
        }
        if let Some(dev) = find_labeled_device() {
            if let Ok(v) = Self::open_device(dev) {
                return Ok(v);
            }
        }
        // GPT scan — does not need the OS to understand ext4 or create s2 nodes.
        match partition::find_any_data_partition() {
            Ok(part) => {
                return Self::open_from_partition(&part).map_err(|e| {
                    anyhow::anyhow!(
                        "found GPT partition native-qemu on {} but could not mount ext4: {e}",
                        part.raw_path.display()
                    )
                });
            }
            Err(e) => {
                #[cfg(target_os = "linux")]
                {
                    if let Ok(v) = mount_linux_label() {
                        return Ok(v);
                    }
                }
                return Err(anyhow::anyhow!(
                    "could not find data volume (flash a stick first, or open a path/device): {e:#}"
                ));
            }
        }
    }

    pub fn root_display(&self) -> String {
        match &self.backend {
            Backend::Ext4(v) => v.display_name().to_string(),
            Backend::Dir(v) => v.display_name(),
        }
    }

    pub fn list(&self, rel: &str) -> Result<Vec<DirEntry>> {
        match &self.backend {
            Backend::Ext4(v) => v.list(rel),
            Backend::Dir(v) => v.list(rel),
        }
    }

    pub fn read_text(&self, rel: &str) -> Result<String> {
        match &self.backend {
            Backend::Ext4(v) => v.read_text(rel),
            Backend::Dir(v) => v.read_text(rel),
        }
    }

    pub fn write_text(&self, rel: &str, text: &str) -> Result<()> {
        match &self.backend {
            Backend::Ext4(v) => v.write_text(rel, text),
            Backend::Dir(v) => v.write_text(rel, text),
        }
    }

    pub fn copy_from_host(&self, host: &Path, rel: &str) -> Result<u64> {
        match &self.backend {
            Backend::Ext4(v) => v.copy_from_host(host, rel),
            Backend::Dir(v) => v.copy_from_host(host, rel),
        }
    }

    pub fn copy_to_host(&self, rel: &str, host: &Path) -> Result<u64> {
        match &self.backend {
            Backend::Ext4(v) => v.copy_to_host(rel, host),
            Backend::Dir(v) => v.copy_to_host(rel, host),
        }
    }

    pub fn remove(&self, rel: &str) -> Result<()> {
        match &self.backend {
            Backend::Ext4(v) => v.remove(rel),
            Backend::Dir(v) => v.remove(rel),
        }
    }

    pub fn exists(&self, rel: &str) -> bool {
        match &self.backend {
            Backend::Ext4(v) => v.exists(rel),
            Backend::Dir(v) => v.exists(rel),
        }
    }

    pub fn put_image(&self, host_image: &Path) -> Result<u64> {
        match &self.backend {
            Backend::Ext4(v) => v.put_image(host_image),
            Backend::Dir(v) => v.put_image(host_image),
        }
    }

    pub fn ensure_config(&self) -> Result<String> {
        match &self.backend {
            Backend::Ext4(v) => v.ensure_config(),
            Backend::Dir(v) => v.ensure_config(),
        }
    }

    pub fn unmount(self) -> Result<()> {
        match self.backend {
            Backend::Ext4(v) => v.close(),
            Backend::Dir(v) => v.close(),
        }
    }
}

/// Seed a directory (tests) or format+seed a raw image/device path.
pub fn seed_volume(root: &Path, image_source: Option<&Path>) -> Result<()> {
    if root.is_dir() {
        let v = DirVolume::open(root)?;
        v.seed(image_source)?;
        v.close()?;
        Ok(())
    } else {
        let meta = std::fs::metadata(root)?;
        let size = meta.len();
        if size == 0 {
            ext4_io::format_seed_slice(
                root,
                0,
                crate::tools::block_device_size(root)?,
                image_source,
            )
        } else {
            ext4_io::format_seed_slice(root, 0, size, image_source)
        }
    }
}

pub fn seed_partition_slice(
    disk: &Path,
    start: u64,
    size: u64,
    image: Option<&Path>,
) -> Result<()> {
    ext4_io::format_seed_slice(disk, start, size, image)
}

fn find_mounted_path() -> Option<PathBuf> {
    use std::process::Command;
    if let Ok(out) = Command::new("findmnt")
        .args(["-n", "-o", "TARGET", "-S", &format!("LABEL={VOLUME_LABEL}")])
        .output()
    {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !s.is_empty() {
                return Some(PathBuf::from(s));
            }
        }
    }
    for base in ["/media", "/mnt", "/Volumes"] {
        let base = Path::new(base);
        if !base.is_dir() {
            continue;
        }
        if let Ok(rd) = std::fs::read_dir(base) {
            for ent in rd.flatten() {
                let p = ent.path();
                if p.join(CONFIG_NAME).is_file() || p.join(IMAGE_NAME).is_file() {
                    return Some(p);
                }
            }
        }
    }
    None
}

fn find_labeled_device() -> Option<PathBuf> {
    use std::process::Command;
    if let Ok(out) = Command::new("blkid")
        .args(["-L", VOLUME_LABEL])
        .output()
    {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !s.is_empty() {
                return Some(PathBuf::from(s));
            }
        }
    }
    None
}

#[cfg(target_os = "linux")]
fn mount_linux_label() -> Result<Volume> {
    use std::process::Command;
    let mp = PathBuf::from("/mnt/native-qemu-data");
    std::fs::create_dir_all(&mp)?;
    let status = Command::new("mount")
        .args([
            "-t",
            "ext4",
            &format!("LABEL={VOLUME_LABEL}"),
            mp.to_str().unwrap(),
        ])
        .status()?;
    if !status.success() {
        anyhow::bail!("mount LABEL={VOLUME_LABEL} failed");
    }
    Volume::open_path(mp)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dir_seed() {
        let dir = std::env::temp_dir().join(format!("nq-vtest-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        seed_volume(&dir, None).unwrap();
        let v = Volume::open_path(&dir).unwrap();
        assert!(v.exists(CONFIG_NAME));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
