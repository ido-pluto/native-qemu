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
                values: &["pentium3", "host", "qemu64", "max"],
                doc: "QEMU CPU model",
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
                values: &["rtl8139", "e1000", "virtio-net-pci"],
                doc: "rtl8139 for Win98/ReactOS",
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
                values: &["sb16", "virtio-sound-pci", "AC97", "ES1370"],
                doc: "sb16 for Win98/ReactOS",
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
                values: &["cirrus", "std", "VGA", "virtio-gpu-pci"],
                doc: "cirrus for Win98/ReactOS",
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
            &["cirrus", "std", "VGA", "virtio-gpu-pci"],
        );
    }
    if let Some(sound) = table.get("sound").and_then(|v| v.as_table()) {
        check_one_of(
            &mut errors,
            "sound.backend",
            sound.get("backend"),
            &["alsa", "pipewire"],
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
}
