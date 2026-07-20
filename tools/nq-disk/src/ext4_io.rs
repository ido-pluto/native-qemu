//! ext4 format + file ops via **bundled lwext4** (no host mke2fs/debugfs).

use crate::blockdev::SliceBlockDevice;
use crate::volume::{CONFIG_NAME, IMAGE_NAME, VOLUME_LABEL};
use anyhow::{bail, Context, Result};
use ext4_lwext4::{mkfs, Ext4Fs, MkfsOptions, OpenFlags};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Open handle to an ext4 volume (partition slice or whole image file).
pub struct Ext4Volume {
    // Ext4Fs is not Sync in a way that allows easy sharing; keep behind mutex
    // for simple sequential UI use.
    fs: Mutex<Option<Ext4Fs>>,
    display: String,
}

impl Ext4Volume {
    /// Format `device` as ext4 with label `native-qemu`, then mount for seeding.
    pub fn format_and_mount_slice(
        disk_path: &Path,
        start_bytes: u64,
        size_bytes: u64,
    ) -> Result<Self> {
        let dev = SliceBlockDevice::open_slice(disk_path, start_bytes, size_bytes)
            .context("open partition slice for mkfs")?;
        let opts = MkfsOptions::ext4()
            .with_label(VOLUME_LABEL)
            .with_journal(true)
            .with_block_size(4096);
        mkfs(dev, &opts).map_err(|e| anyhow::anyhow!("bundled mkfs.ext4 failed: {e}"))?;

        let dev = SliceBlockDevice::open_slice(disk_path, start_bytes, size_bytes)
            .context("re-open slice after mkfs")?;
        let fs = Ext4Fs::mount(dev, false)
            .map_err(|e| anyhow::anyhow!("mount ext4 after mkfs failed: {e}"))?;
        Ok(Self {
            fs: Mutex::new(Some(fs)),
            display: format!(
                "ext4:{}@{}+{}",
                disk_path.display(),
                start_bytes,
                size_bytes
            ),
        })
    }

    /// Mount an existing ext4 on a whole device/file (partition node or image).
    pub fn open_device(path: &Path) -> Result<Self> {
        let dev = SliceBlockDevice::open_path(path)
            .with_context(|| format!("open {}", path.display()))?;
        let fs = Ext4Fs::mount(dev, false)
            .map_err(|e| anyhow::anyhow!("mount ext4 on {} failed: {e}", path.display()))?;
        Ok(Self {
            fs: Mutex::new(Some(fs)),
            display: path.display().to_string(),
        })
    }

    /// Mount an **existing** ext4 at a byte slice of a whole disk (no mkfs).
    /// Used after flash and for GPT-based discovery on macOS (no OS mount).
    pub fn open_slice(disk_path: &Path, start_bytes: u64, size_bytes: u64) -> Result<Self> {
        let dev = SliceBlockDevice::open_slice(disk_path, start_bytes, size_bytes)
            .context("open partition slice for mount")?;
        let fs = Ext4Fs::mount(dev, false).map_err(|e| {
            anyhow::anyhow!(
                "mount ext4 slice {}@{}+{} failed: {e}",
                disk_path.display(),
                start_bytes,
                size_bytes
            )
        })?;
        Ok(Self {
            fs: Mutex::new(Some(fs)),
            display: format!(
                "ext4:{}@{}+{}MiB",
                disk_path.display(),
                start_bytes,
                size_bytes / (1024 * 1024)
            ),
        })
    }

    /// Directory-backed volume for tests / when the OS already mounted the FS.
    pub fn open_dir(root: impl Into<PathBuf>) -> Result<DirVolume> {
        DirVolume::open(root)
    }

    pub fn display_name(&self) -> &str {
        &self.display
    }

