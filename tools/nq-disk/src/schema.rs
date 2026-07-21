//! Known config sections, keys, and enum values for completion + validation.

#[derive(Clone, Copy)]
pub struct Field {
    pub key: &'static str,
    pub values: &'static [&'static str],
    pub doc: &'static str,
}

#[derive(Clone, Copy)]
pub struct Section {
    pub name: &'static str,
    pub fields: &'static [Field],
}

pub const SECTIONS: &[Section] = &[
    Section {
        name: "vm",
        fields: &[
            Field {
                key: "arch",
                values: &["x86_64", "aarch64"],
                doc: "Host architecture (must match the machine)",
            },
            Field {
                key: "firmware",
                values: &["bios", "uefi"],
                doc: "BIOS (SeaBIOS) or UEFI (OVMF)",
            },
            Field {
                key: "machine",
                values: &["pc", "q35"],
                doc: "pc = i440fx (Win98/ReactOS); q35 = modern",
            },
            Field {
                key: "memory",
                values: &["512M", "1G", "2G", "4G", "8G"],
                doc: "Guest RAM",
            },
            Field {
                key: "cpu",
                values: &["host", "pentium3", "qemu64", "max"],
                doc: "host = best 3dfx/KVM (Win98); pentium3 = stricter legacy",
            },
            Field {
                key: "vcpus",
                values: &["1", "2", "4"],
                doc: "Number of vCPUs",
            },
            Field {
                key: "sockets",
                values: &["1", "2"],
                doc: "SMP sockets (0 = use vcpus only)",
            },
            Field {
                key: "cores",
                values: &["1", "2", "4"],
                doc: "Cores per socket",
            },
            Field {
                key: "threads",
                values: &["1", "2"],
                doc: "Threads per core",
            },
        ],
    },
    Section {
        name: "vm.disk",
        fields: &[
            Field {
                key: "format",
                values: &["qcow2", "raw"],
                doc: "Disk image format",
            },
            Field {
                key: "storage",
                values: &["0", "1", "2"],
                doc: "0=USB data volume, 1=internal, 2+=external",
            },
            Field {
                key: "path",
                values: &["image.qcow2"],
                doc: "Path on the storage volume (keep image.qcow2)",
            },
            Field {
                key: "bus",
                values: &["ide", "virtio", "scsi"],
                doc: "ide for Win98/ReactOS; virtio for modern guests",
            },
            Field {
                key: "cache",
                values: &["writeback", "none", "directsync"],
                doc: "QEMU cache mode",
            },
            Field {
                key: "discard",
                values: &["ignore", "unmap"],
                doc: "Discard/TRIM handling",
            },
        ],
    },
    Section {
        name: "network",
        fields: &[
            Field {
                key: "mode",
                values: &["user", "bridge", "macvtap"],
                doc: "user = NAT (simplest)",
            },
            Field {
                key: "model",
                values: &["rtl8139", "pcnet", "e1000", "virtio-net-pci"],
                doc: "rtl8139 default (Win98+ReactOS); pcnet also OK — NIC ≠ 3dfx FPS",
            },
            Field {
                key: "bridge_iface",
                values: &["br0", "eth0"],
                doc: "Host bridge or parent iface",
            },
        ],
    },
    Section {
        name: "sound",
        fields: &[
            Field {
                key: "enabled",
                values: &["true", "false"],
                doc: "Enable guest sound",
            },
            Field {
                key: "backend",
                values: &["alsa", "pipewire"],
                doc: "Host audio backend",
            },
            Field {
                key: "model",
                values: &["AC97", "sb16", "virtio-sound-pci", "ES1370"],
                doc: "AC97 best for Win98+3dfx; sb16 for older DOS-style",
            },
        ],
    },
    Section {
        name: "display",
        fields: &[
            Field {
                key: "backend",
                values: &["sdl", "none"],
                doc: "sdl = guest on physical screen",
            },
            Field {
                key: "vga",
                values: &["VGA", "cirrus", "std", "virtio-gpu-pci"],
                doc: "VGA+BOXV9x for Win98 3dfx; cirrus if you need inbox 2D only",
            },
            Field {
                key: "passthrough",
                values: &["both", "glide", "mesa", "none"],
                doc: "both preferred; glide|mesa|both same host flags (guest picks API); needs machine=pc",
            },
        ],
    },
    Section {
        name: "usb",
        fields: &[
            Field {
                key: "default",
                values: &["passthrough", "host-only"],
                doc: "Default policy for unlisted USB devices",
            },
            Field {
                key: "hotplug",
                values: &["true", "false"],
                doc: "Hot-add USB after VM start",
            },
        ],
    },
    Section {
        name: "lifecycle",
        fields: &[
            Field {
                key: "on_guest_shutdown",
                values: &["poweroff_host", "restart_vm", "drop_to_shell"],
                doc: "When guest shuts down cleanly",
            },
            Field {
                key: "on_guest_crash",
                values: &["drop_to_shell", "restart_vm", "poweroff_host"],
                doc: "When QEMU exits abnormally",
            },
            Field {
                key: "on_missing_resource",
                values: &["rescue_shell", "boot_anyway"],
                doc: "Missing disk or required USB",
            },
        ],
    },
    Section {
        name: "system",
        fields: &[
            Field {
                key: "hostname",
                values: &["native-qemu"],
                doc: "Appliance hostname",
            },
            Field {
                key: "timezone",
                // Values come from the host zoneinfo tree at runtime (see
                // timezone_suggestions) — every valid Linux IANA zone, e.g.
                // Asia/Jerusalem, Israel, America/Chicago, UTC, …
                values: &["auto"],
                doc: "auto | any Linux IANA zone; Texas first (America/Chicago); Israel = Asia/Jerusalem",
            },
            Field {
                key: "rtc_base",
                values: &["localtime", "utc"],
                doc: "localtime for Win98/ReactOS CMOS clock",
            },
            Field {
                key: "ssh_enabled",
                values: &["true", "false"],
                doc: "Start Dropbear SSH",
            },
        ],
    },
];

