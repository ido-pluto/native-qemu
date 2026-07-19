use crate::config::{Config, DocsServerConfig, SharedFolderConfig, SmbShareConfig};
use crate::storage;
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

const VIRTIOFS_SOCKET: &str = "/run/native-qemu-virtiofs.sock";
const PIPEWIRE_RUNTIME_DIR: &str = "/run/native-qemu-pipewire";

/// The QEMU-facing half of the shared-folder service.  The daemon itself is
/// owned by RuntimeServices and is terminated when the guest exits.
pub struct VirtiofsRuntime {
    pub socket: PathBuf,
    pub guest_tag: String,
}

/// The host-side file descriptor QEMU needs for a macvtap netdev.  The
/// interface is created solely for this VM and removed by RuntimeServices.
pub struct MacvtapRuntime {
    file: File,
    interface: String,
}

impl MacvtapRuntime {
    pub fn raw_fd(&self) -> std::os::unix::io::RawFd {
        use std::os::unix::io::AsRawFd;
        self.file.as_raw_fd()
    }
}

/// Processes that must live for exactly as long as the guest does.  Keeping
/// them in one owner prevents a crashed/restarted guest from leaving stale
/// dnsmasq, httpd, or virtiofsd instances behind on the appliance host.
pub struct RuntimeServices {
    children: Vec<Child>,
    virtiofs: Option<VirtiofsRuntime>,
    macvtap: Option<MacvtapRuntime>,
}

impl RuntimeServices {
    pub fn start(cfg: &Config) -> Result<Self, String> {
        let mut services = Self {
            children: Vec::new(),
            virtiofs: None,
            macvtap: None,
        };

        if cfg.shared_folder.enabled {
            services.start_virtiofs(&cfg.shared_folder)?;
        }
        if cfg.sound.enabled && cfg.sound.backend == "pipewire" {
            services.start_pipewire()?;
        }
        if cfg.docs_server.enabled {
            services.start_docs(&cfg.docs_server)?;
        }
        services.start_smb(cfg)?;
        if cfg.network.mode == "macvtap" {
            services.start_macvtap(cfg)?;
        }
        Ok(services)
    }

    pub fn virtiofs(&self) -> Option<&VirtiofsRuntime> {
        self.virtiofs.as_ref()
    }

    pub fn macvtap(&self) -> Option<&MacvtapRuntime> {
        self.macvtap.as_ref()
    }

    fn start_virtiofs(&mut self, cfg: &SharedFolderConfig) -> Result<(), String> {
        let mountpoint = storage::resolve(cfg.storage).map_err(|e| {
            format!(
                "could not resolve shared_folder.storage={}: {e}",
                cfg.storage
            )
        })?;
        let shared_dir = mountpoint.join(&cfg.host_path);
        fs::create_dir_all(&shared_dir).map_err(|e| {
            format!(
                "could not create shared folder {}: {e}",
                shared_dir.display()
            )
        })?;

        let socket = PathBuf::from(VIRTIOFS_SOCKET);
        if socket.exists() {
            fs::remove_file(&socket).map_err(|e| {
                format!(
                    "could not remove stale virtiofs socket {}: {e}",
                    socket.display()
                )
            })?;
        }
        let child = spawn_checked(
            Command::new("/usr/libexec/virtiofsd")
                .arg("--shared-dir")
                .arg(&shared_dir)
                .arg("--socket-path")
                .arg(&socket)
                .arg("--sandbox")
                .arg("namespace"),
            "virtiofsd",
        )?;
        println!(
            "native-qemu: sharing {} with guest tag {}",
            shared_dir.display(),
            cfg.guest_tag
        );
        self.children.push(child);
        self.virtiofs = Some(VirtiofsRuntime {
            socket,
            guest_tag: cfg.guest_tag.clone(),
        });
        Ok(())
    }

