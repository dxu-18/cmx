//! Encoding of user input (keys, mouse, paste, focus) into bytes for the application, honouring
//! the modes the application has set (DECCKM, mouse protocol, bracketed paste, ...).

use crate::{MouseMode, Term};
use cmx_tui::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseKind};

fn push_num(out: &mut Vec<u8>, n: u32) {
    out.extend_from_slice(n.to_string().as_bytes());
}

impl Term {
    /// Bytes to send for a key press. Super (Cmd) is dropped; Shift+Enter becomes `ESC CR`
    /// (Claude Code / readline "newline without submit").
    pub fn encode_key(&self, k: &KeyEvent) -> Vec<u8> {
        let m = k.mods;
        let mp = 1 + (m.0 & 0x7) as u32; // shift/alt/ctrl only
        let mut out = Vec::with_capacity(8);
        let csi_letter = |out: &mut Vec<u8>, l: u8, app: bool| {
            if mp == 1 {
                out.extend_from_slice(if app { b"\x1bO" } else { b"\x1b[" });
                out.push(l);
            } else {
                out.extend_from_slice(b"\x1b[1;");
                push_num(out, mp);
                out.push(l);
            }
        };
        let csi_tilde = |out: &mut Vec<u8>, n: u32| {
            out.extend_from_slice(b"\x1b[");
            push_num(out, n);
            if mp != 1 {
                out.push(b';');
                push_num(out, mp);
            }
            out.push(b'~');
        };
        match k.code {
            KeyCode::Char(c) => {
                if m.ctrl() {
                    let b = match c {
                        'a'..='z' => Some(c as u8 - b'a' + 1),
                        'A'..='Z' => Some(c as u8 - b'A' + 1),
                        '@' | ' ' | '2' | '`' => Some(0),
                        '[' | '3' => Some(0x1b),
                        '\\' | '4' => Some(0x1c),
                        ']' | '5' => Some(0x1d),
                        '^' | '6' => Some(0x1e),
                        '_' | '/' | '7' | '-' => Some(0x1f),
                        '?' | '8' => Some(0x7f),
                        _ => None,
                    };
                    if let Some(b) = b {
                        if m.alt() {
                            out.push(0x1b);
                        }
                        out.push(b);
                        return out;
                    }
                }
                if m.alt() {
                    out.push(0x1b);
                }
                let c = if m.shift() && c.is_ascii_lowercase() { c.to_ascii_uppercase() } else { c };
                let mut tmp = [0u8; 4];
                out.extend_from_slice(c.encode_utf8(&mut tmp).as_bytes());
            }
            KeyCode::Enter => {
                if m.alt() || m.shift() {
                    out.extend_from_slice(b"\x1b\r");
                } else if self.modes.lnm {
                    out.extend_from_slice(b"\r\n");
                } else {
                    out.push(b'\r');
                }
            }
            KeyCode::Tab => {
                if m.shift() {
                    out.extend_from_slice(b"\x1b[Z");
                } else {
                    if m.alt() {
                        out.push(0x1b);
                    }
                    out.push(b'\t');
                }
            }
            KeyCode::BackTab => out.extend_from_slice(b"\x1b[Z"),
            KeyCode::Backspace => {
                if m.alt() {
                    out.push(0x1b);
                }
                out.push(if m.ctrl() { 0x08 } else { 0x7f });
            }
            KeyCode::Esc => {
                if m.alt() {
                    out.push(0x1b);
                }
                out.push(0x1b);
            }
            KeyCode::Up => csi_letter(&mut out, b'A', self.modes.app_cursor),
            KeyCode::Down => csi_letter(&mut out, b'B', self.modes.app_cursor),
            KeyCode::Right => csi_letter(&mut out, b'C', self.modes.app_cursor),
            KeyCode::Left => csi_letter(&mut out, b'D', self.modes.app_cursor),
            KeyCode::Home => csi_letter(&mut out, b'H', self.modes.app_cursor),
            KeyCode::End => csi_letter(&mut out, b'F', self.modes.app_cursor),
            KeyCode::Insert => csi_tilde(&mut out, 2),
            KeyCode::Delete => csi_tilde(&mut out, 3),
            KeyCode::PageUp => csi_tilde(&mut out, 5),
            KeyCode::PageDown => csi_tilde(&mut out, 6),
            KeyCode::F(n @ 1..=4) => {
                let l = b'P' + (n - 1);
                if mp == 1 {
                    out.extend_from_slice(b"\x1bO");
                    out.push(l);
                } else {
                    out.extend_from_slice(b"\x1b[1;");
                    push_num(&mut out, mp);
                    out.push(l);
                }
            }
            KeyCode::F(n) => {
                let code = match n {
                    5 => 15,
                    6 => 17,
                    7 => 18,
                    8 => 19,
                    9 => 20,
                    10 => 21,
                    11 => 23,
                    12 => 24,
                    _ => return out,
                };
                csi_tilde(&mut out, code);
            }
        }
        out
    }