    fn with_fs<R>(&self, f: impl FnOnce(&Ext4Fs) -> Result<R>) -> Result<R> {
        let guard = self.fs.lock().unwrap();
        let fs = guard
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("filesystem already closed"))?;
        f(fs)
    }

    fn with_fs_mut<R>(&self, f: impl FnOnce(&Ext4Fs) -> Result<R>) -> Result<R> {
        // Ext4Fs methods take &self for open/write
        self.with_fs(f)
    }

    pub fn list(&self, rel: &str) -> Result<Vec<crate::volume::DirEntry>> {
        let path = fs_path(rel);
        self.with_fs(|fs| {
            let mut out = Vec::new();
            let dir = fs
                .open_dir(&path)
                .map_err(|e| anyhow::anyhow!("readdir {path}: {e}"))?;
            for ent in dir {
                let ent = ent.map_err(|e| anyhow::anyhow!("dir entry: {e}"))?;
                let name = ent.name().to_string();
                if name == "." || name == ".." {
                    continue;
                }
                let is_dir = matches!(ent.file_type(), ext4_lwext4::FileType::Directory);
                // Dir entries don't include size — look up metadata (opens file briefly).
                let child = if path == "/" {
                    format!("/{name}")
                } else {
                    format!("{path}/{name}")
                };
                let size = if is_dir {
                    0
                } else {
                    fs.metadata(&child).map(|m| m.size).unwrap_or(0)
                };
                out.push(crate::volume::DirEntry {
                    name,
                    is_dir,
                    size,
                });
            }
            out.sort_by(|a, b| a.name.cmp(&b.name));
            Ok(out)
        })
    }

    pub fn read_text(&self, rel: &str) -> Result<String> {
        let path = fs_path(rel);
        self.with_fs(|fs| {
            let mut file = fs
                .open(&path, OpenFlags::READ)
                .map_err(|e| anyhow::anyhow!("open {path}: {e}"))?;
            let mut buf = Vec::new();
            std::io::Read::read_to_end(&mut file, &mut buf)
                .map_err(|e| anyhow::anyhow!("read {path}: {e}"))?;
            String::from_utf8(buf).map_err(|e| anyhow::anyhow!("utf8 {path}: {e}"))
        })
    }

    pub fn write_text(&self, rel: &str, text: &str) -> Result<()> {
        let path = fs_path(rel);
        self.with_fs_mut(|fs| {
            // remove existing
            let _ = fs.remove(&path);
            let mut file = fs
                .open(
                    &path,
                    OpenFlags::CREATE | OpenFlags::WRITE | OpenFlags::TRUNCATE,
                )
                .map_err(|e| anyhow::anyhow!("create {path}: {e}"))?;
            std::io::Write::write_all(&mut file, text.as_bytes())
                .map_err(|e| anyhow::anyhow!("write {path}: {e}"))?;
            Ok(())
        })
    }

    pub fn copy_from_host(&self, host: &Path, rel: &str) -> Result<u64> {
        let data = std::fs::read(host).with_context(|| format!("read {}", host.display()))?;
        let n = data.len() as u64;
        let path = fs_path(rel);
        self.with_fs_mut(|fs| {
            let _ = fs.remove(&path);
            let mut file = fs
                .open(
                    &path,
                    OpenFlags::CREATE | OpenFlags::WRITE | OpenFlags::TRUNCATE,
                )
                .map_err(|e| anyhow::anyhow!("create {path}: {e}"))?;
            const CHUNK: usize = 1024 * 1024;
            for chunk in data.chunks(CHUNK) {
                std::io::Write::write_all(&mut file, chunk)
                    .map_err(|e| anyhow::anyhow!("write {path}: {e}"))?;
            }
            Ok(n)
        })
    }

    pub fn copy_to_host(&self, rel: &str, host: &Path) -> Result<u64> {
        let path = fs_path(rel);
        let data = self.with_fs(|fs| {
            let mut file = fs
                .open(&path, OpenFlags::READ)
                .map_err(|e| anyhow::anyhow!("open {path}: {e}"))?;
            let mut buf = Vec::new();
            std::io::Read::read_to_end(&mut file, &mut buf)
                .map_err(|e| anyhow::anyhow!("read {path}: {e}"))?;
            Ok(buf)
        })?;
        if let Some(parent) = host.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(host, &data)?;
        Ok(data.len() as u64)
    }

    pub fn remove(&self, rel: &str) -> Result<()> {
        let path = fs_path(rel);
        self.with_fs(|fs| {
            // try file then dir
            if fs.remove(&path).is_err() {
                fs.rmdir(&path)
                    .map_err(|e| anyhow::anyhow!("rm {path}: {e}"))?;
            }
            Ok(())
        })
    }

    pub fn rename(&self, from: &str, to: &str) -> Result<()> {
        let a = fs_path(from);
        let b = fs_path(to);
        self.with_fs(|fs| {
            fs.rename(&a, &b)
                .map_err(|e| anyhow::anyhow!("rename {a} -> {b}: {e}"))
        })
    }

    pub fn exists(&self, rel: &str) -> bool {
        let path = fs_path(rel);
        self.with_fs(|fs| {
            Ok(fs
                .open(&path, OpenFlags::READ)
                .map(|_| true)
                .unwrap_or(false))
        })
        .unwrap_or(false)
    }

    pub fn put_image(&self, host: &Path) -> Result<u64> {
        self.copy_from_host(host, IMAGE_NAME)
    }

    pub fn ensure_config(&self) -> Result<String> {
        if self.exists(CONFIG_NAME) {
            self.read_text(CONFIG_NAME)
        } else {
            let text = crate::schema::default_config_toml();
            self.write_text(CONFIG_NAME, text)?;
            Ok(text.to_string())
        }
    }

    pub fn seed(&self, image: Option<&Path>) -> Result<()> {
        self.write_text(CONFIG_NAME, crate::schema::default_config_toml())?;
        if let Some(img) = image {
            if img.is_file() {
                self.put_image(img)?;
            }
        }
        Ok(())
    }

    pub fn close(self) -> Result<()> {
        let mut guard = self.fs.lock().unwrap();
        if let Some(fs) = guard.take() {
            fs.umount()
                .map_err(|e| anyhow::anyhow!("umount ext4: {e}"))?;
        }
        Ok(())
    }
}

