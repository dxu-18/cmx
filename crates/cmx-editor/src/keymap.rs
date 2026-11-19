//! Commands and default (VS Code-style) key bindings. Ctrl is the command modifier on every
//! platform; Cmd (super) is left to macOS and the terminal app (EDITOR-DESIGN D-16).

use cmx_tui::{KeyCode, KeyEvent, Mods};
use std::collections::HashMap;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Cmd {
    Quit,
    Save,
    SaveAs,
    NewFile,
    OpenFile,
    CloseTab,
    NextTab,
    PrevTab,
    GotoTab(u8),
    QuickOpen,
    CommandPalette,
    Help,
    ToggleTree,
    ToggleTerminal,
    ToggleClaude,
    SplitTerminal,
    NewTerminal,
    KillTerminal,
    RestartClaude,
    CycleFocus,
    FocusEditor,
    RevealInTree,
    RefreshTree,
    ToggleVim,
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    SelectAll,
    Find,
    FindNext,
    FindPrev,
    Replace,
    GotoLine,
    SelectLine,
    ToggleComment,
    MoveLineUp,
    MoveLineDown,
    CopyLineUp,
    CopyLineDown,
    DeleteLine,
    InsertLineBelow,
    InsertLineAbove,
    Indent,
    Outdent,
    SelectNextOccurrence,
    JumpBracket,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    DocStart,
    DocEnd,
    WordLeft,
    WordRight,
    SelLeft,
    SelRight,
    SelUp,
    SelDown,
    SelHome,
    SelEnd,
    SelPageUp,
    SelPageDown,
    SelDocStart,
    SelDocEnd,
    SelWordLeft,
    SelWordRight,
    Backspace,
    Delete,
    DeleteWordLeft,
    DeleteWordRight,
    DeleteToLineStart,
    DeleteToLineEnd,
    Newline,
    Tab,
    BackTab,
    Escape,
    ScrollUp,
    ScrollDown,
    StexCompile,
    StexCheck,
    ToggleWrap,
    AddCursorAbove,
    AddCursorBelow,
    SelectAllOccurrences,
    ReloadFile,
    EnableShiftDetection,
}

/// Commands listed in the command palette (label, command).
pub const PALETTE: &[(&str, Cmd)] = &[
    ("File: Save", Cmd::Save),
    ("File: Save As…", Cmd::SaveAs),
    ("File: New Untitled File", Cmd::NewFile),
    ("File: Open File…", Cmd::OpenFile),
    ("File: Quick Open", Cmd::QuickOpen),
    ("View: Close Tab", Cmd::CloseTab),
    ("View: Next Tab", Cmd::NextTab),
    ("View: Previous Tab", Cmd::PrevTab),
    ("View: Toggle Explorer", Cmd::ToggleTree),
    ("View: Toggle Terminal", Cmd::ToggleTerminal),
    ("View: Toggle Claude", Cmd::ToggleClaude),
    ("View: Cycle Focus", Cmd::CycleFocus),
    ("View: Focus Editor", Cmd::FocusEditor),
    ("Explorer: Reveal Active File", Cmd::RevealInTree),
    ("Explorer: Refresh", Cmd::RefreshTree),
    ("Terminal: New Terminal", Cmd::NewTerminal),
    ("Terminal: Split Terminal", Cmd::SplitTerminal),
    ("Terminal: Kill Active Terminal", Cmd::KillTerminal),
    ("Claude: Restart", Cmd::RestartClaude),
    ("Edit: Undo", Cmd::Undo),
    ("Edit: Redo", Cmd::Redo),
    ("Edit: Cut", Cmd::Cut),
    ("Edit: Copy", Cmd::Copy),
    ("Edit: Paste", Cmd::Paste),
    ("Edit: Select All", Cmd::SelectAll),
    ("Edit: Find", Cmd::Find),
    ("Edit: Find Next", Cmd::FindNext),
    ("Edit: Find Previous", Cmd::FindPrev),
    ("Edit: Replace", Cmd::Replace),
    ("Go: Go to Line…", Cmd::GotoLine),
    ("Go: Matching Bracket", Cmd::JumpBracket),
    ("Edit: Select Line", Cmd::SelectLine),
    ("Edit: Select Next Occurrence", Cmd::SelectNextOccurrence),
    ("Edit: Toggle Line Comment", Cmd::ToggleComment),
    ("Edit: Move Line Up", Cmd::MoveLineUp),
    ("Edit: Move Line Down", Cmd::MoveLineDown),
    ("Edit: Copy Line Up", Cmd::CopyLineUp),
    ("Edit: Copy Line Down", Cmd::CopyLineDown),
    ("Edit: Delete Line", Cmd::DeleteLine),
    ("Edit: Insert Line Below", Cmd::InsertLineBelow),
    ("Edit: Insert Line Above", Cmd::InsertLineAbove),
    ("Edit: Indent", Cmd::Indent),
    ("Edit: Outdent", Cmd::Outdent),
    ("Edit: Add Cursor Above", Cmd::AddCursorAbove),
    ("Edit: Add Cursor Below", Cmd::AddCursorBelow),
    ("Edit: Select All Occurrences", Cmd::SelectAllOccurrences),
    ("View: Toggle Word Wrap", Cmd::ToggleWrap),
    ("File: Reload from Disk (discard changes)", Cmd::ReloadFile),
    ("Keyboard: Detect Ctrl+Shift in This Terminal (macOS Input Monitoring)", Cmd::EnableShiftDetection),
    ("Preferences: Toggle Vim Mode", Cmd::ToggleVim),
    ("stex: Compile to LaTeX/PDF (cmx stex compile)", Cmd::StexCompile),
    ("stex: Check with Lean (cmx stex check)", Cmd::StexCheck),
    ("Help: Keyboard Shortcuts", Cmd::Help),
    ("Quit cmx", Cmd::Quit),
];

