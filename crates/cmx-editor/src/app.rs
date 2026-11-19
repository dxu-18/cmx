//! Application state, the poll-based event loop, and input/command dispatch.

use crate::document::Document;
use crate::history::Coalesce;
use crate::keymap::{Cmd, Keymap};
use crate::layout::{self, Layout, LayoutInput};
use crate::palette::{Palette, Picked};
use crate::prompt::{InputResult, Prompt, PromptKind};
use crate::search::{Flags, Query};
use crate::termpane::{PaneKind, TermPane};
use crate::text::Pos;
use crate::theme::{DARK, Theme};
use crate::tree::{Tree, list_files};
use crate::view::{SelKind, Tab};
use crate::vim::{self, Mode, Vim, VimOut};
use cmx_sys::fd;
use cmx_sys::poll::{POLLIN, POLLOUT, PollFd, poll};
use cmx_sys::signals::SignalPipe;
use cmx_sys::{Termios, ffi};
use cmx_tui::clipboard::Clipboard;
use cmx_tui::{
    Buffer, Event, InputParser, KeyCode, KeyEvent, Mods, MouseButton, MouseEvent, MouseKind, Rect, Renderer, term,
};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Focus {
    Editor,
    Tree,
    Term(usize),
    Claude,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kbd {
    Unknown,
    Kitty,
    Legacy,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Drag {
    TreeSplit,
    ClaudeSplit,
    PanelSplit,
    TermSplit(usize),
    EditorSelect,
    Scrollbar,
    TermSelect(Option<usize>),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MsgKind {
    Info,
    Warn,
    Error,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PanelBtn {
    New,
    Kill,
    Hide,
    HideClaude,
    RestartClaude,
    HideTree,
    RefreshTree,
}

/// Clickable regions recorded by the renderer.
#[derive(Default)]
pub struct Hits {
    pub tabs: Vec<(Rect, usize)>,
    pub tab_close: Vec<(Rect, usize)>,
    pub term_tabs: Vec<(Rect, usize)>,
    pub buttons: Vec<(Rect, PanelBtn)>,
    pub status_mode: Rect,
    pub gutter_w: u16,
    pub scrollbar: Option<Rect>,
    pub palette_rows: Vec<(Rect, usize)>,
    pub palette_box: Option<Rect>,
    pub help_box: Option<Rect>,
}

pub struct Options {
    pub files: Vec<PathBuf>,
    pub root: PathBuf,
    pub workspace: bool,
    /// `cmx stex new`: untitled buffer in stex mode, ".stex" appended on save.
    pub stex: bool,
}

pub struct App {
    pub width: u16,
    pub height: u16,
    pub buf: Buffer,
    renderer: Renderer,
    parser: InputParser,
    pub out: Vec<u8>,
    pub theme: &'static Theme,
    pub keymap: Keymap,
    pub clipboard: Clipboard,
    clip_linewise: Option<String>,
    pub kbd: Kbd,
    pub seen_enhanced: bool,
    mok: bool,
    hint_shown: bool,
    pub root: PathBuf,
    pub tabs: Vec<Tab>,
    pub active: usize,
    untitled_n: usize,
    default_ext: Option<&'static str>,
    pub tree: Tree,
    pub show_tree: bool,
    pub tree_w: u16,
    pub terms: Vec<TermPane>,
    pub active_term: usize,
    pub term_ratios: Vec<f32>,
    pub show_panel: bool,
    pub panel_h: u16,
    pub claude: Option<TermPane>,
    pub show_claude: bool,
    pub claude_w: u16,
    pub focus: Focus,
    pub vim_on: bool,
    pub vim: Vim,
    pub prompt: Option<Prompt>,
    pub palette: Option<Palette>,
    pub help: bool,
    pub help_scroll: usize,
    pub message: Option<(String, MsgKind, Instant)>,
    pub search: Option<String>,
    /// Highlight all matches of `search` (cleared by Esc / :noh, query kept for F3 / n).
    pub search_hl: bool,
    /// Find options: Alt+R regex, Alt+C match case, Alt+W whole word (in the Find prompt).
    pub search_flags: Flags,
    last_disk_check: Instant,
    /// Splitter under the mouse: highlighted, and the pointer shows a resize shape (OSC 22).
    pub hover_split: Option<Drag>,
    /// Mouse cell over the hovered splitter (a ↔/↕ marker is drawn there: a resize cursor that
    /// works in every terminal, Terminal.app included).
    pub hover_at: (u16, u16),
    pointer: Option<&'static str>,
    /// The OS can tell whether Shift is held (cmx_sys::modkeys): Ctrl+Shift+letter is then
    /// distinguished even on terminals that send it as Ctrl+letter.
    pub mod_probe: bool,
    /// macOS Terminal.app drops Ctrl+Shift+letter entirely; a listen-only event tap (Input
    /// Monitoring) reports those chords on this pipe (cmx_sys::keytap, EDITOR D-30).
    keytap_fd: Option<i32>,
    /// The tap swallows the chords it handles, so Terminal.app doesn't beep (needs Accessibility).
    keytap_silent: bool,
    /// This terminal tab has focus (focus reporting): tap chords only count while it does.
    term_focused: bool,
    drag: Option<Drag>,
    drag_origin: (u16, u16),
    last_click: Option<(Instant, u16, u16, u8)>,
    pub layout: Layout,
    pub hits: Hits,
    pub quit: bool,
    pub dirty: bool,
    last_render: Instant,
    termios: Option<Termios>,
    next_pane_id: usize,
    pending_close: Option<usize>,
    /// `:wq` on an untitled buffer: quit (or close the tab) once save-as succeeds.
    pending_wq: bool,
    files_cache: Option<(Instant, Vec<String>)>,
}

impl App {
    pub fn new(opts: Options, width: u16, height: u16, termios: Option<Termios>, truecolor: bool) -> App {
        let root = opts.root.clone();
        let mut app = App {
            width,
            height,
            buf: Buffer::new(width, height),
            renderer: Renderer::new(truecolor),
            parser: InputParser::new(),
            out: Vec::new(),
            theme: &DARK,
            keymap: Keymap::default(),
            clipboard: Clipboard::new(),
            clip_linewise: None,
            kbd: Kbd::Unknown,
            seen_enhanced: false,
            mok: false,
            hint_shown: false,
            tree: Tree::new(root.clone()),
            root,
            tabs: Vec::new(),
            active: 0,
            untitled_n: 0,
            default_ext: if opts.stex { Some("stex") } else { None },
            show_tree: false,
            tree_w: 30.min(width / 4).max(layout::MIN_TREE),
            terms: Vec::new(),
            active_term: 0,
            term_ratios: Vec::new(),
            show_panel: false,
            panel_h: (height.saturating_sub(1) * 3 / 10).max(6),
            claude: None,
            show_claude: false,
            claude_w: (width * 2 / 5).max(layout::MIN_CLAUDE),
            focus: Focus::Editor,
            vim_on: false,
            vim: Vim::new(),
            prompt: None,
            palette: None,
            help: false,
            help_scroll: 0,
            message: None,
            search: None,
            search_hl: false,
            search_flags: Flags::default(),
            last_disk_check: Instant::now(),
            hover_split: None,
            hover_at: (0, 0),
            pointer: None,
            mod_probe: false,
            keytap_fd: None,
            keytap_silent: false,
            term_focused: true,
            drag: None,
            drag_origin: (0, 0),
            last_click: None,
            layout: Layout::default(),
            hits: Hits::default(),
            quit: false,
            dirty: true,
            last_render: Instant::now(),
            termios,
            next_pane_id: 1,
            pending_close: None,
            pending_wq: false,
            files_cache: None,
        };
        // Ctrl+Shift detection (EDITOR D-27, D-30): Terminal.app drops Ctrl+Shift+letter, so it gets
        // the event tap; other terminals that send the plain control byte get the Shift probe
        let probe_off = std::env::var("CMX_MODPROBE").is_ok_and(|v| v == "0")
            || std::env::var_os("SSH_CONNECTION").is_some()
            || std::env::var_os("SSH_TTY").is_some();
        let apple_terminal = std::env::var("TERM_PROGRAM").is_ok_and(|t| t == "Apple_Terminal");
        if !probe_off && apple_terminal {
            if let Ok((fd, mode)) = cmx_sys::keytap::start() {
                app.keytap_fd = Some(fd);
                app.keytap_silent = mode == cmx_sys::keytap::Mode::Consume;
            }
            app.mod_probe = app.keytap_fd.is_some();
        } else if !probe_off {
            app.mod_probe = cmx_sys::modkeys::shift_held().is_some();
        }
        for f in &opts.files {
            app.open_path(f.clone(), false, true);
        }
        if app.tabs.is_empty() && !opts.workspace {
            app.new_untitled();
        }
        if opts.workspace {
            app.show_tree = true;
            app.spawn_terminal();
            app.show_panel = true;
            app.focus = Focus::Tree;
        }
        app.info("cmx — F1 shows all shortcuts · Ctrl+K toggles vim keys".to_string());
        app
    }

    /// One-time hint once the keyboard tier is known (EDITOR-DESIGN D-19).
    fn keyboard_hints(&mut self) {
        if self.hint_shown || self.kbd == Kbd::Unknown {
            return;
        }
        self.hint_shown = true;
        // Terminal.app sends Ctrl+Shift+X as Ctrl+X; cmx can read Shift from macOS once Terminal
        // has Input Monitoring (EDITOR D-27): ask macOS to show its prompt (once) and say why
        if self.kbd == Kbd::Legacy
            && !self.mod_probe
            && cmx_sys::modkeys::needs_permission()
            && std::env::var("TERM_PROGRAM").is_ok_and(|t| t == "Apple_Terminal")
        {
            cmx_sys::modkeys::request_access();
            self.message = Some((
                "Ctrl+Shift keys: allow Terminal in System Settings › Privacy & Security › Input Monitoring, then reopen Terminal"
                    .into(),
                MsgKind::Warn,
                Instant::now() + Duration::from_secs(10),
            ));
            return;
        }
        if self.keytap_fd.is_some() && !self.keytap_silent {
            self.info(
                "Ctrl+Shift works; to stop Terminal's beep on them, allow Terminal under Privacy & Security › Accessibility"
                    .into(),
            );
            return;
        }
        if std::env::var("TERM_PROGRAM").is_ok_and(|t| t == "vscode") && self.kbd != Kbd::Kitty {
            self.info(
                "VS Code: enable terminal.integrated.enableKittyKeyboardProtocol + sendKeybindingsToShell for Ctrl+Shift keys"
                    .into(),
            );
        }
    }

    // ------------------------------------------------------------------ messages

    pub fn info(&mut self, s: String) {
        self.message = Some((s, MsgKind::Info, Instant::now()));
    }
    pub fn warn(&mut self, s: String) {
        self.message = Some((s, MsgKind::Warn, Instant::now()));
    }
    pub fn error(&mut self, s: String) {
        self.message = Some((s, MsgKind::Error, Instant::now()));
    }

    // ------------------------------------------------------------------ main loop

    pub fn main_loop(&mut self, sig: &SignalPipe) -> io::Result<()> {
        let mut events: Vec<Event> = Vec::new();
        let mut inbuf = vec![0u8; 1 << 16];
        let mut pending_since: Option<Instant> = None;
        self.render()?;
        loop {
            let mut fds = vec![PollFd::new(0, POLLIN), PollFd::new(sig.fd(), POLLIN)];
            let tap_slot = self.keytap_fd.map(|f| {
                fds.push(PollFd::new(f, POLLIN));
                fds.len() - 1
            });
            let mut pane_slots: Vec<(usize, Option<usize>)> = Vec::new();
            for (i, t) in self.terms.iter().enumerate() {
                if let Some(f) = t.fd() {
                    pane_slots.push((fds.len(), Some(i)));
                    fds.push(PollFd::new(f, POLLIN | if t.wants_write() { POLLOUT } else { 0 }));
                }
            }
            if let Some(c) = &self.claude
                && let Some(f) = c.fd()
            {
                pane_slots.push((fds.len(), None));
                fds.push(PollFd::new(f, POLLIN | if c.wants_write() { POLLOUT } else { 0 }));
            }
            let mut timeout: i64 = -1;
            let mut tighten = |ms: i64| {
                if timeout < 0 || ms < timeout {
                    timeout = ms.max(0);
                }
            };
            if self.parser.has_pending() {
                tighten(25);
            }
            if self.dirty {
                tighten(8 - self.last_render.elapsed().as_millis().min(8) as i64);
            }
            if let Some((_, _, t)) = &self.message {
                let left = 5000 - t.elapsed().as_millis().min(5000) as i64;
                tighten(left + 5);
            }
            if self.any_sync() {
                tighten(20);
            }
            tighten(1000 - self.last_disk_check.elapsed().as_millis().min(1000) as i64);
            poll(&mut fds, timeout.min(i32::MAX as i64) as i32)?;

            let mut interactive = false;
            // Ctrl+Shift chords seen by the event tap (Terminal.app sends nothing for them)
            if let (Some(slot), Some(tfd)) = (tap_slot, self.keytap_fd)
                && fds[slot].readable()
            {
                let mut b = [0u8; 64];
                if let Ok(n) = fd::read(tfd, &mut b) {
                    for &c in &b[..n] {
                        if self.term_focused && c.is_ascii_lowercase() {
                            self.handle_event(Event::Key(KeyEvent::new(KeyCode::Char(c as char), Mods::CTRL_SHIFT)));
                        }
                    }
                }
                interactive = true;
            }
            if fds[1].readable() {
                for s in sig.drain() {
                    if s == ffi::SIGWINCH {
                        self.on_resize();
                    } else if s == ffi::SIGCHLD {
                        self.reap();
                    }
                }
                interactive = true;
            }
            if fds[0].readable() {
                match fd::read(0, &mut inbuf) {
                    Ok(0) => self.quit = true,
                    Ok(n) => {
                        self.parser.feed(&inbuf[..n], &mut events);
                        pending_since = if self.parser.has_pending() { Some(Instant::now()) } else { None };
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
                    Err(e) => return Err(e),
                }
                interactive = true;
            } else if let Some(t) = pending_since
                && t.elapsed() >= Duration::from_millis(20)
            {
                self.parser.flush(&mut events);
                pending_since = None;
                interactive = true;
            }
            for ev in events.drain(..) {
                self.handle_event(ev);
                if self.quit {
                    break;
                }
            }
            for (slot, which) in pane_slots {
                let f = fds[slot];
                let pane = match which {
                    Some(i) => self.terms.get_mut(i),
                    None => self.claude.as_mut(),
                };
                if let Some(p) = pane {
                    if f.readable() && p.on_readable(1 << 20) {
                        self.dirty = true;
                    }
                    if f.writable() {
                        p.flush_writes();
                    }
                }
            }
            self.after_panes();
            if self.quit {
                break;
            }
            if self.last_disk_check.elapsed() >= Duration::from_secs(1) {
                self.check_disk();
            }
            if let Some((_, _, t)) = &self.message
                && t.elapsed() >= Duration::from_secs(5)
            {
                self.message = None;
                self.dirty = true;
            }
            if !self.out.is_empty() {
                fd::write_all(1, &self.out)?;
                self.out.clear();
            }
            if self.dirty
                && (interactive || self.last_render.elapsed() >= Duration::from_millis(8))
                && !self.sync_hold()
            {
                self.render()?;
            }
        }
        Ok(())
    }

    fn any_sync(&self) -> bool {
        self.sync_hold()
    }

    /// Hold rendering while a pane is inside a synchronized-output block (max 100 ms).
    fn sync_hold(&self) -> bool {
        self.terms
            .iter()
            .chain(self.claude.iter())
            .any(|p| p.sync_since.is_some_and(|t| t.elapsed() < Duration::from_millis(100)))
    }

    pub fn render(&mut self) -> io::Result<()> {
        let cursor = crate::render::draw(self);
        let bytes = self.renderer.render(&self.buf, cursor);
        fd::write_all(1, bytes)?;
        self.dirty = false;
        self.last_render = Instant::now();
        Ok(())
    }

    pub fn shutdown(&mut self) {
        let mut out = Vec::new();
        if self.pointer.is_some() {
            out.extend_from_slice(&term::pointer("text"));
        }
        if self.kbd == Kbd::Kitty {
            out.extend_from_slice(term::KITTY_POP);
        }
        if self.mok {
            out.extend_from_slice(term::MOK_DISABLE);
        }
        out.extend_from_slice(term::LEAVE);
        let _ = fd::write_all(1, &out);
        self.terms.clear();
        self.claude = None;
    }

    fn on_resize(&mut self) {
        if let Some((w, h)) = cmx_sys::tty::window_size(1)
            && (w != self.width || h != self.height)
        {
            self.width = w;
            self.height = h;
            self.buf.resize(w, h);
            self.renderer.invalidate();
        }
        self.dirty = true;
    }

    fn reap(&mut self) {
        for t in &mut self.terms {
            t.poll_exit();
        }
        if let Some(c) = &mut self.claude {
            c.poll_exit();
        }
        self.dirty = true;
    }

    fn after_panes(&mut self) {
        let mut clips = Vec::new();
        for p in self.terms.iter_mut().chain(self.claude.iter_mut()) {
            if let Some(t) = p.vt.clipboard.take() {
                clips.push(t);
            }
            p.vt.bell = false;
        }
        for c in clips {
            self.clipboard.copy(&c, &mut self.out);
        }
        // remove exited shells
        let mut i = 0;
        let mut removed = false;
        while i < self.terms.len() {
            if self.terms[i].exited.is_some() {
                self.terms.remove(i);
                removed = true;
                if let Focus::Term(f) = self.focus {
                    if f == i {
                        self.focus = Focus::Editor;
                    } else if f > i {
                        self.focus = Focus::Term(f - 1);
                    }
                }
            } else {
                i += 1;
            }
        }
        if removed {
            self.equalize_terms();
            if self.terms.is_empty() {
                self.show_panel = false;
                if matches!(self.focus, Focus::Term(_)) {
                    self.focus = Focus::Editor;
                }
            }
            self.active_term = self.active_term.min(self.terms.len().saturating_sub(1));
            self.dirty = true;
        }
    }

    fn handle_event(&mut self, ev: Event) {
        self.dirty = true;
        match ev {
            Event::Key(k) => self.on_key(k),
            Event::Mouse(m) => self.on_mouse(m),
            Event::Paste(s) => self.on_paste(s),
            Event::Focus(true) => {
                self.term_focused = true;
                cmx_sys::keytap::set_focused(true);
                self.check_disk();
            }
            Event::Focus(false) => {
                self.term_focused = false;
                cmx_sys::keytap::set_focused(false);
            }
            Event::KittyFlags(_) => {
                if self.kbd != Kbd::Kitty {
                    self.kbd = Kbd::Kitty;
                    self.out.extend_from_slice(term::KITTY_PUSH);
                    self.keyboard_hints();
                }
            }
            Event::DeviceAttributes => {
                if self.kbd == Kbd::Unknown {
                    self.kbd = Kbd::Legacy;
                    self.out.extend_from_slice(term::MOK_ENABLE);
                    self.mok = true;
                    self.keyboard_hints();
                }
            }
        }
    }

    // ------------------------------------------------------------------ tabs & files

    pub fn active_tab(&mut self) -> Option<&mut Tab> {
        self.tabs.get_mut(self.active)
    }

    pub fn new_untitled(&mut self) {
        self.untitled_n += 1;
        let mut doc = Document::untitled(self.untitled_n);
        if let Some(ext) = self.default_ext {
            doc.default_ext = Some(ext);
            if ext == "stex" {
                doc.lang = crate::highlight::Lang::Stex;
            }
        }
        let at = if self.tabs.is_empty() { 0 } else { self.active + 1 };
        self.tabs.insert(at, Tab::new(doc));
        self.active = at;
        self.set_focus(Focus::Editor);
        self.vim_sync_mode();
    }

    pub fn resolve(&self, input: &str) -> PathBuf {
        let s = input.trim();
        if let Some(rest) = s.strip_prefix("~/")
            && let Some(h) = std::env::var_os("HOME")
        {
            return PathBuf::from(h).join(rest);
        }
        let p = PathBuf::from(s);
        if p.is_absolute() { p } else { self.root.join(p) }
    }

    pub fn rel(&self, p: &Path) -> String {
        p.strip_prefix(&self.root).unwrap_or(p).to_string_lossy().into_owned()
    }

    pub fn open_path(&mut self, path: PathBuf, preview: bool, focus_editor: bool) {
        let path = if path.is_absolute() { path } else { self.root.join(path) };
        let path = std::fs::canonicalize(&path).unwrap_or(path);
        if let Some(i) = self.tabs.iter().position(|t| t.doc.path.as_deref() == Some(path.as_path())) {
            self.active = i;
            if !preview {
                self.tabs[i].preview = false;
            }
            if focus_editor {
                self.set_focus(Focus::Editor);
            }
            return;
        }
        if path.is_dir() {
            self.show_tree = true;
            self.tree.reveal(&path.join("x"));
            return;
        }
        let doc = match Document::open(&path) {
            Ok(d) => d,
            Err(e) => {
                self.error(format!("cannot open {}: {e}", path.display()));
                return;
            }
        };
        let lossy = doc.lossy;
        let mut tab = Tab::new(doc);
        tab.preview = preview;
        let replace = if preview { self.tabs.iter().position(|t| t.preview && !t.doc.is_dirty()) } else { None };
        let idx = match replace {
            Some(i) => {
                self.tabs[i] = tab;
                i
            }
            None => {
                // replace a pristine untitled tab
                if self.tabs.len() == 1
                    && self.tabs[0].doc.path.is_none()
                    && !self.tabs[0].doc.is_dirty()
                    && self.tabs[0].doc.text.total_bytes() == 0
                {
                    self.tabs[0] = tab;
                    0
                } else {
                    let at = if self.tabs.is_empty() { 0 } else { self.active + 1 };
                    self.tabs.insert(at, tab);
                    at
                }
            }
        };
        self.active = idx;
        if lossy {
            self.warn("file is not valid UTF-8; invalid bytes were replaced".into());
        }
        if focus_editor {
            self.set_focus(Focus::Editor);
        }
        self.vim_sync_mode();
    }

    fn vim_sync_mode(&mut self) {
        if self.vim_on
            && let Some(t) = self.tabs.get_mut(self.active)
        {
            let mut ed = t.ed();
            self.vim.reset(&mut ed);
        }
    }

    fn save_active(&mut self) {
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        if tab.doc.path.is_none() {
            let hint = if tab.doc.default_ext.is_some() { "Save as (.stex): " } else { "Save as: " };
            self.prompt = Some(Prompt::new(PromptKind::SaveAs, hint, ""));
            return;
        }
        if tab.doc.conflict {
            let label = format!("{} changed on disk since it was opened. Overwrite it? (y/n) ", tab.doc.display_name());
            self.prompt = Some(Prompt::new(PromptKind::ConfirmOverwrite(self.active), label, ""));
            return;
        }
        self.save_now(self.active);
    }

    fn save_now(&mut self, i: usize) {
        let Some(tab) = self.tabs.get_mut(i) else { return };
        match tab.doc.save() {
            Ok(()) => {
                tab.preview = false;
                let name = tab.doc.display_name();
                let n = tab.doc.text.len_lines();
                self.info(format!("Saved {name} ({n} lines)"));
                self.tree.refresh();
            }
            Err(e) => self.error(format!("save failed: {e}")),
        }
    }

    /// Notice files changed on disk by other programs (every second and on focus-in): reload
    /// clean buffers, flag edited ones, mark deleted files, refresh the explorer.
    fn check_disk(&mut self) {
        self.last_disk_check = Instant::now();
        let mut msgs: Vec<(MsgKind, String)> = Vec::new();
        for t in &mut self.tabs {
            let Some(path) = t.doc.path.clone() else { continue };
            let now = crate::document::stamp(&path);
            if now == t.doc.disk {
                continue;
            }
            let name = t.doc.display_name();
            match now {
                None => {
                    if !t.doc.deleted {
                        t.doc.deleted = true;
                        msgs.push((MsgKind::Warn, format!("{name} was deleted on disk (Ctrl+S writes it again)")));
                    }
                    t.doc.disk = None;
                }
                Some(_) if t.doc.is_dirty() => {
                    t.doc.disk = now;
                    t.doc.deleted = false;
                    if !t.doc.conflict {
                        t.doc.conflict = true;
                        msgs.push((
                            MsgKind::Warn,
                            format!("{name} changed on disk; you have unsaved edits (palette: File: Reload from Disk)"),
                        ));
                    }
                }
                Some(_) => match t.doc.reload() {
                    Ok(changed) => {
                        let c = t.doc.text.clamp(t.view.cursor);
                        t.view.cursor = c;
                        t.view.anchor = t.view.anchor.map(|a| t.doc.text.clamp(a));
                        t.view.extra.clear();
                        if changed {
                            msgs.push((MsgKind::Info, format!("Reloaded {name} (changed on disk)")));
                        }
                    }
                    Err(e) => msgs.push((MsgKind::Error, format!("reloading {name}: {e}"))),
                },
            }
            self.dirty = true;
        }
        if self.tree.changed_on_disk() {
            self.tree.refresh();
            self.files_cache = None;
            self.dirty = true;
        }
        if let Some((kind, m)) = msgs.pop() {
            self.message = Some((m, kind, Instant::now()));
        }
    }

    fn reload_active(&mut self) {
        let Some(t) = self.tabs.get_mut(self.active) else { return };
        if t.doc.path.is_none() {
            return;
        }
        let name = t.doc.display_name();
        match t.doc.reload() {
            Ok(_) => {
                let c = t.doc.text.clamp(t.view.cursor);
                t.view.cursor = c;
                t.view.anchor = None;
                t.view.extra.clear();
                self.info(format!("Reloaded {name} from disk (Ctrl+Z restores your version)"));
            }
            Err(e) => self.error(format!("reloading {name}: {e}")),
        }
    }

    fn save_all(&mut self) -> usize {
        let mut n = 0;
        for t in &mut self.tabs {
            if t.doc.path.is_some() && t.doc.is_dirty() && t.doc.save().is_ok() {
                n += 1;
            }
        }
        n
    }

    pub fn request_close(&mut self, i: usize) {
        let Some(t) = self.tabs.get(i) else { return };
        if t.doc.is_dirty() {
            let label = format!("Save changes to {}? (y)es (n)o (c)ancel ", t.doc.display_name());
            self.active = i;
            self.prompt = Some(Prompt::new(PromptKind::ConfirmClose(i), label, ""));
        } else {
            self.close_tab(i);
        }
    }

    fn close_tab(&mut self, i: usize) {
        if i >= self.tabs.len() {
            return;
        }
        self.tabs.remove(i);
        if self.active >= self.tabs.len() {
            self.active = self.tabs.len().saturating_sub(1);
        } else if self.active > i {
            self.active -= 1;
        }
        self.vim_sync_mode();
    }

    fn request_quit(&mut self) {
        let dirty = self.tabs.iter().filter(|t| t.doc.is_dirty()).count();
        if dirty > 0 {
            let label = format!("{dirty} unsaved file{}. Quit anyway? (y/n) ", if dirty == 1 { "" } else { "s" });
            self.prompt = Some(Prompt::new(PromptKind::ConfirmQuit, label, ""));
        } else {
            self.quit = true;
        }
    }

    // ------------------------------------------------------------------ panels & focus

    pub fn set_focus(&mut self, f: Focus) {
        if f == self.focus {
            return;
        }
        let old = self.focus;
        self.focus = f;
        if let Some(p) = self.pane_for(old) {
            p.focus_changed(false);
        }
        if let Some(p) = self.pane_for(f) {
            p.focus_changed(true);
        }
        if let Focus::Term(i) = f {
            self.active_term = i;
        }
    }

    fn pane_for(&mut self, f: Focus) -> Option<&mut TermPane> {
        match f {
            Focus::Term(i) => self.terms.get_mut(i),
            Focus::Claude => self.claude.as_mut(),
            _ => None,
        }
    }

    fn layout_for(&self, show_panel: bool, show_claude: bool, nterms: usize) -> Layout {
        let ratios = vec![1.0 / nterms.max(1) as f32; nterms.max(1)];
        layout::compute(&LayoutInput {
            width: self.width,
            height: self.height,
            show_tree: self.show_tree,
            tree_w: self.tree_w,
            show_claude,
            claude_w: self.claude_w,
            show_panel,
            panel_h: self.panel_h,
            term_ratios: &ratios,
        })
    }

    pub fn spawn_terminal(&mut self) {
        let l = self.layout_for(true, self.show_claude, self.terms.len() + 1);
        let r = l.terms.last().copied().unwrap_or(Rect::new(0, 0, 80, 10));
        let id = self.next_pane_id;
        self.next_pane_id += 1;
        let pane = TermPane::spawn(PaneKind::Shell, id, &self.root, r.w, r.h, self.termios.as_ref(), self.theme);
        if let Some(e) = &pane.error {
            let e = e.clone();
            self.error(e);
            return;
        }
        self.terms.push(pane);
        self.equalize_terms();
        self.active_term = self.terms.len() - 1;
    }

    /// Run `cmx stex compile|check` on the active .stex file in the terminal panel (saving first).
    fn run_stex(&mut self, sub: &str) {
        let Some(tab) = self.tabs.get(self.active) else { return };
        let is_stex = tab.doc.path.as_ref().is_some_and(|p| p.extension().is_some_and(|e| e == "stex"))
            || tab.doc.default_ext == Some("stex");
        if !is_stex {
            self.warn("stex: the active file is not a .stex file".into());
            return;
        }
        if tab.doc.path.is_none() || tab.doc.is_dirty() {
            self.save_active();
        }
        let Some(path) = self.tabs.get(self.active).and_then(|t| t.doc.path.clone()) else { return };
        if self.terms.is_empty() {
            self.spawn_terminal();
        }
        if self.terms.is_empty() {
            return;
        }
        self.show_panel = true;
        let exe = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_else(|_| "cmx".into());
        let q = |s: &str| format!("'{}'", s.replace('\'', "'\\''"));
        let cmd = format!("{} stex {sub} {}\r", q(&exe), q(&path.display().to_string()));
        let i = self.active_term.min(self.terms.len() - 1);
        self.terms[i].send(cmd.as_bytes());
        self.set_focus(Focus::Term(i));
    }

    fn equalize_terms(&mut self) {
        let n = self.terms.len().max(1);
        self.term_ratios = vec![1.0 / n as f32; n];
    }

    fn spawn_claude(&mut self) {
        let l = self.layout_for(self.show_panel, true, self.terms.len());
        let r = l.claude_body.unwrap_or(Rect::new(0, 0, 60, 30));
        let id = self.next_pane_id;
        self.next_pane_id += 1;
        self.claude =
            Some(TermPane::spawn(PaneKind::Claude, id, &self.root, r.w, r.h, self.termios.as_ref(), self.theme));
    }

    fn toggle_tree(&mut self) {
        match (self.show_tree, self.focus == Focus::Tree) {
            (false, _) => {
                self.show_tree = true;
                self.reveal_active();
                self.set_focus(Focus::Tree);
            }
            (true, true) => {
                self.show_tree = false;
                self.set_focus(Focus::Editor);
            }
            (true, false) => self.set_focus(Focus::Tree),
        }
    }

    fn reveal_active(&mut self) {
        if let Some(p) = self.tabs.get(self.active).and_then(|t| t.doc.path.clone()) {
            self.tree.reveal(&p);
        }
    }

    fn toggle_panel(&mut self) {
        let focused = matches!(self.focus, Focus::Term(_));
        match (self.show_panel, focused) {
            (false, _) => {
                self.show_panel = true;
                if self.terms.is_empty() {
                    self.spawn_terminal();
                }
                if self.terms.is_empty() {
                    self.show_panel = false;
                    return;
                }
                let i = self.active_term.min(self.terms.len() - 1);
                self.set_focus(Focus::Term(i));
            }
            (true, true) => {
                self.show_panel = false;
                self.set_focus(Focus::Editor);
            }
            (true, false) => {
                let i = self.active_term.min(self.terms.len().saturating_sub(1));
                self.set_focus(Focus::Term(i));
            }
        }
    }

    fn toggle_claude(&mut self) {
        match (self.show_claude, self.focus == Focus::Claude) {
            (false, _) => {
                self.show_claude = true;
                if self.claude.is_none() {
                    self.spawn_claude();
                }
                self.set_focus(Focus::Claude);
            }
            (true, true) => {
                self.show_claude = false;
                self.set_focus(Focus::Editor);
            }
            (true, false) => self.set_focus(Focus::Claude),
        }
    }

    fn restart_claude(&mut self) {
        self.claude = None;
        self.show_claude = true;
        self.spawn_claude();
        self.set_focus(Focus::Claude);
    }

    fn split_terminal(&mut self) {
        self.show_panel = true;
        self.spawn_terminal();
        if !self.terms.is_empty() {
            let i = self.terms.len() - 1;
            self.set_focus(Focus::Term(i));
        }
    }

    fn kill_terminal(&mut self, i: usize) {
        if i < self.terms.len() {
            self.terms.remove(i);
            self.equalize_terms();
            if self.terms.is_empty() {
                self.show_panel = false;
                self.set_focus(Focus::Editor);
            } else {
                let j = i.min(self.terms.len() - 1);
                self.active_term = j;
                if matches!(self.focus, Focus::Term(_)) {
                    self.focus = Focus::Term(j);
                }
            }
        }
    }

    fn cycle_focus(&mut self) {
        let mut order = Vec::new();
        if self.show_tree {
            order.push(Focus::Tree);
        }
        order.push(Focus::Editor);
        if self.show_panel {
            for i in 0..self.terms.len() {
                order.push(Focus::Term(i));
            }
        }
        if self.show_claude {
            order.push(Focus::Claude);
        }
        let cur = order.iter().position(|f| *f == self.focus).unwrap_or(0);
        let next = order[(cur + 1) % order.len()];
        self.set_focus(next);
    }

    fn toggle_vim(&mut self) {
        self.vim_on = !self.vim_on;
        if let Some(t) = self.tabs.get_mut(self.active) {
            let mut ed = t.ed();
            if self.vim_on {
                self.vim.reset(&mut ed);
            } else {
                ed.v.sel_kind = SelKind::Exclusive;
            }
        }
        self.info(if self.vim_on { "Vim keys ON (Ctrl+K to switch back)".into() } else { "Vim keys OFF".into() });
    }

    fn open_palette(&mut self, commands: bool) {
        let fresh = matches!(&self.files_cache, Some((t, _)) if t.elapsed() < Duration::from_secs(10));
        if !fresh {
            self.files_cache = Some((Instant::now(), list_files(&self.root, 50_000)));
        }
        let files = self.files_cache.as_ref().map(|(_, f)| f.clone()).unwrap_or_default();
        self.palette = Some(Palette::new(files, commands));
    }

    // ------------------------------------------------------------------ commands

    pub fn exec(&mut self, cmd: Cmd) {
        use Cmd::*;
        match cmd {
            Quit => self.request_quit(),
            Save => self.save_active(),
            SaveAs => {
                let init = self
                    .tabs
                    .get(self.active)
                    .and_then(|t| t.doc.path.clone())
                    .map(|p| self.rel(&p))
                    .unwrap_or_default();
                self.prompt = Some(Prompt::new(PromptKind::SaveAs, "Save as: ", &init));
            }
            NewFile => self.new_untitled(),
            OpenFile => {
                let init = self
                    .tabs
                    .get(self.active)
                    .and_then(|t| t.doc.path.as_ref())
                    .and_then(|p| p.parent())
                    .map(|d| {
                        let r = self.rel(d);
                        if r.is_empty() { String::new() } else { format!("{r}/") }
                    })
                    .unwrap_or_default();
                self.prompt = Some(Prompt::new(PromptKind::OpenFile, "Open file: ", &init));
            }
            CloseTab => {
                if !self.tabs.is_empty() {
                    self.request_close(self.active);
                }
            }
            NextTab => {
                if !self.tabs.is_empty() {
                    self.active = (self.active + 1) % self.tabs.len();
                    self.vim_sync_mode();
                }
            }
            PrevTab => {
                if !self.tabs.is_empty() {
                    self.active = (self.active + self.tabs.len() - 1) % self.tabs.len();
                    self.vim_sync_mode();
                }
            }
            GotoTab(n) => {
                let i = (n as usize).saturating_sub(1);
                if i < self.tabs.len() {
                    self.active = if n == 9 { self.tabs.len() - 1 } else { i };
                    self.set_focus(Focus::Editor);
                    self.vim_sync_mode();
                }
            }
            QuickOpen => self.open_palette(false),
            CommandPalette => self.open_palette(true),
            Help => {
                self.help = !self.help;
                self.help_scroll = 0;
            }
            ToggleTree => self.toggle_tree(),
            ToggleTerminal => self.toggle_panel(),
            ToggleClaude => self.toggle_claude(),
            SplitTerminal | NewTerminal => self.split_terminal(),
            KillTerminal => {
                let i = self.active_term;
                self.kill_terminal(i);
            }
            RestartClaude => self.restart_claude(),
            ToggleWrap => {
                if let Some(t) = self.tabs.get_mut(self.active) {
                    t.view.wrap = !t.view.wrap;
                    t.view.scroll_sub = 0;
                    t.view.scroll_x = 0;
                    let on = t.view.wrap;
                    t.ed().ensure_visible();
                    self.info(format!("Word wrap {} (Alt+Z)", if on { "on" } else { "off" }));
                }
            }
            ReloadFile => self.reload_active(),
            EnableShiftDetection => {
                let apple = std::env::var("TERM_PROGRAM").is_ok_and(|t| t == "Apple_Terminal");
                let started = !self.mod_probe && apple && {
                    match cmx_sys::keytap::start() {
                        Ok((f, mode)) => {
                            self.keytap_fd = Some(f);
                            self.keytap_silent = mode == cmx_sys::keytap::Mode::Consume;
                            self.mod_probe = true;
                            true
                        }
                        Err(_) => false,
                    }
                };
                let probe_ok = !self.mod_probe
                    && !apple
                    && cmx_sys::modkeys::request_access()
                    && cmx_sys::modkeys::shift_held().is_some();
                if probe_ok {
                    self.mod_probe = true;
                }
                if self.mod_probe || started {
                    self.info("Ctrl+Shift detection is on (⌨ legacy+⇧)".into());
                } else if cfg!(target_os = "macos") {
                    let _ = std::process::Command::new("/usr/bin/open")
                        .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_ListenEvent")
                        .stdin(std::process::Stdio::null())
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null())
                        .spawn();
                    self.warn("Allow your terminal app under Input Monitoring, then quit and reopen it".into());
                } else {
                    self.warn(
                        "Needs an X11 session ($DISPLAY); on Wayland use a terminal with the kitty keyboard protocol"
                            .into(),
                    );
                }
            }
            StexCompile => self.run_stex("compile"),
            StexCheck => self.run_stex("check"),
            CycleFocus => self.cycle_focus(),
            FocusEditor => self.set_focus(Focus::Editor),
            RevealInTree => {
                self.show_tree = true;
                self.reveal_active();
                self.set_focus(Focus::Tree);
            }
            RefreshTree => {
                self.tree.refresh();
                self.files_cache = None;
            }
            ToggleVim => self.toggle_vim(),
            Find => self.open_find(false),
            Replace => self.open_find(true),
            GotoLine => {
                if !self.tabs.is_empty() {
                    self.prompt = Some(Prompt::new(PromptKind::GotoLine, "Go to line[:col]: ", ""));
                }
            }
            FindNext | FindPrev => self.find_step(cmd == FindNext),
            Paste => {
                if self.tabs.is_empty() {
                    return;
                }
                let text = self.clipboard.paste();
                let linewise = self.clip_linewise.as_deref() == Some(text.as_str());
                let tab = &mut self.tabs[self.active];
                let rev = tab.doc.revision;
                if tab.view.extra.is_empty() {
                    tab.ed().paste(&text, linewise);
                } else {
                    tab.ed().paste_multi(&text);
                }
                self.after_edit(rev);
            }
            _ => self.exec_editor(cmd),
        }
    }

    fn exec_editor(&mut self, cmd: Cmd) {
        use Cmd::*;
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        let rev = tab.doc.revision;
        let mut ensure = true;
        let mut copied: Option<(String, bool)> = None;
        let mut msg: Option<String> = None;
        {
            let mut ed = tab.ed();
            // commands that act at every cursor; anything else collapses to the primary cursor
            let multi_ok = matches!(
                cmd,
                Left | Right
                    | Up
                    | Down
                    | Home
                    | End
                    | WordLeft
                    | WordRight
                    | SelLeft
                    | SelRight
                    | SelUp
                    | SelDown
                    | SelHome
                    | SelEnd
                    | SelWordLeft
                    | SelWordRight
                    | Backspace
                    | Delete
                    | DeleteWordLeft
                    | DeleteWordRight
                    | DeleteToLineStart
                    | DeleteToLineEnd
                    | Newline
                    | Tab
                    | Cut
                    | Copy
                    | SelectNextOccurrence
                    | SelectAllOccurrences
                    | AddCursorAbove
                    | AddCursorBelow
                    | ScrollUp
                    | ScrollDown
                    | Escape
            );
            if !multi_ok {
                ed.collapse();
            }
            let multi = ed.multi();
            match cmd {
                Undo => {
                    if !ed.undo() {
                        msg = Some("Nothing to undo".into());
                    }
                }
                Redo => {
                    if !ed.redo() {
                        msg = Some("Nothing to redo".into());
                    }
                }
                Cut if multi => {
                    copied = Some((ed.copy_multi(), false));
                    ed.each(Coalesce::None, None, |e, _| {
                        if e.v.has_selection() {
                            e.insert_str("");
                        } else {
                            e.delete_lines();
                        }
                    });
                }
                Copy if multi => copied = Some((ed.copy_multi(), false)),
                Cut => copied = Some(ed.cut()),
                Copy => copied = Some(ed.copy_text()),
                SelectAll => ed.select_all(),
                SelectLine => ed.select_line(),
                ToggleComment => {
                    if !ed.toggle_comment() {
                        msg = Some("No line comment syntax for this file type".into());
                    }
                }
                MoveLineUp => ed.move_lines(true),
                MoveLineDown => ed.move_lines(false),
                CopyLineUp => ed.copy_lines(true),
                CopyLineDown => ed.copy_lines(false),
                DeleteLine => ed.delete_lines(),
                InsertLineBelow => ed.insert_line(true),
                InsertLineAbove => ed.insert_line(false),
                Indent => ed.indent_lines(true),
                Outdent | BackTab => ed.indent_lines(false),
                SelectNextOccurrence => {
                    let had = ed.v.has_selection();
                    if !ed.select_next_occurrence() {
                        msg =
                            Some(if had { "All occurrences are selected" } else { "No word under the cursor" }.into());
                    }
                }
                SelectAllOccurrences => {
                    let n = ed.select_all_occurrences();
                    msg = Some(if n == 0 { "No word under the cursor".into() } else { format!("{n} cursors") });
                }
                AddCursorAbove => ed.add_cursor_vertical(false),
                AddCursorBelow => ed.add_cursor_vertical(true),
                JumpBracket => ed.jump_to_bracket(),
                Left => ed.each_move(|e| e.left(false)),
                Right => ed.each_move(|e| e.right(false)),
                Up => ed.each_move(|e| e.up(false)),
                Down => ed.each_move(|e| e.down(false)),
                Home => ed.each_move(|e| e.home(false)),
                End => ed.each_move(|e| e.end(false)),
                PageUp => ed.page(false, false),
                PageDown => ed.page(true, false),
                DocStart => ed.doc_start(false),
                DocEnd => ed.doc_end(false),
                WordLeft => ed.each_move(|e| e.word_left(false)),
                WordRight => ed.each_move(|e| e.word_right(false)),
                SelLeft => ed.each_move(|e| e.left(true)),
                SelRight => ed.each_move(|e| e.right(true)),
                SelUp => ed.each_move(|e| e.up(true)),
                SelDown => ed.each_move(|e| e.down(true)),
                SelHome => ed.each_move(|e| e.home(true)),
                SelEnd => ed.each_move(|e| e.end(true)),
                SelPageUp => ed.page(false, true),
                SelPageDown => ed.page(true, true),
                SelDocStart => ed.doc_start(true),
                SelDocEnd => ed.doc_end(true),
                SelWordLeft => ed.each_move(|e| e.word_left(true)),
                SelWordRight => ed.each_move(|e| e.word_right(true)),
                Backspace => ed.each(Coalesce::Deleting, None, |e, _| e.backspace()),
                Delete => ed.each(Coalesce::Deleting, None, |e, _| e.delete()),
                DeleteWordLeft => ed.each(Coalesce::None, None, |e, _| e.delete_word_left()),
                DeleteWordRight => ed.each(Coalesce::None, None, |e, _| e.delete_word_right()),
                DeleteToLineStart => ed.each(Coalesce::None, None, |e, _| e.delete_to_line_start()),
                DeleteToLineEnd => ed.each(Coalesce::None, None, |e, _| e.delete_to_line_end()),
                Newline => ed.each(Coalesce::None, Some('\n'), |e, _| e.newline()),
                Tab if multi => ed.each(Coalesce::None, None, |e, _| e.tab()),
                Tab => ed.tab(),
                Escape => {
                    if !ed.collapse() {
                        ed.v.clear_selection();
                    }
                }
                ScrollUp => {
                    ed.scroll(-1);
                    ensure = false;
                }
                ScrollDown => {
                    ed.scroll(1);
                    ensure = false;
                }
                _ => {}
            }
        }
        if cmd == Escape {
            self.search_hl = false;
        }
        if let Some((s, linewise)) = copied {
            self.clip_linewise = if linewise { Some(s.clone()) } else { None };
            self.clipboard.copy(&s, &mut self.out);
        }
        if let Some(m) = msg {
            self.info(m);
        }
        if ensure {
            self.after_edit(rev);
        }
    }

    /// Post-edit bookkeeping: pin preview tabs, keep the cursor visible, vim cursor rules.
    fn after_edit(&mut self, rev_before: u64) {
        let vim_normal = self.vim_on && self.vim.mode != Mode::Insert;
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        if tab.doc.revision != rev_before && tab.preview {
            tab.preview = false;
        }
        let mut ed = tab.ed();
        if vim_normal && self.vim.mode == Mode::Normal {
            vim::clamp_normal(&mut ed);
        }
        ed.ensure_visible();
    }

    // ------------------------------------------------------------------ search

    fn open_find(&mut self, replace: bool) {
        let Some(tab) = self.tabs.get(self.active) else { return };
        let init = match tab.view.selection() {
            Some((a, b)) if a.line == b.line => tab.doc.text.slice(a, b),
            _ => self.search.clone().unwrap_or_default(),
        };
        let restore = Some((tab.view.cursor, tab.view.anchor, tab.view.scroll_y));
        let (kind, label) = if replace { (PromptKind::Replace, "Replace: ") } else { (PromptKind::Find, "Find: ") };
        let mut p = Prompt::new(kind, label, &init);
        p.restore = restore;
        p.fresh = !init.is_empty();
        self.prompt = Some(p);
        if !init.is_empty() {
            let n = self.count_matches(&init);
            self.search = Some(init);
            self.search_hl = true;
            self.find_label(n);
        }
    }

    fn find_step(&mut self, forward: bool) {
        let Some(q) = self.search.clone().filter(|s| !s.is_empty()) else {
            self.open_find(false);
            return;
        };
        let Some(query) = self.query(&q) else { return };
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        let found = tab.ed().find_q(&query, forward);
        self.search_hl = true;
        tab.ed().ensure_visible();
        if !found {
            self.info(format!("No results for \"{q}\""));
        }
    }

    fn count_matches(&self, q: &str) -> Option<usize> {
        let tab = self.tabs.get(self.active)?;
        let query = Query::new(q, self.search_flags).ok()?;
        let mut v = Vec::new();
        query.matches(&tab.doc.text, 0, tab.doc.text.len_lines(), &mut v);
        Some(v.len())
    }

    /// The search query under the current options (an invalid regex is reported).
    fn query(&mut self, q: &str) -> Option<Query> {
        match Query::new(q, self.search_flags) {
            Ok(x) => Some(x),
            Err(e) => {
                self.error(format!("Invalid regex: {e}"));
                None
            }
        }
    }

    /// Incremental search while typing in the Find / vim-search prompt.
    fn incremental(&mut self) {
        let Some(p) = &self.prompt else { return };
        let q = p.input.text.clone();
        let fwd = !matches!(p.kind, PromptKind::VimSearch(false));
        let restore = p.restore;
        let vim_search = matches!(p.kind, PromptKind::VimSearch(_));
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        if let Some((c, a, s)) = restore {
            tab.view.cursor = c;
            tab.view.anchor = a;
            tab.view.scroll_y = s;
        }
        if vim_search {
            self.search_flags.regex = true; // vim searches are regular expressions
        }
        if q.is_empty() {
            self.search_hl = false;
            self.find_label(None);
            return;
        }
        let Ok(query) = Query::new(&q, self.search_flags) else {
            self.find_label(None);
            if let Some(p) = &mut self.prompt
                && !vim_search
            {
                p.label = p.label.replace(": ", " (invalid regex): ");
            }
            return;
        };
        let from = tab.view.cursor;
        if let Some((a, b)) = query.find(&tab.doc.text, from, fwd) {
            if vim_search {
                tab.view.cursor = a;
                tab.view.anchor = None;
            } else {
                tab.view.anchor = Some(a);
                tab.view.cursor = b;
            }
            tab.ed().ensure_visible();
        }
        self.search = Some(q.clone());
        self.search_hl = true;
        let n = self.count_matches(&q);
        self.find_label(n);
    }

    /// "Find [3] [.* Aa]: " — match count and the active options.
    fn find_label(&mut self, n: Option<usize>) {
        let badge = self.search_flags.badge();
        if let Some(p) = &mut self.prompt {
            let base = match p.kind {
                PromptKind::Find => "Find",
                PromptKind::Replace => "Replace",
                PromptKind::VimSearch(true) => "/",
                _ => "?",
            };
            p.label = match n {
                _ if base.len() == 1 => base.to_string(),
                Some(n) => format!("{base} [{n}]{badge}: "),
                None => format!("{base}{badge}: "),
            };
        }
    }

    // ------------------------------------------------------------------ prompt handling

    fn prompt_key(&mut self, k: KeyEvent) {
        let Some(p) = self.prompt.as_mut() else { return };
        if p.is_confirm() {
            let c = match k.code {
                KeyCode::Char(c) if !k.mods.ctrl() => c.to_ascii_lowercase(),
                KeyCode::Enter => 'y',
                KeyCode::Esc => 'c',
                KeyCode::Char('c') => 'c',
                _ => return,
            };
            let kind = p.kind.clone();
            self.confirm(kind, c);
            return;
        }
        let kind = p.kind.clone();
        // Find-prompt extras
        if matches!(kind, PromptKind::Find | PromptKind::Replace) {
            match (k.code, k.mods) {
                (KeyCode::Enter, m) if m.shift() && kind == PromptKind::Find => {
                    self.find_step(false);
                    return;
                }
                (KeyCode::Down, _) | (KeyCode::F(3), Mods::NONE) => {
                    self.find_step(true);
                    return;
                }
                (KeyCode::Up, _) | (KeyCode::F(3), Mods::SHIFT) => {
                    self.find_step(false);
                    return;
                }
                (KeyCode::Char(c @ ('r' | 'c' | 'w')), m) if m == Mods::ALT => {
                    let f = &mut self.search_flags;
                    match c {
                        'r' => f.regex = !f.regex,
                        'c' => f.case = !f.case,
                        _ => f.word = !f.word,
                    }
                    self.incremental();
                    return;
                }
                (KeyCode::Tab, _) if kind == PromptKind::Find => {
                    let t = p.input.text.clone();
                    let restore = p.restore;
                    let mut np = Prompt::new(PromptKind::Replace, "Replace: ", &t);
                    np.restore = restore;
                    self.prompt = Some(np);
                    return;
                }
                _ => {}
            }
        }
        if std::mem::take(&mut p.fresh) && k.text_char().is_some() {
            p.input = crate::prompt::LineInput::with("");
        }
        match p.input.handle(&k) {
            InputResult::Submit => self.prompt_submit(),
            InputResult::Cancel => self.prompt_cancel(),
            InputResult::Edited => {
                if matches!(kind, PromptKind::Find | PromptKind::Replace | PromptKind::VimSearch(_)) {
                    self.incremental();
                }
            }
            _ => {}
        }
    }

    fn prompt_cancel(&mut self) {
        let Some(p) = self.prompt.take() else { return };
        if let (PromptKind::VimSearch(_), Some((c, a, s))) = (&p.kind, p.restore) {
            if let Some(tab) = self.tabs.get_mut(self.active) {
                tab.view.cursor = c;
                tab.view.anchor = a;
                tab.view.scroll_y = s;
            }
            self.search_hl = false;
        }
        if matches!(p.kind, PromptKind::Find | PromptKind::Replace) {
            self.search_hl = false;
        }
        if matches!(p.kind, PromptKind::SaveAs) {
            self.pending_close = None;
            self.pending_wq = false;
        }
    }

    fn prompt_submit(&mut self) {
        let Some(p) = self.prompt.take() else { return };
        let text = p.input.text.clone();
        match p.kind {
            PromptKind::SaveAs => {
                if text.trim().is_empty() {
                    self.prompt = Some(p);
                    return;
                }
                let path = self.resolve(&text);
                let Some(tab) = self.tabs.get_mut(self.active) else { return };
                match tab.doc.save_as(path) {
                    Ok(()) => {
                        tab.preview = false;
                        let name = tab.doc.display_name();
                        let pth = tab.doc.path.clone();
                        self.info(format!("Saved {name}"));
                        self.tree.refresh();
                        if let Some(pth) = pth
                            && self.show_tree
                        {
                            self.tree.reveal(&pth);
                        }
                        self.files_cache = None;
                        if std::mem::take(&mut self.pending_wq) {
                            self.quit_or_close(false);
                        } else if let Some(i) = self.pending_close.take() {
                            self.close_tab(i);
                        }
                    }
                    Err(e) => self.error(format!("save failed: {e}")),
                }
            }
            PromptKind::OpenFile => {
                if !text.trim().is_empty() {
                    let path = self.resolve(&text);
                    self.open_path(path, false, true);
                }
            }
            PromptKind::GotoLine => {
                let mut it = text.trim().split(':');
                let line = it.next().and_then(|s| s.trim().parse::<usize>().ok());
                let col = it.next().and_then(|s| s.trim().parse::<usize>().ok());
                if let (Some(l), Some(tab)) = (line, self.tabs.get_mut(self.active)) {
                    let mut ed = tab.ed();
                    ed.goto_line(l.saturating_sub(1));
                    if let Some(c) = col {
                        let ln = ed.v.cursor.line;
                        let bc = ed.doc.text.col_for_display(ln, c.saturating_sub(1), ed.doc.tab_width);
                        ed.v.cursor = Pos::new(ln, bc);
                    }
                    ed.ensure_visible();
                }
            }
            PromptKind::Find => {
                // Enter = next match, keep the prompt open
                self.prompt = Some(p);
                self.find_step(true);
            }
            PromptKind::Replace => {
                if text.is_empty() {
                    return;
                }
                self.search = Some(text.clone());
                self.prompt =
                    Some(Prompt::new(PromptKind::ReplaceWith(text.clone()), format!("Replace \"{text}\" with: "), ""));
            }
            PromptKind::ReplaceWith(find) => {
                let rep = text;
                let Some(query) = self.query(&find) else { return };
                let Some(tab) = self.tabs.get_mut(self.active) else { return };
                tab.view.anchor = None;
                if tab.ed().find_q(&query, true) {
                    tab.ed().ensure_visible();
                    self.prompt = Some(Prompt::new(
                        PromptKind::ReplaceStep { find, rep },
                        "Replace this match? (y)es (n)o (a)ll (q)uit ",
                        "",
                    ));
                } else {
                    self.info(format!("No results for \"{find}\""));
                }
            }
            PromptKind::VimCmd => self.run_ex(&text),
            PromptKind::VimSearch(fwd) => {
                if text.is_empty() {
                    if let Some((q, _)) = self.vim.last_search.clone() {
                        self.vim.last_search = Some((q, fwd));
                    }
                } else {
                    self.vim.last_search = Some((text.clone(), fwd));
                    self.search = Some(text);
                    self.search_hl = true;
                }
                if let Some(tab) = self.tabs.get_mut(self.active) {
                    tab.view.anchor = None;
                    let mut ed = tab.ed();
                    vim::clamp_normal(&mut ed);
                    ed.ensure_visible();
                }
            }
            PromptKind::NewFile(dir) => {
                if text.trim().is_empty() {
                    return;
                }
                let path = dir.join(text.trim());
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
                    Ok(_) => {
                        self.tree.refresh();
                        self.tree.reveal(&path);
                        self.files_cache = None;
                        self.open_path(path, false, true);
                    }
                    Err(e) => self.error(format!("cannot create file: {e}")),
                }
            }
            PromptKind::NewFolder(dir) => {
                if text.trim().is_empty() {
                    return;
                }
                let path = dir.join(text.trim());
                match std::fs::create_dir_all(&path) {
                    Ok(()) => {
                        self.tree.refresh();
                        self.tree.reveal(&path.join("x"));
                    }
                    Err(e) => self.error(format!("cannot create folder: {e}")),
                }
            }
            PromptKind::Rename(old) => {
                let name = text.trim();
                if name.is_empty() {
                    return;
                }
                let new = old.parent().map(|p| p.join(name)).unwrap_or_else(|| PathBuf::from(name));
                match std::fs::rename(&old, &new) {
                    Ok(()) => {
                        for t in &mut self.tabs {
                            if let Some(p) = &t.doc.path {
                                if p == &old {
                                    t.doc.path = Some(new.clone());
                                } else if let Ok(rest) = p.strip_prefix(&old) {
                                    t.doc.path = Some(new.join(rest));
                                }
                            }
                        }
                        self.tree.refresh();
                        self.tree.reveal(&new);
                        self.files_cache = None;
                    }
                    Err(e) => self.error(format!("rename failed: {e}")),
                }
            }
            PromptKind::ConfirmQuit
            | PromptKind::ConfirmClose(_)
            | PromptKind::ConfirmDelete(_)
            | PromptKind::ConfirmOverwrite(_)
            | PromptKind::ReplaceStep { .. } => {}
        }
    }

    fn confirm(&mut self, kind: PromptKind, c: char) {
        self.prompt = None;
        match kind {
            PromptKind::ConfirmQuit => {
                if c == 'y' {
                    self.quit = true;
                }
            }
            PromptKind::ConfirmOverwrite(i) => {
                if c == 'y' {
                    self.save_now(i);
                }
            }
            PromptKind::ConfirmClose(i) => match c {
                'y' => {
                    let untitled = self.tabs.get(i).is_some_and(|t| t.doc.path.is_none());
                    if untitled {
                        self.active = i;
                        self.pending_close = Some(i);
                        self.prompt = Some(Prompt::new(PromptKind::SaveAs, "Save as: ", ""));
                    } else if let Some(t) = self.tabs.get_mut(i) {
                        match t.doc.save() {
                            Ok(()) => self.close_tab(i),
                            Err(e) => self.error(format!("save failed: {e}")),
                        }
                    }
                }
                'n' => self.close_tab(i),
                _ => {}
            },
            PromptKind::ConfirmDelete(path) => {
                if c == 'y' {
                    let r = if path.is_dir() { std::fs::remove_dir_all(&path) } else { std::fs::remove_file(&path) };
                    match r {
                        Ok(()) => {
                            self.info(format!("Deleted {}", self.rel(&path)));
                            self.tree.refresh();
                            self.files_cache = None;
                        }
                        Err(e) => self.error(format!("delete failed: {e}")),
                    }
                }
            }
            PromptKind::ReplaceStep { find, rep } => {
                let Some(query) = self.query(&find) else { return };
                let Some(tab) = self.tabs.get_mut(self.active) else { return };
                match c {
                    'y' | 'n' => {
                        if c == 'y' {
                            tab.ed().replace_selection_if_match(&query, &rep);
                        }
                        if tab.ed().find_q(&query, true) {
                            tab.ed().ensure_visible();
                            self.prompt = Some(Prompt::new(
                                PromptKind::ReplaceStep { find, rep },
                                "Replace this match? (y)es (n)o (a)ll (q)uit ",
                                "",
                            ));
                        } else {
                            self.info("No more matches".into());
                        }
                    }
                    'a' => {
                        let n = tab.ed().replace_all(&query, &rep);
                        tab.ed().ensure_visible();
                        self.info(format!("Replaced {n} occurrence{}", if n == 1 { "" } else { "s" }));
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    /// Vim `:` commands.
    fn run_ex(&mut self, cmdline: &str) {
        let c = cmdline.trim();
        let (name, arg) = match c.find(' ') {
            Some(i) => (&c[..i], c[i + 1..].trim()),
            None => (c, ""),
        };
        if let Ok(n) = c.parse::<usize>() {
            if let Some(tab) = self.tabs.get_mut(self.active) {
                let mut ed = tab.ed();
                ed.goto_line(n.saturating_sub(1));
                vim::clamp_normal(&mut ed);
            }
            return;
        }
        if c == "$" {
            if let Some(tab) = self.tabs.get_mut(self.active) {
                let last = tab.doc.text.len_lines() - 1;
                tab.ed().goto_line(last);
            }
            return;
        }
        if c.starts_with("s/") || c.starts_with("%s/") {
            self.substitute(c);
            return;
        }
        match name {
            "w" | "write" => {
                if !arg.is_empty() {
                    let path = self.resolve(arg);
                    if let Some(tab) = self.tabs.get_mut(self.active) {
                        match tab.doc.save_as(path) {
                            Ok(()) => {
                                let n = tab.doc.display_name();
                                self.info(format!("\"{n}\" written"));
                                self.tree.refresh();
                            }
                            Err(e) => self.error(format!("write failed: {e}")),
                        }
                    }
                } else {
                    self.save_active();
                }
            }
            "q" | "quit" | "clo" | "close" => self.quit_or_close(false),
            "q!" | "quit!" => self.quit_or_close(true),
            "qa" | "qall" | "qa!" | "qall!" | "quitall" => {
                if name.ends_with('!') {
                    self.quit = true;
                } else {
                    self.request_quit();
                }
            }
            "wq" | "x" | "xit" | "exit" => {
                let untitled = self.tabs.get(self.active).is_some_and(|t| t.doc.path.is_none());
                if untitled && arg.is_empty() {
                    self.pending_wq = true;
                    self.save_active();
                    return;
                }
                if !arg.is_empty() {
                    self.run_ex(&format!("w {arg}"));
                } else {
                    self.save_active();
                }
                if self.tabs.get(self.active).is_some_and(|t| !t.doc.is_dirty()) {
                    self.quit_or_close(false);
                }
            }
            "wa" | "wall" => {
                let n = self.save_all();
                self.info(format!("{n} file(s) written"));
            }
            "wqa" | "xa" => {
                self.save_all();
                self.request_quit();
            }
            "e" | "edit" | "tabe" | "tabedit" | "tabnew" => {
                if arg.is_empty() {
                    if name.starts_with("tab") {
                        self.new_untitled();
                    }
                    return;
                }
                let p = self.resolve(arg);
                self.open_path(p, false, true);
            }
            "e!" | "edit!" => {
                if let Some(tab) = self.tabs.get_mut(self.active)
                    && let Some(p) = tab.doc.path.clone()
                    && let Ok(d) = Document::open(&p)
                {
                    tab.doc = d;
                    tab.view = Default::default();
                }
            }
            "noh" | "nohl" | "nohlsearch" => self.search_hl = false,
            "tree" | "explorer" | "Ex" | "Explore" | "Lex" | "NERDTree" => self.toggle_tree(),
            "term" | "terminal" => self.toggle_panel(),
            "claude" => self.toggle_claude(),
            "vsplit-term" | "split-term" | "vterm" => self.split_terminal(),
            "h" | "help" => self.help = true,
            "bn" | "bnext" | "tabn" | "tabnext" => self.exec(Cmd::NextTab),
            "bp" | "bprev" | "bprevious" | "tabp" | "tabprev" | "tabprevious" => self.exec(Cmd::PrevTab),
            "bd" | "bdelete" | "tabc" | "tabclose" => self.exec(Cmd::CloseTab),
            "redo" | "red" => self.exec(Cmd::Redo),
            "undo" | "u" => self.exec(Cmd::Undo),
            "novim" | "vim" => self.toggle_vim(),
            "set" | "se" => self.set_option(arg),
            "" => {}
            _ => self.error(format!("Not an editor command: {c}")),
        }
    }

    fn set_option(&mut self, arg: &str) {
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        let d = &mut tab.doc;
        if let Some(v) = arg
            .strip_prefix("ts=")
            .or_else(|| arg.strip_prefix("tabstop="))
            .or_else(|| arg.strip_prefix("sw="))
            .or_else(|| arg.strip_prefix("shiftwidth="))
        {
            if let Ok(n) = v.parse::<usize>() {
                d.tab_width = n.clamp(1, 16);
            }
        } else if arg == "et" || arg == "expandtab" {
            d.use_tabs = false;
        } else if arg == "noet" || arg == "noexpandtab" {
            d.use_tabs = true;
        } else {
            self.error(format!("Unknown option: {arg}"));
        }
    }

    fn quit_or_close(&mut self, force: bool) {
        if self.tabs.len() <= 1 {
            if force {
                self.quit = true;
            } else if self.tabs.first().is_some_and(|t| t.doc.is_dirty()) {
                self.error("No write since last change (add ! to override)".into());
            } else {
                self.quit = true;
            }
        } else if force {
            let i = self.active;
            self.close_tab(i);
        } else if self.tabs[self.active].doc.is_dirty() {
            self.error("No write since last change (add ! to override)".into());
        } else {
            let i = self.active;
            self.close_tab(i);
        }
    }

    /// `:s/a/b/[g]` on the cursor line or `:%s/a/b/[g]` on all lines (literal match).
    fn substitute(&mut self, c: &str) {
        let whole = c.starts_with('%');
        let body = if whole { &c[2..] } else { &c[1..] };
        let delim = body.chars().next().unwrap_or('/');
        let parts: Vec<&str> = body[delim.len_utf8()..].splitn(3, delim).collect();
        if parts.len() < 2 || parts[0].is_empty() {
            self.error("usage: :s/find/replace/[g]".into());
            return;
        }
        let (find, rep) = (parts[0].to_string(), parts[1].to_string());
        let flags = parts.get(2).copied().unwrap_or("");
        let global = flags.contains('g');
        // vim flags: i = ignore case, I = match case (default: smart case)
        let query = match Query::new(&find, Flags { regex: true, case: flags.contains('I'), word: false }) {
            Ok(q) if flags.contains('i') => {
                match Query::new(&format!("(?i){find}"), Flags { regex: true, ..Default::default() }) {
                    Ok(q) => q,
                    Err(_) => q,
                }
            }
            Ok(q) => q,
            Err(e) => {
                self.error(format!("Invalid regex: {e}"));
                return;
            }
        };
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        let (l0, l1) =
            if whole { (0, tab.doc.text.len_lines() - 1) } else { (tab.view.cursor.line, tab.view.cursor.line) };
        let n = tab.ed().replace_in(&query, &rep, l0, l1 + 1, !global, true);
        if n == 0 {
            self.error(format!("Pattern not found: {find}"));
            return;
        }
        let mut ed = tab.ed();
        vim::clamp_normal(&mut ed);
        self.info(format!("{n} substitution{}", if n == 1 { "" } else { "s" }));
    }

    // ------------------------------------------------------------------ keyboard

    fn on_key(&mut self, k: KeyEvent) {
        if k.mods.ctrl() && !k.ambiguous && matches!(k.code, KeyCode::Char(_)) {
            self.seen_enhanced = true;
        }
        // Terminal.app & co. send Ctrl+Shift+Z as Ctrl+Z: ask the OS whether Shift is held
        let raw = k;
        let mut k = k;
        let mut probed = None;
        if self.keytap_fd.is_some() && k.ambiguous && k.mods == Mods::CTRL {
            // with the tap, Ctrl+Shift chords arrive separately: a control byte is plain Ctrl
            k.ambiguous = false;
        } else if self.keytap_fd.is_none()
            && self.mod_probe
            && k.ambiguous
            && k.mods == Mods::CTRL
            && matches!(k.code, KeyCode::Char(c) if c.is_ascii_alphabetic())
            && let Some(shift) = cmx_sys::modkeys::shift_held()
        {
            probed = Some(shift);
            if shift {
                k.mods = Mods::CTRL_SHIFT;
            }
            k.ambiguous = false;
        }
        // CMX_KEYLOG=/path: one line per key (diagnosing keyboards and terminals)
        if let Ok(path) = std::env::var("CMX_KEYLOG") {
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
                let cmd = self.keymap.lookup(&k, true);
                let _ = writeln!(
                    f,
                    "{:?} mods={:?} ambiguous={} shift_probe={:?} -> {:?} mods={:?} cmd={:?} kbd={:?} tap={} silent={}",
                    raw.code,
                    raw.mods,
                    raw.ambiguous,
                    probed,
                    k.code,
                    k.mods,
                    cmd,
                    self.kbd,
                    self.keytap_fd.is_some(),
                    self.keytap_silent
                );
            }
        }
        if self.prompt.is_some() {
            return self.prompt_key(k);
        }
        if self.palette.is_some() {
            return self.palette_key(k);
        }
        if self.help {
            let h = self.hits.help_box.map(|r| r.h as usize).unwrap_or(10);
            match k.code {
                KeyCode::Up | KeyCode::Char('k') => self.help_scroll = self.help_scroll.saturating_sub(1),
                KeyCode::Down | KeyCode::Char('j') => self.help_scroll += 1,
                KeyCode::PageUp => self.help_scroll = self.help_scroll.saturating_sub(h),
                KeyCode::PageDown | KeyCode::Char(' ') => self.help_scroll += h,
                _ => self.help = false,
            }
            return;
        }
        match self.focus {
            Focus::Term(_) | Focus::Claude => self.term_key(k),
            Focus::Tree => self.tree_key(k),
            Focus::Editor => self.editor_key(k),
        }
    }

    fn editor_key(&mut self, k: KeyEvent) {
        if self.tabs.is_empty() {
            if let Some(cmd) = self.keymap.lookup(&k, true) {
                self.exec(cmd);
            }
            return;
        }
        if self.vim_on {
            let tab = &mut self.tabs[self.active];
            tab.view.extra.clear(); // multiple cursors are a default-mode feature
            let rev = tab.doc.revision;
            let before = self.vim.last_search.clone();
            let out = {
                let mut ed = tab.ed();
                self.vim.handle(&mut ed, &k)
            };
            if self.vim.last_search != before
                && let Some((q, _)) = &self.vim.last_search
            {
                self.search = Some(q.clone());
                self.search_hl = true;
                self.search_flags.regex = true;
            }
            match out {
                VimOut::Done => {
                    self.after_edit(rev);
                    return;
                }
                VimOut::Pass => {
                    if self.vim.mode != Mode::Insert {
                        if let Some(cmd) = self.keymap.lookup(&k, true) {
                            self.exec(cmd);
                        }
                        return;
                    }
                }
                VimOut::Cmdline => {
                    self.prompt = Some(Prompt::new(PromptKind::VimCmd, ":", ""));
                    return;
                }
                VimOut::Search(fwd) => {
                    let tab = &self.tabs[self.active];
                    let mut p = Prompt::new(PromptKind::VimSearch(fwd), if fwd { "/" } else { "?" }, "");
                    p.restore = Some((tab.view.cursor, tab.view.anchor, tab.view.scroll_y));
                    self.prompt = Some(p);
                    return;
                }
                VimOut::Yank(s) => {
                    let n = s.lines().count().max(1);
                    self.clip_linewise = if self.vim.reg_linewise { Some(s.clone()) } else { None };
                    self.clipboard.copy(&s, &mut self.out);
                    if n > 1 {
                        self.info(format!("{n} lines yanked"));
                    }
                    self.after_edit(rev);
                    return;
                }
                VimOut::Replay(keys) => {
                    self.vim.replaying = true;
                    for key in keys {
                        self.editor_key(key);
                    }
                    self.vim.replaying = false;
                    self.after_edit(rev);
                    return;
                }
                VimOut::Message(m) => {
                    self.info(m);
                    self.after_edit(rev);
                    return;
                }
            }
        }
        if let Some(cmd) = self.keymap.lookup(&k, true) {
            self.exec(cmd);
            return;
        }
        if let Some(c) = k.text_char() {
            let tab = &mut self.tabs[self.active];
            let rev = tab.doc.revision;
            tab.ed().each(Coalesce::Typing, Some(c), |e, _| e.type_char(c, true));
            self.after_edit(rev);
        }
    }

    fn tree_height(&self) -> usize {
        self.layout.tree.map(|r| r.h.saturating_sub(1) as usize).unwrap_or(20)
    }

    fn preview_selected(&mut self) {
        if let Some(n) = self.tree.selected_node()
            && !n.is_dir
        {
            let p = n.path.clone();
            self.open_path(p, true, false);
        }
    }

    fn tree_key(&mut self, k: KeyEvent) {
        let h = self.tree_height();
        let plain = k.mods.is_empty();
        match k.code {
            KeyCode::Up if plain => {
                self.tree.move_sel(-1);
                self.preview_selected();
            }
            KeyCode::Down if plain => {
                self.tree.move_sel(1);
                self.preview_selected();
            }
            KeyCode::Char('k') if plain => {
                self.tree.move_sel(-1);
                self.preview_selected();
            }
            KeyCode::Char('j') if plain => {
                self.tree.move_sel(1);
                self.preview_selected();
            }
            KeyCode::PageUp if plain => {
                self.tree.move_sel(-(h as isize));
                self.preview_selected();
            }
            KeyCode::PageDown if plain => {
                self.tree.move_sel(h as isize);
                self.preview_selected();
            }
            KeyCode::Home if plain => {
                self.tree.selected = 0;
                self.preview_selected();
            }
            KeyCode::End if plain => {
                self.tree.selected = self.tree.nodes.len().saturating_sub(1);
                self.preview_selected();
            }
            KeyCode::Enter => {
                if let Some(n) = self.tree.selected_node().cloned() {
                    if n.is_dir {
                        let i = self.tree.selected;
                        self.tree.toggle(i);
                    } else {
                        self.open_path(n.path, false, true);
                    }
                }
            }
            KeyCode::Char(' ') if plain => {
                if let Some(n) = self.tree.selected_node().cloned() {
                    if n.is_dir {
                        let i = self.tree.selected;
                        self.tree.toggle(i);
                    } else {
                        self.preview_selected();
                    }
                }
            }
            KeyCode::Right | KeyCode::Char('l') if plain || k.code == KeyCode::Right => {
                if let Some(n) = self.tree.selected_node().cloned() {
                    if n.is_dir {
                        if n.expanded {
                            self.tree.move_sel(1);
                        } else {
                            let i = self.tree.selected;
                            self.tree.expand(i);
                        }
                    } else {
                        self.preview_selected();
                    }
                }
            }
            KeyCode::Left | KeyCode::Char('h') if plain || k.code == KeyCode::Left => self.tree.collapse_or_parent(),
            KeyCode::Char('a') | KeyCode::Char('n') if plain => {
                let d = self.tree.target_dir();
                let label = format!("New file in {}/: ", self.rel(&d));
                self.prompt = Some(Prompt::new(PromptKind::NewFile(d), label, ""));
            }
            KeyCode::Char('A') | KeyCode::Char('N') if plain => {
                let d = self.tree.target_dir();
                let label = format!("New folder in {}/: ", self.rel(&d));
                self.prompt = Some(Prompt::new(PromptKind::NewFolder(d), label, ""));
            }
            KeyCode::Char('r') | KeyCode::F(2) if plain => {
                if let Some(n) = self.tree.selected_node().cloned() {
                    self.prompt = Some(Prompt::new(PromptKind::Rename(n.path.clone()), "Rename to: ", &n.name));
                }
            }
            KeyCode::Char('d') | KeyCode::Delete if plain => {
                if let Some(n) = self.tree.selected_node().cloned() {
                    let label = format!(
                        "Delete {}{}? (y/n) ",
                        self.rel(&n.path),
                        if n.is_dir { "/ and its contents" } else { "" }
                    );
                    self.prompt = Some(Prompt::new(PromptKind::ConfirmDelete(n.path.clone()), label, ""));
                }
            }
            KeyCode::Char('R') | KeyCode::F(5) if plain => {
                self.tree.refresh();
                self.files_cache = None;
            }
            KeyCode::Char('.') if plain => {
                self.tree.show_hidden = !self.tree.show_hidden;
                self.tree.refresh();
            }
            KeyCode::Esc => self.set_focus(Focus::Editor),
            _ => {
                if let Some(cmd) = self.keymap.lookup(&k, true) {
                    self.exec(cmd);
                }
            }
        }
        self.tree.ensure_visible(h);
    }

    fn term_key(&mut self, k: KeyEvent) {
        // Chords that must reach cmx even while a shell has focus (only unambiguous ones, D-5).
        let strong = k.mods.contains(Mods::CTRL_SHIFT) && !k.ambiguous || matches!(k.code, KeyCode::F(_));
        if strong
            && let Some(cmd) = self.keymap.lookup(&k, false)
            && matches!(
                cmd,
                Cmd::ToggleTree
                    | Cmd::ToggleTerminal
                    | Cmd::ToggleClaude
                    | Cmd::CycleFocus
                    | Cmd::CommandPalette
                    | Cmd::Help
                    | Cmd::SplitTerminal
                    | Cmd::NewTerminal
                    | Cmd::RevealInTree
                    | Cmd::QuickOpen
                    | Cmd::Quit
            )
        {
            self.exec(cmd);
            return;
        }
        let focus = self.focus;
        let is_claude = focus == Focus::Claude;
        let restart = is_claude
            && k.code == KeyCode::Enter
            && self.claude.as_ref().is_some_and(|c| c.exited.is_some() || c.error.is_some());
        if restart {
            self.restart_claude();
            return;
        }
        let Some(pane) = self.pane_for(focus) else {
            self.set_focus(Focus::Editor);
            return;
        };
        if !pane.is_alive() {
            // nothing to type into: behave like the rest of the UI
            if let Some(cmd) = self.keymap.lookup(&k, true) {
                self.exec(cmd);
            } else if k.code == KeyCode::Esc {
                self.set_focus(Focus::Editor);
            }
            return;
        }
        let m = k.mods;
        if k.code == KeyCode::Char('c') && (m == Mods::CTRL_SHIFT || (m == Mods::CTRL && pane.sel.is_some())) {
            if let Some(t) = pane.selected_text() {
                pane.sel = None;
                self.clipboard.copy(&t, &mut self.out);
                self.info("Copied".into());
                return;
            }
            if m != Mods::CTRL {
                return;
            }
        }
        if k.code == KeyCode::Char('v') && (m == Mods::CTRL || m == Mods::CTRL_SHIFT) {
            let t = self.clipboard.paste();
            if let Some(pane) = self.pane_for(focus) {
                pane.paste(&t);
            }
            return;
        }
        let pane = self.pane_for(focus).unwrap();
        match (k.code, m) {
            (KeyCode::PageUp, Mods::SHIFT) => {
                let h = pane.vt.rows() as isize / 2;
                pane.scroll_by(h);
            }
            (KeyCode::PageDown, Mods::SHIFT) => {
                let h = pane.vt.rows() as isize / 2;
                pane.scroll_by(-h);
            }
            (KeyCode::Home, Mods::SHIFT) if !pane.vt.is_alt() => pane.scroll = pane.vt.history_len(),
            (KeyCode::End, Mods::SHIFT) if !pane.vt.is_alt() => pane.scroll = 0,
            _ => pane.send_key(&k),
        }
    }

    fn palette_key(&mut self, k: KeyEvent) {
        let Some(p) = self.palette.as_mut() else { return };
        let h = self.hits.palette_rows.len().max(1);
        match k.code {
            KeyCode::Up => p.move_sel(-1, h),
            KeyCode::Down => p.move_sel(1, h),
            KeyCode::PageUp => p.move_sel(-(h as isize), h),
            KeyCode::PageDown => p.move_sel(h as isize, h),
            KeyCode::Char('p') | KeyCode::Char('k') if k.mods.ctrl() => p.move_sel(-1, h),
            KeyCode::Char('n') | KeyCode::Char('j') if k.mods.ctrl() => p.move_sel(1, h),
            _ => match p.input.handle(&k) {
                InputResult::Submit => self.palette_pick(),
                InputResult::Cancel => self.palette = None,
                InputResult::Edited => p.update(),
                _ => {}
            },
        }
    }

    fn palette_pick(&mut self) {
        let Some(p) = self.palette.take() else { return };
        match p.picked() {
            Some(Picked::File(f)) => {
                let path = self.root.join(f);
                self.open_path(path, false, true);
            }
            Some(Picked::Command(c)) => self.exec(c),
            Some(Picked::GotoLine(n)) => {
                if let Some(tab) = self.tabs.get_mut(self.active) {
                    tab.ed().goto_line(n.saturating_sub(1));
                }
                self.set_focus(Focus::Editor);
            }
            None => {}
        }
    }

    fn on_paste(&mut self, s: String) {
        if let Some(p) = self.prompt.as_mut() {
            p.input.insert(&s);
            let kind = p.kind.clone();
            if matches!(kind, PromptKind::Find | PromptKind::Replace | PromptKind::VimSearch(_)) {
                self.incremental();
            }
            return;
        }
        if let Some(p) = self.palette.as_mut() {
            p.input.insert(&s);
            p.update();
            return;
        }
        match self.focus {
            Focus::Term(_) | Focus::Claude => {
                let f = self.focus;
                if let Some(p) = self.pane_for(f) {
                    p.paste(&s);
                }
            }
            _ => {
                if self.tabs.is_empty() {
                    self.new_untitled();
                }
                self.set_focus(Focus::Editor);
                let tab = &mut self.tabs[self.active];
                let rev = tab.doc.revision;
                tab.ed().insert_str(&s);
                self.after_edit(rev);
            }
        }
    }

    // ------------------------------------------------------------------ mouse

    fn click_count(&mut self, x: u16, y: u16) -> u8 {
        let now = Instant::now();
        let c = match self.last_click {
            Some((t, lx, ly, c)) if now.duration_since(t) < Duration::from_millis(450) && lx == x && ly == y => {
                c % 3 + 1
            }
            _ => 1,
        };
        self.last_click = Some((now, x, y, c));
        c
    }

    /// The splitter at a cell, if any (the panel header doubles as the panel's resize handle).
    fn split_at(&self, x: u16, y: u16) -> Option<Drag> {
        let l = &self.layout;
        if l.tree_split.is_some_and(|r| r.contains(x, y)) {
            return Some(Drag::TreeSplit);
        }
        if l.claude_split.is_some_and(|r| r.contains(x, y)) {
            return Some(Drag::ClaudeSplit);
        }
        if let Some(i) = l.term_splits.iter().position(|r| r.contains(x, y)) {
            return Some(Drag::TermSplit(i));
        }
        if l.panel_header.is_some_and(|r| r.contains(x, y))
            && !self.hits.term_tabs.iter().any(|(r, _)| r.contains(x, y))
            && !self.hits.buttons.iter().any(|(r, _)| r.contains(x, y))
        {
            return Some(Drag::PanelSplit);
        }
        None
    }

    /// Hover feedback for splitters: highlight + a resize pointer (OSC 22) where supported.
    fn update_hover(&mut self, x: u16, y: u16) {
        let over = if self.palette.is_some() || self.help { None } else { self.split_at(x, y) };
        let active = self
            .drag
            .filter(|d| matches!(d, Drag::TreeSplit | Drag::ClaudeSplit | Drag::PanelSplit | Drag::TermSplit(_)));
        let split = active.or(over);
        if split != self.hover_split || (split.is_some() && self.hover_at != (x, y)) {
            self.hover_split = split;
            self.hover_at = (x, y);
            self.dirty = true;
        }
        let shape = match split {
            Some(Drag::PanelSplit) => Some("ns-resize"),
            Some(_) => Some("ew-resize"),
            None => None,
        };
        if shape != self.pointer {
            let s = shape.unwrap_or("text");
            self.out.extend_from_slice(&term::pointer(s));
            self.pointer = shape;
        }
    }

    pub fn split_highlight(&self) -> Option<Drag> {
        self.hover_split
    }

    fn on_mouse(&mut self, m: MouseEvent) {
        if let Some(d) = self.drag {
            match m.kind {
                MouseKind::Drag(_) | MouseKind::Moved => {
                    self.drag_to(d, m);
                    return;
                }
                MouseKind::Up(_) => {
                    self.drag_end(d, m);
                    self.drag = None;
                    self.update_hover(m.x, m.y);
                    return;
                }
                _ => self.drag = None,
            }
        }
        self.update_hover(m.x, m.y);
        if m.kind == MouseKind::Moved && self.split_at(m.x, m.y).is_some() {
            return;
        }
        let (x, y) = (m.x, m.y);
        if self.help {
            match m.kind {
                MouseKind::ScrollUp => self.help_scroll = self.help_scroll.saturating_sub(3),
                MouseKind::ScrollDown => self.help_scroll += 3,
                MouseKind::Down(_) => self.help = false,
                _ => {}
            }
            return;
        }
        if self.palette.is_some() {
            match m.kind {
                MouseKind::ScrollUp => {
                    let h = self.hits.palette_rows.len().max(1);
                    self.palette.as_mut().unwrap().move_sel(-1, h);
                }
                MouseKind::ScrollDown => {
                    let h = self.hits.palette_rows.len().max(1);
                    self.palette.as_mut().unwrap().move_sel(1, h);
                }
                MouseKind::Down(MouseButton::Left) => {
                    if let Some(&(_, i)) = self.hits.palette_rows.iter().find(|(r, _)| r.contains(x, y)) {
                        self.palette.as_mut().unwrap().selected = i;
                        self.palette_pick();
                    } else if !self.hits.palette_box.is_some_and(|b| b.contains(x, y)) {
                        self.palette = None;
                    }
                }
                _ => {}
            }
            return;
        }
        let is_down = matches!(m.kind, MouseKind::Down(_));
        // a click anywhere dismisses a non-confirm prompt
        if is_down && self.prompt.as_ref().is_some_and(|p| !p.is_confirm()) && !self.layout.status.contains(x, y) {
            self.prompt = None;
        }
        // buttons
        if let MouseKind::Down(MouseButton::Left) = m.kind
            && let Some(&(_, b)) = self.hits.buttons.iter().find(|(r, _)| r.contains(x, y))
        {
            match b {
                PanelBtn::New => self.split_terminal(),
                PanelBtn::Kill => {
                    let i = self.active_term;
                    self.kill_terminal(i);
                }
                PanelBtn::Hide => {
                    self.show_panel = false;
                    if matches!(self.focus, Focus::Term(_)) {
                        self.set_focus(Focus::Editor);
                    }
                }
                PanelBtn::HideClaude => {
                    self.show_claude = false;
                    if self.focus == Focus::Claude {
                        self.set_focus(Focus::Editor);
                    }
                }
                PanelBtn::RestartClaude => self.restart_claude(),
                PanelBtn::HideTree => {
                    self.show_tree = false;
                    if self.focus == Focus::Tree {
                        self.set_focus(Focus::Editor);
                    }
                }
                PanelBtn::RefreshTree => self.tree.refresh(),
            }
            return;
        }
        let l = self.layout.clone();
        // splitters
        if let MouseKind::Down(MouseButton::Left) = m.kind
            && let Some(d) = self.split_at(x, y).filter(|d| *d != Drag::PanelSplit)
        {
            self.drag = Some(d);
            self.drag_origin = (x, y);
            self.update_hover(x, y);
            return;
        }
        if l.status.contains(x, y) {
            if is_down && self.hits.status_mode.contains(x, y) {
                self.toggle_vim();
            }
            return;
        }
        if let Some(r) = l.tree
            && r.contains(x, y)
        {
            return self.mouse_tree(m, r);
        }
        if let Some(r) = l.claude
            && r.contains(x, y)
        {
            if y == r.y {
                if is_down {
                    self.set_focus(Focus::Claude);
                }
                return;
            }
            return self.mouse_pane(m, None, l.claude_body.unwrap_or(r));
        }
        if let Some(ph) = l.panel_header
            && ph.contains(x, y)
        {
            if let MouseKind::Down(MouseButton::Left) = m.kind {
                if let Some(&(_, i)) = self.hits.term_tabs.iter().find(|(r, _)| r.contains(x, y)) {
                    self.set_focus(Focus::Term(i));
                } else {
                    self.drag = Some(Drag::PanelSplit);
                    self.drag_origin = (x, y);
                    self.update_hover(x, y);
                }
            }
            return;
        }
        for (i, r) in l.terms.iter().enumerate() {
            if r.contains(x, y) {
                return self.mouse_pane(m, Some(i), *r);
            }
        }
        if l.tabbar.contains(x, y) {
            return self.mouse_tabbar(m);
        }
        if l.text.contains(x, y) {
            self.mouse_text(m);
        }
    }

    fn mouse_tree(&mut self, m: MouseEvent, r: Rect) {
        match m.kind {
            MouseKind::ScrollUp => self.tree.scroll = self.tree.scroll.saturating_sub(3),
            MouseKind::ScrollDown => {
                let max = self.tree.nodes.len().saturating_sub(r.h.saturating_sub(1) as usize);
                self.tree.scroll = (self.tree.scroll + 3).min(max);
            }
            MouseKind::Down(MouseButton::Left) => {
                self.set_focus(Focus::Tree);
                if m.y == r.y {
                    return;
                }
                let idx = self.tree.scroll + (m.y - r.y - 1) as usize;
                if idx >= self.tree.nodes.len() {
                    return;
                }
                let count = self.click_count(m.x, m.y);
                self.tree.selected = idx;
                let n = self.tree.nodes[idx].clone();
                if n.is_dir {
                    self.tree.toggle(idx);
                } else if count >= 2 {
                    self.open_path(n.path, false, true);
                } else {
                    self.open_path(n.path, true, false);
                }
            }
            _ => {}
        }
    }

    fn mouse_tabbar(&mut self, m: MouseEvent) {
        let (x, y) = (m.x, m.y);
        match m.kind {
            MouseKind::Down(MouseButton::Left) => {
                if let Some(&(_, i)) = self.hits.tab_close.iter().find(|(r, _)| r.contains(x, y)) {
                    self.request_close(i);
                    return;
                }
                if let Some(&(_, i)) = self.hits.tabs.iter().find(|(r, _)| r.contains(x, y)) {
                    let count = self.click_count(x, y);
                    self.active = i;
                    if count >= 2 {
                        self.tabs[i].preview = false;
                    }
                    self.set_focus(Focus::Editor);
                    self.vim_sync_mode();
                }
            }
            MouseKind::Down(MouseButton::Middle) => {
                if let Some(&(_, i)) = self.hits.tabs.iter().find(|(r, _)| r.contains(x, y)) {
                    self.request_close(i);
                }
            }
            MouseKind::ScrollUp => self.exec(Cmd::PrevTab),
            MouseKind::ScrollDown => self.exec(Cmd::NextTab),
            _ => {}
        }
    }

    fn text_pos(&mut self, x: u16, y: u16) -> Option<Pos> {
        let r = self.layout.text;
        let g = self.hits.gutter_w;
        let tab = self.tabs.get_mut(self.active)?;
        let row = y.saturating_sub(r.y).min(r.h.saturating_sub(1)) as usize;
        let col = x.saturating_sub(r.x + g) as usize;
        Some(tab.ed().pos_at(row, col))
    }

    fn mouse_text(&mut self, m: MouseEvent) {
        if self.tabs.is_empty() {
            if matches!(m.kind, MouseKind::Down(_)) {
                self.set_focus(Focus::Editor);
            }
            return;
        }
        let r = self.layout.text;
        let (x, y) = (m.x, m.y);
        match m.kind {
            MouseKind::ScrollUp | MouseKind::ScrollDown => {
                let d: isize = if m.kind == MouseKind::ScrollUp { -3 } else { 3 };
                let tab = &mut self.tabs[self.active];
                if m.mods.shift() {
                    tab.view.scroll_x = (tab.view.scroll_x as isize + d * 2).max(0) as usize;
                } else {
                    tab.ed().scroll(d);
                }
            }
            MouseKind::ScrollLeft | MouseKind::ScrollRight => {
                let d: isize = if m.kind == MouseKind::ScrollLeft { -4 } else { 4 };
                let tab = &mut self.tabs[self.active];
                tab.view.scroll_x = (tab.view.scroll_x as isize + d).max(0) as usize;
            }
            MouseKind::Down(MouseButton::Left) => {
                self.set_focus(Focus::Editor);
                if let Some(sb) = self.hits.scrollbar
                    && sb.contains(x, y)
                {
                    self.scrollbar_jump(y);
                    self.drag = Some(Drag::Scrollbar);
                    return;
                }
                let count = self.click_count(x, y);
                let Some(pos) = self.text_pos(x, y) else { return };
                let in_gutter = x < r.x + self.hits.gutter_w;
                let extend = m.mods.ctrl() || m.mods.shift() || m.mods.sup();
                let vim_on = self.vim_on;
                if m.mods.alt() && !vim_on && !in_gutter {
                    // Alt+click adds (or removes) a cursor
                    self.tabs[self.active].ed().toggle_cursor_at(pos);
                    return;
                }
                if !m.mods.alt() {
                    self.tabs[self.active].view.extra.clear();
                }
                if vim_on && self.vim.mode != Mode::Insert && !extend {
                    let tab = &mut self.tabs[self.active];
                    let ed = tab.ed();
                    ed.v.sel_kind = SelKind::Exclusive;
                    ed.v.anchor = None;
                    self.vim.mode = Mode::Normal;
                }
                let tab = &mut self.tabs[self.active];
                let mut ed = tab.ed();
                if in_gutter {
                    ed.select_lines_at(pos.line);
                } else if extend {
                    ed.move_to(pos, true);
                } else {
                    match count {
                        1 => ed.move_to(pos, false),
                        2 => ed.select_word_at(pos),
                        _ => ed.select_lines_at(pos.line),
                    }
                }
                if vim_on && self.vim.mode == Mode::Normal && ed.v.has_selection() {
                    // a mouse selection in vim mode is a visual selection
                    self.vim.mode = Mode::Visual;
                    ed.v.sel_kind = SelKind::Inclusive;
                    ed.v.cursor = ed.doc.text.prev_pos(ed.v.cursor).max(ed.v.anchor.unwrap_or_default());
                } else if vim_on && self.vim.mode == Mode::Normal {
                    vim::clamp_normal(&mut ed);
                }
                self.drag = Some(Drag::EditorSelect);
            }
            _ => {}
        }
    }

    fn scrollbar_jump(&mut self, y: u16) {
        let r = self.layout.text;
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        let total = tab.doc.text.len_lines();
        let h = r.h.max(1) as usize;
        let row = y.saturating_sub(r.y) as usize;
        let target = row * total / h;
        tab.view.scroll_y = target.saturating_sub(h / 2).min(total.saturating_sub(1));
    }

    fn mouse_pane(&mut self, m: MouseEvent, idx: Option<usize>, r: Rect) {
        let focus = match idx {
            Some(i) => Focus::Term(i),
            None => Focus::Claude,
        };
        if matches!(m.kind, MouseKind::Down(_)) {
            self.set_focus(focus);
        }
        let (rx, ry) = (m.x.saturating_sub(r.x), m.y.saturating_sub(r.y));
        let Some(pane) = self.pane_for(focus) else { return };
        if pane.vt.wants_mouse() && !m.mods.shift() && pane.scroll == 0 {
            pane.send_mouse(&m, rx, ry);
            return;
        }
        match m.kind {
            MouseKind::ScrollUp | MouseKind::ScrollDown => {
                let up = m.kind == MouseKind::ScrollUp;
                if pane.vt.is_alt() {
                    if pane.vt.modes.alt_scroll {
                        let k = KeyEvent::plain(if up { KeyCode::Up } else { KeyCode::Down });
                        for _ in 0..3 {
                            pane.send_key(&k);
                        }
                    }
                } else {
                    pane.scroll_by(if up { 3 } else { -3 });
                }
            }
            MouseKind::Down(MouseButton::Left) => {
                let abs = pane.abs_line(ry as usize);
                let count = {
                    let now = Instant::now();
                    match self.last_click {
                        Some((t, lx, ly, c))
                            if now.duration_since(t) < Duration::from_millis(450) && lx == m.x && ly == m.y =>
                        {
                            c % 3 + 1
                        }
                        _ => 1,
                    }
                };
                self.last_click = Some((Instant::now(), m.x, m.y, count));
                let pane = self.pane_for(focus).unwrap();
                match count {
                    2 => {
                        // select word
                        let row = pane.vt.line(abs);
                        let is_word = |c: &cmx_tui::Cell| {
                            let s = c.sym.as_str();
                            !c.sym.is_blank() && s.chars().all(|ch| ch.is_alphanumeric() || "_-./~:".contains(ch))
                        };
                        let x = rx as usize;
                        if x < row.cells.len() && is_word(&row.cells[x]) {
                            let mut a = x;
                            while a > 0 && is_word(&row.cells[a - 1]) {
                                a -= 1;
                            }
                            let mut b = x;
                            while b + 1 < row.cells.len() && is_word(&row.cells[b + 1]) {
                                b += 1;
                            }
                            pane.sel = Some(((abs, a), (abs, b)));
                        }
                    }
                    3 => {
                        let w = pane.vt.cols().saturating_sub(1);
                        pane.sel = Some(((abs, 0), (abs, w)));
                    }
                    _ => {
                        pane.sel = Some(((abs, rx as usize), (abs, rx as usize)));
                        self.drag = Some(Drag::TermSelect(idx));
                    }
                }
            }
            _ => {}
        }
    }

    fn drag_to(&mut self, d: Drag, m: MouseEvent) {
        let (x, y) = (m.x, m.y);
        match d {
            Drag::TreeSplit => {
                self.tree_w = x.max(layout::MIN_TREE).min(self.width.saturating_sub(layout::MIN_CENTER + 1));
            }
            Drag::ClaudeSplit => {
                let w = self.width.saturating_sub(x + 1);
                self.claude_w = w.max(layout::MIN_CLAUDE);
            }
            Drag::PanelSplit => {
                let main_h = self.height.saturating_sub(1);
                self.panel_h = main_h.saturating_sub(y).clamp(3, main_h.saturating_sub(4).max(3));
            }
            Drag::TermSplit(i) => {
                let Some(p) = self.layout.panel else { return };
                let n = self.term_ratios.len();
                if i + 1 >= n {
                    return;
                }
                let usable = p.w.saturating_sub(n as u16 - 1).max(1) as f32;
                let start: f32 = self.term_ratios[..i].iter().sum::<f32>();
                let pair = self.term_ratios[i] + self.term_ratios[i + 1];
                let pos = (x.saturating_sub(p.x).saturating_sub(i as u16)) as f32 / usable;
                let min = 4.0 / usable;
                let left = (pos - start).clamp(min, pair - min);
                self.term_ratios[i] = left;
                self.term_ratios[i + 1] = pair - left;
            }
            Drag::Scrollbar => self.scrollbar_jump(y),
            Drag::EditorSelect => {
                let r = self.layout.text;
                if let Some(tab) = self.tabs.get_mut(self.active) {
                    if y < r.y {
                        tab.ed().scroll(-1);
                    } else if y >= r.bottom() {
                        tab.ed().scroll(1);
                    }
                }
                let Some(pos) = self.text_pos(x, y) else { return };
                let vim_on = self.vim_on;
                let tab = &mut self.tabs[self.active];
                let mut ed = tab.ed();
                if vim_on && self.vim.mode == Mode::Normal {
                    self.vim.mode = Mode::Visual;
                    ed.v.sel_kind = SelKind::Inclusive;
                    ed.v.anchor = Some(ed.v.cursor);
                }
                if vim_on && self.vim.mode != Mode::Insert {
                    ed.v.cursor = pos;
                } else {
                    ed.move_to(pos, true);
                }
            }
            Drag::TermSelect(idx) => {
                let r = match idx {
                    Some(i) => self.layout.terms.get(i).copied(),
                    None => self.layout.claude_body,
                };
                let Some(r) = r else { return };
                let focus = match idx {
                    Some(i) => Focus::Term(i),
                    None => Focus::Claude,
                };
                let Some(pane) = self.pane_for(focus) else { return };
                if y < r.y {
                    pane.scroll_by(1);
                } else if y >= r.bottom() {
                    pane.scroll_by(-1);
                }
                let ry = y.saturating_sub(r.y).min(r.h.saturating_sub(1)) as usize;
                let rx = x.saturating_sub(r.x).min(r.w.saturating_sub(1)) as usize;
                let abs = pane.abs_line(ry);
                if let Some((a, _)) = pane.sel {
                    pane.sel = Some((a, (abs, rx)));
                }
            }
        }
        self.dirty = true;
    }

    fn drag_end(&mut self, d: Drag, _m: MouseEvent) {
        if let Drag::TermSelect(idx) = d {
            let focus = match idx {
                Some(i) => Focus::Term(i),
                None => Focus::Claude,
            };
            if let Some(p) = self.pane_for(focus)
                && let Some((a, b)) = p.sel
                && a == b
            {
                p.sel = None;
            }
        }
        if d == Drag::EditorSelect
            && self.vim_on
            && self.vim.mode == Mode::Visual
            && let Some(tab) = self.tabs.get_mut(self.active)
            && tab.view.anchor == Some(tab.view.cursor)
        {
            tab.view.anchor = None;
            tab.view.sel_kind = SelKind::Exclusive;
            self.vim.mode = Mode::Normal;
        }
    }
}