    fn start_pipewire(&mut self) -> Result<(), String> {
        fs::create_dir_all(PIPEWIRE_RUNTIME_DIR)
            .map_err(|e| format!("could not create PipeWire runtime directory: {e}"))?;
        let mut pipewire = Command::new("pipewire");
        pipewire.env("XDG_RUNTIME_DIR", PIPEWIRE_RUNTIME_DIR);
        self.children
            .push(spawn_checked(&mut pipewire, "PipeWire")?);

        let mut pulse = Command::new("pipewire-pulse");
        pulse.env("XDG_RUNTIME_DIR", PIPEWIRE_RUNTIME_DIR);
        self.children
            .push(spawn_checked(&mut pulse, "PipeWire PulseAudio service")?);
        println!("native-qemu: PipeWire audio service started");
        Ok(())
    }

    fn start_docs(&mut self, cfg: &DocsServerConfig) -> Result<(), String> {
        let docs_dir = Path::new(&cfg.docs_dir);
        if !docs_dir.is_dir() {
            return Err(format!(
                "docs_server.docs_dir is not a directory: {}",
                docs_dir.display()
            ));
        }
        let address = ipv4_address(&cfg.bind_iface)?;
        let address_string = address.to_string();

        let mut dnsmasq = Command::new("dnsmasq");
        dnsmasq
            .arg("--keep-in-foreground")
            .arg("--no-resolv")
            .arg("--no-hosts")
            .arg("--bind-interfaces")
            .arg(format!("--interface={}", cfg.bind_iface))
            .arg(format!("--listen-address={address}"))
            .arg(format!("--address=/{}/{address}", cfg.domain))
            .arg("--log-facility=-");
        if let Some(range) = cfg.dhcp_range.as_deref().filter(|range| !range.is_empty()) {
            dnsmasq.arg(format!("--dhcp-range={range}"));
            dnsmasq.arg("--dhcp-authoritative");
        }
        self.children
            .push(spawn_checked(&mut dnsmasq, "docs dnsmasq")?);

        let mut httpd = Command::new("busybox");
        httpd
            .arg("httpd")
            .arg("-f")
            .arg("-p")
            .arg(format!("{address_string}:{}", cfg.port))
            .arg("-h")
            .arg(docs_dir);
        self.children.push(spawn_checked(&mut httpd, "docs httpd")?);
        println!(
            "native-qemu: serving http://{}:{} on {} ({})",
            cfg.domain, cfg.port, cfg.bind_iface, address
        );
        Ok(())
    }

    fn start_smb(&mut self, cfg: &Config) -> Result<(), String> {
        let enabled: Vec<&SmbShareConfig> =
            cfg.smb_share.iter().filter(|share| share.enabled).collect();
        if enabled.is_empty() {
            return Ok(());
        }

        // One smbd process is bound to the guest bridge and a second (only
        // when requested) is bound to the real LAN.  Samba's interface ACL
        // is global rather than per-share, so separating them is what makes
        // scope = vm_only an actual network boundary.
        let mut vm_only = Vec::new();
        let mut global = Vec::new();
        for share in enabled {
            match share.scope.as_str() {
                "vm_only" => vm_only.push(share),
                "global" => global.push(share),
                other => {
                    return Err(format!(
                        "SMB share {} has invalid scope {other:?}",
                        share.name
                    ))
                }
            }
        }

        let mut passwords: BTreeMap<String, String> = BTreeMap::new();
        for share in vm_only.iter().chain(global.iter()) {
            validate_smb_identifier(&share.share_name, "share_name")?;
            validate_smb_identifier(&share.username, "username")?;
            let password = fs::read_to_string(&share.password_file)
                .map_err(|e| {
                    format!(
                        "could not read SMB password file {}: {e}",
                        share.password_file
                    )
                })?
                .trim()
                .to_owned();
            if password.is_empty() {
                return Err(format!(
                    "SMB password file {} is empty",
                    share.password_file
                ));
            }
            if let Some(existing) = passwords.insert(share.username.clone(), password.clone()) {
                if existing != password {
                    return Err(format!(
                        "SMB username {} is configured with different passwords",
                        share.username
                    ));
                }
            }
        }
        for (username, password) in passwords {
            ensure_smb_user(&username, &password)?;
        }

        if !vm_only.is_empty() {
            self.start_smb_scope("vm", &cfg.smb.vm_iface, &vm_only)?;
        }
        if !global.is_empty() {
            let interface = cfg.smb.lan_iface.as_deref().ok_or_else(|| {
                "[smb].lan_iface is required when an SMB share has scope = global".to_string()
            })?;
            self.start_smb_scope("lan", interface, &global)?;
        }
        Ok(())
    }

