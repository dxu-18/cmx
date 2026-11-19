//! cmx-vt: a VT100/xterm-compatible terminal emulator (DEC ANSI parser + screen model).
//!
//! Enough of xterm for shells, TUIs (vim, htop, less) and Claude Code: cursor movement, erase,
//! insert/delete, scroll regions, alt screen, 16/256/truecolor SGR, DEC line drawing, mouse
//! reporting modes, bracketed paste, synchronized output, OSC title/colors/clipboard, and
//! DA/DSR/DECRQM/XTVERSION replies (queued in [`Term::responses`] for the caller to send).

mod encode;

use cmx_tui::unicode::char_width;
use cmx_tui::{Cell, Color, Style, Sym, attr};
use std::collections::VecDeque;

#[derive(Clone, Debug)]
pub struct Row {
    pub cells: Vec<Cell>,
    /// The line continues on the next row (soft wrap). Used when copying text.
    pub wrapped: bool,
}

impl Row {
    fn blank(cols: usize, style: Style) -> Row {
        Row { cells: vec![Cell::styled(style); cols], wrapped: false }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MouseMode {
    Off,
    X10,
    Normal,
    Button,
    Any,
}

#[derive(Clone, Debug)]
pub struct Modes {
    pub app_cursor: bool,
    pub app_keypad: bool,
    pub autowrap: bool,
    pub origin: bool,
    pub insert: bool,
    pub show_cursor: bool,
    pub bracketed_paste: bool,
    pub mouse: MouseMode,
    pub mouse_sgr: bool,
    pub focus_events: bool,
    pub lnm: bool,
    pub sync: bool,
    pub alt_scroll: bool,
    /// DECSCUSR value requested by the application (0 = default).
    pub cursor_shape: u8,
    pub reverse_video: bool,
}

impl Default for Modes {
    fn default() -> Self {
        Modes {
            app_cursor: false,
            app_keypad: false,
            autowrap: true,
            origin: false,
            insert: false,
            show_cursor: true,
            bracketed_paste: false,
            mouse: MouseMode::Off,
            mouse_sgr: false,
            focus_events: false,
            lnm: false,
            sync: false,
            alt_scroll: true,
            cursor_shape: 0,
            reverse_video: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Cursor {
    x: usize,
    y: usize,
    pen: Style,
    wrap_pending: bool,
    origin: bool,
    g0_line: bool,
    g1_line: bool,
    shift_out: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum State {
    Ground,
    Esc,
    EscInter,
    Csi,
    CsiIgnore,
    Osc,
    OscEsc,
    Str,
    StrEsc,
}

const MAX_PARAMS: usize = 32;

pub struct Term {
    cols: usize,
    rows: usize,
    screens: [Vec<Row>; 2],
    alt: bool,
    pub scrollback: VecDeque<Row>,
    pub max_scrollback: usize,
    cur: Cursor,
    saved: [Cursor; 2],
    top: usize,
    bottom: usize,
    tabs: Vec<bool>,
    pub modes: Modes,
    state: State,
    params: [u32; MAX_PARAMS],
    colon: [bool; MAX_PARAMS],
    nparams: usize,
    cur_param: Option<u32>,
    next_colon: bool,
    private: u8,
    inter: u8,
    osc: Vec<u8>,
    osc_bel: bool,
    utf8: [u8; 4],
    utf8_len: usize,
    utf8_need: usize,
    last_char: Option<char>,
    pub title: String,
    pub cwd: Option<String>,
    /// Bytes the emulator wants written back to the application (DA/DSR replies, ...).
    pub responses: Vec<u8>,
    /// Text the application asked to put on the clipboard (OSC 52).
    pub clipboard: Option<String>,
    pub bell: bool,
    /// Set whenever the screen changes; the owner clears it after drawing.
    pub dirty: bool,
    /// Lines pushed into scrollback since the owner last looked (to keep a scrolled view stable).
    pub scrolled_lines: usize,
    pub default_fg: (u8, u8, u8),
    pub default_bg: (u8, u8, u8),
}

fn dec_line(c: char) -> char {
    match c {
        '`' => '◆',
        'a' => '▒',
        'f' => '°',
        'g' => '±',
        'j' => '┘',
        'k' => '┐',
        'l' => '┌',
        'm' => '└',
        'n' => '┼',
        'o' => '⎺',
        'p' => '⎻',
        'q' => '─',
        'r' => '⎼',
        's' => '⎽',
        't' => '├',
        'u' => '┤',
        'v' => '┴',
        'w' => '┬',
        'x' => '│',
        'y' => '≤',
        'z' => '≥',
        '{' => 'π',
        '|' => '≠',
        '}' => '£',
        '~' => '·',
        _ => c,
    }
}

impl Term {
    pub fn new(cols: u16, rows: u16) -> Term {
        let cols = (cols as usize).max(1);
        let rows = (rows as usize).max(1);
        let mut t = Term {
            cols,
            rows,
            screens: [vec![Row::blank(cols, Style::DEFAULT); rows], vec![Row::blank(cols, Style::DEFAULT); rows]],
            alt: false,
            scrollback: VecDeque::new(),
            max_scrollback: 5000,
            cur: Cursor::default(),
            saved: [Cursor::default(); 2],
            top: 0,
            bottom: rows - 1,
            tabs: Vec::new(),
            modes: Modes::default(),
            state: State::Ground,
            params: [0; MAX_PARAMS],
            colon: [false; MAX_PARAMS],
            nparams: 0,
            cur_param: None,
            next_colon: false,
            private: 0,
            inter: 0,
            osc: Vec::new(),
            osc_bel: true,
            utf8: [0; 4],
            utf8_len: 0,
            utf8_need: 0,
            last_char: None,
            title: String::new(),
            cwd: None,
            responses: Vec::new(),
            clipboard: None,
            bell: false,
            dirty: true,
            scrolled_lines: 0,
            default_fg: (0xcc, 0xcc, 0xcc),
            default_bg: (0x1e, 0x1e, 0x1e),
        };
        t.reset_tabs();
        t
    }

    // ------------------------------------------------------------------ accessors

    pub fn cols(&self) -> usize {
        self.cols
    }
    pub fn rows(&self) -> usize {
        self.rows
    }
    pub fn is_alt(&self) -> bool {
        self.alt
    }
    /// Cursor (x, y) on the live screen.
    pub fn cursor(&self) -> (usize, usize) {
        (self.cur.x.min(self.cols - 1), self.cur.y.min(self.rows - 1))
    }
    pub fn grid(&self) -> &[Row] {
        &self.screens[self.alt as usize]
    }
    /// Lines available above the live screen (0 in the alt screen).
    pub fn history_len(&self) -> usize {
        if self.alt { 0 } else { self.scrollback.len() }
    }
    /// Row `y` of the view scrolled back by `offset` lines.
    pub fn view_row(&self, offset: usize, y: usize) -> &Row {
        let h = self.history_len();
        let off = offset.min(h);
        self.line(h - off + y)
    }
    /// Absolute line index: 0..history_len() is scrollback, then the live grid.
    pub fn line(&self, abs: usize) -> &Row {
        let h = self.history_len();
        if abs < h { &self.scrollback[abs] } else { &self.grid()[(abs - h).min(self.rows - 1)] }
    }
    pub fn total_lines(&self) -> usize {
        self.history_len() + self.rows
    }

    /// Text between two absolute (line, col) positions, inclusive of `a`, exclusive of `b`.
    pub fn text_range(&self, a: (usize, usize), b: (usize, usize)) -> String {
        let (a, b) = if a <= b { (a, b) } else { (b, a) };
        let mut out = String::new();
        let last = b.0.min(self.total_lines().saturating_sub(1));
        for l in a.0..=last {
            let row = self.line(l);
            let from = if l == a.0 { a.1 } else { 0 };
            let to = if l == b.0 { b.1.min(row.cells.len()) } else { row.cells.len() };
            let mut s = String::new();
            for c in row.cells.iter().take(to).skip(from) {
                if !c.sym.is_cont() {
                    s.push_str(c.sym.as_str());
                }
            }
            if (l != b.0 || to >= row.cells.len()) && !row.wrapped {
                let t = s.trim_end_matches(' ');
                s.truncate(t.len());
            }
            out.push_str(&s);
            if l != last && !row.wrapped {
                out.push('\n');
            }
        }
        out
    }

    // ------------------------------------------------------------------ helpers

    fn g(&mut self) -> &mut Vec<Row> {
        &mut self.screens[self.alt as usize]
    }

    fn erase_style(&self) -> Style {
        Style { fg: Color::Reset, bg: self.cur.pen.bg, attrs: 0 }
    }

    fn erase_cell(&self) -> Cell {
        Cell::styled(self.erase_style())
    }

    fn reset_tabs(&mut self) {
        self.tabs = (0..self.cols).map(|i| i % 8 == 0 && i > 0).collect();
    }

    fn param(&self, i: usize, default: u32) -> u32 {
        if i < self.nparams && self.params[i] != 0 { self.params[i] } else { default }
    }

    fn param_n(&self, i: usize) -> usize {
        self.param(i, 1).max(1) as usize
    }

    fn clear_params(&mut self) {
        self.nparams = 0;
        self.cur_param = None;
        self.next_colon = false;
        self.private = 0;
        self.inter = 0;
    }

    fn push_param(&mut self) {
        if self.nparams < MAX_PARAMS {
            self.params[self.nparams] = self.cur_param.unwrap_or(0);
            self.colon[self.nparams] = self.next_colon;
            self.nparams += 1;
        }
        self.cur_param = None;
    }

    // ------------------------------------------------------------------ input

    /// Feed bytes produced by the application.
    pub fn feed(&mut self, data: &[u8]) {
        if data.is_empty() {
            return;
        }
        self.dirty = true;
        let mut i = 0;
        let n = data.len();
        while i < n {
            let b = data[i];
            match self.state {
                State::Ground => {
                    if self.utf8_need > 0 {
                        if b & 0xC0 == 0x80 {
                            self.utf8[self.utf8_len] = b;
                            self.utf8_len += 1;
                            if self.utf8_len == self.utf8_need {
                                let c = std::str::from_utf8(&self.utf8[..self.utf8_len])
                                    .ok()
                                    .and_then(|s| s.chars().next())
                                    .unwrap_or('\u{FFFD}');
                                self.utf8_need = 0;
                                self.utf8_len = 0;
                                self.print(c);
                            }
                            i += 1;
                        } else {
                            self.utf8_need = 0;
                            self.utf8_len = 0;
                            self.print('\u{FFFD}');
                            // reprocess b
                        }
                        continue;
                    }
                    if (0x20..0x7F).contains(&b) {
                        let start = i;
                        while i < n && (0x20..0x7F).contains(&data[i]) {
                            i += 1;
                        }
                        for &c in &data[start..i] {
                            self.print_ascii(c);
                        }
                        continue;
                    }
                    if b < 0x20 {
                        self.execute(b);
                    } else if b == 0x7F {
                        // DEL: ignored
                    } else if (0xC2..=0xDF).contains(&b) {
                        self.utf8[0] = b;
                        self.utf8_len = 1;
                        self.utf8_need = 2;
                    } else if (0xE0..=0xEF).contains(&b) {
                        self.utf8[0] = b;
                        self.utf8_len = 1;
                        self.utf8_need = 3;
                    } else if (0xF0..=0xF4).contains(&b) {
                        self.utf8[0] = b;
                        self.utf8_len = 1;
                        self.utf8_need = 4;
                    } else {
                        self.print('\u{FFFD}');
                    }
                }
                State::Esc => match b {
                    0x1B => self.state = State::Esc,
                    0x18 | 0x1A => self.state = State::Ground,
                    0x00..=0x1F => self.execute(b),
                    b'[' => {
                        self.clear_params();
                        self.state = State::Csi;
                    }
                    b']' => {
                        self.osc.clear();
                        self.state = State::Osc;
                    }
                    b'P' | b'X' | b'^' | b'_' => self.state = State::Str,
                    0x20..=0x2F => {
                        self.inter = b;
                        self.state = State::EscInter;
                    }
                    _ => {
                        self.state = State::Ground;
                        self.esc_dispatch(0, b);
                    }
                },
                State::EscInter => match b {
                    0x1B => self.state = State::Esc,
                    0x18 | 0x1A => self.state = State::Ground,
                    0x00..=0x1F => self.execute(b),
                    0x20..=0x2F => self.inter = b,
                    _ => {
                        self.state = State::Ground;
                        let it = self.inter;
                        self.esc_dispatch(it, b);
                    }
                },
                State::Csi => match b {
                    b'0'..=b'9' => {
                        let v = self.cur_param.unwrap_or(0);
                        self.cur_param = Some((v * 10 + (b - b'0') as u32).min(65535));
                    }
                    b';' => {
                        self.push_param();
                        self.next_colon = false;
                    }
                    b':' => {
                        self.push_param();
                        self.next_colon = true;
                    }
                    b'<' | b'=' | b'>' | b'?' => {
                        if self.nparams == 0 && self.cur_param.is_none() && self.private == 0 {
                            self.private = b;
                        } else {
                            self.state = State::CsiIgnore;
                        }
                    }
                    0x20..=0x2F => self.inter = b,
                    0x40..=0x7E => {
                        if self.cur_param.is_some() || self.nparams > 0 {
                            self.push_param();
                        }
                        self.state = State::Ground;
                        self.csi_dispatch(b);
                    }
                    0x1B => self.state = State::Esc,
                    0x18 | 0x1A => self.state = State::Ground,
                    0x00..=0x1F => self.execute(b),
                    _ => self.state = State::CsiIgnore,
                },
                State::CsiIgnore => match b {
                    0x40..=0x7E => self.state = State::Ground,
                    0x1B => self.state = State::Esc,
                    0x18 | 0x1A => self.state = State::Ground,
                    _ => {}
                },
                State::Osc => match b {
                    0x07 => {
                        self.osc_bel = true;
                        self.state = State::Ground;
                        self.osc_dispatch();
                    }
                    0x1B => self.state = State::OscEsc,
                    0x18 | 0x1A => self.state = State::Ground,
                    _ => {
                        if self.osc.len() < 4 << 20 {
                            self.osc.push(b);
                        }
                    }
                },
                State::OscEsc => {
                    self.osc_bel = false;
                    self.state = State::Ground;
                    self.osc_dispatch();
                    if b != b'\\' {
                        self.state = State::Esc;
                        continue; // reprocess as ESC <b>
                    }
                }
                State::Str => match b {
                    0x1B => self.state = State::StrEsc,
                    0x18 | 0x1A => self.state = State::Ground,
                    _ => {}
                },
                State::StrEsc => {
                    self.state = if b == b'\\' { State::Ground } else { State::Str };
                }
            }
            i += 1;
        }
    }

    fn execute(&mut self, b: u8) {
        match b {
            0x07 => self.bell = true,
            0x08 => {
                self.cur.wrap_pending = false;
                if self.cur.x > 0 {
                    self.cur.x -= 1;
                }
            }
            0x09 => self.tab_forward(1),
            0x0A..=0x0C => {
                self.linefeed();
                if self.modes.lnm {
                    self.cur.x = 0;
                }
            }
            0x0D => {
                self.cur.x = 0;
                self.cur.wrap_pending = false;
            }
            0x0E => self.cur.shift_out = true,
            0x0F => self.cur.shift_out = false,
            0x1B => self.state = State::Esc,
            _ => {}
        }
    }

    // ------------------------------------------------------------------ printing

    #[inline]
    fn print_ascii(&mut self, b: u8) {
        let x = self.cur.x;
        let line_cs = if self.cur.shift_out { self.cur.g1_line } else { self.cur.g0_line };
        if !self.cur.wrap_pending && !self.modes.insert && !line_cs && x + 1 < self.cols {
            let y = self.cur.y;
            let pen = self.cur.pen;
            let row = &mut self.screens[self.alt as usize][y];
            let c = &row.cells[x];
            if c.sym.width() == 1 {
                row.cells[x] = Cell { sym: Sym::from_char(b as char), style: pen };
                self.cur.x = x + 1;
                self.last_char = Some(b as char);
                return;
            }
        }
        self.print(b as char);
    }

    fn print(&mut self, c: char) {
        let line_cs = if self.cur.shift_out { self.cur.g1_line } else { self.cur.g0_line };
        let c = if line_cs { dec_line(c) } else { c };
        let w = char_width(c);
        if w == 0 {
            // combining mark: attach to the previous cell
            let y = self.cur.y;
            let mut x = if self.cur.wrap_pending {
                self.cur.x
            } else if self.cur.x > 0 {
                self.cur.x - 1
            } else {
                return;
            };
            let row = &mut self.g()[y];
            if row.cells[x].sym.is_cont() && x > 0 {
                x -= 1;
            }
            row.cells[x].sym.push_combining(c);
            return;
        }
        if self.cur.wrap_pending {
            if self.modes.autowrap {
                let y = self.cur.y;
                self.g()[y].wrapped = true;
                self.cur.x = 0;
                self.linefeed();
            }
            self.cur.wrap_pending = false;
        }
        if w == 2 && self.cur.x + 1 >= self.cols {
            if self.modes.autowrap && self.cols >= 2 {
                let (x, y) = (self.cur.x, self.cur.y);
                let e = self.erase_cell();
                let row = &mut self.g()[y];
                row.cells[x] = e;
                row.wrapped = true;
                self.cur.x = 0;
                self.linefeed();
            } else if self.cols >= 2 {
                self.cur.x = self.cols - 2;
            } else {
                return;
            }
        }
        let (x, y) = (self.cur.x, self.cur.y);
        if self.modes.insert {
            self.insert_blanks(w);
        }
        let pen = self.cur.pen;
        self.put(x, y, Sym::from_char(c), pen);
        self.last_char = Some(c);
        let nx = x + w;
        if nx >= self.cols {
            self.cur.x = self.cols - 1;
            self.cur.wrap_pending = self.modes.autowrap;
        } else {
            self.cur.x = nx;
        }
    }

    fn put(&mut self, x: usize, y: usize, sym: Sym, style: Style) {
        let cols = self.cols;
        let row = &mut self.screens[self.alt as usize][y];
        let cells = &mut row.cells;
        if cells[x].sym.is_cont() && x > 0 {
            cells[x - 1].sym = Sym::BLANK;
        }
        if cells[x].sym.width() == 2 && x + 1 < cols {
            cells[x + 1].sym = Sym::BLANK;
        }
        if sym.width() == 2 && x + 1 < cols {
            if cells[x + 1].sym.width() == 2 && x + 2 < cols {
                cells[x + 2].sym = Sym::BLANK;
            }
            cells[x] = Cell { sym, style };
            cells[x + 1] = Cell { sym: Sym::CONT, style };
        } else {
            cells[x] = Cell { sym, style };
        }
    }

    // ------------------------------------------------------------------ scrolling

    fn push_scrollback(&mut self, mut row: Row) {
        // trim trailing default blanks to save memory
        while let Some(c) = row.cells.last() {
            if c.sym.is_blank() && c.style == Style::DEFAULT {
                row.cells.pop();
            } else {
                break;
            }
        }
        row.cells.shrink_to_fit();
        if self.scrollback.len() >= self.max_scrollback {
            self.scrollback.pop_front();
        }
        self.scrollback.push_back(row);
        self.scrolled_lines += 1;
    }

    fn scroll_up(&mut self, n: usize) {
        let (top, bottom) = (self.top, self.bottom);
        let n = n.min(bottom - top + 1);
        let blank = Row::blank(self.cols, self.erase_style());
        let to_history = !self.alt && top == 0;
        for _ in 0..n {
            let r = self.g().remove(top);
            self.g().insert(bottom, blank.clone());
            if to_history {
                self.push_scrollback(r);
            }
        }
    }

    fn scroll_down(&mut self, n: usize) {
        let (top, bottom) = (self.top, self.bottom);
        let n = n.min(bottom - top + 1);
        let blank = Row::blank(self.cols, self.erase_style());
        for _ in 0..n {
            self.g().remove(bottom);
            self.g().insert(top, blank.clone());
        }
    }

    fn linefeed(&mut self) {
        self.cur.wrap_pending = false;
        if self.cur.y == self.bottom {
            self.scroll_up(1);
        } else if self.cur.y + 1 < self.rows {
            self.cur.y += 1;
        }
    }

    fn reverse_index(&mut self) {
        self.cur.wrap_pending = false;
        if self.cur.y == self.top {
            self.scroll_down(1);
        } else if self.cur.y > 0 {
            self.cur.y -= 1;
        }
    }

    // ------------------------------------------------------------------ editing ops

    fn erase_cells(&mut self, y: usize, from: usize, to: usize) {
        let e = self.erase_cell();
        let cols = self.cols;
        let row = &mut self.g()[y];
        let to = to.min(cols);
        if from < to {
            // fix wide chars cut in half at the edges
            if from > 0 && row.cells[from].sym.is_cont() {
                row.cells[from - 1].sym = Sym::BLANK;
            }
            if to < cols && row.cells[to].sym.is_cont() {
                row.cells[to].sym = Sym::BLANK;
            }
            row.cells[from..to].iter_mut().for_each(|c| *c = e);
        }
    }

    fn insert_blanks(&mut self, n: usize) {
        let (x, y, cols) = (self.cur.x, self.cur.y, self.cols);
        let e = self.erase_cell();
        let row = &mut self.g()[y];
        let n = n.min(cols - x);
        for _ in 0..n {
            row.cells.insert(x, e);
        }
        row.cells.truncate(cols);
        if let Some(last) = row.cells.last_mut()
            && last.sym.width() == 2
        {
            last.sym = Sym::BLANK;
        }
    }

    fn delete_chars(&mut self, n: usize) {
        let (x, y, cols) = (self.cur.x, self.cur.y, self.cols);
        let e = self.erase_cell();
        let row = &mut self.g()[y];
        let n = n.min(cols - x);
        row.cells.drain(x..x + n);
        row.cells.resize(cols, e);
        if row.cells[x].sym.is_cont() {
            row.cells[x].sym = Sym::BLANK;
        }
    }

    fn insert_lines(&mut self, n: usize) {
        let y = self.cur.y;
        if y < self.top || y > self.bottom {
            return;
        }
        let n = n.min(self.bottom - y + 1);
        let blank = Row::blank(self.cols, self.erase_style());
        let bottom = self.bottom;
        for _ in 0..n {
            self.g().remove(bottom);
            self.g().insert(y, blank.clone());
        }
        self.cur.x = 0;
        self.cur.wrap_pending = false;
    }

    fn delete_lines(&mut self, n: usize) {
        let y = self.cur.y;
        if y < self.top || y > self.bottom {
            return;
        }
        let n = n.min(self.bottom - y + 1);
        let blank = Row::blank(self.cols, self.erase_style());
        let bottom = self.bottom;
        for _ in 0..n {
            self.g().remove(y);
            self.g().insert(bottom, blank.clone());
        }
        self.cur.x = 0;
        self.cur.wrap_pending = false;
    }

    fn tab_forward(&mut self, n: usize) {
        for _ in 0..n {
            let mut x = self.cur.x + 1;
            while x < self.cols && !self.tabs.get(x).copied().unwrap_or(false) {
                x += 1;
            }
            self.cur.x = x.min(self.cols - 1);
        }
        self.cur.wrap_pending = false;
    }

    fn tab_backward(&mut self, n: usize) {
        for _ in 0..n {
            let mut x = self.cur.x;
            while x > 0 {
                x -= 1;
                if self.tabs.get(x).copied().unwrap_or(false) {
                    break;
                }
            }
            self.cur.x = x;
        }
        self.cur.wrap_pending = false;
    }

    fn goto(&mut self, x: usize, y: usize) {
        let (ymin, ymax) = if self.cur.origin { (self.top, self.bottom) } else { (0, self.rows - 1) };
        self.cur.x = x.min(self.cols - 1);
        self.cur.y = (ymin + y).min(ymax);
        self.cur.wrap_pending = false;
    }

    fn save_cursor(&mut self) {
        self.saved[self.alt as usize] = self.cur;
    }

    fn restore_cursor(&mut self) {
        let mut c = self.saved[self.alt as usize];
        c.x = c.x.min(self.cols - 1);
        c.y = c.y.min(self.rows - 1);
        self.cur = c;
    }

    fn set_alt(&mut self, on: bool, clear: bool) {
        if on == self.alt {
            if on && clear {
                let blank = Row::blank(self.cols, self.erase_style());
                self.screens[1].iter_mut().for_each(|r| *r = blank.clone());
            }
            return;
        }
        self.alt = on;
        if on && clear {
            let blank = Row::blank(self.cols, Style::DEFAULT);
            self.screens[1].iter_mut().for_each(|r| *r = blank.clone());
        }
        self.dirty = true;
    }

    fn full_reset(&mut self) {
        let (c, r) = (self.cols as u16, self.rows as u16);
        let sb = std::mem::take(&mut self.scrollback);
        let fg = self.default_fg;
        let bg = self.default_bg;
        *self = Term::new(c, r);
        self.scrollback = sb;
        self.default_fg = fg;
        self.default_bg = bg;
    }

    // ------------------------------------------------------------------ dispatch

    fn esc_dispatch(&mut self, inter: u8, b: u8) {
        match (inter, b) {
            (0, b'7') => self.save_cursor(),
            (0, b'8') => self.restore_cursor(),
            (0, b'D') => self.linefeed(),
            (0, b'E') => {
                self.cur.x = 0;
                self.linefeed();
            }
            (0, b'H') => {
                if self.cur.x < self.tabs.len() {
                    self.tabs[self.cur.x] = true;
                }
            }
            (0, b'M') => self.reverse_index(),
            (0, b'c') => self.full_reset(),
            (0, b'=') => self.modes.app_keypad = true,
            (0, b'>') => self.modes.app_keypad = false,
            (0, b'Z') => self.responses.extend_from_slice(b"\x1b[?62;22c"),
            (b'(', c) => self.cur.g0_line = c == b'0',
            (b')', c) => self.cur.g1_line = c == b'0',
            (b'#', b'8') => {
                let st = Style::DEFAULT;
                for y in 0..self.rows {
                    for x in 0..self.cols {
                        self.g()[y].cells[x] = Cell { sym: Sym::from_char('E'), style: st };
                    }
                }
            }
            _ => {}
        }
    }

    fn csi_dispatch(&mut self, f: u8) {
        let private = self.private;
        let inter = self.inter;
        match (private, inter, f) {
            (0, 0, b'@') => {
                let n = self.param_n(0);
                self.insert_blanks(n);
            }
            (0, 0, b'A') => {
                let n = self.param_n(0);
                let min = if self.cur.y >= self.top { self.top } else { 0 };
                self.cur.y = self.cur.y.saturating_sub(n).max(min);
                self.cur.wrap_pending = false;
            }
            (0, 0, b'B') | (0, 0, b'e') => {
                let n = self.param_n(0);
                let max = if self.cur.y <= self.bottom { self.bottom } else { self.rows - 1 };
                self.cur.y = (self.cur.y + n).min(max);
                self.cur.wrap_pending = false;
            }
            (0, 0, b'C') | (0, 0, b'a') => {
                let n = self.param_n(0);
                self.cur.x = (self.cur.x + n).min(self.cols - 1);
                self.cur.wrap_pending = false;
            }
            (0, 0, b'D') => {
                let n = self.param_n(0);
                self.cur.x = self.cur.x.saturating_sub(n);
                self.cur.wrap_pending = false;
            }
            (0, 0, b'E') => {
                let n = self.param_n(0);
                let max = if self.cur.y <= self.bottom { self.bottom } else { self.rows - 1 };
                self.cur.y = (self.cur.y + n).min(max);
                self.cur.x = 0;
                self.cur.wrap_pending = false;
            }
            (0, 0, b'F') => {
                let n = self.param_n(0);
                let min = if self.cur.y >= self.top { self.top } else { 0 };
                self.cur.y = self.cur.y.saturating_sub(n).max(min);
                self.cur.x = 0;
                self.cur.wrap_pending = false;
            }
            (0, 0, b'G') | (0, 0, b'`') => {
                self.cur.x = (self.param_n(0) - 1).min(self.cols - 1);
                self.cur.wrap_pending = false;
            }
            (0, 0, b'H') | (0, 0, b'f') => {
                let (r, c) = (self.param_n(0) - 1, self.param_n(1) - 1);
                self.goto(c, r);
            }
            (0, 0, b'I') => {
                let n = self.param_n(0);
                self.tab_forward(n);
            }
            (0 | b'?', 0, b'J') => {
                let (x, y) = (self.cur.x, self.cur.y);
                match self.param(0, 0) {
                    0 => {
                        self.erase_cells(y, x, self.cols);
                        for r in y + 1..self.rows {
                            self.erase_cells(r, 0, self.cols);
                            self.g()[r].wrapped = false;
                        }
                    }
                    1 => {
                        for r in 0..y {
                            self.erase_cells(r, 0, self.cols);
                        }
                        self.erase_cells(y, 0, x + 1);
                    }
                    2 => {
                        for r in 0..self.rows {
                            self.erase_cells(r, 0, self.cols);
                            self.g()[r].wrapped = false;
                        }
                    }
                    3 => self.scrollback.clear(),
                    _ => {}
                }
                self.cur.wrap_pending = false;
            }
            (0 | b'?', 0, b'K') => {
                let (x, y) = (self.cur.x, self.cur.y);
                match self.param(0, 0) {
                    0 => {
                        self.erase_cells(y, x, self.cols);
                        self.g()[y].wrapped = false;
                    }
                    1 => self.erase_cells(y, 0, x + 1),
                    2 => {
                        self.erase_cells(y, 0, self.cols);
                        self.g()[y].wrapped = false;
                    }
                    _ => {}
                }
                self.cur.wrap_pending = false;
            }
            (0, 0, b'L') => {
                let n = self.param_n(0);
                self.insert_lines(n);
            }
            (0, 0, b'M') => {
                let n = self.param_n(0);
                self.delete_lines(n);
            }
            (0, 0, b'P') => {
                let n = self.param_n(0);
                self.delete_chars(n);
            }
            (0, 0, b'S') => {
                let n = self.param_n(0);
                self.scroll_up(n);
            }
            (0, 0, b'T') if self.nparams <= 1 => {
                let n = self.param_n(0);
                self.scroll_down(n);
            }
            (0, 0, b'X') => {
                let n = self.param_n(0);
                let (x, y) = (self.cur.x, self.cur.y);
                self.erase_cells(y, x, x + n);
                self.cur.wrap_pending = false;
            }
            (0, 0, b'Z') => {
                let n = self.param_n(0);
                self.tab_backward(n);
            }
            (0, 0, b'b') => {
                if let Some(c) = self.last_char {
                    for _ in 0..self.param_n(0).min(4096) {
                        self.print(c);
                    }
                }
            }
            (0, 0, b'c') => {
                if self.param(0, 0) == 0 {
                    self.responses.extend_from_slice(b"\x1b[?62;22c");
                }
            }
            (b'>', 0, b'c') => self.responses.extend_from_slice(b"\x1b[>1;10;0c"),
            (0, 0, b'd') => {
                let r = self.param_n(0) - 1;
                let x = self.cur.x;
                self.goto(x, r);
            }
            (0, 0, b'g') => match self.param(0, 0) {
                0 => {
                    if self.cur.x < self.tabs.len() {
                        self.tabs[self.cur.x] = false;
                    }
                }
                3 => self.tabs.iter_mut().for_each(|t| *t = false),
                _ => {}
            },
            (b'?', 0, b'h') => self.set_private_modes(true),
            (b'?', 0, b'l') => self.set_private_modes(false),
            (0, 0, b'h') => self.set_ansi_modes(true),
            (0, 0, b'l') => self.set_ansi_modes(false),
            (0, 0, b'm') => self.sgr(),
            (0, 0, b'n') => match self.param(0, 0) {
                5 => self.responses.extend_from_slice(b"\x1b[0n"),
                6 => {
                    let y = if self.cur.origin { self.cur.y - self.top } else { self.cur.y };
                    let s = format!("\x1b[{};{}R", y + 1, self.cur.x.min(self.cols - 1) + 1);
                    self.responses.extend_from_slice(s.as_bytes());
                }
                _ => {}
            },
            (b'?', 0, b'n') => {
                if self.param(0, 0) == 6 {
                    let s = format!("\x1b[?{};{}R", self.cur.y + 1, self.cur.x.min(self.cols - 1) + 1);
                    self.responses.extend_from_slice(s.as_bytes());
                }
            }
            (0, 0, b'r') => {
                let top = self.param_n(0) - 1;
                let bottom = (self.param(1, self.rows as u32) as usize).min(self.rows).max(1) - 1;
                if top < bottom {
                    self.top = top;
                    self.bottom = bottom;
                    self.goto(0, 0);
                }
            }
            (0, 0, b's') if self.nparams == 0 => self.save_cursor(),
            (0, 0, b'u') if self.nparams == 0 => self.restore_cursor(),
            (0, b' ', b'q') => self.modes.cursor_shape = self.param(0, 0) as u8,
            (b'>', 0, b'q') => self.responses.extend_from_slice(b"\x1bP>|cmx(0.1)\x1b\\"),
            (0, 0, b't') => match self.param(0, 0) {
                18 => {
                    let s = format!("\x1b[8;{};{}t", self.rows, self.cols);
                    self.responses.extend_from_slice(s.as_bytes());
                }
                14 => {
                    let s = format!("\x1b[4;{};{}t", self.rows * 16, self.cols * 8);
                    self.responses.extend_from_slice(s.as_bytes());
                }
                16 => self.responses.extend_from_slice(b"\x1b[6;16;8t"),
                _ => {}
            },
            (b'?', b'$', b'p') | (0, b'$', b'p') => {
                let m = self.param(0, 0);
                let v = if private == b'?' { self.query_private_mode(m) } else { self.query_ansi_mode(m) };
                let s = if private == b'?' { format!("\x1b[?{m};{v}$y") } else { format!("\x1b[{m};{v}$y") };
                self.responses.extend_from_slice(s.as_bytes());
            }
            (0, b'!', b'p') => {
                // DECSTR soft reset
                self.modes = Modes { mouse: self.modes.mouse, mouse_sgr: self.modes.mouse_sgr, ..Modes::default() };
                self.cur.pen = Style::DEFAULT;
                self.cur.origin = false;
                self.top = 0;
                self.bottom = self.rows - 1;
            }
            _ => {}
        }
    }

    fn query_private_mode(&self, m: u32) -> u8 {
        let b = |v: bool| if v { 1 } else { 2 };
        match m {
            1 => b(self.modes.app_cursor),
            6 => b(self.cur.origin),
            7 => b(self.modes.autowrap),
            25 => b(self.modes.show_cursor),
            47 | 1047 | 1049 => b(self.alt),
            1000 => b(self.modes.mouse == MouseMode::Normal),
            1002 => b(self.modes.mouse == MouseMode::Button),
            1003 => b(self.modes.mouse == MouseMode::Any),
            1004 => b(self.modes.focus_events),
            1006 => b(self.modes.mouse_sgr),
            2004 => b(self.modes.bracketed_paste),
            2026 => b(self.modes.sync),
            _ => 0,
        }
    }

    fn query_ansi_mode(&self, m: u32) -> u8 {
        match m {
            4 => {
                if self.modes.insert {
                    1
                } else {
                    2
                }
            }
            20 => {
                if self.modes.lnm {
                    1
                } else {
                    2
                }
            }
            _ => 0,
        }
    }

    fn set_ansi_modes(&mut self, on: bool) {
        for i in 0..self.nparams {
            match self.params[i] {
                4 => self.modes.insert = on,
                20 => self.modes.lnm = on,
                _ => {}
            }
        }
    }

    fn set_private_modes(&mut self, on: bool) {
        for i in 0..self.nparams {
            match self.params[i] {
                1 => self.modes.app_cursor = on,
                5 => self.modes.reverse_video = on,
                6 => {
                    self.cur.origin = on;
                    self.goto(0, 0);
                }
                7 => self.modes.autowrap = on,
                9 => self.modes.mouse = if on { MouseMode::X10 } else { MouseMode::Off },
                25 => self.modes.show_cursor = on,
                47 => self.set_alt(on, false),
                66 => self.modes.app_keypad = on,
                1000 => self.modes.mouse = if on { MouseMode::Normal } else { MouseMode::Off },
                1002 => self.modes.mouse = if on { MouseMode::Button } else { MouseMode::Off },
                1003 => self.modes.mouse = if on { MouseMode::Any } else { MouseMode::Off },
                1004 => self.modes.focus_events = on,
                1006 => self.modes.mouse_sgr = on,
                1007 => self.modes.alt_scroll = on,
                1047 => {
                    if !on && self.alt {
                        let blank = Row::blank(self.cols, Style::DEFAULT);
                        self.screens[1].iter_mut().for_each(|r| *r = blank.clone());
                    }
                    self.set_alt(on, false);
                }
                1048 => {
                    if on {
                        self.save_cursor()
                    } else {
                        self.restore_cursor()
                    }
                }
                1049 => {
                    if on {
                        self.saved[0] = self.cur;
                        self.set_alt(true, true);
                    } else {
                        self.set_alt(false, false);
                        let mut c = self.saved[0];
                        c.x = c.x.min(self.cols - 1);
                        c.y = c.y.min(self.rows - 1);
                        self.cur = c;
                    }
                }
                2004 => self.modes.bracketed_paste = on,
                2026 => self.modes.sync = on,
                _ => {}
            }
        }
    }

    fn ext_color(&self, i: usize) -> (Option<Color>, usize) {
        // returns (color, number of extra params consumed after index i)
        let n = self.nparams;
        if i + 1 < n && self.colon[i + 1] {
            let mut j = i + 1;
            while j < n && self.colon[j] {
                j += 1;
            }
            let subs = &self.params[i + 1..j];
            let consumed = j - i - 1;
            let c = match subs.first() {
                Some(5) if subs.len() >= 2 => Some(Color::Idx(subs[1] as u8)),
                Some(2) if subs.len() >= 5 => Some(Color::Rgb(subs[2] as u8, subs[3] as u8, subs[4] as u8)),
                Some(2) if subs.len() == 4 => Some(Color::Rgb(subs[1] as u8, subs[2] as u8, subs[3] as u8)),
                _ => None,
            };
            return (c, consumed);
        }
        match self.params.get(i + 1).copied() {
            Some(5) if i + 2 < n => (Some(Color::Idx(self.params[i + 2] as u8)), 2),
            Some(2) if i + 4 < n => {
                (Some(Color::Rgb(self.params[i + 2] as u8, self.params[i + 3] as u8, self.params[i + 4] as u8)), 4)
            }
            _ => (None, (n - i - 1).min(1)),
        }
    }

    fn sgr(&mut self) {
        if self.nparams == 0 {
            self.cur.pen = Style::DEFAULT;
            return;
        }
        let mut i = 0;
        while i < self.nparams {
            let p = self.params[i];
            let pen = &mut self.cur.pen;
            match p {
                0 => *pen = Style::DEFAULT,
                1 => pen.attrs |= attr::BOLD,
                2 => pen.attrs |= attr::DIM,
                3 => pen.attrs |= attr::ITALIC,
                4 => {
                    if i + 1 < self.nparams && self.colon[i + 1] {
                        let v = self.params[i + 1];
                        i += 1;
                        if v == 0 {
                            self.cur.pen.attrs &= !attr::UNDERLINE;
                        } else {
                            self.cur.pen.attrs |= attr::UNDERLINE;
                        }
                    } else {
                        pen.attrs |= attr::UNDERLINE;
                    }
                }
                5 | 6 => pen.attrs |= attr::BLINK,
                7 => pen.attrs |= attr::REVERSE,
                8 => pen.attrs |= attr::HIDDEN,
                9 => pen.attrs |= attr::STRIKE,
                21 => pen.attrs |= attr::UNDERLINE,
                22 => pen.attrs &= !(attr::BOLD | attr::DIM),
                23 => pen.attrs &= !attr::ITALIC,
                24 => pen.attrs &= !attr::UNDERLINE,
                25 => pen.attrs &= !attr::BLINK,
                27 => pen.attrs &= !attr::REVERSE,
                28 => pen.attrs &= !attr::HIDDEN,
                29 => pen.attrs &= !attr::STRIKE,
                30..=37 => pen.fg = Color::Idx((p - 30) as u8),
                39 => pen.fg = Color::Reset,
                40..=47 => pen.bg = Color::Idx((p - 40) as u8),
                49 => pen.bg = Color::Reset,
                90..=97 => pen.fg = Color::Idx((p - 90 + 8) as u8),
                100..=107 => pen.bg = Color::Idx((p - 100 + 8) as u8),
                38 | 48 | 58 => {
                    let (c, used) = self.ext_color(i);
                    if let Some(c) = c {
                        match p {
                            38 => self.cur.pen.fg = c,
                            48 => self.cur.pen.bg = c,
                            _ => {}
                        }
                    }
                    i += used;
                }
                _ => {}
            }
            i += 1;
        }
    }

    fn osc_dispatch(&mut self) {
        let data = std::mem::take(&mut self.osc);
        let s = String::from_utf8_lossy(&data);
        let (num, rest) = match s.find(';') {
            Some(p) => (&s[..p], &s[p + 1..]),
            None => (&s[..], ""),
        };
        let term: &[u8] = if self.osc_bel { b"\x07" } else { b"\x1b\\" };
        match num {
            "0" | "2" => self.title = rest.chars().filter(|c| !c.is_control()).take(256).collect(),
            "7" => self.cwd = Some(rest.to_string()),
            "10" | "11" | "12" if rest == "?" => {
                let (r, g, b) = if num == "11" { self.default_bg } else { self.default_fg };
                let reply = format!("\x1b]{num};rgb:{r:02x}{r:02x}/{g:02x}{g:02x}/{b:02x}{b:02x}");
                self.responses.extend_from_slice(reply.as_bytes());
                self.responses.extend_from_slice(term);
            }
            "52" => {
                if let Some(p) = rest.find(';') {
                    let payload = &rest[p + 1..];
                    if payload != "?"
                        && let Some(bytes) = cmx_tui::clipboard::base64_decode(payload)
                    {
                        self.clipboard = Some(String::from_utf8_lossy(&bytes).into_owned());
                    }
                }
            }
            _ => {}
        }
        self.osc = data;
        self.osc.clear();
    }

    // ------------------------------------------------------------------ resize

    pub fn resize(&mut self, cols: u16, rows: u16) {
        let cols = (cols as usize).max(1);
        let rows = (rows as usize).max(1);
        if cols == self.cols && rows == self.rows {
            return;
        }
        let blank_cell = Cell::BLANK;
        // width
        if cols != self.cols {
            for screen in self.screens.iter_mut() {
                for r in screen.iter_mut() {
                    r.cells.resize(cols, blank_cell);
                    if let Some(last) = r.cells.last_mut()
                        && last.sym.width() == 2
                    {
                        last.sym = Sym::BLANK;
                    }
                    if r.cells[0].sym.is_cont() {
                        r.cells[0].sym = Sym::BLANK;
                    }
                }
            }
        }
        // height, primary screen (scrollback aware)
        let old_rows = self.rows;
        for s in 0..2 {
            let is_active = (s == 1) == self.alt;
            let cur_y = if is_active { self.cur.y } else { self.saved[s].y };
            let mut cy = cur_y;
            if rows < old_rows {
                let mut excess = old_rows - rows;
                let below = old_rows - 1 - cur_y.min(old_rows - 1);
                let cut = excess.min(below);
                self.screens[s].truncate(old_rows - cut);
                excess -= cut;
                for _ in 0..excess {
                    let r = self.screens[s].remove(0);
                    if s == 0 {
                        self.push_scrollback(r);
                    }
                }
                cy = cur_y.saturating_sub(excess);
            } else if rows > old_rows {
                let mut add = rows - old_rows;
                if s == 0 {
                    while add > 0 && !self.scrollback.is_empty() && !self.scrollback.is_empty() {
                        let mut r = self.scrollback.pop_back().unwrap();
                        r.cells.resize(cols, blank_cell);
                        self.screens[0].insert(0, r);
                        cy += 1;
                        add -= 1;
                    }
                }
                for _ in 0..add {
                    self.screens[s].push(Row::blank(cols, Style::DEFAULT));
                }
            }
            let cy = cy.min(rows - 1);
            if is_active {
                self.cur.y = cy;
            } else {
                self.saved[s].y = cy;
            }
        }
        self.cols = cols;
        self.rows = rows;
        self.cur.x = self.cur.x.min(cols - 1);
        self.cur.wrap_pending = false;
        for s in &mut self.saved {
            s.x = s.x.min(cols - 1);
            s.y = s.y.min(rows - 1);
        }
        self.top = 0;
        self.bottom = rows - 1;
        self.reset_tabs();
        self.dirty = true;
    }
}

#[cfg(test)]
mod tests;
