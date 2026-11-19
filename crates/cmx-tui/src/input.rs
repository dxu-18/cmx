//! Terminal input parser: bytes → key / mouse / paste / focus / protocol-reply events.
//!
//! Understands legacy control bytes, `ESC x` (Alt), SS3 (`ESC O x`), CSI `~` keys, xterm
//! modifyOtherKeys (`CSI 27;m;k~`), the kitty keyboard protocol (`CSI k;m u`), SGR and X10 mouse,
//! bracketed paste, focus events and the DA1 / kitty-flags query replies.

use std::ops::{BitAnd, BitOr, BitOrAssign};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum KeyCode {
    Char(char),
    Enter,
    Tab,
    BackTab,
    Backspace,
    Esc,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    Delete,
    F(u8),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, Default, PartialOrd, Ord)]
pub struct Mods(pub u8);

impl Mods {
    pub const NONE: Mods = Mods(0);
    pub const SHIFT: Mods = Mods(1);
    pub const ALT: Mods = Mods(2);
    pub const CTRL: Mods = Mods(4);
    pub const SUPER: Mods = Mods(8);
    pub const CTRL_SHIFT: Mods = Mods(5);

    #[inline]
    pub fn contains(self, o: Mods) -> bool {
        self.0 & o.0 == o.0
    }
    #[inline]
    pub fn shift(self) -> bool {
        self.0 & 1 != 0
    }
    #[inline]
    pub fn alt(self) -> bool {
        self.0 & 2 != 0
    }
    #[inline]
    pub fn ctrl(self) -> bool {
        self.0 & 4 != 0
    }
    #[inline]
    pub fn sup(self) -> bool {
        self.0 & 8 != 0
    }
    #[inline]
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }
    pub fn without(self, o: Mods) -> Mods {
        Mods(self.0 & !o.0)
    }
    /// xterm modifier parameter (1 + bits).
    pub fn xterm_param(self) -> u32 {
        1 + (self.0 & 0xF) as u32
    }
}