/// Letters for which an ambiguous (legacy) Ctrl+X is treated as Ctrl+Shift+X.
const PROMOTE: [char; 4] = ['l', 'r', 'y', 'p'];

pub struct Keymap {
    map: HashMap<(KeyCode, Mods), Cmd>,
}

fn k(code: KeyCode, mods: Mods) -> (KeyCode, Mods) {
    (code, mods)
}

impl Default for Keymap {
    fn default() -> Self {
        use Cmd::*;
        use KeyCode as K;
        let c = Mods::CTRL;
        let s = Mods::SHIFT;
        let a = Mods::ALT;
        let cs = Mods::CTRL_SHIFT;
        let as_ = Mods::ALT | Mods::SHIFT;
        let n = Mods::NONE;
        let ch = K::Char;
        let mut m = HashMap::new();
        let mut b = |key: (KeyCode, Mods), cmd: Cmd| {
            m.insert(key, cmd);
        };
        // files / tabs / app
        b(k(ch('q'), c), Quit);
        b(k(ch('q'), cs), Quit);
        b(k(ch('s'), c), Save);
        b(k(ch('s'), cs), SaveAs);
        b(k(ch('n'), c), NewFile);
        b(k(ch('o'), c), OpenFile);
        b(k(ch('w'), c), CloseTab);
        b(k(K::PageDown, c), NextTab);
        b(k(K::PageUp, c), PrevTab);
        for d in 1..=9u8 {
            b(k(ch((b'0' + d) as char), a), GotoTab(d));
        }
        b(k(ch('p'), c), QuickOpen);
        b(k(ch('p'), cs), CommandPalette);
        b(k(K::F(1), n), Help);
        // panels
        b(k(ch('l'), cs), ToggleTree);
        b(k(ch('b'), c), ToggleTree);
        b(k(ch('y'), cs), ToggleTerminal);
        b(k(ch('`'), c), ToggleTerminal);
        b(k(ch(' '), c), ToggleTerminal);
        b(k(ch('j'), c), ToggleTerminal);
        b(k(ch('r'), cs), ToggleClaude);
        b(k(ch('5'), cs), SplitTerminal);
        b(k(ch('`'), cs), NewTerminal);
        b(k(K::F(6), n), CycleFocus);
        b(k(ch('e'), cs), RevealInTree);
        b(k(ch('k'), c), ToggleVim);
        // edit
        b(k(ch('z'), c), Undo);
        b(k(ch('z'), cs), Redo);
        b(k(ch('y'), c), Redo);
        b(k(ch('z'), as_), Redo);
        b(k(ch('z'), a), ToggleWrap);
        b(k(K::F(4), n), Redo);
        b(k(ch('x'), c), Cut);
        b(k(ch('c'), c), Copy);
        b(k(ch('v'), c), Paste);
        b(k(K::Delete, s), Cut);
        b(k(K::Insert, c), Copy);
        b(k(K::Insert, s), Paste);
        b(k(ch('a'), c), SelectAll);
        b(k(ch('f'), c), Find);
        b(k(K::F(3), n), FindNext);
        b(k(K::F(3), s), FindPrev);
        b(k(ch('h'), c), Replace);
        b(k(ch('g'), c), GotoLine);
        b(k(ch('l'), c), SelectLine);
        b(k(ch('/'), c), ToggleComment);
        b(k(K::Up, a), MoveLineUp);
        b(k(K::Down, a), MoveLineDown);
        b(k(K::Up, as_), CopyLineUp);
        b(k(K::Down, as_), CopyLineDown);
        b(k(ch('k'), cs), DeleteLine);
        b(k(K::Enter, c), InsertLineBelow);
        b(k(K::Enter, cs), InsertLineAbove);
        b(k(ch(']'), c), Indent);
        b(k(ch('d'), c), SelectNextOccurrence);
        b(k(K::F(2), c), SelectAllOccurrences);
        b(k(K::Up, c | a), AddCursorAbove);
        b(k(K::Down, c | a), AddCursorBelow);
        b(k(ch('\\'), cs), JumpBracket);
        // movement
        for (code, plain, sel) in [
            (K::Left, Left, SelLeft),
            (K::Right, Right, SelRight),
            (K::Up, Up, SelUp),
            (K::Down, Down, SelDown),
            (K::Home, Home, SelHome),
            (K::End, End, SelEnd),
            (K::PageUp, PageUp, SelPageUp),
            (K::PageDown, PageDown, SelPageDown),
        ] {
            b(k(code, n), plain);
            b(k(code, s), sel);
        }
        b(k(K::Home, c), DocStart);
        b(k(K::End, c), DocEnd);
        b(k(K::Home, cs), SelDocStart);
        b(k(K::End, cs), SelDocEnd);
        // Ctrl+arrows: line start/end and document start/end (Ctrl stands in for macOS Cmd);
        // Alt (Option) +arrows move by words
        b(k(K::Left, c), Home);
        b(k(K::Right, c), End);
        b(k(K::Up, c), DocStart);
        b(k(K::Down, c), DocEnd);
        b(k(K::Left, a), WordLeft);
        b(k(K::Right, a), WordRight);
        b(k(ch('b'), a), WordLeft);
        b(k(ch('f'), a), WordRight);
        b(k(K::Left, cs), SelHome);
        b(k(K::Right, cs), SelEnd);
        b(k(K::Up, cs), SelDocStart);
        b(k(K::Down, cs), SelDocEnd);
        b(k(K::Left, as_), SelWordLeft);
        b(k(K::Right, as_), SelWordRight);
        // deletion / insertion
        b(k(K::Backspace, n), Backspace);
        b(k(K::Backspace, s), Backspace);
        b(k(K::Delete, n), Delete);
        b(k(K::Backspace, c), DeleteToLineStart);
        b(k(K::Backspace, a), DeleteWordLeft);
        b(k(ch('w'), c | a), DeleteWordLeft);
        b(k(K::Delete, c), DeleteToLineEnd);
        b(k(K::Delete, a), DeleteWordRight);
        b(k(ch('d'), a), DeleteWordRight);
        b(k(K::Enter, n), Newline);
        b(k(K::Enter, s), Newline);
        b(k(K::Tab, n), Tab);
        b(k(K::BackTab, n), BackTab);
        b(k(K::Esc, n), Escape);
        Keymap { map: m }
    }
}

