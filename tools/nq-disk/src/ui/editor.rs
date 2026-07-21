//! Config editor with VS Code–style completions and Ctrl+Z undo.
//!
//! Completions:
//! - Dropdown appears under the current line when the cursor is in a known key/value/section
//! - ↑ / ↓ move the selection (while open, arrows never leave the popup)
//! - Enter / Tab replace the current token with the selection
//! - Esc closes the popup
//! - Triggers: typing (auto), Tab, F1, Alt+Space, Ctrl+Space (if the OS does not steal it)
//!
//! Undo: Ctrl+Z (and Ctrl+Y / Ctrl+Shift+Z for redo)

use crate::schema::{self, Suggestion};
use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph};
use ratatui::DefaultTerminal;
use std::collections::VecDeque;
use std::time::Duration;

const UNDO_LIMIT: usize = 200;

struct Snapshot {
    lines: Vec<String>,
    cy: usize,
    cx: usize,
}

struct Editor {
    lines: Vec<String>,
    cy: usize,
    cx: usize,
    scroll: usize,
    dirty: bool,
    status: String,
    complete: Option<CompletePopup>,
    search: Option<String>,
    search_mode: bool,
    search_input: String,
    errors: Vec<String>,
    quit: bool,
    save_and_quit: Option<String>,
    /// Ask save / discard / cancel before leaving with unsaved changes.
    quit_prompt: bool,
    undo: VecDeque<Snapshot>,
    redo: VecDeque<Snapshot>,
}

struct CompletePopup {
    items: Vec<Suggestion>,
    selected: usize,
}

