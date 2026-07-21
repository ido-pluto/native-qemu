use serde::Deserialize;
use std::fs;
use std::path::Path;

fn default_true() -> bool {
    true
}

#[derive(Debug, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub version: u32,
    pub vm: VmConfig,
    #[serde(default)]
    pub network: NetworkConfig,
    #[serde(default)]
    pub sound: SoundConfig,
    #[serde(default)]
    pub display: DisplayConfig,
    #[serde(default)]
    pub usb: UsbConfig,
    #[serde(default)]
    pub startup: HookConfig,
    #[serde(default)]
    pub shutdown: HookConfig,
    #[serde(default)]
    pub lifecycle: LifecycleConfig,
    #[serde(default)]
    pub logging: LoggingConfig,
    #[serde(default)]
    pub docs_server: DocsServerConfig,
    #[serde(default)]
    pub shared_folder: SharedFolderConfig,
    #[serde(default)]
    pub smb_share: Vec<SmbShareConfig>,
    #[serde(default)]
    pub smb: SmbConfig,
    #[serde(default)]
    pub system: SystemConfig,
}

#[derive(Debug, Deserialize)]
pub struct VmConfig {
    /// x86_64 | aarch64 — must match the host running this agent.
    pub arch: String,
    #[serde(default = "default_firmware")]
    pub firmware: String,
    /// QEMU machine type. x86 defaults to q35; aarch64 always uses virt in qemu.rs.
    #[serde(default = "default_machine")]
    pub machine: String,
    #[serde(default = "default_memory")]
    pub memory: String,
    #[serde(default = "default_vcpus")]
    pub vcpus: u32,
    /// Topology: when sockets, cores, and threads are all > 0 they are passed
    /// to QEMU as `-smp N,sockets=…,cores=…,threads=…`. Zero (the default)
    /// means omit topology and use vcpus alone.
    #[serde(default)]
    pub sockets: u32,
    #[serde(default)]
    pub cores: u32,
    #[serde(default)]
    pub threads: u32,
    #[serde(default = "default_cpu")]
    pub cpu: String,
    pub disk: DiskConfig,
}
fn default_firmware() -> String {
    // aarch64's "virt" machine always requires UEFI regardless of this
    // value (enforced in qemu.rs). x86_64 defaults to the built-in SeaBIOS;
    // selecting UEFI uses Alpine's bundled OVMF firmware instead.
    "bios".into()
}
fn default_machine() -> String {
    "q35".into()
}
fn default_memory() -> String {
    "2G".into()
}
fn default_vcpus() -> u32 {
    2
}
fn default_cpu() -> String {
    "host".into()
}

#[derive(Debug, Deserialize)]
pub struct DiskConfig {
    /// raw | qcow2
    #[serde(default = "default_disk_format")]
    pub format: String,
    /// 0 = boot device, 1 = first internal disk, 2.. = external disks
    pub storage: u32,
    /// path inside the resolved storage's mountpoint
    pub path: String,
    #[serde(default = "default_bus")]
    pub bus: String,
    #[serde(default = "default_cache")]
    pub cache: String,
    #[serde(default = "default_discard")]
    pub discard: String,
}
fn default_disk_format() -> String {
    "qcow2".into()
}
fn default_bus() -> String {
    "virtio".into()
}
fn default_cache() -> String {
    "none".into()
}
fn default_discard() -> String {
    "unmap".into()
}

