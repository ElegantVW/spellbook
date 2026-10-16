use crate::board;
use crate::listing::{Kind, Listing, Sort};
use crate::paths;
use crossterm::{
    event::{
        self, Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEventKind,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    layout::{Constraint, Direction, Layout, Position, Rect},
    style::{Color, Modifier, Style},
    symbols::border::{PLAIN, ROUNDED},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame, Terminal,
};
use std::io::{self, stdout, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const PINK: Color = Color::Rgb(0xe8, 0x79, 0xa0);
const BLUSH: Color = Color::Rgb(0xf0, 0xb4, 0xc8);
const DIM: Color = Color::Rgb(0x9d, 0x5c, 0x75);
const FG: Color = Color::Rgb(0xff, 0xeb, 0xf2);
const SILVER: Color = Color::Rgb(0xc0, 0xc0, 0xc8);
const GOLD: Color = Color::Rgb(0xf0, 0xd8, 0xa0);
const LILAC: Color = Color::Rgb(0xc4, 0xb0, 0xd8);
const ROSE: Color = Color::Rgb(0xe8, 0xa0, 0xb4);
const TEAL: Color = Color::Rgb(0x8e, 0xc4, 0xc8);
const BLUSH_BG: Color = Color::Rgb(0x3a, 0x18, 0x24);
const FOCUS_MS: u128 = 120;
const SHELF_MS: u128 = 150;

fn unicode() -> bool {
    match std::env::var("PIXIE_UNICODE") {
        Ok(v) => !matches!(v.as_str(), "0" | "false" | "ascii" | "no" | "off"),
        Err(_) => true,
    }
}

fn border_set() -> ratatui::symbols::border::Set {
    if unicode() {
        ROUNDED
    } else {
        PLAIN
    }
}

fn mark() -> &'static str {
    if unicode() { "✦" } else { "*" }
}

fn narrate(cast: &str, answered: &str, ok: bool) -> String {
    let verb = if ok { "answered" } else { "thundered" };
    format!("The Wizard cast {cast} and the Aether {verb}: {answered}")
}

enum Mode {
    Browse,
    PathEdit,
    Help,
    New,
    Rename,
    ConfirmDelete,
}

struct App {
    listing: Listing,
    selected: usize,
    status: String,
    pick: bool,
    output: Option<PathBuf>,
    chosen: Option<PathBuf>,
    mode: Mode,
    path_buf: String,
    path_caret: usize,
    prompt: String,
    prompt_caret: usize,
    dirty: bool,
    last_click: Option<(Instant, usize)>,
    crumb_hits: Vec<(Rect, PathBuf)>,
    row_hits: Vec<(Rect, usize)>,
    path_hit: Option<Rect>,
    pending_editor: Option<PathBuf>,
    hover: Option<usize>,
    offset: usize,
    focus_from: usize,
    focus_t0: Instant,
    shelf_t0: Option<Instant>,
    anim_until: Instant,
}

impl App {
    fn new(start: PathBuf, pick: bool, output: Option<PathBuf>) -> Self {
        let dir = if start.is_dir() {
            start.canonicalize().unwrap_or(start)
        } else if start.exists() {
            start
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or(start)
                .canonicalize()
                .unwrap_or_else(|_| PathBuf::from("."))
        } else {
            std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
        };
        Self {
            listing: Listing::open(dir, Sort::Name, false),
            selected: 0,
            status: String::new(),
            pick,
            output,
            chosen: None,
            mode: Mode::Browse,
            path_buf: String::new(),
            path_caret: 0,
            prompt: String::new(),
            prompt_caret: 0,
            dirty: true,
            last_click: None,
            crumb_hits: Vec::new(),
            row_hits: Vec::new(),
            path_hit: None,
            pending_editor: None,
            hover: None,
            offset: 0,
            focus_from: 0,
            focus_t0: Instant::now(),
            shelf_t0: None,
            anim_until: Instant::now(),
        }
    }

    fn set_selected(&mut self, i: usize) {
        let n = self.listing.entries.len();
        if n == 0 {
            self.selected = 0;
            return;
        }
        let i = i.min(n - 1);
        if i != self.selected {
            self.focus_from = self.selected;
            self.focus_t0 = Instant::now();
            self.anim_until = Instant::now() + Duration::from_millis(FOCUS_MS as u64);
            self.selected = i;
        }
    }

    fn animating(&self) -> bool {
        Instant::now() < self.anim_until
    }

    fn current(&self) -> Option<&crate::listing::Entry> {
        self.listing.entries.get(self.selected)
    }

    fn clamp_sel(&mut self) {
        if self.listing.entries.is_empty() {
            self.selected = 0;
        } else if self.selected >= self.listing.entries.len() {
            self.selected = self.listing.entries.len() - 1;
        }
    }

    fn reload(&mut self) {
        self.listing.reload();
        self.clamp_sel();
        self.dirty = true;
    }

    fn goto_dir(&mut self, dir: PathBuf, select: Option<&Path>) {
        self.listing.dir = dir;
        self.listing.reload();
        self.selected = select
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .and_then(|n| self.listing.select_name(n))
            .unwrap_or(0);
        self.offset = 0;
        self.hover = None;
        self.shelf_t0 = Some(Instant::now());
        self.anim_until = Instant::now() + Duration::from_millis(SHELF_MS as u64);
        self.dirty = true;
    }
}

pub fn run(start: PathBuf, pick: bool, output: Option<PathBuf>) -> anyhow::Result<()> {
    if std::env::var_os("PIXIE_UNICODE").is_none() {
        std::env::set_var("PIXIE_UNICODE", "1");
    }
    let mut app = App::new(start, pick, output);
    enable_raw_mode()?;
    let mut stdout = stdout();
    execute!(
        stdout,
        EnterAlternateScreen,
        event::EnableMouseCapture,
        event::EnableBracketedPaste
    )?;
    let _ = write!(stdout, "\x1b[?1003h\x1b[?1006h");
    let _ = stdout.flush();
    let mut term = Terminal::new(ratatui::backend::CrosstermBackend::new(stdout))?;
    let _ = term.hide_cursor();
    let mut quit = false;
    loop {
        if app.dirty || app.animating() {
            term.draw(|f| draw(f, &mut app))?;
            app.dirty = false;
        }
        if let Some(path) = app.pending_editor.take() {
            suspend_editor(&mut term, &path)?;
            app.status = narrate(&format!("edit {}", path.display()), "the quill was dipped", true);
            app.reload();
        }
        let wait = if app.animating() {
            Duration::from_millis(50)
        } else {
            Duration::from_millis(250)
        };
        if event::poll(wait)? {
            match event::read()? {
                Event::Key(k) => {
                    if handle_key(&mut app, k) {
                        quit = true;
                    }
                    app.dirty = true;
                }
                Event::Mouse(m) => {
                    handle_mouse(&mut app, m);
                }
                Event::Paste(s) => {
                    if matches!(app.mode, Mode::PathEdit) {
                        let t = paths::clean_pasted_path(&s);
                        insert_at(&mut app.path_buf, &mut app.path_caret, &t);
                    } else if matches!(app.mode, Mode::New | Mode::Rename) {
                        insert_at(&mut app.prompt, &mut app.prompt_caret, &s);
                    }
                    app.dirty = true;
                }
                Event::Resize(_, _) => app.dirty = true,
                _ => {}
            }
        }
        if quit || app.chosen.is_some() {
            break;
        }
    }
    disable_raw_mode()?;
    let _ = write!(io::stdout(), "\x1b[?1003l\x1b[?1006l");
    execute!(
        io::stdout(),
        LeaveAlternateScreen,
        event::DisableMouseCapture,
        event::DisableBracketedPaste
    )?;
    if let Some(chosen) = app.chosen {
        paths::write_pick(&chosen, app.output.as_deref())?;
    }
    Ok(())
}

fn insert_at(buf: &mut String, caret: &mut usize, text: &str) {
    let i = byte_at(buf, *caret);
    buf.insert_str(i, text);
    *caret += text.chars().count();
}

fn byte_at(s: &str, char_i: usize) -> usize {
    s.char_indices()
        .nth(char_i)
        .map(|(i, _)| i)
        .unwrap_or(s.len())
}

fn handle_key(app: &mut App, k: KeyEvent) -> bool {
    if k.modifiers.contains(KeyModifiers::CONTROL) && matches!(k.code, KeyCode::Char('c')) {
        return true;
    }
    match app.mode {
        Mode::Help => {
            app.mode = Mode::Browse;
            return false;
        }
        Mode::ConfirmDelete => {
            match k.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    do_delete(app);
                    app.mode = Mode::Browse;
                }
                _ => app.mode = Mode::Browse,
            }
            return false;
        }
        Mode::PathEdit => {
            return handle_path_key(app, k);
        }
        Mode::New | Mode::Rename => {
            handle_prompt_key(app, k);
            return false;
        }
        Mode::Browse => {}
    }
    match k.code {
        KeyCode::Char('q') | KeyCode::Esc => return true,
        KeyCode::Char('j') | KeyCode::Down => {
            if !app.listing.entries.is_empty() && app.selected + 1 < app.listing.entries.len() {
                app.set_selected(app.selected + 1);
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            app.set_selected(app.selected.saturating_sub(1));
        }
        KeyCode::Char('l') if k.modifiers.contains(KeyModifiers::CONTROL) => begin_path(app),
        KeyCode::Char('l') | KeyCode::Right | KeyCode::Enter | KeyCode::Char('o') => {
            open_selected(app);
        }
        KeyCode::Char('h') | KeyCode::Left => go_parent(app),
        KeyCode::Char('g') => app.set_selected(0),
        KeyCode::Char('G') => {
            if !app.listing.entries.is_empty() {
                app.set_selected(app.listing.entries.len() - 1);
            }
        }
        KeyCode::Char('.') => {
            app.listing.hidden = !app.listing.hidden;
            app.reload();
            app.status = if app.listing.hidden {
                narrate("reveal", "the veiled are unveiled", true)
            } else {
                narrate("veil", "the veiled are hidden once more", true)
            };
        }
        KeyCode::Char('s') => {
            app.listing.sort = app.listing.sort.next();
            app.reload();
            app.status = narrate(
                &format!("sort {}", app.listing.sort.as_str()),
                "the scrolls reorder themselves",
                true,
            );
        }
        KeyCode::Char('?') => app.mode = Mode::Help,
        KeyCode::Char(':') => begin_path(app),
        KeyCode::Char('y') => yank(app, false),
        KeyCode::Char('Y') => yank(app, true),
        KeyCode::Char('P') => paste_goto(app),
        KeyCode::Char('p') if app.pick => {
            app.chosen = Some(app.listing.dir.clone());
            app.status = narrate(
                &format!("pick {}", app.listing.dir.display()),
                "this tome is the chosen one",
                true,
            );
        }
        KeyCode::Char('n') => {
            app.mode = Mode::New;
            app.prompt.clear();
            app.prompt_caret = 0;
        }
        KeyCode::Char('d') => {
            if app.current().is_some() {
                app.mode = Mode::ConfirmDelete;
            }
        }
        KeyCode::Char('r') => {
            if let Some(name) = app.current().map(|e| e.name.clone()) {
                app.mode = Mode::Rename;
                app.prompt = name;
                app.prompt_caret = app.prompt.chars().count();
            }
        }
        KeyCode::Char('e') => {
            if let Some(e) = app.current() {
                if !e.is_dir {
                    app.pending_editor = Some(e.path.clone());
                }
            }
        }
        KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => return true,
        _ => {
            if k.modifiers.contains(KeyModifiers::CONTROL) {
                if matches!(k.code, KeyCode::Char('l')) {
                    begin_path(app);
                }
            }
        }
    }
    false
}