impl Keymap {
    /// Look up a key. `promote` enables the legacy Ctrl→Ctrl+Shift fallback (EDITOR-DESIGN D-3).
    /// Chords with Cmd (super) are never cmx commands: Cmd belongs to macOS / the terminal app.
    pub fn lookup(&self, ev: &KeyEvent, promote: bool) -> Option<Cmd> {
        if ev.mods.sup() {
            return None;
        }
        let (code, mods) = (ev.code, ev.mods);
        if promote
            && ev.ambiguous
            && mods == Mods::CTRL
            && let KeyCode::Char(c) = code
            && PROMOTE.contains(&c)
            && let Some(cmd) = self.map.get(&(code, Mods::CTRL_SHIFT))
        {
            return Some(*cmd);
        }
        self.map.get(&(code, mods)).copied()
    }

    /// Human-readable binding for a command (first match, preferring Ctrl+Shift chords).
    pub fn label(&self, cmd: Cmd) -> Option<String> {
        let mut best: Option<(KeyCode, Mods)> = None;
        for (key, c) in &self.map {
            if *c == cmd {
                let better = match best {
                    None => true,
                    Some(b) => key_rank(key) < key_rank(&b),
                };
                if better {
                    best = Some(*key);
                }
            }
        }
        best.map(|(code, mods)| key_label(code, mods))
    }
}

