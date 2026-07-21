//! Step-by-step interactive menu (`>` selection).

use crate::flash::{self, DiskInfo};
use crate::tools;
use crate::ui::editor;
use crate::volume::{Volume, CONFIG_NAME, IMAGE_NAME};
use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::ExecutableCommand;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::DefaultTerminal;
use std::io::{self, stdout};
use std::path::PathBuf;
use std::time::Duration;

enum Screen {
    Main,
    /// Only used when an ISO was auto-detected (confirm / pick another).
    FlashSelectIso,
    FlashIsoPath { input: String, cursor: usize },
    FlashSelectDisk,
    FlashPreflight { report: String, ok: bool },
    FlashConfirm,
    FlashDone { msg: String },
    NeedVolume { next: VolumeAction },
    VolumePathInput { next: VolumeAction, input: String },
    FileManager {
        path: String,
        entries: Vec<String>,
        state: ListState,
    },
    FilePutInput { input: String },
    FileGetInput { remote: String, input: String },
    LoadImageInput { input: String },
    Message { title: String, body: String },
}

#[derive(Clone, Copy)]
enum VolumeAction {
    Files,
    Config,
    LoadImage,
    Unmount,
}

struct App {
    screen: Screen,
    menu_state: ListState,
    menu_items: Vec<&'static str>,
    flash_iso: Option<PathBuf>,
    flash_disks: Vec<DiskInfo>,
    disk_state: ListState,
    selected_disk: Option<PathBuf>,
    volume: Option<Volume>,
    status: String,
    should_quit: bool,
}

impl App {
    fn new() -> Self {
        let mut menu_state = ListState::default();
        menu_state.select(Some(0));
        let mut app = Self {
            screen: Screen::Main,
            menu_state,
            menu_items: vec![
                "Flash ISO to USB",
                "File manager",
                "Edit config",
                "Load image",
                "Unmount USB",
                "Quit",
            ],
            flash_iso: flash::find_iso_near_cwd(),
            flash_disks: Vec::new(),
            disk_state: ListState::default(),
            selected_disk: None,
            volume: None,
            status: String::new(),
            should_quit: false,
        };
        app.refresh_status();
        app
    }

    fn refresh_status(&mut self) {
        let iso = self
            .flash_iso
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "(none)".into());
        let vol = self
            .volume
            .as_ref()
            .map(|v| v.root_display())
            .unwrap_or_else(|| "(not open)".into());
        self.status = format!("ISO: {iso}   volume: {vol}   root ✓");
    }

    fn ensure_volume(&mut self) -> Result<bool> {
        if self.volume.is_some() {
            return Ok(true);
        }
        match Volume::discover() {
            Ok(v) => {
                self.volume = Some(v);
                self.refresh_status();
                Ok(true)
            }
            Err(e) => {
                // Stash error for the NeedVolume screen
                self.status = format!("volume: {e}");
                Ok(false)
            }
        }
    }

    fn selectable_disks(&self) -> Vec<&DiskInfo> {
        let mut disks: Vec<&DiskInfo> = self.flash_disks.iter().filter(|d| !d.is_system).collect();
        // Prefer external USB sticks first
        disks.sort_by_key(|d| if d.is_external { 0 } else { 1 });
        disks
    }
}

pub fn run_app() -> Result<()> {
    install_ctrl_c_handler();
    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;
    let mut terminal = ratatui::init();
    // Editor paints its own yellow caret; hide the OS/terminal block so we
    // never show two cursors (yellow cell + white/default block).
    let _ = terminal.hide_cursor();
    let mut app = App::new();
    let res = run_loop(&mut terminal, &mut app);
    // Always restore terminal, even on error / panic path of run_loop.
    let _ = ratatui::restore();
    let _ = disable_raw_mode();
    let _ = stdout().execute(LeaveAlternateScreen);
    res
}