    fn start_smb_scope(
        &mut self,
        scope: &str,
        interface: &str,
        shares: &[&SmbShareConfig],
    ) -> Result<(), String> {
        let address = ipv4_address(interface)?;
        let mut config = format!(
            "[global]\nworkgroup = WORKGROUP\nserver role = standalone server\ninterfaces = {address}\nbind interfaces only = yes\nmap to guest = Never\ndisable netbios = yes\nsmb ports = 445\nload printers = no\nprinting = bsd\npassdb backend = smbpasswd\nsmb passwd file = /etc/samba/smbpasswd\n"
        );
        for share in shares {
            let mountpoint = storage::resolve(share.storage).map_err(|e| {
                format!(
                    "could not resolve SMB share {} storage={}: {e}",
                    share.name, share.storage
                )
            })?;
            let path = mountpoint.join(&share.host_path);
            fs::create_dir_all(&path).map_err(|e| {
                format!(
                    "could not create SMB share directory {}: {e}",
                    path.display()
                )
            })?;
            config.push_str(&format!(
                "\n[{}]\npath = {}\nread only = {}\nvalid users = {}\nguest ok = no\nbrowseable = yes\n",
                share.share_name,
                path.display(),
                if share.read_only { "yes" } else { "no" },
                share.username
            ));
        }
        let config_path = format!("/run/native-qemu-smb-{scope}.conf");
        fs::write(&config_path, config)
            .map_err(|e| format!("could not write SMB config {config_path}: {e}"))?;
        let child = spawn_checked(
            Command::new("/usr/sbin/smbd")
                .arg("-F")
                .arg("--no-process-group")
                .arg("-s")
                .arg(&config_path),
            &format!("SMB server for {scope}"),
        )?;
        println!(
            "native-qemu: serving {} SMB share(s) on {interface} ({address})",
            shares.len()
        );
        self.children.push(child);
        Ok(())
    }

    fn start_macvtap(&mut self, cfg: &Config) -> Result<(), String> {
        let parent = cfg.network.bridge_iface.as_deref().ok_or_else(|| {
            "network.bridge_iface is required when network.mode = macvtap".to_string()
        })?;
        let interface = format!("nqmv{}", std::process::id());
        if interface.len() > 15 {
            return Err("generated macvtap interface name is too long".into());
        }
        let status = Command::new("ip")
            .args([
                "link", "add", "link", parent, "name", &interface, "type", "macvtap", "mode",
                "bridge",
            ])
            .status()
            .map_err(|e| format!("could not create macvtap on {parent}: {e}"))?;
        if !status.success() {
            return Err(format!("could not create macvtap on {parent}: {status}"));
        }
        let cleanup = || {
            let _ = Command::new("ip")
                .args(["link", "delete", "dev", &interface])
                .status();
        };
        let result = (|| {
            let status = Command::new("ip")
                .args(["link", "set", "dev", &interface, "up"])
                .status()
                .map_err(|e| format!("could not activate macvtap {interface}: {e}"))?;
            if !status.success() {
                return Err(format!("could not activate macvtap {interface}: {status}"));
            }
            let ifindex = fs::read_to_string(format!("/sys/class/net/{interface}/ifindex"))
                .map_err(|e| format!("could not find macvtap interface {interface}: {e}"))?
                .trim()
                .to_owned();
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .open(format!("/dev/tap{ifindex}"))
                .map_err(|e| format!("could not open /dev/tap{ifindex} for macvtap: {e}"))?;
            Ok(file)
        })();
        let file = match result {
            Ok(file) => file,
            Err(error) => {
                cleanup();
                return Err(error);
            }
        };
        println!("native-qemu: attached macvtap {interface} to {parent}");
        self.macvtap = Some(MacvtapRuntime { file, interface });
        Ok(())
    }
}