fn handle_path_key(app: &mut App, k: KeyEvent) -> bool {
    let n = app.path_buf.chars().count();
    match k.code {
        KeyCode::Esc => {
            app.mode = Mode::Browse;
        }
        KeyCode::Enter => {
            let typed = app.path_buf.clone();
            app.mode = Mode::Browse;
            apply_goto(app, &typed);
        }
        KeyCode::Char('q') if k.modifiers.contains(KeyModifiers::CONTROL) => return true,
        KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => return true,
        KeyCode::Char('u') if k.modifiers.contains(KeyModifiers::CONTROL) => {
            let rest: String = app.path_buf.chars().skip(app.path_caret).collect();
            app.path_buf = rest;
            app.path_caret = 0;
        }
        KeyCode::Char('w') if k.modifiers.contains(KeyModifiers::CONTROL) => {
            let byte = byte_at(&app.path_buf, app.path_caret);
            let (s, i) = paths::delete_component(&app.path_buf, byte);
            app.path_buf = s;
            app.path_caret = app.path_buf[..i.min(app.path_buf.len())].chars().count();
        }
        KeyCode::Char('y') | KeyCode::Char('v') if k.modifiers.contains(KeyModifiers::CONTROL) => {
            let t = board::get_path();
            if !t.is_empty() {
                insert_at(&mut app.path_buf, &mut app.path_caret, &t);
            }
        }
        KeyCode::Left => app.path_caret = app.path_caret.saturating_sub(1),
        KeyCode::Right => {
            if app.path_caret < n {
                app.path_caret += 1;
            }
        }
        KeyCode::Home => app.path_caret = 0,
        KeyCode::End => app.path_caret = n,
        KeyCode::Backspace => {
            if app.path_caret > 0 {
                let end = byte_at(&app.path_buf, app.path_caret);
                let start = byte_at(&app.path_buf, app.path_caret - 1);
                app.path_buf.replace_range(start..end, "");
                app.path_caret -= 1;
            }
        }
        KeyCode::Delete => {
            if app.path_caret < n {
                let start = byte_at(&app.path_buf, app.path_caret);
                let end = byte_at(&app.path_buf, app.path_caret + 1);
                app.path_buf.replace_range(start..end, "");
            }
        }
        KeyCode::Char(c) if !k.modifiers.contains(KeyModifiers::CONTROL) => {
            insert_at(&mut app.path_buf, &mut app.path_caret, &c.to_string());
        }
        _ => {}
    }
    false
}