pub fn run_config_editor(terminal: &mut DefaultTerminal, initial: &str) -> Result<Option<String>> {
    let mut ed = Editor {
        lines: if initial.is_empty() {
            vec![String::new()]
        } else {
            initial.lines().map(|l| l.to_string()).collect()
        },
        cy: 0,
        cx: 0,
        scroll: 0,
        dirty: false,
        status: "^O save & exit · ^C / ^X exit · ^Z undo · Tab complete".into(),
        complete: None,
        search: None,
        search_mode: false,
        search_input: String::new(),
        errors: Vec::new(),
        quit: false,
        save_and_quit: None,
        quit_prompt: false,
        undo: VecDeque::new(),
        redo: VecDeque::new(),
    };
    if ed.lines.is_empty() {
        ed.lines.push(String::new());
    }
    refresh_complete(&mut ed, false);

    loop {
        terminal.draw(|f| draw_editor(f, &ed))?;
        if ed.quit {
            return Ok(None);
        }
        if let Some(text) = ed.save_and_quit.take() {
            return Ok(Some(text));
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
        handle_editor_key(&mut ed, key.code, key.modifiers);
    }
}

fn handle_editor_key(ed: &mut Editor, code: KeyCode, mods: KeyModifiers) {
    // --- quit confirm: Save / Discard / Cancel ---
    if ed.quit_prompt {
        match code {
            KeyCode::Char('s') | KeyCode::Char('S') => {
                ed.quit_prompt = false;
                try_save(ed);
            }
            KeyCode::Char('d') | KeyCode::Char('D') => {
                ed.quit_prompt = false;
                ed.quit = true; // discard
            }
            KeyCode::Esc | KeyCode::Char('c') | KeyCode::Char('C') => {
                ed.quit_prompt = false;
                ed.status = "^O save & exit · ^C / ^X exit · ^Z undo · Tab complete".into();
            }
            // Ctrl+O save from prompt too
            _ if mods.contains(KeyModifiers::CONTROL)
                && matches!(code, KeyCode::Char('o') | KeyCode::Char('O')) =>
            {
                ed.quit_prompt = false;
                try_save(ed);
            }
            _ => {
                ed.status =
                    "Unsaved changes —  [S] Save & exit   [D] Discard   [Esc] Keep editing".into();
            }
        }
        return;
    }

    // --- search mode ---
    if ed.search_mode {
        match code {
            KeyCode::Esc => {
                ed.search_mode = false;
                ed.search_input.clear();
                ed.status = "Search cancelled".into();
            }
            KeyCode::Enter => {
                ed.search = Some(ed.search_input.clone());
                ed.search_mode = false;
                find_next(ed);
            }
            KeyCode::Backspace => {
                ed.search_input.pop();
            }
            KeyCode::Char(c) if !c.is_control() => ed.search_input.push(c),
            _ => {}
        }
        return;
    }

    // --- completion popup has focus for arrows / enter / esc ---
    if ed.complete.is_some() {
        match code {
            KeyCode::Esc => {
                ed.complete = None;
                ed.status = "Completion closed".into();
                return;
            }
            KeyCode::Up => {
                // Move selection; at top edge, close menu and go to the line above.
                if let Some(p) = ed.complete.as_mut() {
                    if p.selected > 0 {
                        p.selected -= 1;
                        return;
                    }
                }
                ed.complete = None;
                if ed.cy > 0 {
                    ed.cy -= 1;
                    clamp_cx(ed);
                }
                ed.status = "Completion closed".into();
                return;
            }
            KeyCode::Down => {
                // Move selection; at bottom edge, close menu and go to the line below.
                if let Some(p) = ed.complete.as_mut() {
                    if p.selected + 1 < p.items.len() {
                        p.selected += 1;
                        return;
                    }
                }
                ed.complete = None;
                if ed.cy + 1 < ed.lines.len() {
                    ed.cy += 1;
                    clamp_cx(ed);
                }
                ed.status = "Completion closed".into();
                return;
            }
            KeyCode::Enter | KeyCode::Tab => {
                accept_completion(ed);
                return;
            }
            // Typing filters; fall through to edit then refresh list
            KeyCode::Char(_) | KeyCode::Backspace | KeyCode::Delete => {}
            // Left/right move in text and re-filter
            KeyCode::Left | KeyCode::Right | KeyCode::Home | KeyCode::End => {}
            // Other navigation dismisses popup then moves
            KeyCode::PageUp | KeyCode::PageDown => {
                ed.complete = None;
            }
            _ => {}
        }
    }

    // Manual complete triggers
    if is_complete_trigger(code, mods) {
        refresh_complete(ed, true);
        return;
    }

    // Ctrl shortcuts
    if mods.contains(KeyModifiers::CONTROL) && !mods.contains(KeyModifiers::ALT) {
        match code {
            KeyCode::Char('z') | KeyCode::Char('Z') => {
                if mods.contains(KeyModifiers::SHIFT) {
                    redo(ed);
                } else {
                    undo(ed);
                }
                return;
            }
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                redo(ed);
                return;
            }
            KeyCode::Char('o') | KeyCode::Char('O') => {
                // Save and exit (if valid)
                try_save(ed);
                return;
            }
            // Exit: Ctrl+C or Ctrl+X — if dirty, ask S/D/Esc
            KeyCode::Char('c')
            | KeyCode::Char('C')
            | KeyCode::Char('x')
            | KeyCode::Char('X') => {
                request_exit(ed);
                return;
            }
            KeyCode::Char('w') | KeyCode::Char('W') => {
                ed.search_mode = true;
                ed.search_input.clear();
                ed.status = "Search: ".into();
                return;
            }
            KeyCode::Char('n') | KeyCode::Char('N') => {
                find_next(ed);
                return;
            }
            KeyCode::Char('g') | KeyCode::Char('G') => {
                ed.status =
                    "^O save & exit · ^C exit · ^Z undo · Tab complete · ^W search".into();
                return;
            }
            KeyCode::Char(' ') | KeyCode::Null => {
                refresh_complete(ed, true);
                return;
            }
            _ => return,
        }
    }

    if mods.contains(KeyModifiers::ALT)
        && matches!(code, KeyCode::Char(' ') | KeyCode::Char(' '))
    {
        refresh_complete(ed, true);
        return;
    }

    match code {
        KeyCode::F(1) => {
            refresh_complete(ed, true);
        }
        KeyCode::Tab => {
            // Tab with open list already handled; without list → open
            if ed.complete.is_none() {
                refresh_complete(ed, true);
            }
        }
        KeyCode::Up => {
            // Only when popup closed
            if ed.complete.is_none() {
                if ed.cy > 0 {
                    ed.cy -= 1;
                    clamp_cx(ed);
                }
                refresh_complete(ed, false);
            }
        }
        KeyCode::Down => {
            if ed.complete.is_none() {
                if ed.cy + 1 < ed.lines.len() {
                    ed.cy += 1;
                    clamp_cx(ed);
                }
                refresh_complete(ed, false);
            }
        }
        KeyCode::Left => {
            if ed.cx > 0 {
                ed.cx -= 1;
            } else if ed.cy > 0 {
                ed.cy -= 1;
                ed.cx = ed.lines[ed.cy].chars().count();
            }
            refresh_complete(ed, false);
        }
        KeyCode::Right => {
            let len = ed.lines[ed.cy].chars().count();
            if ed.cx < len {
                ed.cx += 1;
            } else if ed.cy + 1 < ed.lines.len() {
                ed.cy += 1;
                ed.cx = 0;
            }
            refresh_complete(ed, false);
        }
        KeyCode::Home => {
            ed.cx = 0;
            refresh_complete(ed, false);
        }
        KeyCode::End => {
            ed.cx = ed.lines[ed.cy].chars().count();
            refresh_complete(ed, false);
        }
        KeyCode::PageUp => {
            ed.cy = ed.cy.saturating_sub(20);
            clamp_cx(ed);
            refresh_complete(ed, false);
        }
        KeyCode::PageDown => {
            ed.cy = (ed.cy + 20).min(ed.lines.len().saturating_sub(1));
            clamp_cx(ed);
            refresh_complete(ed, false);
        }
        KeyCode::Backspace => {
            push_undo(ed);
            if ed.cx > 0 {
                let line = &mut ed.lines[ed.cy];
                let (i, _) = line.char_indices().nth(ed.cx - 1).unwrap();
                line.remove(i);
                ed.cx -= 1;
                ed.dirty = true;
            } else if ed.cy > 0 {
                let cur = ed.lines.remove(ed.cy);
                ed.cy -= 1;
                ed.cx = ed.lines[ed.cy].chars().count();
                ed.lines[ed.cy].push_str(&cur);
                ed.dirty = true;
            }
            refresh_complete(ed, false);
        }
        KeyCode::Delete => {
            push_undo(ed);
            let len = ed.lines[ed.cy].chars().count();
            if ed.cx < len {
                let line = &mut ed.lines[ed.cy];
                let (i, _) = line.char_indices().nth(ed.cx).unwrap();
                line.remove(i);
                ed.dirty = true;
            } else if ed.cy + 1 < ed.lines.len() {
                let next = ed.lines.remove(ed.cy + 1);
                ed.lines[ed.cy].push_str(&next);
                ed.dirty = true;
            }
            refresh_complete(ed, false);
        }
        KeyCode::Enter => {
            // Accept completion if open (also handled above; safety)
            if ed.complete.is_some() {
                accept_completion(ed);
                return;
            }
            push_undo(ed);
            let line = &ed.lines[ed.cy];
            let (split_idx, _) = line
                .char_indices()
                .nth(ed.cx)
                .unwrap_or((line.len(), '\0'));
            let right = line[split_idx..].to_string();
            ed.lines[ed.cy].truncate(split_idx);
            ed.cy += 1;
            ed.lines.insert(ed.cy, right);
            ed.cx = 0;
            ed.dirty = true;
            refresh_complete(ed, false);
        }
        KeyCode::Char(c) if !c.is_control() => {
            push_undo(ed);
            let line = &mut ed.lines[ed.cy];
            let idx = line
                .char_indices()
                .nth(ed.cx)
                .map(|(i, _)| i)
                .unwrap_or(line.len());
            line.insert(idx, c);
            ed.cx += 1;
            ed.dirty = true;
            refresh_complete(ed, false);
        }
        KeyCode::Esc => {
            if ed.complete.is_some() {
                ed.complete = None;
                ed.status = "Completion closed".into();
            } else if !ed.errors.is_empty() {
                ed.errors.clear();
            } else {
                // Esc = leave editor (same as Ctrl+C when clean / ask when dirty)
                request_exit(ed);
            }
        }
        _ => {}
    }
    ensure_scroll(ed, 20);
}