fn key_rank(k: &(KeyCode, Mods)) -> (u8, u8, String) {
    let pref = match k.1 {
        m if m == Mods::CTRL_SHIFT => 0,
        m if m == Mods::CTRL => 1,
        m if m == Mods::NONE => 2,
        _ => 3,
    };
    let ch = matches!(k.0, KeyCode::Char(c) if c == ' ' || c == '`' || c == 'j' || c == 'b' || c == 'y');
    (pref, ch as u8, format!("{:?}", k.0))
}

pub fn key_label(code: KeyCode, mods: Mods) -> String {
    let mut s = String::new();
    if mods.ctrl() {
        s.push_str("Ctrl+");
    }
    if mods.alt() {
        s.push_str("Alt+");
    }
    if mods.shift() {
        s.push_str("Shift+");
    }
    if mods.sup() {
        s.push_str("Cmd+");
    }
    match code {
        KeyCode::Char(' ') => s.push_str("Space"),
        KeyCode::Char(c) => s.push(c.to_ascii_uppercase()),
        KeyCode::F(n) => s.push_str(&format!("F{n}")),
        KeyCode::PageUp => s.push_str("PgUp"),
        KeyCode::PageDown => s.push_str("PgDn"),
        KeyCode::BackTab => s.push_str("Shift+Tab"),
        other => s.push_str(&format!("{other:?}")),
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn promotion_and_shift_chords() {
        let km = Keymap::default();
        let legacy_l = KeyEvent { code: KeyCode::Char('l'), mods: Mods::CTRL, ambiguous: true };
        assert_eq!(km.lookup(&legacy_l, true), Some(Cmd::ToggleTree));
        assert_eq!(km.lookup(&legacy_l, false), Some(Cmd::SelectLine));
        let kitty_l = KeyEvent::new(KeyCode::Char('l'), Mods::CTRL);
        assert_eq!(km.lookup(&kitty_l, true), Some(Cmd::SelectLine));
        let kitty_shift_l = KeyEvent::new(KeyCode::Char('l'), Mods::CTRL_SHIFT);
        assert_eq!(km.lookup(&kitty_shift_l, true), Some(Cmd::ToggleTree));
        let legacy_z = KeyEvent { code: KeyCode::Char('z'), mods: Mods::CTRL, ambiguous: true };
        assert_eq!(km.lookup(&legacy_z, true), Some(Cmd::Undo));
        let redo = KeyEvent::new(KeyCode::Char('z'), Mods::CTRL_SHIFT);
        assert_eq!(km.lookup(&redo, true), Some(Cmd::Redo));
        // Cmd is reserved for macOS
        assert_eq!(km.lookup(&KeyEvent::new(KeyCode::Char('s'), Mods::SUPER), true), None);
        assert_eq!(km.label(Cmd::ToggleTree).as_deref(), Some("Ctrl+Shift+L"));
        assert_eq!(km.label(Cmd::Redo).as_deref(), Some("Ctrl+Shift+Z"));
    }
}
