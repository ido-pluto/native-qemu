//! Host timezone resolution for the appliance.
//!
//! QEMU is started with `-rtc base=localtime`, so the guest CMOS follows the
//! **host** local clock. Setting `system.timezone` updates the host zone before
//! QEMU starts (Texas Central = America/Chicago by default when auto has
//! nothing better).

use std::fs;
use std::path::{Path, PathBuf};

/// IANA zone used when `timezone = "auto"` and the host has no usable zone.
/// Covers most of Texas (Dallas / Houston / Austin / San Antonio).
/// Far-west Texas (El Paso) is Mountain: America/Denver.
pub const DEFAULT_TEXAS_ZONE: &str = "America/Chicago";

const ZONEINFO_ROOT: &str = "/usr/share/zoneinfo";

/// Reject empty, absolute, traversal, NUL, and non-IANA-like characters.
/// Returns the trimmed name on success.
pub fn sanitize_zone_name(name: &str) -> Option<&str> {
    let t = name.trim();
    if t.is_empty() || t.contains('\0') || t.starts_with('/') || t.contains("..") {
        return None;
    }
    // IANA zones use ASCII letters, digits, `/`, `_`, `+`, `-` (e.g. Etc/GMT+6).
    if !t.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '_' | '+' | '-'))
    {
        return None;
    }
    if t.split('/').any(|part| part.is_empty()) {
        return None;
    }
    Some(t)
}

/// Config-time check for `system.timezone`.
///
/// - `"auto"` / empty → ok
/// - otherwise must sanitize, and if zoneinfo is present the file must exist
///   under `/usr/share/zoneinfo` (hard-fail typos when tzdata is installed)
pub fn validate_configured(configured: &str) -> Result<(), String> {
    let t = configured.trim();
    if t.is_empty() || t.eq_ignore_ascii_case("auto") {
        return Ok(());
    }
    let name = sanitize_zone_name(t).ok_or_else(|| {
        format!(
            "system.timezone must be \"auto\" or an IANA name like \"America/Chicago\" \
             (got {t:?})"
        )
    })?;
    if Path::new(ZONEINFO_ROOT).is_dir() {
        zoneinfo_file(name).map_err(|e| format!("system.timezone: {e}"))?;
    }
    Ok(())
}

/// Resolve config timezone to a concrete IANA name.
///
/// - empty / `"auto"` → detect host zone, else [`DEFAULT_TEXAS_ZONE`]
/// - any other string → sanitized name (must exist under zoneinfo when present)
pub fn resolve(configured: &str) -> Result<String, String> {
    let t = configured.trim();
    if t.is_empty() || t.eq_ignore_ascii_case("auto") {
        Ok(detect_host_timezone().unwrap_or_else(|| DEFAULT_TEXAS_ZONE.to_string()))
    } else {
        let name = sanitize_zone_name(t).ok_or_else(|| {
            format!("invalid timezone name {t:?} (use auto or an IANA zone)")
        })?;
        Ok(name.to_string())
    }
}

/// Best-effort read of the current host timezone (sanitized only).
pub fn detect_host_timezone() -> Option<String> {
    if let Ok(tz) = std::env::var("TZ") {
        if let Some(name) = sanitize_zone_name(tz.trim()) {
            if !name.eq_ignore_ascii_case("auto") && zoneinfo_file(name).is_ok() {
                return Some(name.to_string());
            }
        }
    }
    if let Ok(text) = fs::read_to_string("/etc/timezone") {
        if let Some(name) = sanitize_zone_name(text.trim()) {
            if zoneinfo_file(name).is_ok() {
                return Some(name.to_string());
            }
        }
    }
    // systemd-style: /etc/localtime → …/zoneinfo/Region/City
    if let Ok(target) = fs::read_link("/etc/localtime") {
        if let Some(name) = zone_name_from_zoneinfo_path(&target) {
            if sanitize_zone_name(&name).is_some() && zoneinfo_file(&name).is_ok() {
                return Some(name);
            }
        }
    }
    // Some images copy the file instead of symlinking; still try canonical path.
    if let Ok(canon) = fs::canonicalize("/etc/localtime") {
        if let Some(name) = zone_name_from_zoneinfo_path(&canon) {
            if sanitize_zone_name(&name).is_some() && zoneinfo_file(&name).is_ok() {
                return Some(name);
            }
        }
    }
    None
}

/// Resolve zoneinfo path; require it stays under `/usr/share/zoneinfo`.
fn zoneinfo_file(tz: &str) -> Result<PathBuf, String> {
    let name = sanitize_zone_name(tz).ok_or_else(|| format!("invalid timezone name {tz:?}"))?;
    let path = Path::new(ZONEINFO_ROOT).join(name);
    if !path.is_file() {
        return Err(format!(
            "timezone {name:?} not found at {} (install tzdata or pick another IANA name)",
            path.display()
        ));
    }
    let root = fs::canonicalize(ZONEINFO_ROOT).map_err(|e| {
        format!("cannot resolve {ZONEINFO_ROOT}: {e}")
    })?;
    let canon = fs::canonicalize(&path).map_err(|e| {
        format!("cannot resolve {}: {e}", path.display())
    })?;
    if !canon.starts_with(&root) {
        return Err(format!(
            "timezone {name:?} resolves outside {ZONEINFO_ROOT}"
        ));
    }
    Ok(path)
}