/// Leave the editor. Clean exit if no edits; otherwise show Save/Discard prompt.
fn request_exit(ed: &mut Editor) {
    ed.complete = None;
    if ed.dirty {
        ed.quit_prompt = true;
        ed.status =
            "Unsaved changes —  [S] Save & exit   [D] Discard   [Esc] Keep editing".into();
    } else {
        ed.quit = true;
    }
}

fn is_complete_trigger(code: KeyCode, mods: KeyModifiers) -> bool {
    if matches!(code, KeyCode::F(1)) {
        return true;
    }
    if mods.contains(KeyModifiers::CONTROL)
        && matches!(code, KeyCode::Char(' ') | KeyCode::Char(' ') | KeyCode::Null)
    {
        return true;
    }
    if mods.contains(KeyModifiers::ALT)
        && matches!(code, KeyCode::Char(' ') | KeyCode::Char(' '))
    {
        return true;
    }
    false
}

fn snapshot(ed: &Editor) -> Snapshot {
    Snapshot {
        lines: ed.lines.clone(),
        cy: ed.cy,
        cx: ed.cx,
    }
}

fn push_undo(ed: &mut Editor) {
    ed.undo.push_back(snapshot(ed));
    while ed.undo.len() > UNDO_LIMIT {
        ed.undo.pop_front();
    }
    ed.redo.clear();
}