impl BitOr for Mods {
    type Output = Mods;
    fn bitor(self, o: Mods) -> Mods {
        Mods(self.0 | o.0)
    }
}
impl BitOrAssign for Mods {
    fn bitor_assign(&mut self, o: Mods) {
        self.0 |= o.0
    }
}
impl BitAnd for Mods {
    type Output = Mods;
    fn bitand(self, o: Mods) -> Mods {
        Mods(self.0 & o.0)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub struct KeyEvent {
    pub code: KeyCode,
    pub mods: Mods,
    /// True when the key arrived as a legacy control byte, so Shift could not be reported
    /// (e.g. Ctrl+Shift+L and Ctrl+L both send 0x0C).
    pub ambiguous: bool,
}

impl KeyEvent {
    pub const fn new(code: KeyCode, mods: Mods) -> KeyEvent {
        KeyEvent { code, mods, ambiguous: false }
    }
    pub const fn plain(code: KeyCode) -> KeyEvent {
        KeyEvent { code, mods: Mods::NONE, ambiguous: false }
    }
    pub const fn ctrl(c: char) -> KeyEvent {
        KeyEvent { code: KeyCode::Char(c), mods: Mods::CTRL, ambiguous: false }
    }
    fn legacy(code: KeyCode, mods: Mods) -> KeyEvent {
        KeyEvent { code, mods, ambiguous: true }
    }
    /// Printable character typed without ctrl/alt/super.
    pub fn text_char(&self) -> Option<char> {
        match self.code {
            KeyCode::Char(c) if !self.mods.ctrl() && !self.mods.alt() && !self.mods.sup() => Some(c),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    Other,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MouseKind {
    Down(MouseButton),
    Up(MouseButton),
    Drag(MouseButton),
    Moved,
    ScrollUp,
    ScrollDown,
    ScrollLeft,
    ScrollRight,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct MouseEvent {
    pub kind: MouseKind,
    pub x: u16,
    pub y: u16,
    pub mods: Mods,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Event {
    Key(KeyEvent),
    Mouse(MouseEvent),
    Paste(String),
    Focus(bool),
    /// Reply to `CSI ? u`: the terminal speaks the kitty keyboard protocol.
    KittyFlags(u32),
    /// Reply to DA1 (`CSI c`).
    DeviceAttributes,
}

enum Parsed {
    Ev(Event, usize),
    Skip(usize),
    PasteStart(usize),
    Incomplete,
}

#[derive(Default)]
pub struct InputParser {
    buf: Vec<u8>,
    in_paste: bool,
    paste: Vec<u8>,
}

const PASTE_END: &[u8] = b"\x1b[201~";

fn find(h: &[u8], n: &[u8]) -> Option<usize> {
    h.windows(n.len()).position(|w| w == n)
}

fn key(code: KeyCode, mods: Mods) -> Event {
    Event::Key(KeyEvent::new(code, mods))
}

fn mods_from(p: u32) -> Mods {
    let m = p.saturating_sub(1);
    let mut r = Mods::NONE;
    if m & 1 != 0 {
        r |= Mods::SHIFT;
    }
    if m & (2 | 32) != 0 {
        r |= Mods::ALT;
    }
    if m & 4 != 0 {
        r |= Mods::CTRL;
    }
    if m & (8 | 16) != 0 {
        r |= Mods::SUPER;
    }
    r
}

fn add_mods(ev: Event, extra: Mods) -> Event {
    match ev {
        Event::Key(mut k) => {
            k.mods |= extra;
            Event::Key(k)
        }
        e => e,
    }
}

/// Map a codepoint (kitty / modifyOtherKeys) to a key code.
fn code_to_key(code: u32) -> Option<KeyCode> {
    Some(match code {
        9 => KeyCode::Tab,
        13 => KeyCode::Enter,
        27 => KeyCode::Esc,
        8 | 127 => KeyCode::Backspace,
        57399..=57408 => KeyCode::Char(char::from_digit(code - 57399, 10)?),
        57409 => KeyCode::Char('.'),
        57410 => KeyCode::Char('/'),
        57411 => KeyCode::Char('*'),
        57412 => KeyCode::Char('-'),
        57413 => KeyCode::Char('+'),
        57414 => KeyCode::Enter,
        57415 => KeyCode::Char('='),
        57417 => KeyCode::Left,
        57418 => KeyCode::Right,
        57419 => KeyCode::Up,
        57420 => KeyCode::Down,
        57421 => KeyCode::PageUp,
        57422 => KeyCode::PageDown,
        57423 => KeyCode::Home,
        57424 => KeyCode::End,
        57425 => KeyCode::Insert,
        57426 => KeyCode::Delete,
        57364..=57398 => KeyCode::F((code - 57364 + 13) as u8),
        57344..=63743 => return None, // other private-use functional keys (modifiers, media...)
        _ => KeyCode::Char(char::from_u32(code)?),
    })
}

/// Canonical form: with ctrl/alt/super held, letters are lowercase and Shift is a modifier;
/// with only Shift held, the character itself carries the shift (no SHIFT modifier).
fn normalise(code: KeyCode, mods: Mods) -> (KeyCode, Mods) {
    if let KeyCode::Char(c) = code {
        if mods.ctrl() || mods.alt() || mods.sup() {
            if c.is_ascii_uppercase() {
                return (KeyCode::Char(c.to_ascii_lowercase()), mods | Mods::SHIFT);
            }
            return (code, mods);
        }
        if mods.shift() {
            if c.is_ascii_lowercase() {
                return (KeyCode::Char(c.to_ascii_uppercase()), mods.without(Mods::SHIFT));
            }
            return (code, mods.without(Mods::SHIFT));
        }
    }
    if code == KeyCode::Tab && mods.shift() {
        return (KeyCode::BackTab, mods.without(Mods::SHIFT));
    }
    (code, mods)
}

fn decode_mouse(cb: u32, x: u32, y: u32, release: bool) -> Event {
    let mut mods = Mods::NONE;
    if cb & 4 != 0 {
        mods |= Mods::SHIFT;
    }
    if cb & 8 != 0 {
        mods |= Mods::ALT;
    }
    if cb & 16 != 0 {
        mods |= Mods::CTRL;
    }
    let motion = cb & 32 != 0;
    let wheel = cb & 64 != 0;
    let low = cb & 3;
    let button = match (low, cb & 128 != 0) {
        (0, false) => MouseButton::Left,
        (1, false) => MouseButton::Middle,
        (2, false) => MouseButton::Right,
        _ => MouseButton::Other,
    };
    let kind = if wheel {
        match low {
            0 => MouseKind::ScrollUp,
            1 => MouseKind::ScrollDown,
            2 => MouseKind::ScrollLeft,
            _ => MouseKind::ScrollRight,
        }
    } else if motion {
        if low == 3 { MouseKind::Moved } else { MouseKind::Drag(button) }
    } else if release || low == 3 {
        MouseKind::Up(if low == 3 { MouseButton::Left } else { button })
    } else {
        MouseKind::Down(button)
    };
    Event::Mouse(MouseEvent {
        kind,
        x: x.saturating_sub(1).min(u16::MAX as u32) as u16,
        y: y.saturating_sub(1).min(u16::MAX as u32) as u16,
        mods,
    })
}

impl InputParser {
    pub fn new() -> Self {
        Self::default()
    }

    /// True when bytes are buffered waiting for the rest of a sequence (e.g. a lone ESC).
    pub fn has_pending(&self) -> bool {
        !self.buf.is_empty() && !self.in_paste
    }

    pub fn feed(&mut self, data: &[u8], out: &mut Vec<Event>) {
        self.buf.extend_from_slice(data);
        self.run(out, false);
    }

    /// Resolve buffered partial sequences (call after a short timeout with no new input).
    pub fn flush(&mut self, out: &mut Vec<Event>) {
        self.run(out, true);
    }

    fn run(&mut self, out: &mut Vec<Event>, force: bool) {
        let mut i = 0;
        let len = self.buf.len();
        loop {
            if self.in_paste {
                if let Some(p) = find(&self.buf[i..], PASTE_END) {
                    self.paste.extend_from_slice(&self.buf[i..i + p]);
                    let text = String::from_utf8_lossy(&self.paste).replace("\r\n", "\n").replace('\r', "\n");
                    self.paste.clear();
                    out.push(Event::Paste(text));
                    self.in_paste = false;
                    i += p + PASTE_END.len();
                    continue;
                }
                let keep = (len - i).min(PASTE_END.len() - 1);
                self.paste.extend_from_slice(&self.buf[i..len - keep]);
                i = len - keep;
                break;
            }
            if i >= len {
                break;
            }
            match Self::parse_one(&self.buf[i..], force) {
                Parsed::Ev(e, n) => {
                    out.push(e);
                    i += n;
                }
                Parsed::Skip(n) => i += n.max(1),
                Parsed::PasteStart(n) => {
                    self.in_paste = true;
                    self.paste.clear();
                    i += n;
                }
                Parsed::Incomplete => break,
            }
        }
        self.buf.drain(..i);
    }

    fn parse_one(b: &[u8], force: bool) -> Parsed {
        let c = b[0];
        match c {
            0x1b => Self::parse_esc(b, force),
            0x00 => Parsed::Ev(Event::Key(KeyEvent::legacy(KeyCode::Char(' '), Mods::CTRL)), 1),
            0x08 => Parsed::Ev(Event::Key(KeyEvent::legacy(KeyCode::Backspace, Mods::CTRL)), 1),
            0x09 => Parsed::Ev(Event::Key(KeyEvent::legacy(KeyCode::Tab, Mods::NONE)), 1),
            0x0d => Parsed::Ev(Event::Key(KeyEvent::legacy(KeyCode::Enter, Mods::NONE)), 1),
            0x0a => Parsed::Ev(Event::Key(KeyEvent::legacy(KeyCode::Char('j'), Mods::CTRL)), 1),
            0x01..=0x1a => {
                Parsed::Ev(Event::Key(KeyEvent::legacy(KeyCode::Char((b'a' + c - 1) as char), Mods::CTRL)), 1)
            }
            0x1c => Parsed::Ev(Event::Key(KeyEvent::legacy(KeyCode::Char('\\'), Mods::CTRL)), 1),
            0x1d => Parsed::Ev(Event::Key(KeyEvent::legacy(KeyCode::Char(']'), Mods::CTRL)), 1),
            0x1e => Parsed::Ev(Event::Key(KeyEvent::legacy(KeyCode::Char('^'), Mods::CTRL)), 1),
            0x1f => Parsed::Ev(Event::Key(KeyEvent::legacy(KeyCode::Char('/'), Mods::CTRL)), 1),
            0x7f => Parsed::Ev(Event::Key(KeyEvent::legacy(KeyCode::Backspace, Mods::NONE)), 1),
            _ => {
                let need = match c {
                    0x00..=0x7f => 1,
                    0xc0..=0xdf => 2,
                    0xe0..=0xef => 3,
                    0xf0..=0xf7 => 4,
                    _ => return Parsed::Skip(1),
                };
                if b.len() < need {
                    return if force { Parsed::Skip(b.len()) } else { Parsed::Incomplete };
                }
                match std::str::from_utf8(&b[..need]) {
                    Ok(s) => {
                        let ch = s.chars().next().unwrap();
                        Parsed::Ev(key(KeyCode::Char(ch), Mods::NONE), need)
                    }
                    Err(_) => Parsed::Skip(1),
                }
            }
        }
    }

    fn parse_esc(b: &[u8], force: bool) -> Parsed {
        if b.len() == 1 {
            return if force { Parsed::Ev(key(KeyCode::Esc, Mods::NONE), 1) } else { Parsed::Incomplete };
        }
        match b[1] {
            b'[' => Self::parse_csi(b, force),
            b'O' => {
                if b.len() < 3 {
                    return if force { Parsed::Ev(key(KeyCode::Char('O'), Mods::ALT), 2) } else { Parsed::Incomplete };
                }
                let k = match b[2] {
                    b'A' => KeyCode::Up,
                    b'B' => KeyCode::Down,
                    b'C' => KeyCode::Right,
                    b'D' => KeyCode::Left,
                    b'H' => KeyCode::Home,
                    b'F' => KeyCode::End,
                    b'P' => KeyCode::F(1),
                    b'Q' => KeyCode::F(2),
                    b'R' => KeyCode::F(3),
                    b'S' => KeyCode::F(4),
                    b'M' => KeyCode::Enter,
                    b'j' => KeyCode::Char('*'),
                    b'k' => KeyCode::Char('+'),
                    b'l' => KeyCode::Char(','),
                    b'm' => KeyCode::Char('-'),
                    b'n' => KeyCode::Char('.'),
                    b'o' => KeyCode::Char('/'),
                    b'X' => KeyCode::Char('='),
                    d @ b'p'..=b'y' => KeyCode::Char((b'0' + d - b'p') as char),
                    _ => return Parsed::Skip(3),
                };
                Parsed::Ev(key(k, Mods::NONE), 3)
            }
            0x1b => Parsed::Ev(key(KeyCode::Esc, Mods::NONE), 1),
            _ => match Self::parse_one(&b[1..], force) {
                Parsed::Ev(e @ Event::Key(_), n) => {
                    // Alt+Shift+letter arrives as ESC + uppercase letter.
                    let e = match add_mods(e, Mods::ALT) {
                        Event::Key(k) => {
                            let (code, mods) = normalise(k.code, k.mods);
                            Event::Key(KeyEvent { code, mods, ambiguous: k.ambiguous && k.mods.ctrl() })
                        }
                        e => e,
                    };
                    Parsed::Ev(e, n + 1)
                }
                Parsed::Incomplete => Parsed::Incomplete,
                _ => Parsed::Ev(key(KeyCode::Esc, Mods::NONE), 1),
            },
        }
    }

    fn parse_csi(b: &[u8], force: bool) -> Parsed {
        // X10 mouse: ESC [ M cb cx cy
        if b.len() >= 3 && b[2] == b'M' {
            if b.len() < 6 {
                return if force { Parsed::Skip(b.len()) } else { Parsed::Incomplete };
            }
            let cb = (b[3] as u32).saturating_sub(32);
            let x = (b[4] as u32).saturating_sub(32);
            let y = (b[5] as u32).saturating_sub(32);
            return Parsed::Ev(decode_mouse(cb, x, y, false), 6);
        }
        let mut j = 2;
        while j < b.len() {
            let c = b[j];
            if (0x40..=0x7e).contains(&c) {
                break;
            }
            if !(0x20..=0x3f).contains(&c) {
                return Parsed::Skip(j);
            }
            j += 1;
            if j > 512 {
                return Parsed::Skip(j);
            }
        }
        if j >= b.len() {
            return if force { Parsed::Ev(key(KeyCode::Char('['), Mods::ALT), 2) } else { Parsed::Incomplete };
        }
        let fin = b[j];
        let n = j + 1;
        let mut body = &b[2..j];
        let private = match body.first() {
            Some(&p @ (b'<' | b'=' | b'>' | b'?')) => {
                body = &body[1..];
                Some(p)
            }
            _ => None,
        };
        let inter_start = body.iter().position(|c| (0x20..=0x2f).contains(c)).unwrap_or(body.len());
        let inter = &body[inter_start..];
        let body = &body[..inter_start];
        // params: Vec of (Vec of sub-params)
        let mut params: Vec<[u32; 3]> = Vec::with_capacity(4);
        if !body.is_empty() {
            for part in body.split(|&c| c == b';') {
                let mut sub = [0u32; 3];
                for (k, s) in part.split(|&c| c == b':').enumerate() {
                    if k < 3 {
                        let mut v: u32 = 0;
                        for &d in s {
                            if d.is_ascii_digit() {
                                v = v.saturating_mul(10).saturating_add((d - b'0') as u32);
                            }
                        }
                        sub[k] = v;
                    }
                }
                params.push(sub);
            }
        }
        let p = |i: usize, d: u32| -> u32 {
            match params.get(i) {
                Some(s) if s[0] != 0 => s[0],
                _ => d,
            }
        };

        match (private, fin) {
            (Some(b'<'), b'M') | (Some(b'<'), b'm') => {
                Parsed::Ev(decode_mouse(p(0, 0), p(1, 1), p(2, 1), fin == b'm'), n)
            }
            (Some(b'?'), b'u') => Parsed::Ev(Event::KittyFlags(p(0, 0)), n),
            (Some(b'?'), b'c') => Parsed::Ev(Event::DeviceAttributes, n),
            (Some(_), _) => Parsed::Skip(n),
            (None, _) if !inter.is_empty() => Parsed::Skip(n),
            (None, b'~') => {
                let code = p(0, 0);
                let mods = mods_from(p(1, 1));
                let k = match code {
                    1 | 7 => KeyCode::Home,
                    2 => KeyCode::Insert,
                    3 => KeyCode::Delete,
                    4 | 8 => KeyCode::End,
                    5 => KeyCode::PageUp,
                    6 => KeyCode::PageDown,
                    11..=15 => KeyCode::F((code - 10) as u8),
                    17..=21 => KeyCode::F((code - 11) as u8),
                    23..=26 => KeyCode::F((code - 12) as u8),
                    28 | 29 => KeyCode::F((code - 13) as u8),
                    31..=34 => KeyCode::F((code - 14) as u8),
                    200 => return Parsed::PasteStart(n),
                    201 => return Parsed::Skip(n),
                    27 => {
                        // modifyOtherKeys: CSI 27 ; mods ; code ~
                        let mods = mods_from(p(1, 1));
                        return match code_to_key(p(2, 0)) {
                            Some(k) => {
                                let (k, m) = normalise(k, mods);
                                Parsed::Ev(key(k, m), n)
                            }
                            None => Parsed::Skip(n),
                        };
                    }
                    _ => return Parsed::Skip(n),
                };
                Parsed::Ev(key(k, mods), n)
            }
            (None, b'u') => {
                let code = p(0, 0);
                let mods = mods_from(p(1, 1));
                let event_type = params.get(1).map(|s| s[1]).unwrap_or(0);
                if event_type == 3 {
                    return Parsed::Skip(n);
                }
                match code_to_key(code) {
                    Some(k) => {
                        let (k, m) = normalise(k, mods);
                        Parsed::Ev(key(k, m), n)
                    }
                    None => Parsed::Skip(n),
                }
            }
            (None, b'I') if body.is_empty() => Parsed::Ev(Event::Focus(true), n),
            (None, b'O') if body.is_empty() => Parsed::Ev(Event::Focus(false), n),
            (None, b'Z') => Parsed::Ev(key(KeyCode::BackTab, mods_from(p(1, 1)).without(Mods::SHIFT)), n),
            (None, b'R') if params.len() >= 2 && p(0, 1) != 1 => Parsed::Skip(n), // cursor position report
            (None, f @ (b'A' | b'B' | b'C' | b'D' | b'H' | b'F' | b'P' | b'Q' | b'R' | b'S')) => {
                let k = match f {
                    b'A' => KeyCode::Up,
                    b'B' => KeyCode::Down,
                    b'C' => KeyCode::Right,
                    b'D' => KeyCode::Left,
                    b'H' => KeyCode::Home,
                    b'F' => KeyCode::End,
                    b'P' => KeyCode::F(1),
                    b'Q' => KeyCode::F(2),
                    b'R' => KeyCode::F(3),
                    _ => KeyCode::F(4),
                };
                Parsed::Ev(key(k, mods_from(p(1, 1))), n)
            }
            _ => Parsed::Skip(n),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(bytes: &[u8]) -> Vec<Event> {
        let mut p = InputParser::new();
        let mut out = Vec::new();
        p.feed(bytes, &mut out);
        p.flush(&mut out);
        out
    }

    fn k(code: KeyCode, mods: Mods) -> Event {
        Event::Key(KeyEvent::new(code, mods))
    }

    #[test]
    fn basics() {
        assert_eq!(parse(b"a"), vec![k(KeyCode::Char('a'), Mods::NONE)]);
        assert_eq!(parse("é".as_bytes()), vec![k(KeyCode::Char('é'), Mods::NONE)]);
        assert_eq!(
            parse(b"\x0c"),
            vec![Event::Key(KeyEvent { code: KeyCode::Char('l'), mods: Mods::CTRL, ambiguous: true })]
        );
        assert_eq!(parse(b"\x1b"), vec![k(KeyCode::Esc, Mods::NONE)]);
        assert_eq!(parse(b"\x1bb"), vec![k(KeyCode::Char('b'), Mods::ALT)]);
        assert_eq!(parse(b"\x1b[A"), vec![k(KeyCode::Up, Mods::NONE)]);
        assert_eq!(parse(b"\x1b[1;5C"), vec![k(KeyCode::Right, Mods::CTRL)]);
        assert_eq!(parse(b"\x1bOP"), vec![k(KeyCode::F(1), Mods::NONE)]);
        assert_eq!(parse(b"\x1b[3~"), vec![k(KeyCode::Delete, Mods::NONE)]);
        assert_eq!(parse(b"\x1b[5;5~"), vec![k(KeyCode::PageUp, Mods::CTRL)]);
        assert_eq!(parse(b"\x1b[Z"), vec![k(KeyCode::BackTab, Mods::NONE)]);
    }

    #[test]
    fn kitty_and_mok() {
        assert_eq!(parse(b"\x1b[108;6u"), vec![k(KeyCode::Char('l'), Mods::CTRL_SHIFT)]);
        assert_eq!(parse(b"\x1b[27u"), vec![k(KeyCode::Esc, Mods::NONE)]);
        assert_eq!(parse(b"\x1b[122;6u"), vec![k(KeyCode::Char('z'), Mods::CTRL_SHIFT)]);
        assert_eq!(parse(b"\x1b[13;2u"), vec![k(KeyCode::Enter, Mods::SHIFT)]);
        assert_eq!(parse(b"\x1b[27;6;76~"), vec![k(KeyCode::Char('l'), Mods::CTRL_SHIFT)]);
        assert_eq!(parse(b"\x1bZ"), vec![k(KeyCode::Char('z'), Mods::ALT | Mods::SHIFT)]);
        assert_eq!(parse(b"\x1b[?1u"), vec![Event::KittyFlags(1)]);
        assert_eq!(parse(b"\x1b[?62;22c"), vec![Event::DeviceAttributes]);
        assert_eq!(parse(b"\x1b[97;9u"), vec![k(KeyCode::Char('a'), Mods::SUPER)]);
    }

    #[test]
    fn mouse() {
        assert_eq!(
            parse(b"\x1b[<0;10;5M"),
            vec![Event::Mouse(MouseEvent { kind: MouseKind::Down(MouseButton::Left), x: 9, y: 4, mods: Mods::NONE })]
        );
        assert_eq!(
            parse(b"\x1b[<16;1;1m"),
            vec![Event::Mouse(MouseEvent { kind: MouseKind::Up(MouseButton::Left), x: 0, y: 0, mods: Mods::CTRL })]
        );
        assert_eq!(
            parse(b"\x1b[<64;3;3M"),
            vec![Event::Mouse(MouseEvent { kind: MouseKind::ScrollUp, x: 2, y: 2, mods: Mods::NONE })]
        );
        assert_eq!(
            parse(b"\x1b[<32;3;3M"),
            vec![Event::Mouse(MouseEvent { kind: MouseKind::Drag(MouseButton::Left), x: 2, y: 2, mods: Mods::NONE })]
        );
    }

    #[test]
    fn paste_split_across_reads() {
        let mut p = InputParser::new();
        let mut out = Vec::new();
        p.feed(b"\x1b[200~hello\r\nwor", &mut out);
        assert!(out.is_empty());
        p.feed(b"ld\x1b[20", &mut out);
        assert!(out.is_empty());
        p.feed(b"1~x", &mut out);
        assert_eq!(out, vec![Event::Paste("hello\nworld".into()), k(KeyCode::Char('x'), Mods::NONE)]);
    }

    #[test]
    fn partial_sequences_wait() {
        let mut p = InputParser::new();
        let mut out = Vec::new();
        p.feed(b"\x1b[1;5", &mut out);
        assert!(out.is_empty());
        assert!(p.has_pending());
        p.feed(b"D", &mut out);
        assert_eq!(out, vec![k(KeyCode::Left, Mods::CTRL)]);
    }
}