impl Drop for RuntimeServices {
    fn drop(&mut self) {
        for child in &mut self.children {
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = fs::remove_file(VIRTIOFS_SOCKET);
        if let Some(macvtap) = self.macvtap.take() {
            drop(macvtap.file);
            let _ = Command::new("ip")
                .args(["link", "delete", "dev", &macvtap.interface])
                .status();
        }
    }
}

fn spawn_checked(command: &mut Command, name: &str) -> Result<Child, String> {
    command.stdout(Stdio::inherit()).stderr(Stdio::inherit());
    let mut child = command
        .spawn()
        .map_err(|e| format!("could not start {name}: {e}"))?;
    std::thread::sleep(Duration::from_millis(100));
    if let Some(status) = child
        .try_wait()
        .map_err(|e| format!("could not inspect {name} startup: {e}"))?
    {
        return Err(format!("{name} exited during startup with {status}"));
    }
    Ok(child)
}

fn ipv4_address(interface: &str) -> Result<Ipv4Addr, String> {
    let output = Command::new("ip")
        .args(["-o", "-4", "addr", "show", "dev", interface])
        .output()
        .map_err(|e| format!("could not inspect interface {interface}: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "could not inspect interface {interface}: ip exited with {}",
            output.status
        ));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    for field in text.split_whitespace() {
        if let Some((address, _prefix)) = field.split_once('/') {
            if let Ok(parsed) = address.parse() {
                return Ok(parsed);
            }
        }
    }
    Err(format!("interface {interface} has no IPv4 address"))
}

fn validate_smb_identifier(value: &str, field: &str) -> Result<(), String> {
    if value.is_empty()
        || !value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(format!(
            "SMB {field} must contain only letters, digits, '_' or '-'"
        ));
    }
    Ok(())
}

fn ensure_smb_user(username: &str, password: &str) -> Result<(), String> {
    let exists = fs::read_to_string("/etc/passwd")
        .map(|contents| {
            contents
                .lines()
                .any(|line| line.starts_with(&format!("{username}:")))
        })
        .unwrap_or(false);
    if !exists {
        let status = Command::new("adduser")
            .args(["-D", "-H", "-s", "/sbin/nologin", username])
            .status()
            .map_err(|e| format!("could not create SMB user {username}: {e}"))?;
        if !status.success() {
            return Err(format!("could not create SMB user {username}: {status}"));
        }
    }
    let mut child = Command::new("smbpasswd")
        .args(["-s", "-a", username])
        .stdin(Stdio::piped())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| format!("could not set SMB password for {username}: {e}"))?;
    let stdin = child
        .stdin
        .as_mut()
        .ok_or_else(|| format!("could not open SMB password input for {username}"))?;
    writeln!(stdin, "{password}").map_err(|e| format!("could not send SMB password: {e}"))?;
    writeln!(stdin, "{password}").map_err(|e| format!("could not confirm SMB password: {e}"))?;
    let status = child
        .wait()
        .map_err(|e| format!("could not wait for smbpasswd: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "could not set SMB password for {username}: {status}"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{ipv4_address, validate_smb_identifier};

    #[test]
    fn unknown_interface_reports_a_useful_error() {
        let error = ipv4_address("native-qemu-no-such-interface").unwrap_err();
        assert!(error.contains("native-qemu-no-such-interface"));
    }

    #[test]
    fn smb_identifiers_cannot_inject_samba_config() {
        assert!(validate_smb_identifier("native-qemu", "share_name").is_ok());
        assert!(validate_smb_identifier("bad\n[global]", "share_name").is_err());
    }
}