pub const SECTION_NAMES: &[&str] = &[
    "vm",
    "vm.disk",
    "network",
    "sound",
    "display",
    "usb",
    "lifecycle",
    "logging",
    "docs_server",
    "shared_folder",
    "smb",
    "system",
    "startup",
    "shutdown",
];

/// Context for completion at a cursor position in TOML text.
#[derive(Debug, Clone)]
pub enum CompleteContext {
    SectionHeader { prefix: String },
    Key { section: String, prefix: String },
    Value { section: String, key: String, prefix: String },
    Unknown,
}

pub fn completion_context(text: &str, cursor: usize) -> CompleteContext {
    let cursor = cursor.min(text.len());
    let before = &text[..cursor];
    let line_start = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
    let line = &before[line_start..];
    let trimmed = line.trim_start();

    // Current [section]
    let mut section = String::new();
    for l in before.lines() {
        let t = l.trim();
        if t.starts_with('[') && t.ends_with(']') && !t.starts_with("[[") {
            section = t.trim_matches(|c| c == '[' || c == ']').to_string();
        }
    }

    if trimmed.starts_with('[') {
        let prefix = trimmed
            .trim_start_matches('[')
            .trim_end_matches(']')
            .to_string();
        return CompleteContext::SectionHeader { prefix };
    }

    // Completions are **values only** (right-hand side after `=`), like VS Code
    // enum completion — never field/key names.
    if let Some(eq) = line.find('=') {
        let key = line[..eq].trim().to_string();
        // Text after '=' up to the cursor (line is already truncated at cursor).
        let after = line[eq + 1..].trim_start();
        // Strip a single optional opening quote for filtering.
        let prefix = after
            .strip_prefix('"')
            .unwrap_or(after)
            .trim_end_matches('"')
            .to_string();
        let eq_abs = line_start + eq;
        if cursor > eq_abs {
            return CompleteContext::Value {
                section,
                key,
                prefix,
            };
        }
    }

    CompleteContext::Unknown
}

#[derive(Debug, Clone)]
pub struct Suggestion {
    pub insert: String,
    pub label: String,
    pub doc: String,
}