#[derive(Debug, Deserialize)]
pub struct NetworkConfig {
    /// user | bridge | macvtap
    #[serde(default = "default_net_mode")]
    pub mode: String,
    #[serde(default)]
    pub bridge_iface: Option<String>,
    #[serde(default = "default_net_model")]
    pub model: String,
}
fn default_net_mode() -> String {
    "user".into()
}
fn default_net_model() -> String {
    "virtio-net-pci".into()
}
impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            mode: default_net_mode(),
            bridge_iface: None,
            model: default_net_model(),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct SoundConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_sound_backend")]
    pub backend: String,
    #[serde(default = "default_sound_model")]
    pub model: String,
}
fn default_sound_backend() -> String {
    "alsa".into()
}
fn default_sound_model() -> String {
    "virtio-sound-pci".into()
}
impl Default for SoundConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            backend: default_sound_backend(),
            model: default_sound_model(),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct DisplayConfig {
    /// `sdl` renders the guest directly on the host's KMS/DRM console; `none`
    /// is for deliberately headless guests managed through another channel.
    #[serde(default = "default_display_backend")]
    pub backend: String,
    /// Guest VGA adapter name used with display.backend = "sdl".
    /// Config values: VGA | cirrus | std | virtio | virtio-gpu-pci
    #[serde(default = "default_display_vga")]
    pub vga: String,
    /// qemu-3dfx Glide/MESA host pass-through (patched QEMU only).
    /// `none` | `glide` | `mesa` | `both`. Non-`none` requires SDL with
    /// `gl=off` (not `gl=on`) and the appliance binary under `/usr/local/bin`.
    /// Devices are auto-mapped by qemu-3dfx on the `pc` machine — the agent
    /// does not pass `-device glidept` / `mesapt`.
    ///
    /// Host-side, `glide` / `mesa` / `both` are currently equivalent (same
    /// QEMU flags); guest wrappers select Glide vs OpenGL. Prefer `both`.
    #[serde(default = "default_display_passthrough")]
    pub passthrough: String,
}
fn default_display_backend() -> String {
    "sdl".into()
}
fn default_display_vga() -> String {
    "VGA".into()
}
fn default_display_passthrough() -> String {
    "none".into()
}
impl Default for DisplayConfig {
    fn default() -> Self {
        Self {
            backend: default_display_backend(),
            vga: default_display_vga(),
            passthrough: default_display_passthrough(),
        }
    }
}