fn undo(ed: &mut Editor) {
    let Some(prev) = ed.undo.pop_back() else {
        ed.status = "Nothing to undo".into();
        return;
    };
    ed.redo.push_back(snapshot(ed));
    while ed.redo.len() > UNDO_LIMIT {
        ed.redo.pop_front();
    }
    ed.lines = prev.lines;
    ed.cy = prev.cy.min(ed.lines.len().saturating_sub(1));
    ed.cx = prev.cx;
    clamp_cx(ed);
    ed.dirty = true;
    ed.complete = None;
    ed.status = "Undo".into();
    refresh_complete(ed, false);
}

fn redo(ed: &mut Editor) {
    let Some(next) = ed.redo.pop_back() else {
        ed.status = "Nothing to redo".into();
        return;
    };
    ed.undo.push_back(snapshot(ed));
    ed.lines = next.lines;
    ed.cy = next.cy.min(ed.lines.len().saturating_sub(1));
    ed.cx = next.cx;
    clamp_cx(ed);
    ed.dirty = true;
    ed.complete = None;
    ed.status = "Redo".into();
    refresh_complete(ed, false);
}

fn accept_completion(ed: &mut Editor) {
    let Some(popup) = ed.complete.take() else {
        return;
    };
    let Some(item) = popup.items.get(popup.selected).cloned() else {
        return;
    };
    push_undo(ed);
    apply_completion(ed, &item);
    // Close the menu so ↑↓ move between lines again (re-open with Tab / Ctrl+Space).
    ed.complete = None;
    ed.status = format!("Replaced → {}  ·  Tab for completions", item.label);
}

fn refresh_complete(ed: &mut Editor, forced: bool) {
    let text = ed.lines.join("\n");
    let off = cursor_byte_offset(ed);
    let ctx = schema::completion_context(&text, off);
    let items = schema::suggestions(&ctx);
    if items.is_empty() {
        if forced {
            ed.status =
                "No value completions here — put the cursor after = (e.g. arch = ▌)".into();
        }
        ed.complete = None;
        return;
    }
    let selected = ed
        .complete
        .as_ref()
        .map(|p| p.selected.min(items.len() - 1))
        .unwrap_or(0);
    ed.complete = Some(CompletePopup { items, selected });
    if forced {
        ed.status = "↑↓ select · Enter replace · Esc close".into();
    }
}

fn cursor_byte_offset(ed: &Editor) -> usize {
    let mut off = 0usize;
    for (i, line) in ed.lines.iter().enumerate() {
        if i == ed.cy {
            let (byte, _) = line
                .char_indices()
                .nth(ed.cx)
                .unwrap_or((line.len(), '\0'));
            off += byte;
            break;
        }
        off += line.len() + 1;
    }
    off
}