pub fn suggestions(ctx: &CompleteContext) -> Vec<Suggestion> {
    match ctx {
        // Field names and section headers: no dropdown (value-only completion).
        CompleteContext::SectionHeader { .. } | CompleteContext::Key { .. } => Vec::new(),
        CompleteContext::Value {
            section,
            key,
            prefix,
        } if section == "system" && key == "timezone" => timezone_suggestions(prefix),
        CompleteContext::Value {
            section,
            key,
            prefix,
        } => SECTIONS
            .iter()
            .filter(|s| s.name == section)
            .flat_map(|s| s.fields.iter())
            .filter(|f| f.key == key)
            .flat_map(|f| f.values.iter().map(move |v| (f, *v)))
            .filter(|(_, v)| {
                // Case-insensitive filter for convenience
                let p = prefix.to_ascii_lowercase();
                let vv = v.to_ascii_lowercase();
                p.is_empty() || vv.starts_with(&p) || vv.contains(&p)
            })
            .map(|(f, v)| {
                let v: &str = v;
                let insert = if v == "true" || v == "false" || v.parse::<i64>().is_ok() {
                    v.to_string()
                } else {
                    format!("\"{v}\"")
                };
                Suggestion {
                    insert,
                    label: v.to_string(),
                    doc: f.doc.to_string(),
                }
            })
            .collect(),
        CompleteContext::Unknown => Vec::new(),
    }
}

/// Completions for `system.timezone`: `auto` plus every zone under
/// `/usr/share/zoneinfo` (full Linux/IANA set on this machine), including
/// Israel (`Asia/Jerusalem`, legacy `Israel`), US zones, UTC, etc.
fn timezone_suggestions(prefix: &str) -> Vec<Suggestion> {
    let doc = "auto | any Linux IANA zone from /usr/share/zoneinfo";
    let p = prefix.to_ascii_lowercase();
    let mut out = Vec::new();

    let mut push = |label: &str| {
        let ll = label.to_ascii_lowercase();
        if p.is_empty() || ll.starts_with(&p) || ll.contains(&p) {
            out.push(Suggestion {
                insert: format!("\"{label}\""),
                label: label.to_string(),
                doc: doc.to_string(),
            });
        }
    };

    push("auto");
    for z in list_system_timezones() {
        push(&z);
    }
    // Cap popup size when prefix is empty (full list is 500+ entries).
    // Order: auto → Texas first among pins → other common zones (one name per
    // region; Israel = Asia/Jerusalem only, not the legacy "Israel" alias).
    if p.is_empty() && out.len() > 80 {
        let priority = [
            "auto",
            "America/Chicago", // Texas Central (default auto fallback)
            "America/Denver",  // west Texas (El Paso)
            "America/New_York",
            "America/Los_Angeles",
            "UTC",
            "Asia/Jerusalem",
            "Europe/London",
            "Europe/Berlin",
            "Europe/Paris",
            "Asia/Tokyo",
            "Australia/Sydney",
        ];
        let mut kept: Vec<Suggestion> = Vec::new();
        for name in priority {
            if let Some(s) = out.iter().find(|s| s.label == name) {
                kept.push(s.clone());
            }
        }
        for s in out {
            if kept.len() >= 80 {
                break;
            }
            // Drop legacy "Israel" alias — prefer Asia/Jerusalem only.
            if s.label == "Israel" {
                continue;
            }
            if !kept.iter().any(|k| k.label == s.label) {
                kept.push(s);
            }
        }
        out = kept;
    } else {
        // Filtered list: hide legacy Israel alias when Jerusalem is present.
        if out.iter().any(|s| s.label == "Asia/Jerusalem") {
            out.retain(|s| s.label != "Israel");
        }
    }
    out
}

/// Walk host zoneinfo (same tree Linux uses). Skips posix/right copies and
/// metadata files. Returns sorted unique zone names (e.g. `Asia/Jerusalem`).
pub fn list_system_timezones() -> Vec<String> {
    let root = std::path::Path::new("/usr/share/zoneinfo");
    if !root.is_dir() {
        return fallback_timezone_list();
    }
    let mut zones = Vec::new();
    collect_zones(root, root, &mut zones);
    zones.sort();
    zones.dedup();
    if zones.is_empty() {
        return fallback_timezone_list();
    }
    zones
}