fn handle_prompt_key(app: &mut App, k: KeyEvent) {
    let n = app.prompt.chars().count();
    match k.code {
        KeyCode::Esc => app.mode = Mode::Browse,
        KeyCode::Enter => match app.mode {
            Mode::New => do_new(app),
            Mode::Rename => do_rename(app),
            _ => {}
        },
        KeyCode::Left => app.prompt_caret = app.prompt_caret.saturating_sub(1),
        KeyCode::Right => {
            if app.prompt_caret < n {
                app.prompt_caret += 1;
            }
        }
        KeyCode::Home => app.prompt_caret = 0,
        KeyCode::End => app.prompt_caret = n,
        KeyCode::Backspace => {
            if app.prompt_caret > 0 {
                let end = byte_at(&app.prompt, app.prompt_caret);
                let start = byte_at(&app.prompt, app.prompt_caret - 1);
                app.prompt.replace_range(start..end, "");
                app.prompt_caret -= 1;
            }
        }
        KeyCode::Char(c) if !k.modifiers.contains(KeyModifiers::CONTROL) => {
            insert_at(&mut app.prompt, &mut app.prompt_caret, &c.to_string());
        }
        _ => {}
    }
}

fn begin_path(app: &mut App) {
    app.mode = Mode::PathEdit;
    app.path_buf = app.listing.dir.display().to_string();
    app.path_caret = app.path_buf.chars().count();
}