fn try_save(ed: &mut Editor) {
    let text = ed.lines.join("\n");
    match schema::validate_config_text(&text) {
        Ok(()) => {
            ed.dirty = false;
            ed.errors.clear();
            ed.status = "Saved (validated).".into();
            ed.save_and_quit = Some(if text.ends_with('\n') {
                text
            } else {
                format!("{text}\n")
            });
        }
        Err(errs) => {
            ed.errors = errs;
            ed.status = "Save blocked — fix errors listed below.".into();
        }
    }
}

fn apply_completion(ed: &mut Editor, item: &Suggestion) {
    let line = ed.lines[ed.cy].clone();
    // Value-only: replace RHS token; keep key spacing / quotes / comments.
    if !line.contains('=') {
        ed.status = "Completions only apply to values after =".into();
        return;
    }
    let (new_line, cursor_at_value) = replace_value_only(&line, &item.insert);
    ed.lines[ed.cy] = new_line;
    // Land on the start of the new value — never jump to end-of-line.
    ed.cx = cursor_at_value;
    ed.dirty = true;
}

/// Replace only the value after `=`.
/// Returns `(new_line, cursor_col)` where cursor is on the **first character of the
/// new value** (opening `"` or first digit/bool char). Stays put across different
/// value lengths and never leaps to EOL / past trailing comments.
fn replace_value_only(line: &str, insert: &str) -> (String, usize) {
    let Some(eq) = line.find('=') else {
        return (line.to_string(), line.chars().count());
    };
    let head = &line[..=eq]; // through '='
    let tail = &line[eq + 1..];

    let value_start = tail
        .char_indices()
        .find(|(_, c)| !c.is_whitespace())
        .map(|(i, _)| i)
        .unwrap_or(tail.len());
    let leading_ws = &tail[..value_start];
    let rest = &tail[value_start..];

    let new_inner = insert.trim().trim_matches('"');

    let (new_value, after_value) = if rest.starts_with('"') {
        let after_open = &rest[1..];
        if let Some(end) = after_open.find('"') {
            let after = &after_open[end + 1..];
            (format!("\"{new_inner}\""), after.to_string())
        } else {
            (format!("\"{new_inner}\""), String::new())
        }
    } else {
        let end = rest
            .char_indices()
            .find(|(_, c)| c.is_whitespace() || *c == '#')
            .map(|(i, _)| i)
            .unwrap_or(rest.len());
        let after = rest[end..].to_string();
        let bare = insert.trim();
        let new_value = if bare == "true" || bare == "false" || bare.parse::<i64>().is_ok() {
            bare.trim_matches('"').to_string()
        } else if bare.starts_with('"') {
            bare.to_string()
        } else {
            format!("\"{new_inner}\"")
        };
        (new_value, after)
    };

    let new_line = format!("{head}{leading_ws}{new_value}{after_value}");
    // Cursor on first char of the value (same column for any replacement length).
    let cursor = head.chars().count() + leading_ws.chars().count();
    (new_line, cursor)
}

fn find_next(ed: &mut Editor) {
    let Some(pat) = ed.search.clone() else {
        ed.status = "No search pattern (Ctrl+W)".into();
        return;
    };
    if pat.is_empty() {
        return;
    }
    let total = ed.lines.len();
    for delta in 0..total {
        let yi = (ed.cy + delta) % total;
        let start_x = if delta == 0 { ed.cx + 1 } else { 0 };
        let slice_start = char_byte(&ed.lines[yi], start_x);
        if let Some(pos) = ed.lines[yi][slice_start..].find(&pat) {
            let abs = slice_start + pos;
            ed.cy = yi;
            ed.cx = ed.lines[yi][..abs].chars().count();
            ed.status = format!("Found '{pat}'");
            return;
        }
        if delta > 0 {
            if let Some(pos) = ed.lines[yi].find(&pat) {
                ed.cy = yi;
                ed.cx = ed.lines[yi][..pos].chars().count();
                ed.status = format!("Found '{pat}'");
                return;
            }
        }
    }
    ed.status = format!("'{pat}' not found");
}

fn char_byte(s: &str, char_idx: usize) -> usize {
    s.char_indices()
        .nth(char_idx)
        .map(|(i, _)| i)
        .unwrap_or(s.len())
}

