//! Block device wrapper for macOS/Linux raw disks.
//!
//! macOS `/dev/rdiskN` constraints:
//! - `SeekFrom::End` → ENOTTY (25) — we synthesize End seeks from a known size
//! - `File::sync_all` / F_FULLFSYNC → ENOTTY — use plain fsync and ignore ENOTTY
//! - **Unaligned or non-multiple-of-sector R/W → EINVAL (22)** — all I/O is
//!   rounded to 512-byte sectors with read-modify-write when needed
//!
//! Alpine hybrid ISOs + the `gpt` crate read 92-byte headers; without sector
//! buffering that fails immediately with "Invalid argument (os error 22)".

use anyhow::{bail, Context, Result};
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// Physical sector size we use for raw device I/O (512 is universal for USB).
pub const SECTOR: u64 = 512;

/// Open a whole-disk device for raw R/W with known size + sector-aligned I/O.
#[derive(Debug)]
pub struct SizedDisk {
    file: File,
    size: u64,
    /// Logical stream position (may be unaligned).
    pos: u64,
    path: PathBuf,
}

impl SizedDisk {
    /// Prefer `/dev/rdiskN` on macOS for raw I/O; size from diskutil.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_rw(path)
    }

    pub fn open_rw(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_with(path, true)
    }

    /// Read-only open (verify path). Still needs the disk unmounted on macOS.
    pub fn open_ro(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_with(path, false)
    }

    fn open_with(path: impl AsRef<Path>, writable: bool) -> Result<Self> {
        let path = path.as_ref();
        let size = probe_size(path)?;
        if size == 0 {
            bail!("could not determine size of {}", path.display());
        }
        // Round size down to sector — never I/O past a partial trailing sector.
        let size = (size / SECTOR) * SECTOR;
        let open_path = prefer_raw(path);
        let mut opts = OpenOptions::new();
        opts.read(true);
        if writable {
            opts.write(true);
        }
        let file = opts.open(&open_path).with_context(|| {
            format!(
                "open {} for {}",
                open_path.display(),
                if writable { "R/W" } else { "read" }
            )
        })?;
        Ok(Self {
            file,
            size,
            pos: 0,
            path: open_path,
        })
    }

    pub fn size(&self) -> u64 {
        self.size
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn logical_path(&self) -> PathBuf {
        PathBuf::from(
            self.path
                .to_string_lossy()
                .replace("/dev/rdisk", "/dev/disk"),
        )
    }

    /// Read exactly `buf.len()` bytes at absolute offset (sector-aligned under the hood).
    pub fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        if buf.is_empty() {
            return Ok(());
        }
        if offset.saturating_add(buf.len() as u64) > self.size {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "read past end of device",
            ));
        }
        let start = (offset / SECTOR) * SECTOR;
        let end = ((offset + buf.len() as u64 + SECTOR - 1) / SECTOR) * SECTOR;
        let end = end.min(self.size);
        let len = (end - start) as usize;
        let mut tmp = vec![0u8; len];
        self.file.seek(SeekFrom::Start(start))?;
        self.file.read_exact(&mut tmp)?;
        let off = (offset - start) as usize;
        buf.copy_from_slice(&tmp[off..off + buf.len()]);
        Ok(())
    }

    /// Write `buf` at absolute offset using sector RMW when needed.
    pub fn write_at(&mut self, offset: u64, buf: &[u8]) -> io::Result<()> {
        if buf.is_empty() {
            return Ok(());
        }
        if offset.saturating_add(buf.len() as u64) > self.size {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "write past end of device",
            ));
        }
        let start = (offset / SECTOR) * SECTOR;
        let end = ((offset + buf.len() as u64 + SECTOR - 1) / SECTOR) * SECTOR;
        let end = end.min(self.size);
        let len = (end - start) as usize;
        let mut tmp = vec![0u8; len];

        // RMW if write is not a full aligned multi-sector block covering tmp exactly
        // from a pure overwrite of whole sectors starting mid-sector or partial end.
        let fully_aligned = offset % SECTOR == 0 && (buf.len() as u64) % SECTOR == 0;
        if !fully_aligned {
            self.file.seek(SeekFrom::Start(start))?;
            self.file.read_exact(&mut tmp)?;
        } else {
            // Still need to fill if start is aligned but we write a subset of sectors
            // Actually fully aligned means we can write buf directly without RMW.
            self.file.seek(SeekFrom::Start(offset))?;
            self.file.write_all(buf)?;
            return Ok(());
        }

        let off = (offset - start) as usize;
        tmp[off..off + buf.len()].copy_from_slice(buf);
        self.file.seek(SeekFrom::Start(start))?;
        self.file.write_all(&tmp)?;
        Ok(())
    }
}

impl Read for SizedDisk {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.pos >= self.size || buf.is_empty() {
            return Ok(0);
        }
        let max = ((self.size - self.pos) as usize).min(buf.len());
        self.read_at(self.pos, &mut buf[..max])?;
        self.pos += max as u64;
        Ok(max)
    }
}

impl Write for SizedDisk {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.pos >= self.size || buf.is_empty() {
            return Ok(0);
        }
        let max = ((self.size - self.pos) as usize).min(buf.len());
        self.write_at(self.pos, &buf[..max])?;
        self.pos += max as u64;
        Ok(max)
    }

    fn flush(&mut self) -> io::Result<()> {
        crate::syncutil::safe_sync(&mut self.file)
    }
}

impl Seek for SizedDisk {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let new_pos: u64 = match pos {
            SeekFrom::Start(p) => p,
            SeekFrom::Current(off) => {
                let n = (self.pos as i64).checked_add(off).ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidInput, "seek overflow")
                })?;
                if n < 0 {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "seek before start",
                    ));
                }
                n as u64
            }
            SeekFrom::End(off) => {
                // Never OS SEEK_END on macOS rdisk (ENOTTY).
                let n = (self.size as i64).checked_add(off).ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidInput, "seek end overflow")
                })?;
                if n < 0 {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "seek before start",
                    ));
                }
                n as u64
            }
        };
        if new_pos > self.size {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                format!(
                    "seek {} past device size {} ({})",
                    new_pos,
                    self.size,
                    self.path.display()
                ),
            ));
        }
        // Only need OS seek when we next do aligned I/O; track logically.
        self.pos = new_pos;
        Ok(new_pos)
    }
}

fn prefer_raw(path: &Path) -> PathBuf {
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

pub fn probe_size(path: &Path) -> Result<u64> {
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
            if line.contains("Disk Size:") || line.contains("Total Size:") {
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
        bail!("diskutil did not report size for {logical}");
    }
    #[cfg(target_os = "linux")]
    {
        use std::process::Command;
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let sys = format!("/sys/block/{name}/size");
        if let Ok(s) = std::fs::read_to_string(&sys) {
            if let Ok(sectors) = s.trim().parse::<u64>() {
                return Ok(sectors * 512);
            }
        }
        if let Ok(out) = Command::new("blockdev")
            .args(["--getsize64", path.to_str().unwrap()])
            .output()
        {
            if out.status.success() {
                if let Ok(n) = String::from_utf8_lossy(&out.stdout).trim().parse::<u64>() {
                    return Ok(n);
                }
            }
        }
        if let Ok(meta) = std::fs::metadata(path) {
            if meta.is_file() && meta.len() > 0 {
                return Ok(meta.len());
            }
        }
        bail!("could not determine size of {}", path.display());
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        Ok(std::fs::metadata(path)?.len())
    }
}