fn apply_goto(app: &mut App, typed: &str) {
    match paths::resolve_goto(&app.listing.dir, typed) {
        Ok((dir, select)) => {
            let name = dir
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("/")
                .to_string();
            app.goto_dir(dir, select.as_deref());
            app.status = narrate(&format!("cd {name}/"), "a new path unfolds", true);
        }
        Err(_) => {
            app.status = narrate(typed, "that shelf is not on the shelf", false);
        }
    }
}

fn go_parent(app: &mut App) {
    if let Some(parent) = app.listing.dir.parent() {
        let here = app.listing.dir.clone();
        app.goto_dir(parent.to_path_buf(), Some(&here));
        app.status = narrate("cd ..", "the path folds back", true);
    }
}

fn open_selected(app: &mut App) {
    let Some(e) = app.current().cloned() else { return };
    if e.is_dir {
        app.goto_dir(e.path, None);
        app.status = narrate(&format!("cd {}/", e.name), "a new path unfolds", true);
        return;
    }
    if app.pick {
        app.chosen = Some(e.path);
        return;
    }
    let _ = std::process::Command::new("xdg-open")
        .arg(&e.path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    app.status = narrate(&format!("open {}", e.name), "the scroll was unsealed", true);
}

fn yank(app: &mut App, shelf: bool) {
    let path = if shelf {
        app.listing.dir.clone()
    } else {
        app.current()
            .map(|e| e.path.clone())
            .unwrap_or_else(|| app.listing.dir.clone())
    };
    let s = path.display().to_string();
    if board::set_path(&s) {
        app.status = narrate(&format!("yank {s}"), "the board remembers", true);
    } else {
        app.status = narrate("yank", "imbue did not take the path", false);
    }
}

fn paste_goto(app: &mut App) {
    let t = board::get_path();
    if t.is_empty() {
        app.status = narrate("paste", "the board is empty", false);
        return;
    }
    apply_goto(app, &t);
}

fn do_new(app: &mut App) {
    let raw = app.prompt.trim().to_string();
    app.mode = Mode::Browse;
    if raw.is_empty() {
        return;
    }
    let is_dir = raw.ends_with('/');
    let name = raw.trim_end_matches('/');
    if name.is_empty() {
        return;
    }
    let p = app.listing.dir.join(name);
    let r = if is_dir {
        std::fs::create_dir(&p)
    } else {
        std::fs::write(&p, b"")
    };
    match r {
        Ok(()) => {
            app.reload();
            if let Some(i) = app.listing.select_name(name) {
                app.selected = i;
            }
            app.status = narrate(&format!("new {name}"), "a blank scroll was bound", true);
        }
        Err(e) => {
            app.status = format!("spellbook: {e}");
        }
    }
}

fn do_rename(app: &mut App) {
    let new_name = app.prompt.trim().to_string();
    app.mode = Mode::Browse;
    if new_name.is_empty() {
        return;
    }
    let Some(e) = app.current().cloned() else { return };
    let dest = e.path.parent().unwrap_or(&app.listing.dir).join(&new_name);
    match std::fs::rename(&e.path, &dest) {
        Ok(()) => {
            app.reload();
            if let Some(i) = app.listing.select_name(&new_name) {
                app.selected = i;
            }
            app.status = narrate(
                &format!("rename {} {new_name}", e.name),
                &format!("{} is now {new_name}", e.name),
                true,
            );
        }
        Err(err) => app.status = format!("spellbook: {err}"),
    }
}

fn do_delete(app: &mut App) {
    let Some(e) = app.current().cloned() else { return };
    let r = if e.is_dir {
        std::fs::remove_dir_all(&e.path)
    } else {
        std::fs::remove_file(&e.path)
    };
    match r {
        Ok(()) => {
            app.reload();
            app.status = narrate(&format!("delete {}", e.name), &format!("{} was unmade", e.name), true);
        }
        Err(err) => app.status = format!("spellbook: {err}"),
    }
}

fn handle_mouse(app: &mut App, m: crossterm::event::MouseEvent) {
    if m.modifiers.contains(KeyModifiers::SHIFT) {
        return;
    }
    if matches!(app.mode, Mode::Help) {
        if matches!(m.kind, MouseEventKind::Down(MouseButton::Left)) {
            app.mode = Mode::Browse;
            app.dirty = true;
        }
        return;
    }
    if matches!(app.mode, Mode::ConfirmDelete | Mode::New | Mode::Rename) {
        return;
    }
    match m.kind {
        MouseEventKind::ScrollUp => {
            app.set_selected(app.selected.saturating_sub(1));
            app.dirty = true;
        }
        MouseEventKind::ScrollDown => {
            if !app.listing.entries.is_empty() && app.selected + 1 < app.listing.entries.len() {
                app.set_selected(app.selected + 1);
            }
            app.dirty = true;
        }
        MouseEventKind::Moved => {
            let col = m.column;
            let row = m.row;
            let mut h = None;
            for (rect, idx) in &app.row_hits {
                if hit(*rect, col, row) {
                    h = Some(*idx);
                    break;
                }
            }
            if h != app.hover {
                app.hover = h;
                app.dirty = true;
            }
        }
        MouseEventKind::Down(MouseButton::Left) => {
            app.dirty = true;
            let col = m.column;
            let row = m.row;
            if matches!(app.mode, Mode::PathEdit) {
                for (rect, dest) in &app.crumb_hits {
                    if hit(*rect, col, row) {
                        app.mode = Mode::Browse;
                        apply_goto(app, &dest.display().to_string());
                        return;
                    }
                }
                return;
            }
            for (rect, dest) in &app.crumb_hits {
                if hit(*rect, col, row) {
                    apply_goto(app, &dest.display().to_string());
                    return;
                }
            }
            if let Some(r) = app.path_hit {
                if hit(r, col, row) {
                    begin_path(app);
                    return;
                }
            }
            for (rect, idx) in &app.row_hits {
                if hit(*rect, col, row) {
                    let idx = *idx;
                    let now = Instant::now();
                    let dbl = app
                        .last_click
                        .map(|(t, i)| i == idx && now.duration_since(t) < Duration::from_millis(350))
                        .unwrap_or(false);
                    app.set_selected(idx);
                    app.last_click = Some((now, idx));
                    if dbl {
                        open_selected(app);
                    }
                    return;
                }
            }
        }
        _ => {}
    }
}

fn hit(r: Rect, x: u16, y: u16) -> bool {
    x >= r.x && x < r.x.saturating_add(r.width) && y >= r.y && y < r.y.saturating_add(r.height)
}

fn draw(f: &mut Frame, app: &mut App) {
    let size = f.area();
    let extra = match app.mode {
        Mode::Help => 8,
        Mode::New | Mode::Rename | Mode::ConfirmDelete | Mode::PathEdit => 3,
        Mode::Browse => 0,
    };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(extra),
            Constraint::Length(1),
        ])
        .split(size);

    app.crumb_hits.clear();
    app.row_hits.clear();
    draw_header(f, chunks[0], app);
    draw_list(f, chunks[1], app);
    match app.mode {
        Mode::Help => draw_help(f, chunks[2]),
        Mode::PathEdit => draw_path_edit(f, chunks[2], app),
        Mode::New => draw_prompt(f, chunks[2], " new · end with / for a dir ", &app.prompt),
        Mode::Rename => draw_prompt(f, chunks[2], " rename ", &app.prompt),
        Mode::ConfirmDelete => {
            let name = app
                .current()
                .map(|e| e.name.as_str())
                .unwrap_or("?");
            draw_prompt(f, chunks[2], " delete ", &format!("{name}?  y / N"));
        }
        Mode::Browse => {}
    }
    let keys = if app.pick {
        format!(
            " {} pick · enter choose · p this dir · j/k · h parent · q  ·  {} ",
            mark(),
            app.listing.sort.as_str()
        )
    } else {
        format!(
            " {} j/k · enter open · h parent · : path · n/d/r/e · . hidden · s sort · ?  ·  {} ",
            mark(),
            app.listing.sort.as_str()
        )
    };
    let footer = if app.status.is_empty() {
        keys
    } else {
        format!(" {} ", app.status)
    };
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(footer, Style::default().fg(SILVER)))),
        chunks[3],
    );

    if matches!(app.mode, Mode::PathEdit) {
        let inner = chunks[2].x.saturating_add(1);
        let y = chunks[2].y.saturating_add(1);
        let x = inner.saturating_add(app.path_caret as u16);
        f.set_cursor_position(Position { x, y });
    } else if matches!(app.mode, Mode::New | Mode::Rename) {
        let inner = chunks[2].x.saturating_add(1);
        let y = chunks[2].y.saturating_add(1);
        let x = inner.saturating_add(app.prompt_caret as u16);
        f.set_cursor_position(Position { x, y });
    }
}