#[cfg(test)]
mod value_replace_tests {
    use super::replace_value_only;

    #[test]
    fn keeps_spacing_quotes_and_cursor_at_value_start() {
        let (line, cx) = replace_value_only(r#"machine = "pc""#, r#""q35""#);
        assert_eq!(line, r#"machine = "q35""#);
        // Cursor on opening quote of the value — not EOL.
        assert_eq!(&line.chars().take(cx).collect::<String>(), "machine = ");
        assert_eq!(line.chars().nth(cx), Some('"'));
        assert!(cx < line.chars().count());

        let (line, cx) = replace_value_only(r#"machine  =  "pc""#, r#""q35""#);
        assert_eq!(line, r#"machine  =  "q35""#);
        assert_eq!(&line.chars().take(cx).collect::<String>(), "machine  =  ");
        assert_eq!(line.chars().nth(cx), Some('"'));

        let (line, cx) = replace_value_only(r#"arch     = "x86_64"  # note"#, r#""aarch64""#);
        assert_eq!(line, r#"arch     = "aarch64"  # note"#);
        assert_eq!(&line.chars().take(cx).collect::<String>(), "arch     = ");
        assert_eq!(line.chars().nth(cx), Some('"'));
        assert!(!line.chars().skip(cx).take(8).collect::<String>().contains('#'));

        let (line, cx) = replace_value_only(r#"vcpus    = 1"#, "4");
        assert_eq!(line, r#"vcpus    = 4"#);
        assert_eq!(&line.chars().take(cx).collect::<String>(), "vcpus    = ");
        assert_eq!(line.chars().nth(cx), Some('4'));
        assert!(cx < line.chars().count());

        // Longer → shorter value: cursor column must not jump to EOL.
        let (line, cx) = replace_value_only(
            r#"on_guest_shutdown    = "poweroff_host""#,
            r#""drop_to_shell""#,
        );
        assert_eq!(line, r#"on_guest_shutdown    = "drop_to_shell""#);
        assert_eq!(
            &line.chars().take(cx).collect::<String>(),
            "on_guest_shutdown    = "
        );
        assert!(cx < line.chars().count());
    }
}

fn clamp_cx(ed: &mut Editor) {
    let len = ed.lines[ed.cy].chars().count();
    if ed.cx > len {
        ed.cx = len;
    }
}

fn ensure_scroll(ed: &mut Editor, height: usize) {
    if ed.cy < ed.scroll {
        ed.scroll = ed.cy;
    } else if ed.cy >= ed.scroll + height {
        ed.scroll = ed.cy + 1 - height;
    }
}

fn draw_editor(f: &mut ratatui::Frame, ed: &Editor) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(5),
            Constraint::Length(if ed.errors.is_empty() { 1 } else { 5 }),
            Constraint::Length(1),
        ])
        .split(f.area());

    let title = format!(
        " config.toml {}  Ln {} Col {}  ·  ^O save & exit  ·  ^C exit ",
        if ed.dirty { "[modified]" } else { "" },
        ed.cy + 1,
        ed.cx + 1
    );
    let title_style = if ed.quit_prompt {
        Style::default().bg(Color::Yellow).fg(Color::Black)
    } else if ed.dirty {
        Style::default().bg(Color::DarkGray).fg(Color::White)
    } else {
        Style::default().bg(Color::Blue).fg(Color::White)
    };
    f.render_widget(Paragraph::new(title).style(title_style), chunks[0]);

    let edit_area = chunks[1];
    let height = edit_area.height as usize;
    let mut lines: Vec<Line> = Vec::new();
    for (i, line) in ed.lines.iter().enumerate().skip(ed.scroll).take(height) {
        let row = i;
        let mut spans = highlight_toml_line(line);
        if row == ed.cy {
            let mut rebuilt = Vec::new();
            let mut col = 0usize;
            for sp in spans {
                let content = sp.content.to_string();
                for ch in content.chars() {
                    let style = if col == ed.cx {
                        Style::default().bg(Color::Yellow).fg(Color::Black)
                    } else {
                        sp.style
                    };
                    rebuilt.push(Span::styled(ch.to_string(), style));
                    col += 1;
                }
            }
            if ed.cx >= col {
                rebuilt.push(Span::styled(
                    " ",
                    Style::default().bg(Color::Yellow).fg(Color::Black),
                ));
            }
            spans = rebuilt;
        }
        let gutter = Span::styled(
            format!("{:4}│", row + 1),
            Style::default().fg(Color::DarkGray),
        );
        let mut all = vec![gutter];
        all.extend(spans);
        lines.push(Line::from(all));
    }
    f.render_widget(Paragraph::new(lines), edit_area);

    // One cursor only: the yellow cell above. Do not also place the terminal
    // caret — that drew a second block in a different color next to (or on) it.

    if !ed.errors.is_empty() {
        let err = ed.errors.join("\n");
        f.render_widget(
            Paragraph::new(err).style(Style::default().fg(Color::Red)).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Validation errors"),
            ),
            chunks[2],
        );
    } else {
        f.render_widget(Paragraph::new(""), chunks[2]);
    }

    let status = if ed.quit_prompt {
        "Unsaved changes —  [S] Save & exit   [D] Discard changes   [Esc] Keep editing".into()
    } else if ed.search_mode {
        format!("Search: {}_", ed.search_input)
    } else {
        ed.status.clone()
    };
    let status_style = if ed.quit_prompt {
        Style::default()
            .bg(Color::Yellow)
            .fg(Color::Black)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().bg(Color::DarkGray)
    };
    f.render_widget(Paragraph::new(status).style(status_style), chunks[3]);

    // VS Code–style popup: directly under the current line
    if let Some(popup) = &ed.complete {
        let n = popup.items.len().min(12);
        let pop_h = (n as u16).saturating_add(2).max(3);
        let area = popup_below_line(edit_area, ed, pop_h);
        f.render_widget(Clear, area);
        let items: Vec<ListItem> = popup
            .items
            .iter()
            .take(12)
            .enumerate()
            .map(|(i, s)| {
                let selected = i == popup.selected;
                let prefix = if selected { "› " } else { "  " };
                let style = if selected {
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::Gray)
                };
                ListItem::new(Line::from(Span::styled(
                    format!("{prefix}{:<18} {}", s.label, s.doc),
                    style,
                )))
            })
            .collect();
        f.render_widget(
            List::new(items).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" ↑↓  Enter = replace ")
                    .border_style(Style::default().fg(Color::Cyan))
                    .style(Style::default().bg(Color::Rgb(24, 24, 32))),
            ),
            area,
        );
    }
}

/// Place the completion list just under the active editor line (like VS Code).
fn popup_below_line(edit_area: Rect, ed: &Editor, height: u16) -> Rect {
    let gutter = 6u16; // "  12│ "
    let rel_row = ed.cy.saturating_sub(ed.scroll) as u16;
    let mut y = edit_area.y + rel_row + 1;
    // If near bottom, open above the line instead
    if y + height > edit_area.y + edit_area.height {
        y = edit_area.y + rel_row.saturating_sub(height);
        if y < edit_area.y {
            y = edit_area.y;
        }
    }
    let max_w = edit_area.width.saturating_sub(gutter + 2).min(56);
    let x = edit_area.x + gutter;
    let h = height.min(edit_area.height.saturating_sub(y.saturating_sub(edit_area.y)));
    Rect {
        x,
        y,
        width: max_w.max(24),
        height: h.max(3),
    }
}

fn highlight_toml_line(line: &str) -> Vec<Span<'_>> {
    let t = line.trim_start();
    if t.starts_with('#') {
        return vec![Span::styled(
            line.to_string(),
            Style::default().fg(Color::DarkGray),
        )];
    }
    if t.starts_with('[') {
        return vec![Span::styled(
            line.to_string(),
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        )];
    }
    if let Some(eq) = line.find('=') {
        let (k, v) = line.split_at(eq);
        return vec![
            Span::styled(k.to_string(), Style::default().fg(Color::Cyan)),
            Span::styled("=", Style::default().fg(Color::White)),
            Span::styled(v[1..].to_string(), Style::default().fg(Color::Green)),
        ];
    }
    vec![Span::raw(line.to_string())]
}
