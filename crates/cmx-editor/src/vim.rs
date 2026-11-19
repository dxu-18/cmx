//! Vim emulation (toggled with Ctrl+K): normal / insert / visual / visual-line modes, counts,
//! operators with motions and text objects, registers and dot-repeat.
//!
//! The pending key sequence is re-parsed from scratch on every key, which keeps the grammar
//! simple: `[count] operator [count] (motion | textobj | operator)` or `[count] command`.

use crate::history::Coalesce;
use crate::text::{CharClass, Pos, Text, char_class};
use crate::view::{Ed, SelKind};
use cmx_tui::unicode::prev_boundary;
use cmx_tui::{KeyCode, KeyEvent};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Normal,
    Insert,
    Visual,
    VisualLine,
}

/// What the app should do after vim handled a key.
#[derive(Debug, PartialEq)]
pub enum VimOut {
    /// Key consumed.
    Done,
    /// Not a vim key in this mode: run the default editor handling (insert-mode typing, ctrl-shortcuts).
    Pass,
    /// Open the `:` command line.
    Cmdline,
    /// Open the `/` (true) or `?` (false) search prompt.
    Search(bool),
    /// Text was yanked; copy it to the system clipboard too.
    Yank(String),
    /// Replay these keys (dot-repeat).
    Replay(Vec<KeyEvent>),
    Message(String),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum VK {
    Ch(char),
    Ctrl(char),
    Esc,
    Enter,
    Bs,
    Del,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
}

fn vk(k: &KeyEvent) -> Option<VK> {
    let m = k.mods;
    Some(match k.code {
        KeyCode::Char(c) if m.ctrl() && !m.alt() && !m.shift() && !m.sup() => VK::Ctrl(c.to_ascii_lowercase()),
        KeyCode::Char(_) if m.ctrl() => return None,
        KeyCode::Char(_) if m.alt() || m.sup() => return None,
        KeyCode::Char(c) => VK::Ch(c),
        KeyCode::Esc => VK::Esc,
        KeyCode::Enter => VK::Enter,
        KeyCode::Backspace => VK::Bs,
        KeyCode::Delete => VK::Del,
        KeyCode::Left if m.is_empty() => VK::Left,
        KeyCode::Right if m.is_empty() => VK::Right,
        KeyCode::Up if m.is_empty() => VK::Up,
        KeyCode::Down if m.is_empty() => VK::Down,
        KeyCode::Home if m.is_empty() => VK::Home,
        KeyCode::End if m.is_empty() => VK::End,
        _ => return None,
    })
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum FindKind {
    F,
    BigF,
    T,
    BigT,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Motion {
    Left,
    Right,
    Up,
    Down,
    WordFwd(bool),
    WordBack(bool),
    WordEnd(bool),
    WordEndBack(bool),
    LineStart,
    FirstNonBlank,
    LineEnd,
    GotoLine,
    FileStart,
    Find(FindKind, char),
    RepeatFind(bool),
    Bracket,
    ParaFwd,
    ParaBack,
    ScreenTop,
    ScreenMid,
    ScreenBottom,
    NextLine,
    PrevLine,
    SearchNext(bool),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum MType {
    Exclusive,
    Inclusive,
    Linewise,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Op {
    Delete,
    Change,
    Yank,
    Indent,
    Outdent,
    ToggleCase,
    Lower,
    Upper,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Obj {
    Word { inner: bool, big: bool },
    Quote { inner: bool, q: char },
    Bracket { inner: bool, open: char, close: char },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Target {
    Motion(Motion),
    Obj(Obj),
    Lines,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Simple {
    InsertI,
    InsertA,
    InsertBigI,
    InsertBigA,
    OpenBelow,
    OpenAbove,
    DelChar,
    DelCharBack,
    Subst,
    SubstLine,
    ReplaceChar(char),
    ToggleCaseChar,
    DelToEnd,
    ChangeToEnd,
    YankLine,
    PasteAfter,
    PasteBefore,
    Join,
    Undo,
    Redo,
    Dot,
    Visual,
    VisualLine,
    Cmdline,
    Search(bool),
    StarSearch(bool),
    ScrollCenter,
    ScrollTop,
    ScrollBottom,
    HalfDown,
    HalfUp,
    PageDown,
    PageUp,
    LineDown,
    LineUp,
    Esc,
    // visual-only
    SwapEnds,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Parsed {
    Motion(Motion),
    Op(Op, Target),
    Simple(Simple),
}

enum ParseResult {
    Incomplete,
    Invalid,
    Ok(usize, Parsed),
}

pub struct Vim {
    pub mode: Mode,
    pending: Vec<KeyEvent>,
    pub register: String,
    pub reg_linewise: bool,
    last_find: Option<(FindKind, char)>,
    pub last_search: Option<(String, bool)>,
    dot: Vec<KeyEvent>,
    rec: Option<Vec<KeyEvent>>,
    pub replaying: bool,
}

impl Default for Vim {
    fn default() -> Self {
        Vim::new()
    }
}

fn digit(k: Option<&VK>) -> Option<u32> {
    match k {
        Some(VK::Ch(c)) if c.is_ascii_digit() => c.to_digit(10),
        _ => None,
    }
}

impl Vim {
    pub fn new() -> Vim {
        Vim {
            mode: Mode::Normal,
            pending: Vec::new(),
            register: String::new(),
            reg_linewise: false,
            last_find: None,
            last_search: None,
            dot: Vec::new(),
            rec: None,
            replaying: false,
        }
    }

    pub fn pending_str(&self) -> String {
        self.pending
            .iter()
            .filter_map(vk)
            .map(|k| match k {
                VK::Ch(c) => c.to_string(),
                VK::Ctrl(c) => format!("^{}", c.to_ascii_uppercase()),
                _ => String::new(),
            })
            .collect()
    }

    pub fn mode_label(&self) -> &'static str {
        match self.mode {
            Mode::Normal => "NORMAL",
            Mode::Insert => "INSERT",
            Mode::Visual => "VISUAL",
            Mode::VisualLine => "V-LINE",
        }
    }

    /// Reset to normal mode (e.g. when vim is toggled on).
    pub fn reset(&mut self, ed: &mut Ed) {
        self.mode = Mode::Normal;
        self.pending.clear();
        ed.v.sel_kind = SelKind::Exclusive;
        ed.v.anchor = None;
        clamp_normal(ed);
    }

    fn parse_motion(&self, keys: &[VK], i: usize) -> Option<Result<(usize, Motion), bool>> {
        // Some(Ok) = motion, Some(Err(true)) = incomplete, None = not a motion
        let k = keys.get(i)?;
        let m = match *k {
            VK::Ch('h') | VK::Left | VK::Bs => Motion::Left,
            VK::Ch('l') | VK::Right | VK::Ch(' ') => Motion::Right,
            VK::Ch('j') | VK::Down | VK::Ctrl('n') => Motion::Down,
            VK::Ch('k') | VK::Up | VK::Ctrl('p') => Motion::Up,
            VK::Ch('w') => Motion::WordFwd(false),
            VK::Ch('W') => Motion::WordFwd(true),
            VK::Ch('b') => Motion::WordBack(false),
            VK::Ch('B') => Motion::WordBack(true),
            VK::Ch('e') => Motion::WordEnd(false),
            VK::Ch('E') => Motion::WordEnd(true),
            VK::Ch('0') | VK::Home => Motion::LineStart,
            VK::Ch('^') | VK::Ch('_') => Motion::FirstNonBlank,
            VK::Ch('$') | VK::End => Motion::LineEnd,
            VK::Ch('G') => Motion::GotoLine,
            VK::Ch('%') => Motion::Bracket,
            VK::Ch('}') => Motion::ParaFwd,
            VK::Ch('{') => Motion::ParaBack,
            VK::Ch('H') => Motion::ScreenTop,
            VK::Ch('M') => Motion::ScreenMid,
            VK::Ch('L') => Motion::ScreenBottom,
            VK::Ch('+') | VK::Enter => Motion::NextLine,
            VK::Ch('-') => Motion::PrevLine,
            VK::Ch(';') => Motion::RepeatFind(false),
            VK::Ch(',') => Motion::RepeatFind(true),
            VK::Ch('n') => Motion::SearchNext(false),
            VK::Ch('N') => Motion::SearchNext(true),
            VK::Ch(c @ ('f' | 'F' | 't' | 'T')) => {
                let kind = match c {
                    'f' => FindKind::F,
                    'F' => FindKind::BigF,
                    't' => FindKind::T,
                    _ => FindKind::BigT,
                };
                return Some(match keys.get(i + 1) {
                    None => Err(true),
                    Some(VK::Ch(t)) => Ok((2, Motion::Find(kind, *t))),
                    Some(_) => Err(false),
                });
            }
            VK::Ch('g') => {
                return Some(match keys.get(i + 1) {
                    None => Err(true),
                    Some(VK::Ch('g')) => Ok((2, Motion::FileStart)),
                    Some(VK::Ch('e')) => Ok((2, Motion::WordEndBack(false))),
                    Some(VK::Ch('E')) => Ok((2, Motion::WordEndBack(true))),
                    Some(VK::Ch('_')) => Ok((2, Motion::LineEnd)),
                    Some(VK::Ch('j')) => Ok((2, Motion::Down)),
                    Some(VK::Ch('k')) => Ok((2, Motion::Up)),
                    _ => return None,
                });
            }
            _ => return None,
        };
        Some(Ok((1, m)))
    }

    fn parse(&self, keys: &[VK], visual: bool) -> ParseResult {
        let mut i = 0;
        let mut count1: Option<usize> = None;
        while let Some(d) = digit(keys.get(i)) {
            if d == 0 && count1.is_none() {
                break; // '0' is a motion
            }
            count1 = Some(count1.unwrap_or(0) * 10 + d as usize);
            i += 1;
        }
        let count = count1.unwrap_or(1);
        let Some(&k) = keys.get(i) else { return ParseResult::Incomplete };

        // ---- operators ----
        let op = match k {
            VK::Ch('d') => Some((Op::Delete, 1)),
            VK::Ch('c') => Some((Op::Change, 1)),
            VK::Ch('y') => Some((Op::Yank, 1)),
            VK::Ch('>') => Some((Op::Indent, 1)),
            VK::Ch('<') => Some((Op::Outdent, 1)),
            VK::Ch('g') => match keys.get(i + 1) {
                None => return ParseResult::Incomplete,
                Some(VK::Ch('~')) => Some((Op::ToggleCase, 2)),
                Some(VK::Ch('u')) => Some((Op::Lower, 2)),
                Some(VK::Ch('U')) => Some((Op::Upper, 2)),
                _ => None,
            },
            _ => None,
        };
        if let Some((op, len)) = op {
            if visual {
                return ParseResult::Ok(count, Parsed::Op(op, Target::Lines));
            }
            let op_key = keys[i + len - 1];
            let mut j = i + len;
            let mut count2: Option<usize> = None;
            while let Some(d) = digit(keys.get(j)) {
                if d == 0 && count2.is_none() {
                    break;
                }
                count2 = Some(count2.unwrap_or(0) * 10 + d as usize);
                j += 1;
            }
            let total = count * count2.unwrap_or(1);
            let Some(&k2) = keys.get(j) else { return ParseResult::Incomplete };
            if k2 == op_key || (len == 2 && k2 == VK::Ch('g')) {
                if len == 2 && k2 == VK::Ch('g') && keys.get(j + 1).is_none() {
                    return ParseResult::Incomplete;
                }
                return ParseResult::Ok(total, Parsed::Op(op, Target::Lines));
            }
            if let VK::Ch(ia @ ('i' | 'a')) = k2 {
                let Some(&k3) = keys.get(j + 1) else { return ParseResult::Incomplete };
                return match obj_of(ia == 'i', k3) {
                    Some(o) => ParseResult::Ok(total, Parsed::Op(op, Target::Obj(o))),
                    None => ParseResult::Invalid,
                };
            }
            return match self.parse_motion(keys, j) {
                Some(Ok((_, m))) => ParseResult::Ok(total, Parsed::Op(op, Target::Motion(m))),
                Some(Err(true)) => ParseResult::Incomplete,
                _ => ParseResult::Invalid,
            };
        }

        // ---- text objects in visual mode ----
        if visual && let VK::Ch(ia @ ('i' | 'a')) = k {
            let Some(&k3) = keys.get(i + 1) else { return ParseResult::Incomplete };
            return match obj_of(ia == 'i', k3) {
                Some(o) => ParseResult::Ok(count, Parsed::Op(Op::Yank, Target::Obj(o))), // Yank = "select"
                None => ParseResult::Invalid,
            };
        }

        // ---- motions ----
        match self.parse_motion(keys, i) {
            Some(Ok((_, m))) => {
                return ParseResult::Ok(
                    count1.unwrap_or(0).max(if m == Motion::GotoLine { 0 } else { 1 }),
                    Parsed::Motion(m),
                );
            }
            Some(Err(true)) => return ParseResult::Incomplete,
            Some(Err(false)) => return ParseResult::Invalid,
            None => {}
        }

        // ---- simple commands ----
        let s = match k {
            VK::Ch('i') => Simple::InsertI,
            VK::Ch('a') => Simple::InsertA,
            VK::Ch('I') => Simple::InsertBigI,
            VK::Ch('A') => Simple::InsertBigA,
            VK::Ch('o') if visual => Simple::SwapEnds,
            VK::Ch('o') => Simple::OpenBelow,
            VK::Ch('O') => Simple::OpenAbove,
            VK::Ch('x') | VK::Del => Simple::DelChar,
            VK::Ch('X') => Simple::DelCharBack,
            VK::Ch('s') => Simple::Subst,
            VK::Ch('S') => Simple::SubstLine,
            VK::Ch('r') => match keys.get(i + 1) {
                None => return ParseResult::Incomplete,
                Some(VK::Ch(c)) => Simple::ReplaceChar(*c),
                Some(VK::Enter) => Simple::ReplaceChar('\n'),
                _ => return ParseResult::Invalid,
            },
            VK::Ch('~') => Simple::ToggleCaseChar,
            VK::Ch('D') => Simple::DelToEnd,
            VK::Ch('C') => Simple::ChangeToEnd,
            VK::Ch('Y') => Simple::YankLine,
            VK::Ch('p') => Simple::PasteAfter,
            VK::Ch('P') => Simple::PasteBefore,
            VK::Ch('J') => Simple::Join,
            VK::Ch('u') if visual => return ParseResult::Ok(count, Parsed::Op(Op::Lower, Target::Lines)),
            VK::Ch('U') if visual => return ParseResult::Ok(count, Parsed::Op(Op::Upper, Target::Lines)),
            VK::Ch('u') => Simple::Undo,
            VK::Ctrl('r') => Simple::Redo,
            VK::Ch('.') => Simple::Dot,
            VK::Ch('v') => Simple::Visual,
            VK::Ch('V') => Simple::VisualLine,
            VK::Ch(':') => Simple::Cmdline,
            VK::Ch('/') => Simple::Search(true),
            VK::Ch('?') => Simple::Search(false),
            VK::Ch('*') => Simple::StarSearch(true),
            VK::Ch('#') => Simple::StarSearch(false),
            VK::Ch('z') => match keys.get(i + 1) {
                None => return ParseResult::Incomplete,
                Some(VK::Ch('z')) | Some(VK::Ch('.')) => Simple::ScrollCenter,
                Some(VK::Ch('t')) | Some(VK::Enter) => Simple::ScrollTop,
                Some(VK::Ch('b')) | Some(VK::Ch('-')) => Simple::ScrollBottom,
                _ => return ParseResult::Invalid,
            },
            VK::Ctrl('d') => Simple::HalfDown,
            VK::Ctrl('u') => Simple::HalfUp,
            VK::Ctrl('f') => Simple::PageDown,
            VK::Ctrl('b') => Simple::PageUp,
            VK::Ctrl('e') => Simple::LineDown,
            VK::Ctrl('y') => Simple::LineUp,
            VK::Esc | VK::Ctrl('[') => Simple::Esc,
            _ => return ParseResult::Invalid,
        };
        ParseResult::Ok(count, Parsed::Simple(s))
    }

    /// Handle a key. Returns what the app should do next.
    pub fn handle(&mut self, ed: &mut Ed, key: &KeyEvent) -> VimOut {
        match self.mode {
            Mode::Insert => self.handle_insert(ed, key),
            Mode::Normal | Mode::Visual | Mode::VisualLine => self.handle_cmd(ed, key),
        }
    }

    fn handle_insert(&mut self, ed: &mut Ed, key: &KeyEvent) -> VimOut {
        if let Some(r) = self.rec.as_mut()
            && !self.replaying
        {
            r.push(*key);
        }
        match vk(key) {
            Some(VK::Esc) | Some(VK::Ctrl('c')) | Some(VK::Ctrl('[')) => {
                self.mode = Mode::Normal;
                if let Some(mut r) = self.rec.take()
                    && !self.replaying
                {
                    if let Some(last) = r.last_mut() {
                        *last = KeyEvent::plain(KeyCode::Esc);
                    }
                    self.dot = r;
                }
                // vim moves the cursor left when leaving insert mode
                let c = ed.v.cursor;
                if c.col > 0 {
                    let p = prev_boundary(ed.doc.text.line(c.line), c.col);
                    ed.v.cursor = Pos::new(c.line, p);
                }
                ed.v.anchor = None;
                ed.doc.history.seal();
                VimOut::Done
            }
            Some(VK::Ctrl('w')) => {
                ed.delete_word_left();
                VimOut::Done
            }
            Some(VK::Ctrl('u')) => {
                ed.delete_to_line_start();
                VimOut::Done
            }
            _ => VimOut::Pass,
        }
    }

    fn handle_cmd(&mut self, ed: &mut Ed, key: &KeyEvent) -> VimOut {
        let Some(k) = vk(key) else {
            self.pending.clear();
            return VimOut::Pass;
        };
        // Ctrl chords that aren't vim commands fall through to the app keymap (Ctrl+S, Ctrl+P, ...).
        if let VK::Ctrl(c) = k {
            if !matches!(c, 'r' | 'd' | 'u' | 'f' | 'b' | 'e' | 'y' | 'n' | 'p' | '[' | 'c') && self.pending.is_empty()
            {
                return VimOut::Pass;
            }
            if c == 'c' {
                // Ctrl+C: yank in visual (to clipboard), otherwise cancel
                if self.mode != Mode::Normal
                    && let Some((a, b)) = ed.v.selection_in(&ed.doc.text)
                {
                    let text = ed.doc.text.slice(a, b);
                    self.register = text.clone();
                    self.reg_linewise = self.mode == Mode::VisualLine;
                    self.exit_visual(ed);
                    return VimOut::Yank(text);
                }
                self.pending.clear();
                return VimOut::Done;
            }
        }
        self.pending.push(*key);
        let keys: Vec<VK> = self.pending.iter().filter_map(vk).collect();
        let visual = self.mode != Mode::Normal;
        match self.parse(&keys, visual) {
            ParseResult::Incomplete => VimOut::Done,
            ParseResult::Invalid => {
                self.pending.clear();
                VimOut::Done
            }
            ParseResult::Ok(count, p) => {
                let keys_used = std::mem::take(&mut self.pending);
                let out = self.execute(ed, count, p);
                if !self.replaying && is_change(&p, visual) {
                    if self.mode == Mode::Insert {
                        self.rec = Some(keys_used);
                    } else {
                        self.dot = keys_used;
                    }
                }
                if self.mode == Mode::Normal {
                    clamp_normal(ed);
                }
                out
            }
        }
    }

    fn exit_visual(&mut self, ed: &mut Ed) {
        self.mode = Mode::Normal;
        ed.v.sel_kind = SelKind::Exclusive;
        ed.v.anchor = None;
    }

    fn execute(&mut self, ed: &mut Ed, count: usize, p: Parsed) -> VimOut {
        let visual = self.mode != Mode::Normal;
        match p {
            Parsed::Motion(m) => {
                if let Some((pos, _)) = self.motion(ed, m, count) {
                    if visual {
                        ed.v.cursor = pos;
                    } else {
                        ed.v.cursor = pos;
                        ed.v.anchor = None;
                    }
                    if !matches!(m, Motion::Up | Motion::Down) {
                        ed.v.want_col = None;
                    }
                    ed.doc.history.seal();
                } else if let Motion::SearchNext(_) = m {
                    return VimOut::Message("Pattern not found".into());
                }
                VimOut::Done
            }
            Parsed::Op(op, target) => {
                if visual {
                    if let Target::Obj(o) = target {
                        if let Some((a, b)) = text_object(&ed.doc.text, ed.v.cursor, o) {
                            ed.v.anchor = Some(a);
                            ed.v.cursor = ed.doc.text.prev_pos(b).max(a);
                        }
                        return VimOut::Done;
                    }
                    let Some((a, b)) = ed.v.selection_in(&ed.doc.text) else { return VimOut::Done };
                    let linewise = self.mode == Mode::VisualLine;
                    self.exit_visual(ed);
                    return self.apply_op(ed, op, a, b, linewise);
                }
                let cur = ed.v.cursor;
                let (a, b, linewise) = match target {
                    Target::Lines => {
                        let last = ed.doc.text.len_lines() - 1;
                        let l1 = (cur.line + count - 1).min(last);
                        (Pos::new(cur.line, 0), Pos::new(l1, 0), true)
                    }
                    Target::Obj(o) => match text_object(&ed.doc.text, cur, o) {
                        Some((a, b)) => (a, b, false),
                        None => return VimOut::Done,
                    },
                    Target::Motion(m) => {
                        // cw / cW behave like ce / cE on a word
                        let m = match (op, m) {
                            (Op::Change, Motion::WordFwd(big)) if ed.class_at(cur) != CharClass::Space => {
                                Motion::WordEnd(big)
                            }
                            _ => m,
                        };
                        let Some((pos, mt)) = self.motion_for_op(ed, m, count) else { return VimOut::Done };
                        let (s, e) = if pos < cur { (pos, cur) } else { (cur, pos) };
                        match mt {
                            MType::Linewise => (Pos::new(s.line, 0), Pos::new(e.line, 0), true),
                            MType::Inclusive => (s, ed.doc.text.next_pos(e).max(e), false),
                            MType::Exclusive => {
                                if e.line > s.line && e.col == 0 && matches!(m, Motion::WordFwd(_)) {
                                    let l = e.line - 1;
                                    (s, Pos::new(l, ed.doc.text.line_len(l)), false)
                                } else {
                                    (s, e, false)
                                }
                            }
                        }
                    }
                };
                if linewise {
                    let last = ed.doc.text.len_lines() - 1;
                    let end = if b.line < last { Pos::new(b.line + 1, 0) } else { ed.doc.text.end() };
                    return self.apply_op(ed, op, a, end, true);
                }
                self.apply_op(ed, op, a, b, false)
            }
            Parsed::Simple(s) => self.simple(ed, count, s),
        }
    }

    fn apply_op(&mut self, ed: &mut Ed, op: Op, a: Pos, b: Pos, linewise: bool) -> VimOut {
        let t = &ed.doc.text;
        match op {
            Op::Yank => {
                let mut s = t.slice(a, b);
                if linewise && !s.ends_with('\n') {
                    s.push('\n');
                }
                self.register = s.clone();
                self.reg_linewise = linewise;
                ed.v.cursor = a;
                ed.v.anchor = None;
                VimOut::Yank(s)
            }
            Op::Delete | Op::Change => {
                let change = op == Op::Change;
                let mut s = t.slice(a, b);
                // linewise delete at the end of the file also eats the preceding newline
                let (a, b) = if linewise && b == t.end() && a.line > 0 && !s.ends_with('\n') && !change {
                    (Pos::new(a.line - 1, t.line_len(a.line - 1)), b)
                } else {
                    (a, b)
                };
                if linewise && !s.ends_with('\n') {
                    s.push('\n');
                }
                self.register = s;
                self.reg_linewise = linewise;
                if change && linewise {
                    // keep one (indented) line to type on
                    let indent = ed.doc.text.indent_of(a.line).to_string();
                    let end_line = if b.col == 0 && b.line > a.line { b.line - 1 } else { b.line };
                    let e = Pos::new(end_line, ed.doc.text.line_len(end_line));
                    ed.transact(Coalesce::None, None, |e2| {
                        e2.doc.remove(Pos::new(a.line, 0), e);
                        e2.doc.insert(Pos::new(a.line, 0), &indent);
                        e2.v.cursor = Pos::new(a.line, indent.len());
                        e2.v.anchor = None;
                    });
                } else {
                    ed.transact(Coalesce::None, None, |e2| {
                        e2.doc.remove(a, b);
                        e2.v.cursor = e2.doc.text.clamp(a);
                        e2.v.anchor = None;
                    });
                    if linewise {
                        let l = ed.v.cursor.line.min(ed.doc.text.len_lines() - 1);
                        let l = if a.line < l || a.col == 0 { l } else { l + 1 }.min(ed.doc.text.len_lines() - 1);
                        ed.v.cursor = Pos::new(l, ed.doc.text.first_non_blank(l));
                    }
                }
                if change {
                    self.mode = Mode::Insert;
                }
                VimOut::Done
            }
            Op::Indent | Op::Outdent => {
                let end_line = if b.col == 0 && b.line > a.line { b.line - 1 } else { b.line };
                ed.v.anchor = Some(Pos::new(a.line, 0));
                ed.v.cursor = Pos::new(end_line, ed.doc.text.line_len(end_line));
                ed.indent_lines(op == Op::Indent);
                ed.v.anchor = None;
                ed.v.cursor = Pos::new(a.line, ed.doc.text.first_non_blank(a.line));
                VimOut::Done
            }
            Op::ToggleCase | Op::Lower | Op::Upper => {
                let s = t.slice(a, b);
                let r: String = match op {
                    Op::Lower => s.to_lowercase(),
                    Op::Upper => s.to_uppercase(),
                    _ => s
                        .chars()
                        .map(|c| {
                            if c.is_uppercase() {
                                c.to_lowercase().next().unwrap_or(c)
                            } else {
                                c.to_uppercase().next().unwrap_or(c)
                            }
                        })
                        .collect(),
                };
                ed.transact(Coalesce::None, None, |e2| {
                    e2.doc.remove(a, b);
                    e2.doc.insert(a, &r);
                    e2.v.cursor = a;
                    e2.v.anchor = None;
                });
                VimOut::Done
            }
        }
    }

    fn simple(&mut self, ed: &mut Ed, count: usize, s: Simple) -> VimOut {
        let cur = ed.v.cursor;
        let line_len = ed.doc.text.line_len(cur.line);
        match s {
            Simple::InsertI => self.enter_insert(ed, cur),
            Simple::InsertA => {
                let p = if line_len == 0 { cur } else { ed.doc.text.next_pos(cur).min(Pos::new(cur.line, line_len)) };
                self.enter_insert(ed, p)
            }
            Simple::InsertBigI => {
                let p = Pos::new(cur.line, ed.doc.text.first_non_blank(cur.line));
                self.enter_insert(ed, p)
            }
            Simple::InsertBigA => self.enter_insert(ed, Pos::new(cur.line, line_len)),
            Simple::OpenBelow | Simple::OpenAbove => {
                if self.mode != Mode::Normal {
                    return VimOut::Done;
                }
                ed.insert_line(s == Simple::OpenBelow);
                self.mode = Mode::Insert;
                VimOut::Done
            }
            Simple::DelChar | Simple::Subst => {
                if self.mode != Mode::Normal {
                    let op = if s == Simple::Subst { Op::Change } else { Op::Delete };
                    if let Some((a, b)) = ed.v.selection_in(&ed.doc.text) {
                        let lw = self.mode == Mode::VisualLine;
                        self.exit_visual(ed);
                        return self.apply_op(ed, op, a, b, lw);
                    }
                    return VimOut::Done;
                }
                let mut e = cur;
                for _ in 0..count {
                    if e.col >= ed.doc.text.line_len(e.line) {
                        break;
                    }
                    e = ed.doc.text.next_pos(e);
                }
                let op = if s == Simple::Subst { Op::Change } else { Op::Delete };
                if e == cur && op == Op::Delete {
                    return VimOut::Done;
                }
                self.apply_op(ed, op, cur, e, false)
            }
            Simple::DelCharBack => {
                let mut a = cur;
                for _ in 0..count {
                    if a.col == 0 {
                        break;
                    }
                    a = ed.doc.text.prev_pos(a);
                }
                self.apply_op(ed, Op::Delete, a, cur, false)
            }
            Simple::SubstLine => {
                let last = ed.doc.text.len_lines() - 1;
                let l1 = (cur.line + count - 1).min(last);
                let end = if l1 < last { Pos::new(l1 + 1, 0) } else { ed.doc.text.end() };
                self.apply_op(ed, Op::Change, Pos::new(cur.line, 0), end, true)
            }
            Simple::ReplaceChar(c) => {
                if self.mode != Mode::Normal {
                    if let Some((a, b)) = ed.v.selection_in(&ed.doc.text) {
                        let src = ed.doc.text.slice(a, b);
                        let r: String = src.chars().map(|x| if x == '\n' { x } else { c }).collect();
                        self.exit_visual(ed);
                        ed.transact(Coalesce::None, None, |e2| {
                            e2.doc.remove(a, b);
                            e2.doc.insert(a, &r);
                            e2.v.cursor = a;
                        });
                    }
                    return VimOut::Done;
                }
                let mut e = cur;
                for _ in 0..count {
                    if e.col >= ed.doc.text.line_len(e.line) {
                        return VimOut::Done;
                    }
                    e = ed.doc.text.next_pos(e);
                }
                let rep: String = if c == '\n' { "\n".into() } else { std::iter::repeat_n(c, count).collect() };
                ed.transact(Coalesce::None, None, |e2| {
                    e2.doc.remove(cur, e);
                    let end = e2.doc.insert(cur, &rep);
                    e2.v.cursor = if c == '\n' { end } else { e2.doc.text.prev_pos(end) };
                });
                VimOut::Done
            }
            Simple::ToggleCaseChar => {
                if self.mode != Mode::Normal {
                    if let Some((a, b)) = ed.v.selection_in(&ed.doc.text) {
                        let lw = self.mode == Mode::VisualLine;
                        self.exit_visual(ed);
                        return self.apply_op(ed, Op::ToggleCase, a, b, lw);
                    }
                    return VimOut::Done;
                }
                let mut e = cur;
                for _ in 0..count {
                    if e.col >= ed.doc.text.line_len(e.line) {
                        break;
                    }
                    e = ed.doc.text.next_pos(e);
                }
                self.apply_op(ed, Op::ToggleCase, cur, e, false);
                ed.v.cursor = e;
                VimOut::Done
            }
            Simple::DelToEnd | Simple::ChangeToEnd => {
                let last = ed.doc.text.len_lines() - 1;
                let l = (cur.line + count - 1).min(last);
                let e = Pos::new(l, ed.doc.text.line_len(l));
                let op = if s == Simple::DelToEnd { Op::Delete } else { Op::Change };
                self.apply_op(ed, op, cur, e, false)
            }
            Simple::YankLine => {
                if self.mode != Mode::Normal
                    && let Some((a, _)) = ed.v.selection_in(&ed.doc.text)
                {
                    let b_line = ed.v.anchor.unwrap_or(cur).max(cur).line;
                    let last = ed.doc.text.len_lines() - 1;
                    let end = if b_line < last { Pos::new(b_line + 1, 0) } else { ed.doc.text.end() };
                    self.exit_visual(ed);
                    return self.apply_op(ed, Op::Yank, Pos::new(a.line, 0), end, true);
                }
                let last = ed.doc.text.len_lines() - 1;
                let l1 = (cur.line + count - 1).min(last);
                let end = if l1 < last { Pos::new(l1 + 1, 0) } else { ed.doc.text.end() };
                let out = self.apply_op(ed, Op::Yank, Pos::new(cur.line, 0), end, true);
                ed.v.cursor = cur;
                out
            }
            Simple::PasteAfter | Simple::PasteBefore => {
                if self.register.is_empty() {
                    return VimOut::Done;
                }
                let reg: String = self.register.repeat(count.max(1));
                if self.mode != Mode::Normal {
                    if let Some((a, b)) = ed.v.selection_in(&ed.doc.text) {
                        let lw = self.reg_linewise;
                        let mut ins = reg.clone();
                        if lw && self.mode != Mode::VisualLine && ins.ends_with('\n') {
                            ins.insert(0, '\n');
                            ins.pop();
                        }
                        self.exit_visual(ed);
                        ed.transact(Coalesce::None, None, |e2| {
                            e2.doc.remove(a, b);
                            e2.doc.insert(a, &ins);
                            e2.v.cursor = a;
                        });
                    }
                    return VimOut::Done;
                }
                let after = s == Simple::PasteAfter;
                if self.reg_linewise {
                    let line = if after { cur.line + 1 } else { cur.line };
                    let last = ed.doc.text.len_lines();
                    ed.transact(Coalesce::None, None, |e2| {
                        if line >= last {
                            let end = e2.doc.text.end();
                            let body = reg.strip_suffix('\n').unwrap_or(&reg);
                            e2.doc.insert(end, &format!("\n{body}"));
                        } else {
                            e2.doc.insert(Pos::new(line, 0), &reg);
                        }
                        let fnb = e2.doc.text.first_non_blank(line.min(e2.doc.text.len_lines() - 1));
                        e2.v.cursor = Pos::new(line.min(e2.doc.text.len_lines() - 1), fnb);
                        e2.v.anchor = None;
                    });
                } else {
                    let at = if after && line_len > 0 {
                        ed.doc.text.next_pos(cur).min(Pos::new(cur.line, line_len))
                    } else {
                        cur
                    };
                    ed.transact(Coalesce::None, None, |e2| {
                        let end = e2.doc.insert(at, &reg);
                        e2.v.cursor = e2.doc.text.prev_pos(end).max(at);
                        e2.v.anchor = None;
                    });
                }
                VimOut::Done
            }
            Simple::Join => {
                let (l0, n) = if self.mode != Mode::Normal {
                    let a = ed.v.anchor.unwrap_or(cur);
                    let (s, e) = if a < cur { (a, cur) } else { (cur, a) };
                    self.exit_visual(ed);
                    (s.line, (e.line - s.line).max(1))
                } else {
                    (cur.line, count.max(2) - 1)
                };
                ed.transact(Coalesce::None, None, |e2| {
                    for _ in 0..n {
                        if l0 + 1 >= e2.doc.text.len_lines() {
                            break;
                        }
                        let len = e2.doc.text.line_len(l0);
                        let next = e2.doc.text.line(l0 + 1).to_string();
                        let lead = next.len() - next.trim_start().len();
                        e2.doc.remove(Pos::new(l0, len), Pos::new(l0 + 1, lead));
                        let cur_line = e2.doc.text.line(l0).to_string();
                        let needs_space = !cur_line[..len].ends_with(' ')
                            && !next.trim_start().is_empty()
                            && !next.trim_start().starts_with(')');
                        let trimmed_len = cur_line[..len].trim_end().len();
                        if trimmed_len < len {
                            e2.doc.remove(Pos::new(l0, trimmed_len), Pos::new(l0, len));
                        }
                        let at = Pos::new(l0, trimmed_len);
                        if needs_space && trimmed_len > 0 {
                            e2.doc.insert(at, " ");
                        }
                        e2.v.cursor = at;
                    }
                });
                VimOut::Done
            }
            Simple::Undo => {
                for _ in 0..count {
                    if !ed.undo() {
                        break;
                    }
                }
                ed.v.anchor = None;
                VimOut::Done
            }
            Simple::Redo => {
                for _ in 0..count {
                    if !ed.redo() {
                        break;
                    }
                }
                ed.v.anchor = None;
                VimOut::Done
            }
            Simple::Dot => {
                if self.dot.is_empty() {
                    VimOut::Done
                } else {
                    VimOut::Replay(self.dot.clone())
                }
            }
            Simple::Visual | Simple::VisualLine => {
                let target = if s == Simple::Visual { Mode::Visual } else { Mode::VisualLine };
                if self.mode == target {
                    self.exit_visual(ed);
                } else {
                    if self.mode == Mode::Normal {
                        ed.v.anchor = Some(cur);
                    }
                    self.mode = target;
                    ed.v.sel_kind = if target == Mode::Visual { SelKind::Inclusive } else { SelKind::Line };
                }
                VimOut::Done
            }
            Simple::SwapEnds => {
                if let Some(a) = ed.v.anchor {
                    ed.v.anchor = Some(cur);
                    ed.v.cursor = a;
                }
                VimOut::Done
            }
            Simple::Cmdline => VimOut::Cmdline,
            Simple::Search(fwd) => VimOut::Search(fwd),
            Simple::StarSearch(fwd) => {
                let Some((a, b)) = ed.doc.text.word_at(cur) else { return VimOut::Done };
                let w = ed.doc.text.slice(a, b);
                // whole word, like vim's \<word\>; n / N continue with the same pattern
                let pat = format!("\\b{}\\b", crate::regex::escape(&w));
                self.last_search = Some((pat.clone(), fwd));
                let from = if fwd { b } else { a };
                let q = crate::search::Query::new(&pat, crate::search::Flags { regex: true, case: true, word: false });
                if let Some((p, _)) = q.ok().and_then(|q| q.find(&ed.doc.text, from, fwd)) {
                    ed.v.cursor = p;
                }
                VimOut::Message(format!("/{pat}"))
            }
            Simple::ScrollCenter => {
                ed.center();
                VimOut::Done
            }
            Simple::ScrollTop => {
                ed.v.scroll_y = cur.line;
                VimOut::Done
            }
            Simple::ScrollBottom => {
                ed.v.scroll_y = (cur.line + 1).saturating_sub(ed.v.height);
                VimOut::Done
            }
            Simple::HalfDown | Simple::HalfUp | Simple::PageDown | Simple::PageUp => {
                let h = ed.v.height.max(2);
                let amount = match s {
                    Simple::HalfDown | Simple::HalfUp => h / 2,
                    _ => h - 2,
                } as isize;
                let d = if matches!(s, Simple::HalfDown | Simple::PageDown) { amount } else { -amount };
                ed.scroll(d);
                let last = ed.doc.text.len_lines() as isize - 1;
                let l = (cur.line as isize + d).clamp(0, last) as usize;
                ed.v.cursor = Pos::new(l, ed.doc.text.first_non_blank(l));
                VimOut::Done
            }
            Simple::LineDown | Simple::LineUp => {
                ed.scroll(if s == Simple::LineDown { count as isize } else { -(count as isize) });
                let top = ed.v.scroll_y;
                let bot = top + ed.v.height.saturating_sub(1);
                let l = cur.line.clamp(top, bot.max(top)).min(ed.doc.text.len_lines() - 1);
                if l != cur.line {
                    ed.v.cursor = ed.doc.text.clamp(Pos::new(l, cur.col));
                }
                VimOut::Done
            }
            Simple::Esc => {
                if self.mode != Mode::Normal {
                    self.exit_visual(ed);
                }
                VimOut::Done
            }
        }
    }

    fn enter_insert(&mut self, ed: &mut Ed, p: Pos) -> VimOut {
        if self.mode != Mode::Normal {
            // `I`/`A` in visual: insert at start/end of selection
            let sel = ed.v.selection_in(&ed.doc.text);
            self.exit_visual(ed);
            if let Some((a, b)) = sel {
                ed.v.cursor = if p < ed.v.cursor { a } else { b };
            }
        } else {
            ed.v.cursor = p;
        }
        ed.v.anchor = None;
        ed.doc.history.seal();
        self.mode = Mode::Insert;
        VimOut::Done
    }

    /// Target position of a motion for an operator (j/k become linewise etc.).
    fn motion_for_op(&mut self, ed: &Ed, m: Motion, count: usize) -> Option<(Pos, MType)> {
        if m == Motion::Right {
            // `dl` / `3x`-style: may go up to (not past) the end of line
            let cur = ed.v.cursor;
            let len = ed.doc.text.line_len(cur.line);
            let mut p = cur;
            for _ in 0..count {
                if p.col >= len {
                    break;
                }
                p = ed.doc.text.next_pos(p);
            }
            return Some((p, MType::Exclusive));
        }
        self.motion(ed, m, count)
    }

    fn motion(&mut self, ed: &Ed, m: Motion, count: usize) -> Option<(Pos, MType)> {
        let t = &ed.doc.text;
        let cur = ed.v.cursor;
        let n = count.max(1);
        let last = t.len_lines() - 1;
        let tw = ed.doc.tab_width;
        let vert = |delta: isize| -> Pos {
            let want = ed.v.want_col.unwrap_or_else(|| t.display_col(cur.line, cur.col, tw));
            let l = (cur.line as isize + delta).clamp(0, last as isize) as usize;
            Pos::new(l, t.col_for_display(l, want, tw))
        };
        Some(match m {
            Motion::Left => {
                let mut p = cur;
                for _ in 0..n {
                    if p.col == 0 {
                        break;
                    }
                    p = t.prev_pos(p);
                }
                (p, MType::Exclusive)
            }
            Motion::Right => {
                let len = t.line_len(cur.line);
                let mut p = cur;
                for _ in 0..n {
                    let q = t.next_pos(p);
                    if q.line != p.line || q.col >= len {
                        break;
                    }
                    p = q;
                }
                (p, MType::Exclusive)
            }
            Motion::Up => (vert(-(n as isize)), MType::Linewise),
            Motion::Down => (vert(n as isize), MType::Linewise),
            Motion::WordFwd(big) => {
                let mut p = cur;
                for _ in 0..n {
                    p = word_fwd(t, p, big);
                }
                (p, MType::Exclusive)
            }
            Motion::WordBack(big) => {
                let mut p = cur;
                for _ in 0..n {
                    p = word_back(t, p, big);
                }
                (p, MType::Exclusive)
            }
            Motion::WordEnd(big) => {
                let mut p = cur;
                for _ in 0..n {
                    p = word_end(t, p, big);
                }
                (p, MType::Inclusive)
            }
            Motion::WordEndBack(big) => {
                let mut p = cur;
                for _ in 0..n {
                    p = word_end_back(t, p, big);
                }
                (p, MType::Inclusive)
            }
            Motion::LineStart => (Pos::new(cur.line, 0), MType::Exclusive),
            Motion::FirstNonBlank => (Pos::new(cur.line, t.first_non_blank(cur.line)), MType::Exclusive),
            Motion::LineEnd => {
                let l = (cur.line + n - 1).min(last);
                let len = t.line_len(l);
                let p = if len == 0 { Pos::new(l, 0) } else { Pos::new(l, prev_boundary(t.line(l), len)) };
                (p, MType::Inclusive)
            }
            Motion::GotoLine => {
                let l = if count == 0 { last } else { (count - 1).min(last) };
                (Pos::new(l, t.first_non_blank(l)), MType::Linewise)
            }
            Motion::FileStart => {
                let l = if count <= 1 { 0 } else { (count - 1).min(last) };
                (Pos::new(l, t.first_non_blank(l)), MType::Linewise)
            }
            Motion::Find(kind, c) => {
                self.last_find = Some((kind, c));
                (
                    find_char(t, cur, kind, c, n, false)?,
                    if matches!(kind, FindKind::F | FindKind::T) { MType::Inclusive } else { MType::Exclusive },
                )
            }
            Motion::RepeatFind(rev) => {
                let (kind, c) = self.last_find?;
                let kind = if rev {
                    match kind {
                        FindKind::F => FindKind::BigF,
                        FindKind::BigF => FindKind::F,
                        FindKind::T => FindKind::BigT,
                        FindKind::BigT => FindKind::T,
                    }
                } else {
                    kind
                };
                (
                    find_char(t, cur, kind, c, n, true)?,
                    if matches!(kind, FindKind::F | FindKind::T) { MType::Inclusive } else { MType::Exclusive },
                )
            }
            Motion::Bracket => {
                // first bracket at/after the cursor on this line, then its match
                let line = t.line(cur.line);
                let off = line[cur.col..].find(|c| "()[]{}".contains(c))?;
                let start = Pos::new(cur.line, cur.col + off);
                (t.match_bracket(start)?, MType::Inclusive)
            }
            Motion::ParaFwd => {
                let mut l = cur.line;
                for _ in 0..n {
                    while l < last && t.line(l).trim().is_empty() {
                        l += 1;
                    }
                    while l < last && !t.line(l).trim().is_empty() {
                        l += 1;
                    }
                }
                let p =
                    if l == last && !t.line(l).trim().is_empty() { Pos::new(l, t.line_len(l)) } else { Pos::new(l, 0) };
                (p, MType::Exclusive)
            }
            Motion::ParaBack => {
                let mut l = cur.line;
                for _ in 0..n {
                    while l > 0 && t.line(l).trim().is_empty() {
                        l -= 1;
                    }
                    while l > 0 && !t.line(l).trim().is_empty() {
                        l -= 1;
                    }
                }
                (Pos::new(l, 0), MType::Exclusive)
            }
            Motion::ScreenTop | Motion::ScreenMid | Motion::ScreenBottom => {
                let top = ed.v.scroll_y;
                let bottom = (top + ed.v.height.saturating_sub(1)).min(last);
                let l = match m {
                    Motion::ScreenTop => top,
                    Motion::ScreenMid => (top + bottom) / 2,
                    _ => bottom,
                };
                (Pos::new(l, t.first_non_blank(l)), MType::Linewise)
            }
            Motion::NextLine => {
                let l = (cur.line + n).min(last);
                (Pos::new(l, t.first_non_blank(l)), MType::Linewise)
            }
            Motion::PrevLine => {
                let l = cur.line.saturating_sub(n);
                (Pos::new(l, t.first_non_blank(l)), MType::Linewise)
            }
            Motion::SearchNext(rev) => {
                let (pat, fwd) = self.last_search.clone()?;
                let dir = if rev { !fwd } else { fwd };
                let q =
                    crate::search::Query::new(&pat, crate::search::Flags { regex: true, ..Default::default() }).ok()?;
                let mut p = cur;
                for _ in 0..n {
                    let from = if dir { t.next_pos(p) } else { p };
                    let (a, _) = q.find(t, from, dir)?;
                    p = a;
                }
                (p, MType::Exclusive)
            }
        })
    }
}

fn is_change(p: &Parsed, visual: bool) -> bool {
    if visual {
        return false;
    }
    match p {
        Parsed::Motion(_) => false,
        Parsed::Op(op, _) => *op != Op::Yank,
        Parsed::Simple(s) => matches!(
            s,
            Simple::InsertI
                | Simple::InsertA
                | Simple::InsertBigI
                | Simple::InsertBigA
                | Simple::OpenBelow
                | Simple::OpenAbove
                | Simple::DelChar
                | Simple::DelCharBack
                | Simple::Subst
                | Simple::SubstLine
                | Simple::ReplaceChar(_)
                | Simple::ToggleCaseChar
                | Simple::DelToEnd
                | Simple::ChangeToEnd
                | Simple::PasteAfter
                | Simple::PasteBefore
                | Simple::Join
        ),
    }
}

/// In normal mode the cursor sits on a character, never past the end of a non-empty line.
pub fn clamp_normal(ed: &mut Ed) {
    let c = ed.doc.text.clamp(ed.v.cursor);
    let len = ed.doc.text.line_len(c.line);
    ed.v.cursor =
        if len > 0 && c.col >= len { Pos::new(c.line, prev_boundary(ed.doc.text.line(c.line), len)) } else { c };
}

fn obj_of(inner: bool, k: VK) -> Option<Obj> {
    let VK::Ch(c) = k else { return None };
    Some(match c {
        'w' => Obj::Word { inner, big: false },
        'W' => Obj::Word { inner, big: true },
        '"' | '\'' | '`' => Obj::Quote { inner, q: c },
        '(' | ')' | 'b' => Obj::Bracket { inner, open: '(', close: ')' },
        '{' | '}' | 'B' => Obj::Bracket { inner, open: '{', close: '}' },
        '[' | ']' => Obj::Bracket { inner, open: '[', close: ']' },
        '<' | '>' => Obj::Bracket { inner, open: '<', close: '>' },
        _ => return None,
    })
}

fn cls(c: Option<char>, big: bool) -> CharClass {
    match c {
        None | Some('\n') => CharClass::Space,
        Some(c) => {
            let k = char_class(c);
            if big && k == CharClass::Punct { CharClass::Word } else { k }
        }
    }
}

fn word_fwd(t: &Text, p: Pos, big: bool) -> Pos {
    let mut q = p;
    let start_cls = cls(t.char_at(q), big);
    if start_cls != CharClass::Space {
        while q.line == p.line && cls(t.char_at(q), big) == start_cls {
            let nq = t.next_pos(q);
            if nq == q {
                return q;
            }
            q = nq;
        }
    }
    loop {
        match t.char_at(q) {
            None => return q,
            Some('\n') => {
                let nq = t.next_pos(q);
                if nq == q {
                    return q;
                }
                q = nq;
                if t.line_len(q.line) == 0 {
                    return q;
                }
            }
            Some(c) if c.is_whitespace() => {
                let nq = t.next_pos(q);
                if nq == q {
                    return q;
                }
                q = nq;
            }
            _ => return q,
        }
    }
}

fn word_end(t: &Text, p: Pos, big: bool) -> Pos {
    let mut q = t.next_pos(p);
    while cls(t.char_at(q), big) == CharClass::Space {
        let nq = t.next_pos(q);
        if nq == q {
            return q;
        }
        q = nq;
    }
    let c = cls(t.char_at(q), big);
    loop {
        let nq = t.next_pos(q);
        if nq == q || nq.line != q.line || cls(t.char_at(nq), big) != c {
            return q;
        }
        q = nq;
    }
}

fn word_back(t: &Text, p: Pos, big: bool) -> Pos {
    let mut q = t.prev_pos(p);
    if q == p {
        return q;
    }
    while cls(t.char_at(q), big) == CharClass::Space {
        if t.line_len(q.line) == 0 && q != p {
            return q;
        }
        let nq = t.prev_pos(q);
        if nq == q {
            return q;
        }
        q = nq;
    }
    let c = cls(t.char_at(q), big);
    loop {
        if q.col == 0 {
            return q;
        }
        let nq = t.prev_pos(q);
        if cls(t.char_at(nq), big) != c {
            return q;
        }
        q = nq;
    }
}

fn word_end_back(t: &Text, p: Pos, big: bool) -> Pos {
    let start = cls(t.char_at(p), big);
    let mut q = p;
    // leave the current word
    while cls(t.char_at(q), big) == start && start != CharClass::Space {
        let nq = t.prev_pos(q);
        if nq == q {
            return q;
        }
        q = nq;
    }
    while cls(t.char_at(q), big) == CharClass::Space {
        let nq = t.prev_pos(q);
        if nq == q {
            return q;
        }
        q = nq;
    }
    q
}

fn find_char(t: &Text, cur: Pos, kind: FindKind, c: char, n: usize, repeat: bool) -> Option<Pos> {
    let line = t.line(cur.line);
    let mut col = cur.col;
    for i in 0..n {
        match kind {
            FindKind::F | FindKind::T => {
                let mut from = line[col..].chars().next().map(|x| col + x.len_utf8()).unwrap_or(line.len());
                if kind == FindKind::T && (repeat || i > 0) {
                    // skip the char we're already "before"
                    from = line[from..].chars().next().map(|x| from + x.len_utf8()).unwrap_or(line.len());
                }
                let off = line.get(from..)?.find(c)?;
                col = from + off;
            }
            FindKind::BigF | FindKind::BigT => {
                let mut to = col;
                if kind == FindKind::BigT && (repeat || i > 0) {
                    to = prev_boundary(line, to);
                }
                col = line[..to].rfind(c)?;
            }
        }
    }
    let p = match kind {
        FindKind::F | FindKind::BigF => col,
        FindKind::T => prev_boundary(line, col),
        FindKind::BigT => col + c.len_utf8(),
    };
    Some(Pos::new(cur.line, p))
}

/// Range `[a, b)` of a text object around `p`.
fn text_object(t: &Text, p: Pos, o: Obj) -> Option<(Pos, Pos)> {
    match o {
        Obj::Word { inner, big } => {
            let line = t.line(p.line);
            if line.is_empty() {
                return None;
            }
            let at = p.col.min(line.len().saturating_sub(1));
            let c0 = cls(line[at..].chars().next(), big);
            let mut a = at;
            while a > 0 {
                let pc = line[..a].chars().next_back();
                if cls(pc, big) != c0 {
                    break;
                }
                a -= pc.unwrap().len_utf8();
            }
            let mut b = at;
            while b < line.len() {
                let nc = line[b..].chars().next();
                if cls(nc, big) != c0 {
                    break;
                }
                b += nc.unwrap().len_utf8();
            }
            if !inner {
                // include trailing (or else leading) whitespace
                let mut e = b;
                while e < line.len() && line[e..].starts_with([' ', '\t']) {
                    e += 1;
                }
                if e > b {
                    b = e;
                } else {
                    while a > 0 && line[..a].ends_with([' ', '\t']) {
                        a -= 1;
                    }
                }
            }
            Some((Pos::new(p.line, a), Pos::new(p.line, b)))
        }
        Obj::Quote { inner, q } => {
            let line = t.line(p.line);
            let idx: Vec<usize> = line.char_indices().filter(|(_, c)| *c == q).map(|(i, _)| i).collect();
            // find the pair surrounding (or first pair after) the cursor
            let mut pair = None;
            for w in idx.chunks(2) {
                if w.len() == 2 && (p.col <= w[1]) {
                    pair = Some((w[0], w[1]));
                    break;
                }
            }
            let (s, e) = pair?;
            if inner {
                Some((Pos::new(p.line, s + q.len_utf8()), Pos::new(p.line, e)))
            } else {
                Some((Pos::new(p.line, s), Pos::new(p.line, e + q.len_utf8())))
            }
        }
        Obj::Bracket { inner, open, close } => {
            // search backward for an unmatched open bracket
            let mut depth = 0i32;
            let mut q = p;
            let start = loop {
                match t.char_at(q) {
                    Some(c) if c == close && q != p => depth += 1,
                    Some(c) if c == open => {
                        if depth == 0 {
                            break q;
                        }
                        depth -= 1;
                    }
                    _ => {}
                }
                let nq = t.prev_pos(q);
                if nq == q {
                    return None;
                }
                q = nq;
            };
            // forward for its match
            let mut depth = 0i32;
            let mut q = start;
            let end = loop {
                match t.char_at(q) {
                    Some(c) if c == open => depth += 1,
                    Some(c) if c == close => {
                        depth -= 1;
                        if depth == 0 {
                            break q;
                        }
                    }
                    None => return None,
                    _ => {}
                }
                q = t.next_pos(q);
            };
            if inner {
                let a = t.next_pos(start);
                // `ci{` on a multi-line block keeps the brace lines
                let a = if t.char_at(a) == Some('\n') { t.next_pos(a) } else { a };
                let b = if end.col == t.first_non_blank(end.line) && end.line > a.line {
                    Pos::new(end.line - 1, t.line_len(end.line - 1))
                } else {
                    end
                };
                Some((a, b.max(a)))
            } else {
                Some((start, t.next_pos(end)))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::view::Tab;

    fn setup(s: &str) -> (Tab, Vim) {
        let mut d = Document::untitled(1);
        d.text = Text::parse(s).0;
        let mut t = Tab::new(d);
        t.view.height = 20;
        t.view.width = 80;
        (t, Vim::new())
    }

    fn keys(t: &mut Tab, v: &mut Vim, s: &str) {
        for c in s.chars() {
            let k = if c == '\x1b' { KeyEvent::plain(KeyCode::Esc) } else { KeyEvent::plain(KeyCode::Char(c)) };
            let mut ed = t.ed();
            match v.handle(&mut ed, &k) {
                VimOut::Pass => {
                    if let Some(ch) = k.text_char() {
                        ed.type_char(ch, false);
                    }
                }
                VimOut::Replay(ks) => {
                    v.replaying = true;
                    for k in ks {
                        let mut ed = t.ed();
                        if v.handle(&mut ed, &k) == VimOut::Pass
                            && let Some(ch) = k.text_char()
                        {
                            ed.type_char(ch, false);
                        }
                    }
                    v.replaying = false;
                }
                _ => {}
            }
        }
    }

    fn content(t: &Tab) -> String {
        t.doc.text.to_string_with(Default::default())
    }

    #[test]
    fn motions() {
        let (mut t, mut v) = setup("foo bar.baz\n  qux");
        keys(&mut t, &mut v, "w");
        assert_eq!(t.view.cursor, Pos::new(0, 4));
        keys(&mut t, &mut v, "w");
        assert_eq!(t.view.cursor, Pos::new(0, 7));
        keys(&mut t, &mut v, "e");
        assert_eq!(t.view.cursor, Pos::new(0, 10));
        keys(&mut t, &mut v, "j^");
        assert_eq!(t.view.cursor, Pos::new(1, 2));
        keys(&mut t, &mut v, "gg$");
        assert_eq!(t.view.cursor, Pos::new(0, 10));
        keys(&mut t, &mut v, "0fb");
        assert_eq!(t.view.cursor, Pos::new(0, 4));
        keys(&mut t, &mut v, ";");
        assert_eq!(t.view.cursor, Pos::new(0, 8));
        keys(&mut t, &mut v, "b");
        assert_eq!(t.view.cursor, Pos::new(0, 7));
    }

    #[test]
    fn operators() {
        let (mut t, mut v) = setup("one two three\nfour\nfive");
        keys(&mut t, &mut v, "dw");
        assert_eq!(content(&t), "two three\nfour\nfive");
        keys(&mut t, &mut v, "cwTWO\x1b");
        assert_eq!(content(&t), "TWO three\nfour\nfive");
        keys(&mut t, &mut v, "jdd");
        assert_eq!(content(&t), "TWO three\nfive");
        keys(&mut t, &mut v, "P");
        assert_eq!(content(&t), "TWO three\nfour\nfive");
        keys(&mut t, &mut v, "u");
        assert_eq!(content(&t), "TWO three\nfive");
        keys(&mut t, &mut v, "gg2x");
        assert_eq!(content(&t), "O three\nfive");
        keys(&mut t, &mut v, ".");
        assert_eq!(content(&t), "three\nfive");
        keys(&mut t, &mut v, "Gdd");
        assert_eq!(content(&t), "three");
    }

    #[test]
    fn text_objects_and_visual() {
        let (mut t, mut v) = setup("call(foo, \"bar baz\")");
        keys(&mut t, &mut v, "fbci\"X\x1b");
        assert_eq!(content(&t), "call(foo, \"X\")");
        keys(&mut t, &mut v, "0fodi(");
        assert_eq!(content(&t), "call()");
        let (mut t, mut v) = setup("abc\ndef\nghi");
        keys(&mut t, &mut v, "Vjd");
        assert_eq!(content(&t), "ghi");
        let (mut t, mut v) = setup("hello world");
        keys(&mut t, &mut v, "wviwy0P");
        assert_eq!(content(&t), "worldhello world");
        assert_eq!(v.mode, Mode::Normal);
    }

    #[test]
    fn insert_and_open() {
        let (mut t, mut v) = setup("a");
        keys(&mut t, &mut v, "Ab\x1boc\x1bOz\x1b");
        assert_eq!(content(&t), "ab\nz\nc");
        keys(&mut t, &mut v, "ggJ");
        assert_eq!(content(&t), "ab z\nc");
    }
}