    /// Whether the application wants mouse events.
    pub fn wants_mouse(&self) -> bool {
        self.modes.mouse != MouseMode::Off
    }

    /// Encode a mouse event at cell (x, y) relative to the terminal's top-left (0-based).
    pub fn encode_mouse(&self, ev: &MouseEvent, x: u16, y: u16) -> Option<Vec<u8>> {
        let mode = self.modes.mouse;
        if mode == MouseMode::Off {
            return None;
        }
        let btn = |b: MouseButton| match b {
            MouseButton::Left => 0u32,
            MouseButton::Middle => 1,
            MouseButton::Right => 2,
            MouseButton::Other => 0,
        };
        let (mut cb, release) = match ev.kind {
            MouseKind::Down(b) => (btn(b), false),
            MouseKind::Up(b) => {
                if mode == MouseMode::X10 {
                    return None;
                }
                (btn(b), true)
            }
            MouseKind::Drag(b) => {
                if mode != MouseMode::Button && mode != MouseMode::Any {
                    return None;
                }
                (btn(b) + 32, false)
            }
            MouseKind::Moved => {
                if mode != MouseMode::Any {
                    return None;
                }
                (3 + 32, false)
            }
            MouseKind::ScrollUp => (64, false),
            MouseKind::ScrollDown => (65, false),
            MouseKind::ScrollLeft => (66, false),
            MouseKind::ScrollRight => (67, false),
        };
        if mode != MouseMode::X10 {
            if ev.mods.shift() {
                cb += 4;
            }
            if ev.mods.alt() {
                cb += 8;
            }
            if ev.mods.ctrl() {
                cb += 16;
            }
        }
        let mut out = Vec::with_capacity(16);
        if self.modes.mouse_sgr {
            out.extend_from_slice(b"\x1b[<");
            push_num(&mut out, cb);
            out.push(b';');
            push_num(&mut out, x as u32 + 1);
            out.push(b';');
            push_num(&mut out, y as u32 + 1);
            out.push(if release { b'm' } else { b'M' });
        } else {
            if release {
                cb = (cb & !3) | 3;
            }
            out.extend_from_slice(b"\x1b[M");
            out.push((32 + cb).min(255) as u8);
            out.push((33 + x as u32).min(255) as u8);
            out.push((33 + y as u32).min(255) as u8);
        }
        Some(out)
    }

    /// Encode pasted text (bracketed when the application asked for it).
    pub fn encode_paste(&self, text: &str) -> Vec<u8> {
        let clean: String = text.replace("\r\n", "\r").replace('\n', "\r").chars().filter(|&c| c != '\x1b').collect();
        let mut out = Vec::with_capacity(clean.len() + 12);
        if self.modes.bracketed_paste {
            out.extend_from_slice(b"\x1b[200~");
            out.extend_from_slice(clean.as_bytes());
            out.extend_from_slice(b"\x1b[201~");
        } else {
            out.extend_from_slice(clean.as_bytes());
        }
        out
    }

    pub fn encode_focus(&self, focused: bool) -> Option<&'static [u8]> {
        if self.modes.focus_events { Some(if focused { b"\x1b[I" } else { b"\x1b[O" }) } else { None }
    }
}