fn draw_header(f: &mut Frame, area: Rect, app: &mut App) {
    let title = if app.pick {
        format!(" {} Spellbook · pick {} ", mark(), mark())
    } else {
        format!(" {} Spellbook {} ", mark(), mark())
    };
    let crumbs = paths::path_crumbs(&app.listing.dir);
    let mut spans: Vec<Span> = Vec::new();
    let mut x = area.x.saturating_add(2);
    let y = area.y.saturating_add(1);
    for (i, (label, dest)) in crumbs.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" / ", Style::default().fg(DIM)));
            x = x.saturating_add(3);
        }
        let w = label.chars().count() as u16;
        app.crumb_hits.push((
            Rect {
                x,
                y,
                width: w.max(1),
                height: 1,
            },
            dest.clone(),
        ));
        spans.push(Span::styled(
            label.clone(),
            Style::default().fg(PINK).add_modifier(Modifier::BOLD),
        ));
        x = x.saturating_add(w);
    }
    app.path_hit = Some(Rect {
        x: area.x.saturating_add(1),
        y: area.y.saturating_add(1),
        width: area.width.saturating_sub(2),
        height: 1,
    });
    let status = if app.status.is_empty() {
        Line::from(spans)
    } else {
        Line::from(spans)
    };
    let _ = status;
    let mut lines = vec![Line::from({
        let mut s = Vec::new();
        for (i, (label, _)) in crumbs.iter().enumerate() {
            if i > 0 {
                s.push(Span::styled("/", Style::default().fg(DIM)));
            }
            s.push(Span::styled(
                label.clone(),
                Style::default().fg(PINK).add_modifier(Modifier::BOLD),
            ));
        }
        s
    })];
    if !app.status.is_empty() {
        lines.push(Line::from(Span::styled(
            app.status.clone(),
            Style::default().fg(GOLD),
        )));
    }
    f.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_style(Style::default().fg(PINK))
                .border_set(border_set()),
        ),
        area,
    );
}

