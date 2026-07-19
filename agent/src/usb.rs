use crate::config::UsbConfig;
use std::collections::HashSet;
use std::fs;
use std::path::Path;

/// USB vendor ID of Linux's virtual root hub devices — never something
/// meaningful to pass through, so it's excluded from the "passthrough
/// everything" default regardless of config.
const ROOT_HUB_VENDOR: &str = "1d6b";

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct UsbEntry {
    pub vendor_id: String,
    pub product_id: String,
    /// QEMU's usb-host device can select the exact physical device with this
    /// bus/address pair.  Vendor/product alone is ambiguous for two
    /// identical keyboards, mice, or storage devices.
    pub bus_num: u16,
    pub device_num: u16,
}

fn read_id(path: &Path) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_lowercase())
}

/// Enumerates physically present USB devices (not interfaces, not the
/// virtual root hubs) by reading /sys/bus/usb/devices directly.
pub fn enumerate_present() -> Vec<UsbEntry> {
    let mut out = Vec::new();
    let entries = match fs::read_dir("/sys/bus/usb/devices") {
        Ok(e) => e,
        Err(_) => return out,
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        // Interfaces look like "1-1:1.0"; devices look like "1-1" or "usb1".
        if name.contains(':') {
            continue;
        }
        let path = entry.path();
        let vendor = read_id(&path.join("idVendor"));
        let product = read_id(&path.join("idProduct"));
        let bus_num = read_id(&path.join("busnum")).and_then(|n| n.parse().ok());
        let device_num = read_id(&path.join("devnum")).and_then(|n| n.parse().ok());
        if let (Some(vendor_id), Some(product_id), Some(bus_num), Some(device_num)) =
            (vendor, product, bus_num, device_num)
        {
            if vendor_id == ROOT_HUB_VENDOR {
                continue;
            }
            out.push(UsbEntry {
                vendor_id,
                product_id,
                bus_num,
                device_num,
            });
        }
    }
    out
}

fn normalize(id: &str) -> String {
    id.trim().trim_start_matches("0x").to_lowercase()
}

/// Applies config.usb's policy (default passthrough/host-only + exclude
/// list + explicit device entries) to the currently present USB devices,
/// returning the final set to pass through to the guest.
pub fn resolve(cfg: &UsbConfig) -> Vec<UsbEntry> {
    let present = enumerate_present();
    let excluded: HashSet<String> = cfg
        .exclude
        .iter()
        .map(|s| {
            let (v, p) = s.split_once(':').unwrap_or((s.as_str(), ""));
            format!("{}:{}", normalize(v), normalize(p))
        })
        .collect();

    let key = |e: &UsbEntry| format!("{}:{}", e.vendor_id, e.product_id);

    let mut result: Vec<UsbEntry> = if cfg.default == "passthrough" {
        present
            .iter()
            .filter(|e| !excluded.contains(&key(e)))
            .cloned()
            .collect()
    } else {
        Vec::new()
    };

    // Explicit [[usb.device]] entries are always included if physically
    // present, overriding both the default policy and the exclude list.
    for dev in &cfg.device {
        let entry = UsbEntry {
            vendor_id: normalize(&dev.vendor_id),
            product_id: normalize(&dev.product_id),
            // These fields are not used when comparing a configured ID with
            // a present device; see the ID-only comparison below.
            bus_num: 0,
            device_num: 0,
        };
        for present_entry in &present {
            if present_entry.vendor_id == entry.vendor_id
                && present_entry.product_id == entry.product_id
                && !result.contains(present_entry)
            {
                result.push(present_entry.clone());
            }
        }
    }

    result
}

/// Which of the explicitly configured, `required = true` devices are not
/// currently present — used to trigger `lifecycle.on_missing_resource`.
pub fn missing_required(cfg: &UsbConfig) -> Vec<String> {
    let present = enumerate_present();
    cfg.device
        .iter()
        .filter(|d| d.required)
        .filter(|d| {
            let entry = UsbEntry {
                vendor_id: normalize(&d.vendor_id),
                product_id: normalize(&d.product_id),
                bus_num: 0,
                device_num: 0,
            };
            !present.iter().any(|present_entry| {
                present_entry.vendor_id == entry.vendor_id
                    && present_entry.product_id == entry.product_id
            })
        })
        .map(|d| {
            d.name
                .clone()
                .unwrap_or_else(|| format!("{}:{}", d.vendor_id, d.product_id))
        })
        .collect()
}
