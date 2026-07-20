mod blockdev;
mod ext4_io;
mod flash;
mod format_size;
mod partition;
mod schema;
mod sized_disk;
mod syncutil;
mod tools;
mod ui;
mod volume;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "nq-disk",
    about = "native-qemu USB tool: flash ISO, edit config, manage data volume"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Option<Commands>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Interactive step-by-step menu (default)
    Menu,
    /// List whole disks (system disk marked)
    ListDisks,
    /// Check tools/permissions/ISO/disk before flashing
    Preflight {
        #[arg(long)]
        iso: PathBuf,
        #[arg(long)]
        disk: PathBuf,
    },
    /// Flash ISO to a whole disk (destructive)
    Flash {
        #[arg(long)]
        iso: PathBuf,
        #[arg(long)]
        disk: PathBuf,
        /// Optional host path to seed as image.qcow2
        #[arg(long)]
        image: Option<PathBuf>,
        /// Required confirmation
        #[arg(long)]
        yes: bool,
    },
    /// Validate a config.toml file
    ValidateConfig { path: PathBuf },
    /// Open labeled volume and print summary
    Status,
    /// Copy host image to volume as image.qcow2
    PutImage {
        /// Mounted directory OR raw ext4 partition/device
        #[arg(long)]
        volume: Option<PathBuf>,
        /// Host qcow2/raw image
        image: PathBuf,
    },
    /// Write default config.toml onto a volume
    SeedConfig {
        #[arg(long)]
        volume: PathBuf,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd.unwrap_or(Commands::Menu) {
        Commands::Menu => ui::run_app(),
        Commands::ListDisks => {
            for d in flash::list_disks()? {
                let mut marks = Vec::new();
                if d.is_system {
                    marks.push("SYSTEM");
                }
                if d.is_external {
                    marks.push("external");
                }
                let mark = if marks.is_empty() {
                    String::new()
                } else {
                    format!(" [{}]", marks.join(", "))
                };
                println!(
                    "{}  {}  {}{mark}",
                    d.display_path.display(),
                    human(d.size_bytes),
                    d.model
                );
            }
            Ok(())
        }
        Commands::Preflight { iso, disk } => {
            let p = tools::preflight_flash(&iso, &disk);
            for m in &p.messages {
                println!("ok: {m}");
            }
            for e in &p.errors {
                eprintln!("error: {e}");
            }
            if !p.ok {
                anyhow::bail!("preflight failed");
            }
            println!("preflight OK");
            Ok(())
        }
        Commands::Flash {
            iso,
            disk,
            image,
            yes,
        } => {
            if !yes {
                anyhow::bail!("refusing to flash without --yes (destructive)");
            }
            let part = flash::flash_iso(&iso, &disk, image.as_deref(), |p| {
                let pct = if p.bytes_total > 0 {
                    (p.bytes_done * 100) / p.bytes_total
                } else {
                    0
                };
                eprint!("\r[{pct:3}%] {:<40}", p.phase);
                let _ = std::io::Write::flush(&mut std::io::stderr());
            })?;
            eprintln!();
            println!(
                "flash complete — data volume on {} LBA {} ({} MiB)",
                part.raw_path.display(),
                part.first_lba,
                part.size_bytes / (1024 * 1024)
            );
            println!("edit config / load image, then unmount and boot target");
            Ok(())
        }
        Commands::ValidateConfig { path } => {
            let text = std::fs::read_to_string(&path)?;
            match schema::validate_config_text(&text) {
                Ok(()) => {
                    println!("OK: {}", path.display());
                    Ok(())
                }
                Err(errs) => {
                    for e in errs {
                        eprintln!("error: {e}");
                    }
                    anyhow::bail!("validation failed");
                }
            }
        }
        Commands::Status => {
            let v = volume::Volume::discover()?;
            println!("volume: {}", v.root_display());
            println!(
                "config.toml: {}",
                if v.exists(volume::CONFIG_NAME) {
                    "yes"
                } else {
                    "no"
                }
            );
            println!(
                "image.qcow2: {}",
                if v.exists(volume::IMAGE_NAME) {
                    "yes"
                } else {
                    "no"
                }
            );
            for e in v.list("")? {
                println!(
                    "  {}{}  {}",
                    e.name,
                    if e.is_dir { "/" } else { "" },
                    human(e.size)
                );
            }
            Ok(())
        }
        Commands::PutImage { volume, image } => {
            let v = match volume {
                Some(p) if p.is_dir() => volume::Volume::open_path(p)?,
                Some(p) => volume::Volume::open_device(p)?,
                None => volume::Volume::discover()?,
            };
            let n = v.put_image(&image)?;
            println!(
                "wrote {n} bytes → {}/{}",
                v.root_display(),
                volume::IMAGE_NAME
            );
            Ok(())
        }
        Commands::SeedConfig { volume } => {
            volume::seed_volume(&volume, None)?;
            println!("seeded config.toml on {}", volume.display());
            Ok(())
        }
    }
}

fn human(n: u64) -> String {
    format_size::format_size(n)
}
