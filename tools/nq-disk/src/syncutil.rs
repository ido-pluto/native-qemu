//! Safe fsync for raw disks.
//!
//! On macOS, `File::sync_all()` uses `F_FULLFSYNC`, which returns **ENOTTY
//! (os error 25)** on `/dev/rdiskN`. Treat that as success after a best-effort
//! flush/`fsync`.

use std::fs::File;
use std::io::{self, Write};
use std::os::fd::AsRawFd;

/// Flush + fsync a file/device; ignore ENOTTY/EINVAL which raw disks often return.
pub fn safe_sync(file: &mut File) -> io::Result<()> {
    let _ = file.flush();
    #[cfg(unix)]
    {
        let fd = file.as_raw_fd();
        // Prefer plain fsync over F_FULLFSYNC (what Rust sync_all uses on macOS).
        let rc = unsafe { libc::fsync(fd) };
        if rc != 0 {
            let err = io::Error::last_os_error();
            match err.raw_os_error() {
                // Inappropriate ioctl / not a tty / invalid for this device
                Some(libc::ENOTTY) | Some(libc::EINVAL) => return Ok(()),
                _ => return Err(err),
            }
        }
        return Ok(());
    }
    #[cfg(not(unix))]
    {
        match file.sync_all() {
            Ok(()) => Ok(()),
            Err(e) if e.raw_os_error() == Some(25) || e.raw_os_error() == Some(22) => Ok(()),
            Err(e) => Err(e),
        }
    }
}

pub fn safe_sync_path(path: &std::path::Path) -> io::Result<()> {
    let mut f = std::fs::OpenOptions::new().write(true).open(path)?;
    safe_sync(&mut f)
}
