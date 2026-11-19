//! A terminal pane: a PTY child process + the cmx-vt emulator + scrollback view + selection.

use cmx_sys::Termios;
use cmx_sys::pty::{CommandSpec, Pty, find_program};
use cmx_tui::{Buffer, Color, KeyEvent, MouseEvent, Rect, Style, Sym, attr};
use cmx_vt::Term;
use std::io;
use std::path::{Path, PathBuf};

use crate::theme::Theme;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PaneKind {
    Shell,
    Claude,
}

pub struct TermPane {
    pub kind: PaneKind,
    pub pty: Option<Pty>,
    pub vt: Term,
    write_q: Vec<u8>,
    pub exited: Option<i32>,
    /// Lines scrolled back from the live screen.
    pub scroll: usize,
    /// Selection in absolute line coordinates (line, col).
    pub sel: Option<((usize, usize), (usize, usize))>,
    pub error: Option<String>,
    pub cwd: PathBuf,
    pub sync_since: Option<std::time::Instant>,
    pub id: usize,
}

/// Locate the `claude` CLI: `$CMX_CLAUDE` (explicit override), then PATH, then the usual
/// install locations.
pub fn find_claude() -> Option<PathBuf> {
    if let Some(custom) = std::env::var_os("CMX_CLAUDE").filter(|v| !v.is_empty()) {
        return find_program(&custom.to_string_lossy());
    }
    if let Some(p) = find_program("claude") {
        return Some(p);
    }
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    let cands = [
        home.join(".local/bin/claude"),
        home.join(".claude/local/claude"),
        home.join(".npm-global/bin/claude"),
        PathBuf::from("/opt/homebrew/bin/claude"),
        PathBuf::from("/usr/local/bin/claude"),
    ];
    cands.into_iter().find(|p| p.is_file())
}

pub fn default_shell() -> String {
    std::env::var("SHELL")
        .ok()
        .filter(|s| !s.is_empty() && find_program(s).is_some())
        .unwrap_or_else(|| if Path::new("/bin/zsh").exists() { "/bin/zsh".into() } else { "/bin/sh".into() })
}

impl TermPane {
    pub fn spawn(
        kind: PaneKind,
        id: usize,
        cwd: &Path,
        cols: u16,
        rows: u16,
        termios: Option<&Termios>,
        theme: &Theme,
    ) -> TermPane {
        let mut vt = Term::new(cols.max(2), rows.max(1));
        if let Color::Rgb(r, g, b) = theme.term_bg {
            vt.default_bg = (r, g, b);
        }
        if let Color::Rgb(r, g, b) = theme.term_fg {
            vt.default_fg = (r, g, b);
        }
        let mut pane = TermPane {
            kind,
            pty: None,
            vt,
            write_q: Vec::new(),
            exited: None,
            scroll: 0,
            sel: None,
            error: None,
            cwd: cwd.to_path_buf(),
            sync_since: None,
            id,
        };
        let spec = match kind {
            PaneKind::Shell => {
                let sh = default_shell();
                let mut spec = CommandSpec::new(sh.clone());
                if sh.ends_with("bash") || sh.ends_with("zsh") || sh.ends_with("fish") || sh.ends_with("/sh") {
                    spec = spec.arg("-l");
                }
                Some(spec)
            }
            PaneKind::Claude => match find_claude() {
                Some(p) => Some(CommandSpec::new(p.to_string_lossy().into_owned())),
                None => {
                    pane.error = Some(
                        "The `claude` CLI was not found on PATH.\n\nInstall Claude Code:\n  curl -fsSL https://claude.ai/install.sh | bash\n  (or: npm install -g @anthropic-ai/claude-code)\n\nThen press Enter here to retry."
                            .into(),
                    );
                    None
                }
            },
        };
        if let Some(spec) = spec {
            let spec = spec
                .cwd(cwd)
                .env("TERM", "xterm-256color")
                .env("COLORTERM", "truecolor")
                .env("TERM_PROGRAM", "cmx")
                .env("TERM_PROGRAM_VERSION", env!("CARGO_PKG_VERSION"))
                .env("CMX_TERMINAL", "1");
            let mut spec = spec;
            spec.env_remove =
                vec!["COLUMNS".into(), "LINES".into(), "TERM_SESSION_ID".into(), "ITERM_SESSION_ID".into()];
            match Pty::spawn(&spec, cols.max(2), rows.max(1), termios) {
                Ok(p) => pane.pty = Some(p),
                Err(e) => pane.error = Some(format!("failed to start {}: {e}", spec.program)),
            }
        }
        pane
    }