fn kind_color(k: Kind) -> Color {
    match k {
        Kind::Dir => BLUSH,
        Kind::Link => LILAC,
        Kind::Exec => GOLD,
        Kind::Image => ROSE,
        Kind::Audio => TEAL,
        Kind::Archive => DIM,
        Kind::Code => SILVER,
        Kind::Text => FG,
    }
}

fn ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn mix(a: Color, b: Color, t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    match (a, b) {
        (Color::Rgb(ar, ag, ab), Color::Rgb(br, bg, bb)) => Color::Rgb(
            (ar as f32 + (br as f32 - ar as f32) * t) as u8,
            (ag as f32 + (bg as f32 - ag as f32) * t) as u8,
            (ab as f32 + (bb as f32 - ab as f32) * t) as u8,
        ),
        _ => b,
    }
}

fn focus_t(app: &App) -> f32 {
    ease(app.focus_t0.elapsed().as_millis() as f32 / FOCUS_MS as f32)
}

fn shelf_dim(app: &App) -> f32 {
    match app.shelf_t0 {
        Some(t0) => {
            let t = ease(t0.elapsed().as_millis() as f32 / SHELF_MS as f32);
            0.35 + 0.65 * t
        }
        None => 1.0,
    }
}

fn draw_list(f: &mut Frame, area: Rect, app: &mut App) {
    let inner = Rect {
        x: area.x.saturating_add(1),
        y: area.y.saturating_add(1),
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(PINK))
        .border_set(border_set())
        .title(format!(" {} ", app.listing.dir.display()));
    f.render_widget(block, area);
    if inner.width < 8 || inner.height == 0 {
        return;
    }
    if app.listing.entries.is_empty() {
        let msg = app
            .listing
            .error
            .clone()
            .unwrap_or_else(|| "(empty shelf)".into());
        f.render_widget(
            Paragraph::new(Span::styled(msg, Style::default().fg(DIM))),
            inner,
        );
        return;
    }
    let vis = inner.height as usize;
    if app.selected < app.offset {
        app.offset = app.selected;
    }
    if app.selected >= app.offset + vis {
        app.offset = app.selected + 1 - vis;
    }
    let dim = shelf_dim(app);
    let ft = focus_t(app);
    let mark_sel = if unicode() { "▸" } else { ">" };
    let mark_hov = if unicode() { "·" } else { "." };
    let mut lines: Vec<Line> = Vec::new();
    for (row, i) in (app.offset..).take(vis).enumerate() {
        let Some(e) = app.listing.entries.get(i) else { break };
        let y = inner.y.saturating_add(row as u16);
        app.row_hits.push((
            Rect {
                x: inner.x,
                y,
                width: inner.width,
                height: 1,
            },
            i,
        ));
        let sel = i == app.selected;
        let hov = app.hover == Some(i) && !sel;
        let leaving = i == app.focus_from && !sel && ft < 1.0;
        let focus = if sel {
            ft
        } else if leaving {
            1.0 - ft
        } else {
            0.0
        };
        let gutter = if sel && focus > 0.4 {
            mark_sel
        } else if hov {
            mark_hov
        } else {
            " "
        };
        let tag = e.kind.tag();
        let shown = if e.is_dir {
            format!("{tag} {}/", e.name)
        } else {
            format!("{tag} {}", e.name)
        };
        let info = e.size_label();
        let w = inner.width as usize;
        let gutter_s = format!("{gutter} ");
        let pad = w
            .saturating_sub(gutter_s.chars().count())
            .saturating_sub(shown.chars().count())
            .saturating_sub(info.chars().count());
        let mut ink = kind_color(e.kind);
        if dim < 0.99 {
            ink = mix(Color::Rgb(0x1a, 0x0a, 0x12), ink, dim);
        }
        let mut name_style = Style::default().fg(ink);
        if sel {
            name_style = name_style
                .fg(mix(ink, PINK, 0.35 * focus))
                .bg(mix(Color::Rgb(0x1a, 0x0a, 0x12), BLUSH_BG, 0.55 * focus.max(0.25)))
                .add_modifier(Modifier::BOLD);
        } else if hov {
            name_style = name_style.bg(Color::Rgb(0x28, 0x12, 0x1a));
        }
        lines.push(Line::from(vec![
            Span::styled(
                gutter_s,
                Style::default().fg(if sel { PINK } else { DIM }),
            ),
            Span::styled(shown, name_style),
            Span::raw(" ".repeat(pad)),
            Span::styled(info, Style::default().fg(DIM)),
        ]));
    }
    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_help(f: &mut Frame, area: Rect) {
    let t = "j/k move · enter/l/o open · h parent · : / ctrl-l path\n\
             y yank · Y yank shelf · P paste-go · . hidden · s sort\n\
             n new (name/ = dir) · d delete · r rename · e edit\n\
             mouse: click · double-open · wheel · click crumbs\n\
             any key closes this page";
    f.render_widget(
        Paragraph::new(t).style(Style::default().fg(SILVER)).block(
            Block::default()
                .title(" keys ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(DIM))
                .border_set(border_set()),
        ),
        area,
    );
}

fn draw_path_edit(f: &mut Frame, area: Rect, app: &App) {
    draw_prompt(f, area, " path ", &app.path_buf);
}

fn draw_prompt(f: &mut Frame, area: Rect, title: &str, text: &str) {
    f.render_widget(Clear, area);
    f.render_widget(
        Paragraph::new(text).block(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_style(Style::default().fg(PINK))
                .border_set(border_set()),
        ),
        area,
    );
}

fn suspend_editor<B: ratatui::backend::Backend>(
    term: &mut Terminal<B>,
    path: &Path,
) -> anyhow::Result<()> {
    disable_raw_mode()?;
    execute!(
        io::stdout(),
        LeaveAlternateScreen,
        event::DisableMouseCapture,
        event::DisableBracketedPaste
    )?;
    let _ = term.show_cursor();
    let raw = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| "nano".into());
    let mut parts = raw.split_whitespace();
    let bin = parts.next().unwrap_or("nano");
    let rest: Vec<_> = parts.collect();
    let _ = std::process::Command::new(bin)
        .args(&rest)
        .arg(path)
        .status();
    enable_raw_mode()?;
    execute!(
        io::stdout(),
        EnterAlternateScreen,
        event::EnableMouseCapture,
        event::EnableBracketedPaste
    )?;
    term.clear()?;
    Ok(())
}