fn fs_path(rel: &str) -> String {
    let r = rel.trim().trim_start_matches('/');
    if r.is_empty() {
        "/".into()
    } else {
        format!("/{r}")
    }
}

/// Simple directory backend for unit tests (no lwext4).
pub struct DirVolume {
    root: PathBuf,
}

impl DirVolume {
    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        if !root.is_dir() {
            bail!("{} is not a directory", root.display());
        }
        Ok(Self { root })
    }

    pub fn display_name(&self) -> String {
        self.root.display().to_string()
    }

    pub fn list(&self, rel: &str) -> Result<Vec<crate::volume::DirEntry>> {
        let dir = self.root.join(rel.trim_start_matches('/'));
        let mut out = Vec::new();
        for ent in std::fs::read_dir(&dir)? {
            let ent = ent?;
            let meta = ent.metadata()?;
            out.push(crate::volume::DirEntry {
                name: ent.file_name().to_string_lossy().into_owned(),
                is_dir: meta.is_dir(),
                size: meta.len(),
            });
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    pub fn read_text(&self, rel: &str) -> Result<String> {
        Ok(std::fs::read_to_string(
            self.root.join(rel.trim_start_matches('/')),
        )?)
    }

    pub fn write_text(&self, rel: &str, text: &str) -> Result<()> {
        let path = self.root.join(rel.trim_start_matches('/'));
        if let Some(p) = path.parent() {
            std::fs::create_dir_all(p)?;
        }
        std::fs::write(path, text)?;
        Ok(())
    }

    pub fn copy_from_host(&self, host: &Path, rel: &str) -> Result<u64> {
        let dest = self.root.join(rel.trim_start_matches('/'));
        if let Some(p) = dest.parent() {
            std::fs::create_dir_all(p)?;
        }
        Ok(std::fs::copy(host, dest)?)
    }

    pub fn copy_to_host(&self, rel: &str, host: &Path) -> Result<u64> {
        if let Some(p) = host.parent() {
            std::fs::create_dir_all(p)?;
        }
        Ok(std::fs::copy(
            self.root.join(rel.trim_start_matches('/')),
            host,
        )?)
    }

    pub fn remove(&self, rel: &str) -> Result<()> {
        let path = self.root.join(rel.trim_start_matches('/'));
        if path.is_dir() {
            std::fs::remove_dir_all(path)?;
        } else {
            std::fs::remove_file(path)?;
        }
        Ok(())
    }

    pub fn rename(&self, from: &str, to: &str) -> Result<()> {
        std::fs::rename(
            self.root.join(from.trim_start_matches('/')),
            self.root.join(to.trim_start_matches('/')),
        )?;
        Ok(())
    }

    pub fn exists(&self, rel: &str) -> bool {
        self.root.join(rel.trim_start_matches('/')).exists()
    }

    pub fn put_image(&self, host: &Path) -> Result<u64> {
        self.copy_from_host(host, IMAGE_NAME)
    }

    pub fn ensure_config(&self) -> Result<String> {
        if self.exists(CONFIG_NAME) {
            self.read_text(CONFIG_NAME)
        } else {
            let text = crate::schema::default_config_toml();
            self.write_text(CONFIG_NAME, text)?;
            Ok(text.to_string())
        }
    }

    pub fn seed(&self, image: Option<&Path>) -> Result<()> {
        self.write_text(CONFIG_NAME, crate::schema::default_config_toml())?;
        if let Some(img) = image {
            if img.is_file() {
                self.put_image(img)?;
            }
        }
        Ok(())
    }

    pub fn close(self) -> Result<()> {
        Ok(())
    }
}

/// Format + seed using bundled lwext4 on a GPT partition slice.
pub fn format_seed_slice(
    disk: &Path,
    start_bytes: u64,
    size_bytes: u64,
    image: Option<&Path>,
) -> Result<()> {
    let vol = Ext4Volume::format_and_mount_slice(disk, start_bytes, size_bytes)?;
    vol.seed(image)?;
    vol.close()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn bundled_lwext4_mkfs_seed_and_read() {
        let img = std::env::temp_dir().join(format!("nq-lwext4-{}.img", std::process::id()));
        let _ = std::fs::remove_file(&img);
        // 64 MiB image
        {
            let f = std::fs::File::create(&img).unwrap();
            f.set_len(64 * 1024 * 1024).unwrap();
        }
        format_seed_slice(&img, 0, 64 * 1024 * 1024, None).expect("format+seed");

        let vol = Ext4Volume::open_device(&img).expect("open");
        assert!(vol.exists(CONFIG_NAME));
        let text = vol.read_text(CONFIG_NAME).unwrap();
        assert!(text.contains("image.qcow2"));
        let host = std::env::temp_dir().join(format!("nq-guest-{}", std::process::id()));
        {
            let mut f = std::fs::File::create(&host).unwrap();
            f.write_all(b"guest-image-bytes").unwrap();
        }
        vol.put_image(&host).unwrap();
        assert!(vol.exists(IMAGE_NAME));
        vol.close().unwrap();
        let _ = std::fs::remove_file(&img);
        let _ = std::fs::remove_file(&host);
    }

    #[test]
    fn dir_volume_seed() {
        let dir = std::env::temp_dir().join(format!("nq-dirvol-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let v = DirVolume::open(&dir).unwrap();
        v.seed(None).unwrap();
        assert!(v.exists(CONFIG_NAME));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