fn zone_name_from_zoneinfo_path(path: &Path) -> Option<String> {
    let s = path.to_string_lossy();
    const MARKER: &str = "/zoneinfo/";
    if let Some(idx) = s.find(MARKER) {
        let name = &s[idx + MARKER.len()..];
        if sanitize_zone_name(name).is_some() {
            return Some(name.to_string());
        }
    }
    None
}

/// Apply timezone on the appliance host so `localtime` matches the zone.
///
/// Writes `/etc/timezone` when possible, atomically updates `/etc/localtime`,
/// then sets process `TZ` only after localtime is in place (so soft-fail paths
/// never leave a mismatched env).
pub fn apply(tz: &str) -> Result<(), String> {
    let path = zoneinfo_file(tz)?;
    let name = sanitize_zone_name(tz).ok_or_else(|| format!("invalid timezone name {tz:?}"))?;

    // Persist for other host services when the rootfs is writable.
    if let Err(e) = fs::write("/etc/timezone", format!("{name}\n")) {
        eprintln!("native-qemu: warning: could not write /etc/timezone: {e}");
    }

    install_localtime(&path)?;

    // Only after localtime is updated — children (QEMU) inherit a consistent clock.
    // SAFETY: appliance startup is single-threaded before QEMU spawn.
    unsafe {
        std::env::set_var("TZ", name);
    }
    Ok(())
}

/// Atomically point `/etc/localtime` at `zone_path` (symlink preferred, else copy).
fn install_localtime(zone_path: &Path) -> Result<(), String> {
    let localtime = Path::new("/etc/localtime");
    let tmp = Path::new("/etc/localtime.native-qemu.tmp");

    // Clean any leftover temp from a previous crash.
    let _ = fs::remove_file(tmp);

    let prepared = match std::os::unix::fs::symlink(zone_path, tmp) {
        Ok(()) => true,
        Err(symlink_err) => match fs::copy(zone_path, tmp) {
            Ok(_) => true,
            Err(copy_err) => {
                let _ = fs::remove_file(tmp);
                return Err(format!(
                    "could not stage /etc/localtime from {} (symlink: {symlink_err}; copy: {copy_err})",
                    zone_path.display()
                ));
            }
        },
    };
    debug_assert!(prepared);

    // rename replaces the destination atomically on the same filesystem.
    fs::rename(tmp, localtime).map_err(|e| {
        let _ = fs::remove_file(tmp);
        format!(
            "could not install /etc/localtime from {} (rename: {e})",
            zone_path.display()
        )
    })?;
    Ok(())
}

/// Resolve + apply. Returns the concrete zone name used.
pub fn resolve_and_apply(configured: &str) -> Result<String, String> {
    let tz = resolve(configured)?;
    apply(&tz)?;
    Ok(tz)
}

/// True when the config uses auto detection (soft-fail apply is OK).
pub fn is_auto(configured: &str) -> bool {
    let t = configured.trim();
    t.is_empty() || t.eq_ignore_ascii_case("auto")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_falls_back_to_texas_central() {
        assert_eq!(resolve("America/Denver").unwrap(), "America/Denver");
        assert_eq!(resolve("  America/Chicago  ").unwrap(), "America/Chicago");
        let z = resolve("auto").unwrap();
        assert!(!z.is_empty());
        if detect_host_timezone().is_none() {
            assert_eq!(z, DEFAULT_TEXAS_ZONE);
        }
    }

    #[test]
    fn empty_means_auto() {
        let z = resolve("").unwrap();
        assert!(!z.is_empty());
    }

    #[test]
    fn sanitize_rejects_traversal_and_absolute() {
        assert!(sanitize_zone_name("../etc/passwd").is_none());
        assert!(sanitize_zone_name("/etc/localtime").is_none());
        assert!(sanitize_zone_name("America/../Chicago").is_none());
        assert!(sanitize_zone_name("America/Chicago").is_some());
        assert!(sanitize_zone_name("Asia/Jerusalem").is_some());
        assert!(sanitize_zone_name("Etc/GMT+6").is_some());
    }

    #[test]
    fn validate_configured_accepts_auto() {
        assert!(validate_configured("auto").is_ok());
        assert!(validate_configured("").is_ok());
    }

    #[test]
    fn validate_configured_rejects_traversal() {
        assert!(validate_configured("../../etc/passwd").is_err());
        assert!(validate_configured("/usr/share/zoneinfo/UTC").is_err());
    }

    #[test]
    fn zone_name_from_symlink_target() {
        assert_eq!(
            zone_name_from_zoneinfo_path(Path::new("/usr/share/zoneinfo/America/Chicago")),
            Some("America/Chicago".into())
        );
        assert_eq!(
            zone_name_from_zoneinfo_path(Path::new("/usr/share/zoneinfo/UTC")),
            Some("UTC".into())
        );
        assert_eq!(zone_name_from_zoneinfo_path(Path::new("/etc/localtime")), None);
    }

    #[test]
    fn resolve_rejects_bad_explicit_names() {
        assert!(resolve("../x").is_err());
        assert!(resolve("/UTC").is_err());
    }
}
