use crate::config::LoggingConfig;
use crate::storage;
use std::fs::OpenOptions;
use std::os::unix::io::AsRawFd;
use std::path::PathBuf;

/// Resolves logging.storage/path to a real file path, falling back to
/// /var/log/native-qemu.log (tmpfs, lost on reboot) if the configured
/// storage can't be resolved — logging should never be the reason boot
/// fails.
fn resolve_log_path(cfg: &LoggingConfig) -> PathBuf {
    if cfg.enabled {
        if let Ok(mountpoint) = storage::resolve(cfg.storage) {
            let dir = mountpoint.join(&cfg.path);
            if std::fs::create_dir_all(&dir).is_ok() {
                return dir.join("native-qemu.log");
            }
        }
    }
    PathBuf::from("/var/log/native-qemu.log")
}

/// Keeps the active log bounded before opening it.  `rotate = 0` disables
/// rotation; otherwise `.1` is the most recent previous log and older files
/// are shifted up to `.rotate`.  Rotation failures are intentionally
/// non-fatal: logging must never prevent the appliance from booting.
fn rotate_if_needed(path: &PathBuf, cfg: &LoggingConfig) {
    let Some(max_size) = crate::config::parse_size(&cfg.max_size) else {
        return;
    };
    if cfg.rotate == 0
        || std::fs::metadata(path)
            .map(|metadata| metadata.len() < max_size)
            .unwrap_or(true)
    {
        return;
    }

    let oldest = PathBuf::from(format!("{}.{}", path.display(), cfg.rotate));
    let _ = std::fs::remove_file(oldest);
    for index in (1..cfg.rotate).rev() {
        let from = PathBuf::from(format!("{}.{}", path.display(), index));
        let to = PathBuf::from(format!("{}.{}", path.display(), index + 1));
        if from.exists() {
            let _ = std::fs::rename(from, to);
        }
    }
    if path.exists() {
        let _ = std::fs::rename(path, format!("{}.1", path.display()));
    }
}

/// Redirects this process's stdout and stderr (fds 1 and 2) to the resolved
/// log file, so our own eprintln!/println! output *and* anything inherited
/// by child processes we spawn (qemu, hook scripts) end up in one place.
/// Set NATIVE_QEMU_NO_LOG_REDIRECT=1 (e.g. when running interactively from
/// a console for debugging) to keep output on the current tty instead.
pub fn init(cfg: &LoggingConfig) -> PathBuf {
    let path = resolve_log_path(cfg);
    if std::env::var_os("NATIVE_QEMU_NO_LOG_REDIRECT").is_some() {
        println!("native-qemu: NATIVE_QEMU_NO_LOG_REDIRECT set, logging to this console instead of {path:?}");
        return path;
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    rotate_if_needed(&path, cfg);
    match OpenOptions::new().create(true).append(true).open(&path) {
        Ok(file) => {
            let fd = file.as_raw_fd();
            unsafe {
                libc::dup2(fd, libc::STDOUT_FILENO);
                libc::dup2(fd, libc::STDERR_FILENO);
            }
            // Leak the File so the fd stays valid — it now *is* fd 1/2.
            std::mem::forget(file);
        }
        Err(e) => {
            eprintln!("native-qemu: could not open log file {path:?}: {e}, logging to stderr only");
        }
    }
    path
}

#[cfg(test)]
mod tests {
    use super::rotate_if_needed;
    use crate::config::LoggingConfig;

    #[test]
    fn rotates_an_oversized_log() {
        let dir = std::env::temp_dir().join(format!("native-qemu-log-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join("native-qemu.log");
        std::fs::write(&log, b"12345").unwrap();
        let cfg = LoggingConfig {
            enabled: true,
            storage: 1,
            path: String::new(),
            max_size: "4".into(),
            rotate: 2,
        };

        rotate_if_needed(&log, &cfg);
        assert!(!log.exists());
        assert_eq!(
            std::fs::read(log.with_extension("log.1")).unwrap(),
            b"12345"
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