fn install_ctrl_c_handler() {
    // Second line of defence: if raw mode ate the first Ctrl+C as a key,
    // SIGINT still restores the terminal and exits. Atomic so double Ctrl+C
    // force-exits even if the first one is mid-flash.
    use std::sync::atomic::{AtomicU8, Ordering};
    static CTRL_C_COUNT: AtomicU8 = AtomicU8::new(0);

    let _ = ctrlc::set_handler(move || {
        let n = CTRL_C_COUNT.fetch_add(1, Ordering::SeqCst) + 1;
        // Best-effort restore so the user's shell is usable again.
        let _ = disable_raw_mode();
        let _ = stdout().execute(LeaveAlternateScreen);
        let _ = ratatui::restore();
        eprintln!();
        if n >= 2 {
            eprintln!("nq-disk: forced exit (Ctrl+C ×2)");
            std::process::exit(130);
        }
        eprintln!("nq-disk: interrupted — press Ctrl+C again to force quit");
        // First signal: exit cleanly from UI if possible; if we're stuck in
        // flash I/O, second Ctrl+C force-exits.
        std::process::exit(130);
    });
}

fn run_loop(terminal: &mut DefaultTerminal, app: &mut App) -> Result<()> {
    loop {
        terminal.draw(|f| draw(f, app))?;
        if app.should_quit {
            break;
        }
        if !event::poll(Duration::from_millis(200))? {
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        // Ctrl+C / Ctrl+Q always quit from the TUI (raw mode eats SIGINT otherwise).
        if key.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(key.code, KeyCode::Char('c') | KeyCode::Char('C') | KeyCode::Char('q'))
        {
            app.should_quit = true;
            break;
        }
        handle_key(app, key.code, key.modifiers, terminal)?;
    }
    Ok(())
}

fn handle_key(
    app: &mut App,
    code: KeyCode,
    mods: KeyModifiers,
    terminal: &mut DefaultTerminal,
) -> Result<()> {
    let _ = mods;
    let mut fm_reload: Option<String> = None;
    let mut fm_delete: Option<(String, String)> = None;
    let mut fm_put = false;
    let mut fm_get: Option<String> = None;

    match &mut app.screen {
        Screen::Main => match code {
            KeyCode::Up | KeyCode::Char('k') => {
                select_prev(&mut app.menu_state, app.menu_items.len())
            }
            KeyCode::Down | KeyCode::Char('j') => {
                select_next(&mut app.menu_state, app.menu_items.len())
            }
            KeyCode::Enter => activate_main(app, terminal)?,
            KeyCode::Char('q') | KeyCode::Esc => app.should_quit = true,
            _ => {}
        },
        Screen::FlashSelectIso => match code {
            KeyCode::Enter => {
                if app.flash_iso.is_some() {
                    load_disks(app)?;
                    app.screen = Screen::FlashSelectDisk;
                }
            }
            KeyCode::Char('p') | KeyCode::Char('P') => {
                app.screen = Screen::FlashIsoPath {
                    input: String::new(),
                    cursor: 0,
                };
            }
            KeyCode::Esc => app.screen = Screen::Main,
            _ => {}
        },
        Screen::FlashIsoPath { input, cursor } => match code {
            KeyCode::Left => {
                if *cursor > 0 {
                    *cursor -= 1;
                }
            }
            KeyCode::Right => {
                if *cursor < input.chars().count() {
                    *cursor += 1;
                }
            }
            KeyCode::Home => *cursor = 0,
            KeyCode::End => *cursor = input.chars().count(),
            KeyCode::Char(c) => {
                // Ignore control-ish chars; insert at cursor.
                if !c.is_control() {
                    let byte = char_byte_index(input, *cursor);
                    input.insert(byte, c);
                    *cursor += 1;
                }
            }
            KeyCode::Backspace => {
                if *cursor > 0 {
                    let byte = char_byte_index(input, *cursor - 1);
                    input.remove(byte);
                    *cursor -= 1;
                }
            }
            KeyCode::Delete => {
                if *cursor < input.chars().count() {
                    let byte = char_byte_index(input, *cursor);
                    input.remove(byte);
                }
            }
            KeyCode::Enter => {
                let trimmed = input.trim().to_string();
                // Expand ~ if present
                let expanded = if let Some(rest) = trimmed.strip_prefix("~/") {
                    if let Some(home) = std::env::var_os("HOME") {
                        PathBuf::from(home).join(rest)
                    } else {
                        PathBuf::from(&trimmed)
                    }
                } else {
                    PathBuf::from(&trimmed)
                };
                if expanded.is_file() {
                    app.flash_iso = Some(expanded);
                    app.refresh_status();
                    load_disks(app)?;
                    app.screen = Screen::FlashSelectDisk;
                } else {
                    app.screen = Screen::Message {
                        title: "ISO not found".into(),
                        body: format!(
                            "Not a file:\n  {trimmed}\n\n\
                             Tip: paste a full path, e.g.\n  /Users/you/Downloads/native-qemu-….iso\n\
                             Use ← → to move the cursor."
                        ),
                    };
                }
            }
            KeyCode::Esc => app.screen = Screen::Main,
            _ => {}
        },
        Screen::FlashSelectDisk => {
            let n = app.selectable_disks().len();
            match code {
                KeyCode::Up | KeyCode::Char('k') => select_prev(&mut app.disk_state, n),
                KeyCode::Down | KeyCode::Char('j') => select_next(&mut app.disk_state, n),
                KeyCode::Enter => {
                    let disk_path = {
                        let disks = app.selectable_disks();
                        app.disk_state
                            .selected()
                            .and_then(|i| disks.get(i).map(|d| d.path.clone()))
                    };
                    if let Some(disk) = disk_path {
                        app.selected_disk = Some(disk.clone());
                        let iso = app.flash_iso.clone().unwrap_or_default();
                        let report = tools::preflight_flash(&iso, &disk);
                        let mut body = String::new();
                        for m in &report.messages {
                            body.push_str(&format!("  ✓ {m}\n"));
                        }
                        for e in &report.errors {
                            body.push_str(&format!("  ✗ {e}\n"));
                        }
                        app.screen = Screen::FlashPreflight {
                            report: body,
                            ok: report.ok,
                        };
                    }
                }
                KeyCode::Esc => app.screen = Screen::FlashSelectIso,
                _ => {}
            }
        }
        Screen::FlashPreflight { ok, .. } => match code {
            KeyCode::Enter if *ok => app.screen = Screen::FlashConfirm,
            KeyCode::Enter if !*ok => app.screen = Screen::FlashSelectDisk,
            KeyCode::Esc => app.screen = Screen::FlashSelectDisk,
            _ => {}
        },
        Screen::FlashConfirm => match code {
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                run_flash(app, terminal)?;
            }
            KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => {
                app.screen = Screen::FlashSelectDisk;
            }
            _ => {}
        },
        Screen::FlashDone { .. } | Screen::Message { .. } => match code {
            KeyCode::Enter | KeyCode::Esc | KeyCode::Char('q') => {
                app.screen = Screen::Main;
                app.refresh_status();
            }
            _ => {}
        },
        Screen::NeedVolume { next } => match code {
            KeyCode::Enter => {
                let next = *next;
                app.screen = Screen::VolumePathInput {
                    next,
                    input: String::new(),
                };
            }
            KeyCode::Esc => app.screen = Screen::Main,
            _ => {}
        },
        Screen::VolumePathInput { next, input } => match code {
            KeyCode::Char(c) => {
                if !c.is_control() {
                    input.push(c);
                }
            }
            KeyCode::Backspace => {
                input.pop();
            }
            KeyCode::Enter => {
                let path = PathBuf::from(input.trim());
                let next = *next;
                // Directory → host mount. Whole disk → GPT scan for native-qemu.
                // Partition node → open as ext4 device.
                let opened = if path.is_dir() {
                    Volume::open_path(&path)
                } else if let Ok(part) = crate::partition::find_data_partition_on_disk(&path) {
                    Volume::open_from_partition(&part)
                } else {
                    Volume::open_device(&path)
                };
                match opened {
                    Ok(v) => {
                        app.volume = Some(v);
                        app.refresh_status();
                        open_volume_action(app, next, terminal)?;
                    }
                    Err(e) => {
                        app.screen = Screen::Message {
                            title: "Open volume failed".into(),
                            body: format!("{e:#}"),
                        };
                    }
                }
            }
            KeyCode::Esc => app.screen = Screen::Main,
            _ => {}
        },
        Screen::FileManager {
            path,
            entries,
            state,
        } => {
            let n = entries.len();
            match code {
                KeyCode::Up | KeyCode::Char('k') => select_prev(state, n),
                KeyCode::Down | KeyCode::Char('j') => select_next(state, n),
                KeyCode::Esc | KeyCode::Char('q') => app.screen = Screen::Main,
                KeyCode::Char('d') => {
                    if let Some(i) = state.selected() {
                        if let Some(name) = entries.get(i) {
                            if name.as_str() != "../" {
                                fm_delete = Some((
                                    path.clone(),
                                    join_rel(path, name.split_whitespace().next().unwrap_or("")),
                                ));
                            }
                        }
                    }
                }
                KeyCode::Char('p') => fm_put = true,
                KeyCode::Char('c') => {
                    if let Some(i) = state.selected() {
                        if let Some(name) = entries.get(i) {
                            if !name.ends_with('/') && name != "../" {
                                let remote =
                                    join_rel(path, name.split_whitespace().next().unwrap_or(""));
                                fm_get = Some(remote);
                            }
                        }
                    }
                }
                KeyCode::Enter => {
                    if let Some(i) = state.selected() {
                        if let Some(name) = entries.get(i) {
                            let bare = name.split_whitespace().next().unwrap_or("");
                            if bare.ends_with('/') || bare == "../" {
                                fm_reload = Some(if bare == "../" {
                                    parent_rel(path)
                                } else {
                                    join_rel(path, bare.trim_end_matches('/'))
                                });
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        Screen::FilePutInput { input } => match code {
            KeyCode::Char(c) => input.push(c),
            KeyCode::Backspace => {
                input.pop();
            }
            KeyCode::Enter => {
                let host = PathBuf::from(input.trim());
                let name = host
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("file")
                    .to_string();
                match app.volume.as_ref() {
                    Some(v) => match v.copy_from_host(&host, &name) {
                        Ok(n) => {
                            app.screen = Screen::Message {
                                title: "Copied to volume".into(),
                                body: format!("{n} bytes → {name}"),
                            };
                        }
                        Err(e) => {
                            app.screen = Screen::Message {
                                title: "Copy failed".into(),
                                body: format!("{e:#}"),
                            };
                        }
                    },
                    None => {
                        app.screen = Screen::Message {
                            title: "No volume".into(),
                            body: "Open a volume first.".into(),
                        };
                    }
                }
            }
            KeyCode::Esc => {
                let _ = reload_fm(app, "".into());
            }
            _ => {}
        },
        Screen::FileGetInput { remote, input } => match code {
            KeyCode::Char(c) => input.push(c),
            KeyCode::Backspace => {
                input.pop();
            }
            KeyCode::Enter => {
                let host = PathBuf::from(input.trim());
                let remote = remote.clone();
                match app.volume.as_ref() {
                    Some(v) => match v.copy_to_host(&remote, &host) {
                        Ok(n) => {
                            app.screen = Screen::Message {
                                title: "Copied to host".into(),
                                body: format!("{n} bytes → {}", host.display()),
                            };
                        }
                        Err(e) => {
                            app.screen = Screen::Message {
                                title: "Copy failed".into(),
                                body: format!("{e:#}"),
                            };
                        }
                    },
                    None => {}
                }
            }
            KeyCode::Esc => {
                let _ = reload_fm(app, "".into());
            }
            _ => {}
        },
        Screen::LoadImageInput { input } => match code {
            KeyCode::Char(c) => input.push(c),
            KeyCode::Backspace => {
                input.pop();
            }
            KeyCode::Enter => {
                let host = PathBuf::from(input.trim());
                match app.volume.as_ref() {
                    Some(v) => match v.put_image(&host) {
                        Ok(n) => {
                            app.screen = Screen::Message {
                                title: "Image loaded".into(),
                                body: format!(
                                    "Wrote {n} bytes as {IMAGE_NAME}\n\
                                     Config still points at image.qcow2 — no edit needed."
                                ),
                            };
                        }
                        Err(e) => {
                            app.screen = Screen::Message {
                                title: "Load image failed".into(),
                                body: format!("{e:#}"),
                            };
                        }
                    },
                    None => {
                        app.screen = Screen::Message {
                            title: "No volume".into(),
                            body: "Open a volume first.".into(),
                        };
                    }
                }
            }
            KeyCode::Esc => app.screen = Screen::Main,
            _ => {}
        },
    }

    if fm_put {
        app.screen = Screen::FilePutInput {
            input: String::new(),
        };
    } else if let Some(remote) = fm_get {
        app.screen = Screen::FileGetInput {
            remote,
            input: String::new(),
        };
    } else if let Some((dir, rel)) = fm_delete {
        if let Some(v) = app.volume.as_ref() {
            let _ = v.remove(&rel);
        }
        reload_fm(app, dir)?;
    } else if let Some(path) = fm_reload {
        reload_fm(app, path)?;
    }
    Ok(())
}

fn activate_main(app: &mut App, terminal: &mut DefaultTerminal) -> Result<()> {
    let i = app.menu_state.selected().unwrap_or(0);
    match i {
        0 => {
            if app.flash_iso.is_none() {
                app.flash_iso = flash::find_iso_near_cwd();
            }
            if app.flash_iso.is_none() {
                // No ISO nearby — go straight to path entry (no extra menu step).
                app.screen = Screen::FlashIsoPath {
                    input: String::new(),
                    cursor: 0,
                };
            } else {
                // Confirm auto-found ISO, or continue to disks.
                app.screen = Screen::FlashSelectIso;
            }
        }
        1 => begin_volume_action(app, VolumeAction::Files, terminal)?,
        2 => begin_volume_action(app, VolumeAction::Config, terminal)?,
        3 => begin_volume_action(app, VolumeAction::LoadImage, terminal)?,
        4 => begin_volume_action(app, VolumeAction::Unmount, terminal)?,
        5 => app.should_quit = true,
        _ => {}
    }
    Ok(())
}

fn begin_volume_action(
    app: &mut App,
    action: VolumeAction,
    terminal: &mut DefaultTerminal,
) -> Result<()> {
    if app.ensure_volume()? {
        open_volume_action(app, action, terminal)?;
    } else {
        app.screen = Screen::NeedVolume { next: action };
    }
    Ok(())
}

fn open_volume_action(
    app: &mut App,
    action: VolumeAction,
    terminal: &mut DefaultTerminal,
) -> Result<()> {
    match action {
        VolumeAction::Files => reload_fm(app, "".into())?,
        VolumeAction::Config => run_editor(app, terminal)?,
        VolumeAction::LoadImage => {
            app.screen = Screen::LoadImageInput {
                input: String::new(),
            };
        }
        VolumeAction::Unmount => {
            if let Some(v) = app.volume.take() {
                match v.unmount() {
                    Ok(()) => {
                        app.screen = Screen::Message {
                            title: "Unmounted".into(),
                            body: "Safe to remove the USB or plug it into the target machine."
                                .into(),
                        };
                    }
                    Err(e) => {
                        app.screen = Screen::Message {
                            title: "Unmount failed".into(),
                            body: format!("{e:#}"),
                        };
                    }
                }
                app.refresh_status();
            }
        }
    }
    Ok(())
}

fn run_editor(app: &mut App, terminal: &mut DefaultTerminal) -> Result<()> {
    let text = {
        let vol = app.volume.as_ref().expect("volume");
        vol.ensure_config()?
    };
    let result = editor::run_config_editor(terminal, &text)?;
    if let Some(new_text) = result {
        let vol = app.volume.as_ref().expect("volume");
        vol.write_text(CONFIG_NAME, &new_text)?;
        app.screen = Screen::Message {
            title: "Config saved".into(),
            body: format!("{CONFIG_NAME} written and validated."),
        };
    } else {
        app.screen = Screen::Main;
    }
    Ok(())
}

fn reload_fm(app: &mut App, path: String) -> Result<()> {
    let vol = app.volume.as_ref().expect("volume");
    let mut entries = Vec::new();
    if !path.is_empty() {
        entries.push("../".to_string());
    }
    for e in vol.list(&path)? {
        if e.is_dir {
            entries.push(format!("{}/", e.name));
        } else {
            entries.push(format!("{:<24} {:>10}", e.name, human_size(e.size)));
        }
    }
    let mut state = ListState::default();
    state.select(Some(0));
    app.screen = Screen::FileManager {
        path,
        entries,
        state,
    };
    Ok(())
}

fn load_disks(app: &mut App) -> Result<()> {
    app.flash_disks = flash::list_disks().unwrap_or_default();
    app.disk_state.select(Some(0));
    Ok(())
}

fn run_flash(app: &mut App, terminal: &mut DefaultTerminal) -> Result<()> {
    let iso = match &app.flash_iso {
        Some(p) => p.clone(),
        None => {
            app.screen = Screen::Message {
                title: "No ISO".into(),
                body: "Place a native-qemu*.iso next to nq-disk or type its path.".into(),
            };
            return Ok(());
        }
    };
    let disk = match &app.selected_disk {
        Some(d) => d.clone(),
        None => return Ok(()),
    };

    disable_for_flash(terminal)?;
    println!();
    println!("Flashing {} → {}", iso.display(), disk.display());
    println!("This ERASES the whole device.");
    println!("Ctrl+C aborts (press twice to force-quit if stuck).");
    println!();
    let result = flash::flash_iso(&iso, &disk, None, |p| {
        let pct = if p.bytes_total > 0 {
            (p.bytes_done * 100) / p.bytes_total
        } else {
            0
        };
        print!("\r{:<40} {pct:3}%", p.phase);
        let _ = io::Write::flush(&mut io::stdout());
    });
    println!();
    reenable_after_flash(terminal)?;

    match result {
        Ok(part) => {
            // Open the volume we just created via GPT slice (macOS has no ext4 mount).
            match Volume::open_from_partition(&part) {
                Ok(v) => {
                    app.volume = Some(v);
                    app.refresh_status();
                }
                Err(e) => {
                    eprintln!("warning: could not re-open data volume: {e:#}");
                    let _ = app.ensure_volume();
                }
            }
            app.screen = Screen::FlashDone {
                msg: format!(
                    "Flash complete.\n\n\
                     Data volume: GPT 'native-qemu' on {}\n\
                     LBA {} · {} MiB · label native-qemu\n\
                     Seeded: config.toml (+ image.qcow2 if in ISO)\n\n\
                     Next: Edit config / Load image / File manager, then Unmount.\n\
                     (macOS does not mount ext4 in Finder — use this tool.)",
                    part.raw_path.display(),
                    part.first_lba,
                    part.size_bytes / (1024 * 1024)
                ),
            };
        }
        Err(e) => {
            app.screen = Screen::Message {
                title: "Flash failed".into(),
                body: format!("{e:#}\n\nPress Enter to return to menu (or Ctrl+C to quit)."),
            };
        }
    }
    Ok(())
}

fn disable_for_flash(terminal: &mut DefaultTerminal) -> Result<()> {
    disable_raw_mode()?;
    stdout().execute(LeaveAlternateScreen)?;
    terminal.clear()?;
    Ok(())
}

fn reenable_after_flash(terminal: &mut DefaultTerminal) -> Result<()> {
    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;
    terminal.clear()?;
    let _ = terminal.hide_cursor();
    Ok(())
}

fn draw(f: &mut ratatui::Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(3),
        ])
        .split(f.area());

    let header = Paragraph::new(vec![
        Line::from(Span::styled(
            " native-qemu disk tool (nq-disk)",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(app.status.as_str()),
    ])
    .block(Block::default().borders(Borders::ALL));
    f.render_widget(header, chunks[0]);

    match &app.screen {
        Screen::Main => {
            draw_chooser(
                f,
                chunks[1],
                "Main menu — what do you want to do?",
                "↑↓ select   Enter open   q quit",
                &app.menu_items,
                &app.menu_state,
            );
        }
        Screen::FlashSelectIso => {
            let iso = app
                .flash_iso
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "(no .iso found — press p to type a path)".into());
            let items = [iso.as_str(), "Type ISO path…"];
            let mut state = ListState::default();
            state.select(Some(0));
            draw_chooser(
                f,
                chunks[1],
                "Flash — step 1: ISO file",
                "Enter continue   p type path   Esc cancel",
                &items,
                &state,
            );
        }
        Screen::FlashIsoPath { input, cursor } => {
            let body = format!(
                "No ISO found next to nq-disk.\n\
                 Type or paste the full path to the .iso file:\n\n\
                 {}\n\n\
                 ← → move cursor   Backspace/Delete edit   Enter continue   Esc cancel\n\
                 ~ / expands to your home directory.",
                path_with_cursor(input, *cursor)
            );
            f.render_widget(
                Paragraph::new(body).block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title("ISO path"),
                ),
                chunks[1],
            );
        }
        Screen::FlashSelectDisk => {
            let disks = app.selectable_disks();
            let labels: Vec<String> = disks
                .iter()
                .map(|d| {
                    let ext = if d.is_external { " USB" } else { "" };
                    format!(
                        "{}  {}  {}{ext}",
                        d.display_path.display(),
                        human_size(d.size_bytes),
                        d.model
                    )
                })
                .collect();
            let items: Vec<&str> = labels.iter().map(|s| s.as_str()).collect();
            draw_chooser(
                f,
                chunks[1],
                "Flash — step 2: select USB (system disk hidden)",
                "↑↓ select   Enter preflight   Esc back",
                &items,
                &app.disk_state,
            );
        }
        Screen::FlashPreflight { report, ok } => {
            let title = if *ok {
                "Flash — preflight OK"
            } else {
                "Flash — preflight FAILED"
            };
            let hint = if *ok {
                "\nEnter continue to confirm erase   Esc back"
            } else {
                "\nEnter/Esc back — re-run with:  sudo ./nq-disk"
            };
            f.render_widget(
                Paragraph::new(format!("{report}{hint}"))
                    .wrap(Wrap { trim: false })
                    .block(Block::default().borders(Borders::ALL).title(title)),
                chunks[1],
            );
        }
        Screen::FlashConfirm => {
            let disk = app
                .selected_disk
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default();
            let iso = app
                .flash_iso
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default();
            let body = format!(
                "ERASE and flash:\n  ISO:  {iso}\n  Disk: {disk}\n\n\
                 Steps: write ISO → verify → ext4 partition label=native-qemu\n\
                        → seed config.toml + image.qcow2\n\n\
                 > Yes, erase and flash\n  No (Esc)"
            );
            f.render_widget(
                Paragraph::new(body)
                    .wrap(Wrap { trim: false })
                    .block(
                        Block::default()
                            .borders(Borders::ALL)
                            .title("Flash — FINAL CONFIRM")
                            .border_style(Style::default().fg(Color::Red)),
                    ),
                chunks[1],
            );
        }
        Screen::FlashDone { msg } => {
            f.render_widget(
                Paragraph::new(msg.as_str())
                    .wrap(Wrap { trim: false })
                    .block(Block::default().borders(Borders::ALL).title("Flash done")),
                chunks[1],
            );
        }
        Screen::NeedVolume { .. } => {
            let body = format!(
                "Could not auto-detect the data volume.\n\
                 After flash, the tool scans GPT for partition name 'native-qemu'\n\
                 (macOS never shows ext4 in Finder).\n\n\
                 {}\n\n\
                 > Enter path or device manually\n  Esc — back\n\n\
                 Examples:\n  /dev/rdisk4      (whole disk — we find the GPT slice)\n  /dev/sdb2        (Linux partition)\n  /mnt/usb         (mounted dir)",
                if app.status.starts_with("volume:") {
                    app.status.as_str()
                } else {
                    ""
                }
            );
            f.render_widget(
                Paragraph::new(body)
                    .wrap(Wrap { trim: false })
                    .block(Block::default().borders(Borders::ALL).title("Open volume")),
                chunks[1],
            );
        }
        Screen::VolumePathInput { input, .. } => {
            f.render_widget(
                Paragraph::new(format!("Mount path or raw ext4 device:\n\n> {input}_")).block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title("Volume path"),
                ),
                chunks[1],
            );
        }
        Screen::FileManager {
            path,
            entries,
            state,
        } => {
            let items: Vec<&str> = entries.iter().map(|s| s.as_str()).collect();
            draw_chooser(
                f,
                chunks[1],
                &format!("File manager — /{path}"),
                "↑↓  Enter open  p put from host  c copy out  d delete  Esc back",
                &items,
                state,
            );
        }
        Screen::FilePutInput { input } => {
            f.render_widget(
                Paragraph::new(format!("Host file to copy onto volume root:\n\n> {input}_"))
                    .block(Block::default().borders(Borders::ALL).title("Put file")),
                chunks[1],
            );
        }
        Screen::FileGetInput { remote, input } => {
            f.render_widget(
                Paragraph::new(format!(
                    "Copy volume:/{remote} to host path:\n\n> {input}_"
                ))
                .block(Block::default().borders(Borders::ALL).title("Copy out")),
                chunks[1],
            );
        }
        Screen::LoadImageInput { input } => {
            f.render_widget(
                Paragraph::new(format!(
                    "Load guest image → {IMAGE_NAME}\n\
                     (config path stays image.qcow2)\n\nHost file:\n> {input}_"
                ))
                .block(Block::default().borders(Borders::ALL).title("Load image")),
                chunks[1],
            );
        }
        Screen::Message { title, body } => {
            f.render_widget(
                Paragraph::new(body.as_str())
                    .wrap(Wrap { trim: false })
                    .block(Block::default().borders(Borders::ALL).title(title.as_str())),
                chunks[1],
            );
        }
    }

    let footer = Paragraph::new(
        "Running as root ✓  ·  Editor: ^O save  ^W search  ^X exit  Tab complete",
    )
    .block(Block::default().borders(Borders::ALL));
    f.render_widget(footer, chunks[2]);
}

fn draw_chooser(
    f: &mut ratatui::Frame,
    area: Rect,
    title: &str,
    help: &str,
    items: &[&str],
    state: &ListState,
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(1)])
        .split(area);

    let list_items: Vec<ListItem> = items
        .iter()
        .enumerate()
        .map(|(i, label)| {
            let selected = state.selected() == Some(i);
            let prefix = if selected { "> " } else { "  " };
            let style = if selected {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            ListItem::new(Line::from(Span::styled(
                format!("{prefix}{label}"),
                style,
            )))
        })
        .collect();

    let list = List::new(list_items).block(Block::default().borders(Borders::ALL).title(title));
    f.render_stateful_widget(list, chunks[0], &mut state.clone());
    f.render_widget(
        Paragraph::new(help).style(Style::default().fg(Color::DarkGray)),
        chunks[1],
    );
    let _ = Clear;
}

fn select_next(state: &mut ListState, len: usize) {
    if len == 0 {
        return;
    }
    let i = match state.selected() {
        Some(i) => (i + 1) % len,
        None => 0,
    };
    state.select(Some(i));
}

fn select_prev(state: &mut ListState, len: usize) {
    if len == 0 {
        return;
    }
    let i = match state.selected() {
        Some(0) => len - 1,
        Some(i) => i - 1,
        None => 0,
    };
    state.select(Some(i));
}

fn human_size(n: u64) -> String {
    crate::format_size::format_size(n)
}

fn join_rel(base: &str, name: &str) -> String {
    if base.is_empty() {
        name.to_string()
    } else {
        format!("{base}/{name}")
    }
}

fn parent_rel(path: &str) -> String {
    match path.rsplit_once('/') {
        Some((p, _)) => p.to_string(),
        None => String::new(),
    }
}

fn char_byte_index(s: &str, char_idx: usize) -> usize {
    s.char_indices()
        .nth(char_idx)
        .map(|(i, _)| i)
        .unwrap_or(s.len())
}

fn path_with_cursor(input: &str, cursor: usize) -> String {
    let mut out = String::from("> ");
    for (i, ch) in input.chars().enumerate() {
        if i == cursor {
            out.push('▌');
        }
        out.push(ch);
    }
    if cursor >= input.chars().count() {
        out.push('▌');
    }
    out
}