impl DisplayConfig {
    /// True when Glide and/or MESA 3dfx pass-through is requested.
    pub fn wants_3dfx(&self) -> bool {
        matches!(self.passthrough.as_str(), "glide" | "mesa" | "both")
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct UsbDevice {
    #[serde(default)]
    pub name: Option<String>,
    pub vendor_id: String,
    pub product_id: String,
    #[serde(default)]
    pub required: bool,
}

#[derive(Debug, Deserialize, Clone)]
pub struct UsbConfig {
    /// passthrough | host-only — policy for devices not explicitly listed below
    #[serde(default = "default_usb_policy")]
    pub default: String,
    #[serde(default)]
    pub exclude: Vec<String>,
    #[serde(default)]
    pub hotplug: bool,
    #[serde(default)]
    pub device: Vec<UsbDevice>,
}
fn default_usb_policy() -> String {
    "passthrough".into()
}
impl Default for UsbConfig {
    fn default() -> Self {
        Self {
            default: default_usb_policy(),
            exclude: Vec::new(),
            hotplug: false,
            device: Vec::new(),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct HookConfig {
    #[serde(default)]
    pub script: Option<String>,
    #[serde(default = "default_true")]
    pub blocking: bool,
    #[serde(default = "default_hook_timeout")]
    pub timeout: String,
    #[serde(default = "default_on_failure")]
    pub on_failure: String,
}
fn default_hook_timeout() -> String {
    "30s".into()
}
fn default_on_failure() -> String {
    "continue".into()
}
impl Default for HookConfig {
    fn default() -> Self {
        Self {
            script: None,
            blocking: default_true(),
            timeout: default_hook_timeout(),
            on_failure: default_on_failure(),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct LifecycleConfig {
    #[serde(default = "default_on_shutdown")]
    pub on_guest_shutdown: String,
    #[serde(default = "default_on_crash")]
    pub on_guest_crash: String,
    #[serde(default = "default_on_missing")]
    pub on_missing_resource: String,
    #[serde(default = "default_max_restarts")]
    pub max_restart_attempts: u32,
}
fn default_on_shutdown() -> String {
    "poweroff_host".into()
}
fn default_on_crash() -> String {
    "drop_to_shell".into()
}
fn default_on_missing() -> String {
    "rescue_shell".into()
}
fn default_max_restarts() -> u32 {
    3
}
impl Default for LifecycleConfig {
    fn default() -> Self {
        Self {
            on_guest_shutdown: default_on_shutdown(),
            on_guest_crash: default_on_crash(),
            on_missing_resource: default_on_missing(),
            max_restart_attempts: default_max_restarts(),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct LoggingConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Storage index to write logs to. Deliberately NOT 0 by default — the
    /// boot media (index 0) is read-only, so logging there would always
    /// silently fall back to tmpfs (see logging.rs).
    #[serde(default = "default_log_storage")]
    pub storage: u32,
    #[serde(default = "default_log_path")]
    pub path: String,
    #[serde(default)]
    pub max_size: String,
    #[serde(default)]
    pub rotate: u32,
}
fn default_log_storage() -> u32 {
    1
}
fn default_log_path() -> String {
    "native-qemu/logs".into()
}
impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            storage: default_log_storage(),
            path: default_log_path(),
            max_size: String::new(),
            rotate: 0,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct DocsServerConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_docs_domain")]
    pub domain: String,
    #[serde(default = "default_docs_port")]
    pub port: u16,
    #[serde(default = "default_docs_interface")]
    pub bind_iface: String,
    #[serde(default = "default_docs_dir")]
    pub docs_dir: String,
    /// Optional dnsmasq DHCP range, for example "192.168.76.10,192.168.76.250,12h".
    /// Leaving it empty keeps the docs service DNS-only, so it never starts a
    /// DHCP server on a network the operator has configured independently.
    #[serde(default)]
    pub dhcp_range: Option<String>,
}
fn default_docs_domain() -> String {
    "light-docs.local".into()
}
fn default_docs_port() -> u16 {
    80
}
fn default_docs_interface() -> String {
    "br0".into()
}
fn default_docs_dir() -> String {
    "/etc/native-qemu/docs".into()
}
impl Default for DocsServerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            domain: default_docs_domain(),
            port: default_docs_port(),
            bind_iface: default_docs_interface(),
            docs_dir: default_docs_dir(),
            dhcp_range: None,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct SharedFolderConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_shared_storage")]
    pub storage: u32,
    #[serde(default = "default_shared_path")]
    pub host_path: String,
    #[serde(default = "default_shared_tag")]
    pub guest_tag: String,
}
fn default_shared_storage() -> u32 {
    1
}
fn default_shared_path() -> String {
    "native-qemu/shared".into()
}
fn default_shared_tag() -> String {
    "native-qemu-share".into()
}
impl Default for SharedFolderConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            storage: default_shared_storage(),
            host_path: default_shared_path(),
            guest_tag: default_shared_tag(),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct SmbShareConfig {
    pub name: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub storage: u32,
    pub host_path: String,
    pub share_name: String,
    #[serde(default = "default_smb_scope")]
    pub scope: String,
    pub username: String,
    pub password_file: String,
    #[serde(default)]
    pub read_only: bool,
}
fn default_smb_scope() -> String {
    "vm_only".into()
}

#[derive(Debug, Deserialize, Default)]
pub struct SmbConfig {
    #[serde(default = "default_vm_interface")]
    pub vm_iface: String,
    #[serde(default)]
    pub lan_iface: Option<String>,
}
fn default_vm_interface() -> String {
    "br0".into()
}

#[derive(Debug, Deserialize)]
pub struct SystemConfig {
    #[allow(dead_code)]
    #[serde(default)]
    pub hostname: Option<String>,
    /// Host IANA timezone applied before QEMU starts (guest uses
    /// `-rtc base=localtime`).
    ///
    /// - `"auto"` (default): detect host zone from `/etc/timezone` /
    ///   `/etc/localtime` / `TZ`; if none, **America/Chicago** (Texas Central).
    /// - any valid Linux IANA zone under `/usr/share/zoneinfo` (e.g.
    ///   `America/Chicago`, `Asia/Jerusalem`, UTC).
    #[serde(default = "default_timezone")]
    pub timezone: String,
    /// QEMU RTC base: `localtime` (Win9x/ReactOS) or `utc`.
    #[serde(default = "default_rtc_base")]
    pub rtc_base: String,
    #[serde(default)]
    pub ssh_enabled: bool,
    #[serde(default)]
    pub ssh_authorized_key: Option<String>,
}
fn default_timezone() -> String {
    "auto".into()
}
fn default_rtc_base() -> String {
    "localtime".into()
}
impl Default for SystemConfig {
    fn default() -> Self {
        Self {
            hostname: None,
            timezone: default_timezone(),
            rtc_base: default_rtc_base(),
            ssh_enabled: false,
            ssh_authorized_key: None,
        }
    }
}

#[derive(Debug)]
pub enum ConfigError {
    Io(std::io::Error),
    Parse(toml::de::Error),
}
impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::Io(e) => write!(f, "could not read config file: {e}"),
            ConfigError::Parse(e) => write!(f, "could not parse config.toml: {e}"),
        }
    }
}

/// Parses a "8G" / "512M" style size string into bytes. Accepts a bare
/// number (bytes), or a number suffixed with K/M/G (case-insensitive,
/// base-1024).
pub fn parse_size(s: &str) -> Option<u64> {
    let s = s.trim();
    let (num_part, mult): (&str, u64) = if let Some(n) = s.strip_suffix(['G', 'g']) {
        (n, 1024 * 1024 * 1024)
    } else if let Some(n) = s.strip_suffix(['M', 'm']) {
        (n, 1024 * 1024)
    } else if let Some(n) = s.strip_suffix(['K', 'k']) {
        (n, 1024)
    } else {
        (s, 1)
    };
    num_part.trim().parse::<u64>().ok().map(|n| n * mult)
}

/// Parses a "30s" / "5m" style duration string into seconds. Accepts a bare
/// number (seconds), or a number suffixed with s/m/h.
pub fn parse_duration_secs(s: &str) -> Option<u64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let (num_part, mult): (&str, u64) = if let Some(n) = s.strip_suffix(['h', 'H']) {
        (n, 3600)
    } else if let Some(n) = s.strip_suffix(['m', 'M']) {
        (n, 60)
    } else if let Some(n) = s.strip_suffix(['s', 'S']) {
        (n, 1)
    } else {
        (s, 1)
    };
    num_part.trim().parse::<u64>().ok().map(|n| n * mult)
}

pub fn load(path: &Path) -> Result<Config, ConfigError> {
    let text = fs::read_to_string(path).map_err(ConfigError::Io)?;
    toml::from_str(&text).map_err(ConfigError::Parse)
}

/// Checks values whose syntax TOML can parse but which have a finite set of
/// appliance meanings.  Rejecting them here is safer than silently choosing a
/// QEMU fallback (for example, treating an unknown network mode as `user`) or
/// failing only after host-side services have been started.
pub fn validate(config: &Config) -> Result<(), String> {
    one_of("vm.arch", &config.vm.arch, &["x86_64", "aarch64"])?;
    one_of("vm.firmware", &config.vm.firmware, &["bios", "uefi"])?;
    if config.vm.arch == "x86_64" {
        one_of("vm.machine", &config.vm.machine, &["pc", "q35"])?;
    }
    positive_size("vm.memory", &config.vm.memory)?;
    if config.vm.vcpus == 0 {
        return Err("vm.vcpus must be at least 1".into());
    }
    if config.vm.arch == "aarch64" && config.vm.firmware != "uefi" {
        return Err("vm.firmware must be \"uefi\" when vm.arch = \"aarch64\"".into());
    }
    one_of("vm.disk.format", &config.vm.disk.format, &["raw", "qcow2"])?;
    one_of(
        "vm.disk.bus",
        &config.vm.disk.bus,
        &["virtio", "scsi", "ide"],
    )?;
    if config.vm.arch == "aarch64" && config.vm.disk.bus == "ide" {
        return Err("vm.disk.bus = \"ide\" is unavailable on the aarch64 virt machine".into());
    }
    one_of(
        "network.mode",
        &config.network.mode,
        &["user", "bridge", "macvtap"],
    )?;
    let missing_bridge_iface = match config.network.bridge_iface.as_deref() {
        Some(interface) => interface.is_empty(),
        None => true,
    };
    if matches!(config.network.mode.as_str(), "bridge" | "macvtap") && missing_bridge_iface {
        return Err("network.bridge_iface is required for bridge or macvtap networking".into());
    }
    one_of(
        "sound.backend",
        &config.sound.backend,
        &["alsa", "pipewire"],
    )?;
    // Keep the allow-list broad — QEMU accepts many sound models; we only
    // reject obvious typos rather than force a short modern-only set.
    one_of(
        "sound.model",
        &config.sound.model,
        &[
            "sb16",
            "virtio-sound-pci",
            "AC97",
            "ES1370",
            "intel-hda",
            "hda-duplex",
            "hda-micro",
            "hda-output",
            "ich9-intel-hda",
            "adlib",
            "gus",
            "cs4231a",
            "pcspk",
        ],
    )?;
    one_of("display.backend", &config.display.backend, &["sdl", "none"])?;
    one_of(
        "display.vga",
        &config.display.vga,
        &["VGA", "cirrus", "std", "virtio", "virtio-gpu-pci"],
    )?;
    one_of(
        "display.passthrough",
        &config.display.passthrough,
        &["none", "glide", "mesa", "both"],
    )?;
    if config.display.wants_3dfx() {
        if config.display.backend != "sdl" {
            return Err(
                "display.backend must be \"sdl\" when display.passthrough enables 3dfx \
                 (glide/mesa/both); qemu-3dfx conflicts with headless display"
                    .into(),
            );
        }
        if config.vm.arch != "x86_64" {
            return Err(
                "display.passthrough 3dfx modes require vm.arch = \"x86_64\"".into(),
            );
        }
        if config.vm.machine != "pc" {
            return Err(
                "display.passthrough 3dfx modes require vm.machine = \"pc\" \
                 (qemu-3dfx glidept/mesapt auto-map on i440fx)"
                    .into(),
            );
        }
    }
    one_of(
        "usb.default",
        &config.usb.default,
        &["passthrough", "host-only"],
    )?;
    one_of(
        "startup.on_failure",
        &config.startup.on_failure,
        &["continue", "abort_to_rescue"],
    )?;
    one_of(
        "shutdown.on_failure",
        &config.shutdown.on_failure,
        &["continue", "abort_to_rescue"],
    )?;
    positive_duration("startup.timeout", &config.startup.timeout)?;
    positive_duration("shutdown.timeout", &config.shutdown.timeout)?;
    one_of(
        "lifecycle.on_guest_shutdown",
        &config.lifecycle.on_guest_shutdown,
        &["poweroff_host", "restart_vm", "drop_to_shell"],
    )?;
    one_of(
        "lifecycle.on_guest_crash",
        &config.lifecycle.on_guest_crash,
        &["poweroff_host", "restart_vm", "drop_to_shell"],
    )?;
    one_of(
        "lifecycle.on_missing_resource",
        &config.lifecycle.on_missing_resource,
        &["rescue_shell", "boot_anyway"],
    )?;
    for share in &config.smb_share {
        one_of("smb_share.scope", &share.scope, &["vm_only", "global"])?;
    }
    if config
        .smb_share
        .iter()
        .any(|share| share.enabled && share.scope == "vm_only")
        && config.smb.vm_iface.is_empty()
    {
        return Err("smb.vm_iface is required for vm_only SMB shares".into());
    }
    if config
        .smb_share
        .iter()
        .any(|share| share.enabled && share.scope == "global")
        && config
            .smb
            .lan_iface
            .as_deref()
            .unwrap_or_default()
            .is_empty()
    {
        return Err("smb.lan_iface is required for global SMB shares".into());
    }
    if config.docs_server.enabled {
        nonempty("docs_server.domain", &config.docs_server.domain)?;
        nonempty("docs_server.bind_iface", &config.docs_server.bind_iface)?;
        if config.docs_server.port == 0 {
            return Err("docs_server.port must be greater than 0".into());
        }
    }
    if config.shared_folder.enabled {
        nonempty("shared_folder.guest_tag", &config.shared_folder.guest_tag)?;
    }
    if !config.logging.max_size.is_empty() {
        positive_size("logging.max_size", &config.logging.max_size)?;
    }
    one_of(
        "system.rtc_base",
        &config.system.rtc_base,
        &["localtime", "utc"],
    )?;
    nonempty("system.timezone", &config.system.timezone)?;
    // Path-safety + zone existence (when tzdata/zoneinfo is installed).
    crate::timezone::validate_configured(&config.system.timezone)?;
    Ok(())
}

fn one_of(field: &str, value: &str, allowed: &[&str]) -> Result<(), String> {
    if allowed.contains(&value) {
        Ok(())
    } else {
        Err(format!(
            "{field} must be one of {}; got {value:?}",
            allowed
                .iter()
                .map(|item| format!("{item:?}"))
                .collect::<Vec<_>>()
                .join(", ")
        ))
    }
}

fn nonempty(field: &str, value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        Err(format!("{field} must not be empty"))
    } else {
        Ok(())
    }
}

fn positive_size(field: &str, value: &str) -> Result<(), String> {
    match parse_size(value) {
        Some(size) if size > 0 => Ok(()),
        _ => Err(format!("{field} must be a positive size such as \"2G\"")),
    }
}

fn positive_duration(field: &str, value: &str) -> Result<(), String> {
    match parse_duration_secs(value) {
        Some(duration) if duration > 0 => Ok(()),
        _ => Err(format!(
            "{field} must be a positive duration such as \"30s\""
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_duration_secs, parse_size, validate, Config};

    fn bundled_config() -> String {
        include_str!("../../overlay/etc/native-qemu/config.toml.example").to_owned()
    }

    #[test]
    fn parses_sizes() {
        assert_eq!(parse_size("8G"), Some(8 * 1024 * 1024 * 1024));
        assert_eq!(parse_size("512m"), Some(512 * 1024 * 1024));
        assert_eq!(parse_size("42"), Some(42));
        assert_eq!(parse_size("wat"), None);
    }

    #[test]
    fn parses_durations() {
        assert_eq!(parse_duration_secs("30s"), Some(30));
        assert_eq!(parse_duration_secs("5m"), Some(300));
        assert_eq!(parse_duration_secs("2H"), Some(7200));
        assert_eq!(parse_duration_secs(""), None);
    }

    #[test]
    fn bundled_config_example_matches_the_current_schema() {
        let config: Config =
            toml::from_str(&bundled_config()).expect("bundled config.toml.example must parse");
        assert_eq!(config.version, 1);
        assert_eq!(config.vm.arch, "x86_64");
        assert_eq!(config.vm.machine, "q35");
        assert_eq!(config.display.vga, "VGA");
        assert!(!config.docs_server.enabled);
        assert!(!config.shared_folder.enabled);
        validate(&config).expect("bundled config.toml.example must be valid");
    }

    #[test]
    fn win98_default_config_matches_the_current_schema() {
        let config: Config = toml::from_str(include_str!("../../assets/default/config.toml"))
            .expect("assets/default/config.toml must parse");
        validate(&config).expect("assets/default/config.toml must be valid");
        assert_eq!(config.vm.machine, "pc");
        assert_eq!(config.vm.cpu, "host");
        assert_eq!(config.vm.memory, "512M");
        assert_eq!(config.vm.vcpus, 1);
        assert_eq!(config.vm.sockets, 1);
        assert_eq!(config.vm.cores, 1);
        assert_eq!(config.vm.threads, 1);
        assert_eq!(config.network.model, "rtl8139");
        assert_eq!(config.display.vga, "VGA");
        assert_eq!(config.display.passthrough, "both");
        assert!(config.display.wants_3dfx());
        assert_eq!(config.sound.model, "AC97");
        assert_eq!(config.vm.disk.bus, "ide");
        assert_eq!(config.system.timezone, "auto");
        assert_eq!(config.system.rtc_base, "localtime");
    }

    #[test]
    fn rejects_3dfx_passthrough_without_pc_sdl() {
        let bad_backend = include_str!("../../assets/default/config.toml")
            .replace("backend = \"sdl\"", "backend = \"none\"");
        let config: Config = toml::from_str(&bad_backend).unwrap();
        assert!(
            config.display.backend == "none",
            "test replace must hit display.backend"
        );
        assert!(validate(&config)
            .unwrap_err()
            .contains("display.backend must be \"sdl\""));

        let bad_machine = include_str!("../../assets/default/config.toml")
            .replace("machine  = \"pc\"", "machine  = \"q35\"");
        let config: Config = toml::from_str(&bad_machine).unwrap();
        assert!(validate(&config)
            .unwrap_err()
            .contains("vm.machine = \"pc\""));

        let bad_mode = bundled_config().replace(
            "passthrough = \"none\"",
            "passthrough = \"not-a-mode\"",
        );
        let config: Config = toml::from_str(&bad_mode).unwrap();
        assert!(validate(&config)
            .unwrap_err()
            .contains("display.passthrough"));
    }

    #[test]
    fn rejects_invalid_or_unknown_timezone() {
        let bad_path = include_str!("../../assets/default/config.toml")
            .replace("timezone = \"auto\"", "timezone = \"../etc/passwd\"");
        let config: Config = toml::from_str(&bad_path).unwrap();
        assert!(validate(&config).unwrap_err().contains("timezone"));

        // Only when zoneinfo is present on the test host.
        if std::path::Path::new("/usr/share/zoneinfo").is_dir() {
            let typo = include_str!("../../assets/default/config.toml").replace(
                "timezone = \"auto\"",
                "timezone = \"America/NotARealZone\"",
            );
            let config: Config = toml::from_str(&typo).unwrap();
            assert!(
                validate(&config).unwrap_err().contains("timezone"),
                "unknown zone must fail when zoneinfo exists"
            );
        }
    }

    #[test]
    fn rejects_values_that_would_otherwise_fall_back_or_fail_late() {
        let invalid_network = bundled_config().replace(
            "mode         = \"user\"",
            "mode         = \"not-a-network-mode\"",
        );
        let config: Config = toml::from_str(&invalid_network).unwrap();
        assert!(validate(&config).unwrap_err().contains("network.mode"));

        let arm_ide = bundled_config()
            .replace("arch     = \"x86_64\"", "arch     = \"aarch64\"")
            .replace("firmware = \"bios\"", "firmware = \"uefi\"")
            .replace("bus     = \"virtio\"", "bus     = \"ide\"");
        let config: Config = toml::from_str(&arm_ide).unwrap();
        assert!(validate(&config).unwrap_err().contains("aarch64 virt"));

        let missing_smb_lan = format!(
            "{}\n[[smb_share]]\nname = \"lan\"\nstorage = 1\nhost_path = \"share\"\nshare_name = \"lan\"\nscope = \"global\"\nusername = \"vmuser\"\npassword_file = \"/run/secret\"\n",
            bundled_config()
        );
        let config: Config = toml::from_str(&missing_smb_lan).unwrap();
        assert!(validate(&config).unwrap_err().contains("smb.lan_iface"));

        let bad_machine = bundled_config().replace(
            "machine  = \"q35\"",
            "machine  = \"not-a-machine\"",
        );
        let config: Config = toml::from_str(&bad_machine).unwrap();
        assert!(validate(&config).unwrap_err().contains("vm.machine"));

        let bad_vga = bundled_config().replace(
            "vga         = \"VGA\"",
            "vga         = \"not-a-vga\"",
        );
        let config: Config = toml::from_str(&bad_vga).unwrap();
        assert!(validate(&config).unwrap_err().contains("display.vga"));
    }

    #[test]
    fn xp_virtio_example_matches_the_current_schema() {
        let config: Config = toml::from_str(include_str!("../../examples/winxp-virtio.toml"))
            .expect("XP VirtIO example must parse");
        validate(&config).expect("XP VirtIO example must be valid");
        assert_eq!(config.vm.arch, "x86_64");
        assert_eq!(config.vm.disk.format, "qcow2");
        assert_eq!(config.vm.disk.bus, "virtio");
        assert_eq!(config.display.backend, "sdl");
        assert_eq!(config.startup.timeout, "30s");
        assert_eq!(config.shutdown.on_failure, "continue");
    }
}