fn collect_zones(root: &std::path::Path, dir: &std::path::Path, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for ent in entries.flatten() {
        let name = ent.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.')
            || name == "posix"
            || name == "right"
            || name == "+VERSION"
            || name.ends_with(".tab")
            || name.ends_with(".zi")
            || name == "leapseconds"
            || name == "tzdata.zi"
            || name == "Factory"
        {
            continue;
        }
        let path = ent.path();
        let meta = match ent.file_type() {
            Ok(t) => t,
            Err(_) => continue,
        };
        if meta.is_dir() {
            collect_zones(root, &path, out);
            continue;
        }
        // Regular file or symlink to zone data.
        if !(meta.is_file() || meta.is_symlink()) {
            continue;
        }
        // Skip huge non-zone blobs if any.
        if let Ok(m) = ent.metadata() {
            if m.len() > 512 * 1024 {
                continue;
            }
        }
        if let Ok(rel) = path.strip_prefix(root) {
            let z = rel.to_string_lossy().replace('\\', "/");
            if !z.is_empty() && !z.contains("..") {
                out.push(z);
            }
        }
    }
}

/// Used when zoneinfo is missing (minimal host / incomplete image).
/// Texas Central first after auto (auto is separate). One Israel name only.
fn fallback_timezone_list() -> Vec<String> {
    [
        "America/Chicago", // Texas Central
        "America/Denver",  // west Texas
        "America/New_York",
        "America/Los_Angeles",
        "UTC",
        "Asia/Jerusalem",
        "America/Phoenix",
        "America/Anchorage",
        "America/Toronto",
        "America/Mexico_City",
        "America/Sao_Paulo",
        "Europe/London",
        "Europe/Berlin",
        "Europe/Paris",
        "Europe/Moscow",
        "Europe/Istanbul",
        "Africa/Cairo",
        "Africa/Johannesburg",
        "Asia/Dubai",
        "Asia/Tokyo",
        "Asia/Shanghai",
        "Asia/Kolkata",
        "Asia/Singapore",
        "Australia/Sydney",
        "Pacific/Auckland",
        "Etc/UTC",
        "Etc/GMT",
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

/// True for `auto` or a zone that exists under `/usr/share/zoneinfo`.
pub fn is_valid_timezone(name: &str) -> bool {
    let t = name.trim();
    if t.is_empty() || t.contains("..") || t.starts_with('/') || t.contains('\0') {
        return false;
    }
    if t.eq_ignore_ascii_case("auto") {
        return true;
    }
    let path = std::path::Path::new("/usr/share/zoneinfo").join(t);
    if path.is_file() {
        return true;
    }
    // Some installs only ship a subset; allow known fallback names too.
    fallback_timezone_list().iter().any(|z| z == t)
}

/// Lightweight validation of config.toml text (parse + known field checks).
pub fn validate_config_text(text: &str) -> Result<(), Vec<String>> {
    let mut errors = Vec::new();
    let value: toml::Value = match toml::from_str(text) {
        Ok(v) => v,
        Err(e) => return Err(vec![format!("TOML parse error: {e}")]),
    };

    let table = match value.as_table() {
        Some(t) => t,
        None => return Err(vec!["config root must be a table".into()]),
    };

    if let Some(v) = table.get("version") {
        if v.as_integer() != Some(1) {
            errors.push("version must be 1".into());
        }
    }

    let vm = table.get("vm").and_then(|v| v.as_table());
    if vm.is_none() {
        errors.push("missing [vm] section".into());
    }
    if let Some(vm) = vm {
        check_one_of(&mut errors, "vm.arch", vm.get("arch"), &["x86_64", "aarch64"]);
        check_one_of(&mut errors, "vm.firmware", vm.get("firmware"), &["bios", "uefi"]);
        check_one_of(&mut errors, "vm.machine", vm.get("machine"), &["pc", "q35"]);
        if let Some(mem) = vm.get("memory").and_then(|v| v.as_str()) {
            if parse_size(mem).is_none() {
                errors.push(format!("vm.memory invalid size: {mem}"));
            }
        }
        if let Some(disk) = vm.get("disk").and_then(|v| v.as_table()) {
            check_one_of(
                &mut errors,
                "vm.disk.format",
                disk.get("format"),
                &["raw", "qcow2"],
            );
            check_one_of(
                &mut errors,
                "vm.disk.bus",
                disk.get("bus"),
                &["ide", "virtio", "scsi"],
            );
            if let Some(path) = disk.get("path").and_then(|v| v.as_str()) {
                if path != "image.qcow2" {
                    // warning-as-soft: still ok, but we only push if empty
                    if path.is_empty() {
                        errors.push("vm.disk.path must not be empty".into());
                    }
                }
            } else {
                errors.push("vm.disk.path is required".into());
            }
        } else {
            errors.push("missing [vm.disk] section".into());
        }
    }

    if let Some(net) = table.get("network").and_then(|v| v.as_table()) {
        check_one_of(
            &mut errors,
            "network.mode",
            net.get("mode"),
            &["user", "bridge", "macvtap"],
        );
    }
    if let Some(display) = table.get("display").and_then(|v| v.as_table()) {
        check_one_of(
            &mut errors,
            "display.backend",
            display.get("backend"),
            &["sdl", "none"],
        );
        check_one_of(
            &mut errors,
            "display.vga",
            display.get("vga"),
            &["VGA", "cirrus", "std", "virtio-gpu-pci"],
        );
        check_one_of(
            &mut errors,
            "display.passthrough",
            display.get("passthrough"),
            &["both", "glide", "mesa", "none"],
        );
        let wants_3dfx = display
            .get("passthrough")
            .and_then(|v| v.as_str())
            .is_some_and(|p| matches!(p, "glide" | "mesa" | "both"));
        if wants_3dfx {
            // Match agent: missing backend defaults to sdl; only reject explicit non-sdl.
            if display
                .get("backend")
                .and_then(|v| v.as_str())
                .is_some_and(|b| b != "sdl")
            {
                errors.push(
                    "display.backend must be \"sdl\" when display.passthrough enables 3dfx"
                        .into(),
                );
            }
            // Agent defaults machine to q35 when omitted — that fails 3dfx.
            // Require an explicit pc (missing counts as wrong).
            let machine = table
                .get("vm")
                .and_then(|v| v.as_table())
                .and_then(|vm| vm.get("machine"))
                .and_then(|v| v.as_str());
            if machine != Some("pc") {
                errors.push(
                    "display.passthrough 3dfx modes require vm.machine = \"pc\" \
                     (set explicitly; default q35 is not valid for 3dfx)"
                        .into(),
                );
            }
            let arch = table
                .get("vm")
                .and_then(|v| v.as_table())
                .and_then(|vm| vm.get("arch"))
                .and_then(|v| v.as_str());
            // arch is required in full configs; if present it must be x86_64.
            if let Some(a) = arch {
                if a != "x86_64" {
                    errors.push(
                        "display.passthrough 3dfx modes require vm.arch = \"x86_64\"".into(),
                    );
                }
            }
        }
    }
    if let Some(sound) = table.get("sound").and_then(|v| v.as_table()) {
        check_one_of(
            &mut errors,
            "sound.backend",
            sound.get("backend"),
            &["alsa", "pipewire"],
        );
    }
    if let Some(system) = table.get("system").and_then(|v| v.as_table()) {
        if let Some(tz) = system.get("timezone").and_then(|v| v.as_str()) {
            if !is_valid_timezone(tz) {
                errors.push(format!(
                    "system.timezone must be \"auto\" or a valid Linux IANA zone \
                     (see /usr/share/zoneinfo); got {tz:?}"
                ));
            }
        }
        check_one_of(
            &mut errors,
            "system.rtc_base",
            system.get("rtc_base"),
            &["localtime", "utc"],
        );
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

fn check_one_of(errors: &mut Vec<String>, field: &str, val: Option<&toml::Value>, allowed: &[&str]) {
    let Some(val) = val else { return };
    let Some(s) = val.as_str() else {
        // allow integers for vcpus etc. — only check strings
        return;
    };
    if !allowed.contains(&s) {
        errors.push(format!(
            "{field} must be one of {}; got {s:?}",
            allowed.join(", ")
        ));
    }
}

fn parse_size(s: &str) -> Option<u64> {
    let s = s.trim();
    let (num, mult) = if let Some(n) = s.strip_suffix(['G', 'g']) {
        (n, 1024u64 * 1024 * 1024)
    } else if let Some(n) = s.strip_suffix(['M', 'm']) {
        (n, 1024 * 1024)
    } else if let Some(n) = s.strip_suffix(['K', 'k']) {
        (n, 1024)
    } else {
        (s, 1)
    };
    num.trim().parse::<u64>().ok().map(|n| n * mult)
}

pub fn default_config_toml() -> &'static str {
    include_str!("../../../assets/default/config.toml")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_validates() {
        validate_config_text(default_config_toml()).expect("default config must validate");
    }

    #[test]
    fn completes_machine_values() {
        let text = "[vm]\nmachine = \"";
        let ctx = completion_context(text, text.len());
        let s = suggestions(&ctx);
        assert!(s.iter().any(|x| x.label == "pc"));
    }

    #[test]
    fn does_not_complete_field_names() {
        let text = "[vm]\nme";
        let ctx = completion_context(text, text.len());
        assert!(matches!(ctx, CompleteContext::Unknown));
        assert!(suggestions(&ctx).is_empty());
    }

    #[test]
    fn completes_values_after_equals() {
        let text = "[vm]\narch = \"";
        let ctx = completion_context(text, text.len());
        let s = suggestions(&ctx);
        assert!(s.iter().any(|x| x.label == "x86_64"));
        assert!(s.iter().any(|x| x.label == "aarch64"));
    }

    #[test]
    fn timezone_accepts_jerusalem_and_auto() {
        assert!(is_valid_timezone("auto"));
        assert!(is_valid_timezone("Asia/Jerusalem") || is_valid_timezone("America/Chicago"));
        assert!(is_valid_timezone("UTC") || is_valid_timezone("America/Chicago"));
        assert!(!is_valid_timezone("../etc/passwd"));
        assert!(!is_valid_timezone("/etc/localtime"));
    }

    #[test]
    fn completes_timezone_jerusalem_not_legacy_israel_alias() {
        let text = "[system]\ntimezone = \"Jer";
        let ctx = completion_context(text, text.len());
        let s = suggestions(&ctx);
        assert!(
            s.iter().any(|x| x.label == "Asia/Jerusalem"),
            "expected Asia/Jerusalem, got: {:?}",
            s.iter().map(|x| &x.label).collect::<Vec<_>>()
        );
        assert!(
            !s.iter().any(|x| x.label == "Israel"),
            "legacy Israel alias must not appear when Jerusalem is listed"
        );
        let text_auto = "[system]\ntimezone = \"au";
        let ctx = completion_context(text_auto, text_auto.len());
        let s = suggestions(&ctx);
        assert!(s.iter().any(|x| x.label == "auto"));
    }

    #[test]
    fn empty_timezone_prefix_lists_texas_first_after_auto() {
        let text = "[system]\ntimezone = \"";
        let ctx = completion_context(text, text.len());
        let s = suggestions(&ctx);
        assert!(!s.is_empty());
        assert_eq!(s[0].label, "auto");
        // First concrete zone should be Texas Central.
        let first_zone = s.iter().find(|x| x.label != "auto").map(|x| x.label.as_str());
        assert_eq!(first_zone, Some("America/Chicago"));
        assert!(!s.iter().any(|x| x.label == "Israel"));
    }

    #[test]
    fn list_system_timezones_is_nonempty() {
        let zones = list_system_timezones();
        assert!(!zones.is_empty());
        assert!(
            zones
                .iter()
                .any(|z| z == "Asia/Jerusalem" || z == "America/Chicago" || z == "UTC"),
            "zones sample: {:?}",
            &zones[..zones.len().min(20)]
        );
    }

    #[test]
    fn threedfx_requires_explicit_pc_machine() {
        let missing_machine = r#"
version = 1
[vm]
arch = "x86_64"
[vm.disk]
path = "image.qcow2"
[display]
passthrough = "both"
backend = "sdl"
"#;
        let err = validate_config_text(missing_machine).unwrap_err();
        assert!(
            err.iter().any(|e| e.contains("vm.machine")),
            "expected machine error, got {err:?}"
        );

        let ok = r#"
version = 1
[vm]
arch = "x86_64"
machine = "pc"
[vm.disk]
path = "image.qcow2"
[display]
passthrough = "both"
backend = "sdl"
"#;
        validate_config_text(ok).expect("pc + both should validate");
    }
}
