//! Diff renderer: compares the new frame against the previous one and emits the minimal escape
//! sequences, wrapped in synchronized output so the terminal never shows a half-drawn frame.

use crate::buffer::Buffer;
use crate::style::{Color, Style, attr, rgb_to_256};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum CursorShape {
    #[default]
    Bar,
    Block,
    Underline,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct CursorState {
    pub x: u16,
    pub y: u16,
    pub shape: CursorShape,
    pub visible: bool,
}

pub struct Renderer {
    prev: Option<Buffer>,
    pub truecolor: bool,
    out: Vec<u8>,
    shape: Option<CursorShape>,
}

#[inline]
pub fn push_num(out: &mut Vec<u8>, mut n: u32) {
    if n == 0 {
        out.push(b'0');
        return;
    }
    let mut tmp = [0u8; 10];
    let mut i = tmp.len();
    while n > 0 {
        i -= 1;
        tmp[i] = b'0' + (n % 10) as u8;
        n /= 10;
    }
    out.extend_from_slice(&tmp[i..]);
}

fn push_color(out: &mut Vec<u8>, c: Color, fg: bool, truecolor: bool) {
    match c {
        Color::Reset => {}
        Color::Idx(n) => {
            out.push(b';');
            if n < 8 {
                push_num(out, (if fg { 30 } else { 40 }) + n as u32);
            } else if n < 16 {
                push_num(out, (if fg { 90 } else { 100 }) + n as u32 - 8);
            } else {
                out.extend_from_slice(if fg { b"38;5;" } else { b"48;5;" });
                push_num(out, n as u32);
            }
        }
        Color::Rgb(r, g, b) => {
            out.push(b';');
            if truecolor {
                out.extend_from_slice(if fg { b"38;2;" } else { b"48;2;" });
                push_num(out, r as u32);
                out.push(b';');
                push_num(out, g as u32);
                out.push(b';');
                push_num(out, b as u32);
            } else {
                out.extend_from_slice(if fg { b"38;5;" } else { b"48;5;" });
                push_num(out, rgb_to_256(r, g, b) as u32);
            }
        }
    }
}

/// Append an SGR sequence that sets exactly `s` (starting from a reset).
pub fn push_sgr(out: &mut Vec<u8>, s: Style, truecolor: bool) {
    out.extend_from_slice(b"\x1b[0");
    let a = s.attrs;
    if a & attr::BOLD != 0 {
        out.extend_from_slice(b";1");
    }
    if a & attr::DIM != 0 {
        out.extend_from_slice(b";2");
    }
    if a & attr::ITALIC != 0 {
        out.extend_from_slice(b";3");
    }
    if a & attr::UNDERLINE != 0 {
        out.extend_from_slice(b";4");
    }
    if a & attr::BLINK != 0 {
        out.extend_from_slice(b";5");
    }
    if a & attr::REVERSE != 0 {
        out.extend_from_slice(b";7");
    }
    if a & attr::HIDDEN != 0 {
        out.extend_from_slice(b";8");
    }
    if a & attr::STRIKE != 0 {
        out.extend_from_slice(b";9");
    }
    push_color(out, s.fg, true, truecolor);
    push_color(out, s.bg, false, truecolor);
    out.push(b'm');
}

fn push_cup(out: &mut Vec<u8>, x: u16, y: u16) {
    out.extend_from_slice(b"\x1b[");
    push_num(out, y as u32 + 1);
    out.push(b';');
    push_num(out, x as u32 + 1);
    out.push(b'H');
}

impl Renderer {
    pub fn new(truecolor: bool) -> Renderer {
        Renderer { prev: None, truecolor, out: Vec::with_capacity(64 * 1024), shape: None }
    }

    /// Forget the previous frame so the next render repaints everything.
    pub fn invalidate(&mut self) {
        self.prev = None;
    }

    /// Produce the bytes that turn the screen into `buf`.
    pub fn render(&mut self, buf: &Buffer, cursor: CursorState) -> &[u8] {
        let out = &mut self.out;
        out.clear();
        out.extend_from_slice(b"\x1b[?2026h\x1b[?25l");
        let full = match &self.prev {
            Some(p) => p.width != buf.width || p.height != buf.height,
            None => true,
        };
        if full {
            out.extend_from_slice(b"\x1b[0m\x1b[2J");
        }
        let w = buf.width as usize;
        let mut pen: Option<Style> = None;
        let mut at: Option<(u16, u16)> = None;
        for y in 0..buf.height {
            let row = y as usize * w;
            for x in 0..buf.width {
                let i = row + x as usize;
                let c = &buf.cells[i];
                if c.sym.is_cont() {
                    continue;
                }
                if !full {
                    let p = &self.prev.as_ref().unwrap().cells;
                    if p[i] == *c && (c.sym.width() < 2 || (i + 1 < p.len() && p[i + 1] == buf.cells[i + 1])) {
                        continue;
                    }
                }
                if at != Some((x, y)) {
                    push_cup(out, x, y);
                }
                if pen != Some(c.style) {
                    push_sgr(out, c.style, self.truecolor);
                    pen = Some(c.style);
                }
                out.extend_from_slice(c.sym.as_str().as_bytes());
                let nx = x + c.sym.width().max(1) as u16;
                at = if nx >= buf.width { None } else { Some((nx, y)) };
            }
        }
        out.extend_from_slice(b"\x1b[0m");
        if cursor.visible && cursor.x < buf.width && cursor.y < buf.height {
            push_cup(out, cursor.x, cursor.y);
            if self.shape != Some(cursor.shape) {
                out.extend_from_slice(match cursor.shape {
                    CursorShape::Block => b"\x1b[1 q",
                    CursorShape::Bar => b"\x1b[5 q",
                    CursorShape::Underline => b"\x1b[3 q",
                });
                self.shape = Some(cursor.shape);
            }
            out.extend_from_slice(b"\x1b[?25h");
        }
        out.extend_from_slice(b"\x1b[?2026l");
        match &mut self.prev {
            Some(p) if p.width == buf.width && p.height == buf.height => p.cells.copy_from_slice(&buf.cells),
            _ => self.prev = Some(buf.clone()),
        }
        &self.out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diff_is_small() {
        let mut r = Renderer::new(true);
        let mut b = Buffer::new(10, 3);
        b.put_str(0, 0, "hello", Style::DEFAULT, 10);
        let first = r.render(&b, CursorState::default()).len();
        assert!(first > 10);
        b.put_char(1, 0, 'a', Style::DEFAULT);
        let second = r.render(&b, CursorState::default()).to_vec();
        let s = String::from_utf8_lossy(&second);
        assert!(s.contains("\x1b[1;2H"), "{s:?}");
        assert!(s.contains('a'));
        assert!(!s.contains("hello"));
    }
}
