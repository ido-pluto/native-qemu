//! Block-device adapters for bundled lwext4.
//!
//! We talk to whole disks and partition slices with pure Rust I/O; the
//! filesystem itself is handled by the vendored lwext4 C library.

use anyhow::{bail, Context, Result};
use ext4_lwext4::BlockDevice;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::Mutex;

const DEFAULT_BLOCK: u32 = 512;

/// A byte-range on a host file/device, presented as an lwext4 block device.
///
/// Used for the data partition after GPT is written: we do not depend on the
/// OS creating `/dev/diskNs2` before we can mkfs and seed files.
pub struct SliceBlockDevice {
    file: Mutex<File>,
    /// Absolute byte offset of block 0 on the host device.
    start_bytes: u64,
    block_size: u32,
    block_count: u64,
}

impl SliceBlockDevice {
    /// Open a whole-disk or partition node and use the entire device size.
    pub fn open_path(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let file = crate::rawdisk::open_raw(path, true)
            .with_context(|| format!("open block device {}", path.display()))?;
        let open_path = crate::rawdisk::prefer_raw(path);
        let size = device_size(&file, &open_path)?;
        if size < 1024 * 1024 {
            bail!("{} is too small ({size} bytes)", path.display());
        }
        let block_count = size / DEFAULT_BLOCK as u64;
        Ok(Self {
            file: Mutex::new(file),
            start_bytes: 0,
            block_size: DEFAULT_BLOCK,
            block_count,
        })
    }

    /// Open a whole disk and address only `[start_bytes, start_bytes + size_bytes)`.
    pub fn open_slice(path: impl AsRef<Path>, start_bytes: u64, size_bytes: u64) -> Result<Self> {
        let path = path.as_ref();
        // Retries + unmount: macOS remounts hybrid ISO volumes between GPT and mkfs.
        let file = crate::rawdisk::open_raw(path, true)
            .with_context(|| format!("open {}", path.display()))?;
        if size_bytes < 1024 * 1024 {
            bail!("partition slice too small ({size_bytes} bytes)");
        }
        if start_bytes % DEFAULT_BLOCK as u64 != 0 {
            bail!("partition start {start_bytes} is not sector-aligned");
        }
        let block_count = size_bytes / DEFAULT_BLOCK as u64;
        Ok(Self {
            file: Mutex::new(file),
            start_bytes,
            block_size: DEFAULT_BLOCK,
            block_count,
        })
    }

    pub fn total_size(&self) -> u64 {
        self.block_count * self.block_size as u64
    }
}

impl BlockDevice for SliceBlockDevice {
    fn read_blocks(
        &self,
        block_id: u64,
        buf: &mut [u8],
    ) -> ext4_lwext4::Result<u32> {
        let nblocks = (buf.len() as u32) / self.block_size;
        if nblocks == 0 {
            return Ok(0);
        }
        if block_id + nblocks as u64 > self.block_count {
            return Err(ext4_lwext4::Error::Io(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "read past end of partition",
            )));
        }
        // lwext4 always uses full blocks; size is a multiple of block_size.
        // On macOS, still ensure the host transfer is sector-aligned (block_size
        // is 512).
        let offset = self.start_bytes + block_id * self.block_size as u64;
        let mut file = self.file.lock().map_err(|_| {
            ext4_lwext4::Error::Io(std::io::Error::other("device lock poisoned"))
        })?;
        file.seek(SeekFrom::Start(offset)).map_err(ext4_lwext4::Error::Io)?;
        // read_exact of N*512 is fine on rdisk when N*512 is the transfer size.
        file.read_exact(buf).map_err(ext4_lwext4::Error::Io)?;
        Ok(nblocks)
    }

    fn write_blocks(&mut self, block_id: u64, buf: &[u8]) -> ext4_lwext4::Result<u32> {
        let nblocks = (buf.len() as u32) / self.block_size;
        if nblocks == 0 {
            return Ok(0);
        }
        if block_id + nblocks as u64 > self.block_count {
            return Err(ext4_lwext4::Error::Io(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "write past end of partition",
            )));
        }
        let offset = self.start_bytes + block_id * self.block_size as u64;
        let mut file = self.file.lock().map_err(|_| {
            ext4_lwext4::Error::Io(std::io::Error::other("device lock poisoned"))
        })?;
        file.seek(SeekFrom::Start(offset)).map_err(ext4_lwext4::Error::Io)?;
        file.write_all(buf).map_err(ext4_lwext4::Error::Io)?;
        Ok(nblocks)
    }

    fn flush(&mut self) -> ext4_lwext4::Result<()> {
        let mut file = self.file.lock().map_err(|_| {
            ext4_lwext4::Error::Io(std::io::Error::other("device lock poisoned"))
        })?;
        crate::syncutil::safe_sync(&mut file).map_err(ext4_lwext4::Error::Io)?;
        Ok(())
    }

    fn block_size(&self) -> u32 {
        self.block_size
    }

    fn block_count(&self) -> u64 {
        self.block_count
    }
}

fn device_size(file: &File, path: &Path) -> Result<u64> {
    // Prefer shared probe (diskutil / sysfs) — no SEEK_END on block devices.
    if let Ok(n) = crate::sized_disk::probe_size(path) {
        if n > 0 {
            return Ok(n);
        }
    }
    // Regular files: metadata length.
    if let Ok(meta) = file.metadata() {
        if meta.len() > 0 && meta.file_type().is_file() {
            return Ok(meta.len());
        }
    }
    // Block devices: platform-specific size probe.
    #[cfg(target_os = "macos")]
    {
        use std::process::Command;
        let logical = path
            .to_string_lossy()
            .replace("/dev/rdisk", "/dev/disk");
        let out = Command::new("diskutil")
            .args(["info", &logical])
            .output()
            .context("diskutil info")?;
        let text = String::from_utf8_lossy(&out.stdout);
        for line in text.lines() {
            if line.contains("Disk Size:") || line.contains("Total Size:") || line.contains("Volume Total Space:") {
                if let Some(start) = line.find('(') {
                    let rest = &line[start + 1..];
                    if let Some(num) = rest.split_whitespace().next() {
                        if let Ok(n) = num.parse::<u64>() {
                            return Ok(n);
                        }
                    }
                }
            }
        }
        // Never SeekFrom::End on macOS block devices — returns ENOTTY (os error 25).
        bail!("could not determine size of {} via diskutil", path.display());
    }
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::io::AsRawFd;
        // BLKGETSIZE64 = 0x80081272 on linux
        let mut size: u64 = 0;
        let fd = file.as_raw_fd();
        let rc = unsafe { libc::ioctl(fd, 0x80081272u64 as _, &mut size as *mut u64) };
        if rc == 0 && size > 0 {
            return Ok(size);
        }
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        // strip partition suffix for sysfs on whole disks; for partitions use size file
        let sys = format!("/sys/class/block/{name}/size");
        if let Ok(s) = std::fs::read_to_string(&sys) {
            if let Ok(sectors) = s.trim().parse::<u64>() {
                return Ok(sectors * 512);
            }
        }
        bail!("could not determine size of {}", path.display());
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let mut f = file.try_clone()?;
        Ok(f.seek(SeekFrom::End(0))?)
    }
}