    pub fn title(&self) -> String {
        if !self.vt.title.is_empty() {
            return self.vt.title.clone();
        }
        match self.kind {
            PaneKind::Claude => "claude".into(),
            PaneKind::Shell => Path::new(&default_shell())
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "shell".into()),
        }
    }

    pub fn fd(&self) -> Option<i32> {
        if self.exited.is_some() { None } else { self.pty.as_ref().map(|p| p.fd()) }
    }

    pub fn wants_write(&self) -> bool {
        !self.write_q.is_empty() && self.pty.is_some() && self.exited.is_none()
    }

    pub fn is_alive(&self) -> bool {
        self.pty.is_some() && self.exited.is_none()
    }

    /// Drain readable output into the emulator. Returns true if something changed.
    pub fn on_readable(&mut self, budget: usize) -> bool {
        let Some(pty) = self.pty.as_ref() else { return false };
        let mut buf = [0u8; 65536];
        let mut total = 0;
        let mut changed = false;
        loop {
            match pty.read(&mut buf) {
                Ok(0) => {
                    self.mark_exited();
                    break;
                }
                Ok(n) => {
                    self.vt.feed(&buf[..n]);
                    changed = true;
                    total += n;
                    if total >= budget {
                        break;
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(_) => {
                    // EIO: slave side closed (child exited)
                    self.mark_exited();
                    break;
                }
            }
        }
        if changed {
            if !self.vt.responses.is_empty() {
                let r = std::mem::take(&mut self.vt.responses);
                self.write_q.extend_from_slice(&r);
            }
            if self.scroll > 0 && self.vt.scrolled_lines > 0 {
                // keep a scrolled-back view anchored on the same text
                self.scroll = (self.scroll + self.vt.scrolled_lines).min(self.vt.history_len());
            }
            self.vt.scrolled_lines = 0;
            if self.vt.modes.sync {
                self.sync_since.get_or_insert_with(std::time::Instant::now);
            } else {
                self.sync_since = None;
            }
        }
        changed
    }

    fn mark_exited(&mut self) {
        if self.exited.is_none() {
            let code = self.pty.as_mut().and_then(|p| p.try_wait()).unwrap_or(0);
            self.exited = Some(code);
        }
    }

    pub fn poll_exit(&mut self) -> bool {
        if self.exited.is_some() {
            return true;
        }
        if let Some(p) = self.pty.as_mut()
            && let Some(code) = p.try_wait()
        {
            self.exited = Some(code);
            return true;
        }
        false
    }

    pub fn flush_writes(&mut self) {
        let Some(pty) = self.pty.as_ref() else {
            self.write_q.clear();
            return;
        };
        while !self.write_q.is_empty() {
            match pty.write(&self.write_q) {
                Ok(0) => break,
                Ok(n) => {
                    self.write_q.drain(..n);
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(_) => {
                    self.write_q.clear();
                    break;
                }
            }
        }
    }

    pub fn send(&mut self, bytes: &[u8]) {
        if self.is_alive() {
            self.write_q.extend_from_slice(bytes);
            self.flush_writes();
        }
    }

    pub fn send_key(&mut self, k: &KeyEvent) {
        self.scroll = 0;
        self.sel = None;
        let b = self.vt.encode_key(k);
        self.send(&b);
    }

    pub fn paste(&mut self, text: &str) {
        self.scroll = 0;
        let b = self.vt.encode_paste(text);
        self.send(&b);
    }

    pub fn focus_changed(&mut self, focused: bool) {
        if let Some(b) = self.vt.encode_focus(focused) {
            let b = b.to_vec();
            self.send(&b);
        }
    }

    /// Forward a mouse event (coordinates relative to the pane content). Returns false if the
    /// application didn't ask for mouse events.
    pub fn send_mouse(&mut self, ev: &MouseEvent, x: u16, y: u16) -> bool {
        match self.vt.encode_mouse(ev, x, y) {
            Some(b) => {
                self.send(&b);
                true
            }
            None => false,
        }
    }

    pub fn resize(&mut self, cols: u16, rows: u16) {
        let (cols, rows) = (cols.max(2), rows.max(1));
        if self.vt.cols() == cols as usize && self.vt.rows() == rows as usize {
            return;
        }
        self.vt.resize(cols, rows);
        if let Some(p) = self.pty.as_ref() {
            let _ = p.resize(cols, rows);
        }
    }

    pub fn scroll_by(&mut self, delta: isize) {
        let max = self.vt.history_len() as isize;
        self.scroll = (self.scroll as isize + delta).clamp(0, max) as usize;
    }

    /// Absolute line for a content row.
    pub fn abs_line(&self, y: usize) -> usize {
        let h = self.vt.history_len();
        h - self.scroll.min(h) + y
    }

    pub fn selected_text(&self) -> Option<String> {
        let (a, b) = self.sel?;
        if a == b {
            return None;
        }
        let (a, b) = if a <= b { (a, b) } else { (b, a) };
        let s = self.vt.text_range(a, (b.0, b.1 + 1));
        if s.is_empty() { None } else { Some(s) }
    }

    /// Draw into `r`; returns the cursor position (absolute screen coords) when visible.
    pub fn render(&self, buf: &mut Buffer, r: Rect, theme: &Theme, focused: bool) -> Option<(u16, u16, u8)> {
        let base = theme.term();
        buf.fill(r, ' ', base);
        if let Some(err) = &self.error {
            for (i, line) in err.lines().enumerate() {
                if i as u16 >= r.h {
                    break;
                }
                buf.put_str(r.x + 1, r.y + i as u16, line, base.fg(theme.dim_fg), r.right());
            }
            return None;
        }
        let sel = self.sel.map(|(a, b)| if a <= b { (a, b) } else { (b, a) });
        let rows = (r.h as usize).min(self.vt.rows());
        for y in 0..rows {
            let row = self.vt.view_row(self.scroll, y);
            let abs = self.abs_line(y);
            let sy = r.y + y as u16;
            let mut x = 0usize;
            let n = row.cells.len().min(r.w as usize);
            while x < n {
                let c = &row.cells[x];
                if c.sym.is_cont() {
                    x += 1;
                    continue;
                }
                let mut st = map_style(c.style, base);
                if c.style.has(attr::HIDDEN) {
                    st.attrs &= !attr::HIDDEN;
                    st.fg = st.bg;
                }
                if let Some((a, b)) = sel
                    && (abs, x) >= a
                    && (abs, x) <= b
                {
                    st = Style { fg: theme.editor_fg, bg: theme.selection_bg, attrs: st.attrs & !attr::REVERSE };
                }
                let sym = if c.sym.width() == 2 && x + 1 >= r.w as usize { Sym::BLANK } else { c.sym };
                buf.set(r.x + x as u16, sy, sym, st, r.right());
                x += c.sym.width().max(1);
            }
        }
        if let Some(code) = self.exited {
            let msg = match self.kind {
                PaneKind::Claude => format!(" claude exited ({code}) — press Enter to restart "),
                PaneKind::Shell => format!(" process exited ({code}) "),
            };
            let y = r.bottom().saturating_sub(1);
            buf.put_str(r.x, y, &msg, Style::new(theme.status_fg, theme.split_focus_fg), r.right());
            return None;
        }
        if self.scroll > 0 {
            let label = format!(" ↑ {} lines ", self.scroll);
            let x = r.right().saturating_sub(label.chars().count() as u16);
            buf.put_str(x, r.y, &label, Style::new(theme.status_fg, theme.split_focus_fg), r.right());
            return None;
        }
        if focused && self.vt.modes.show_cursor {
            let (cx, cy) = self.vt.cursor();
            if (cx as u16) < r.w && (cy as u16) < r.h {
                return Some((r.x + cx as u16, r.y + cy as u16, self.vt.modes.cursor_shape));
            }
        }
        None
    }
}

/// Map emulator colors: "default" becomes the theme's terminal colors.
fn map_style(s: Style, base: Style) -> Style {
    let mut out = s;
    if out.fg == Color::Reset {
        out.fg = base.fg;
    }
    if out.bg == Color::Reset {
        out.bg = base.bg;
    }
    out
}
