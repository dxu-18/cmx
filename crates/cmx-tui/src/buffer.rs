//! Off-screen cell buffer. Each frame is drawn into a [`Buffer`] and then diffed by the renderer.

use crate::style::Style;
use crate::unicode::{char_width, cluster_width, next_boundary};

/// One grapheme stored inline (no heap): up to 14 bytes of UTF-8 plus its display width.
/// `width == 0` marks the continuation cell to the right of a wide character.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Sym {
    len: u8,
    width: u8,
    bytes: [u8; 14],
}

impl std::fmt::Debug for Sym {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.as_str())
    }
}

impl Sym {
    pub const BLANK: Sym = Sym { len: 1, width: 1, bytes: [b' ', 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0] };
    pub const CONT: Sym = Sym { len: 0, width: 0, bytes: [0; 14] };

    /// A single character. Control characters become U+FFFD-ish '?' so they never reach the tty.
    #[inline]
    pub fn from_char(c: char) -> Sym {
        let mut w = char_width(c);
        let c = if w == 0 && (c as u32) < 0xA0 {
            w = 1;
            '?'
        } else {
            c
        };
        let mut s = Sym { len: 0, width: w.max(1) as u8, bytes: [0; 14] };
        let n = c.encode_utf8(&mut s.bytes).len();
        s.len = n as u8;
        s
    }

    /// A grapheme cluster (truncated to 14 bytes at a char boundary if longer).
    pub fn from_cluster(g: &str) -> Sym {
        let mut first = g.chars();
        match first.next() {
            None => return Sym::BLANK,
            Some(c) if g.len() == c.len_utf8() => return Sym::from_char(c),
            _ => {}
        }
        let mut s = Sym { len: 0, width: cluster_width(g).max(1) as u8, bytes: [0; 14] };
        let mut n = 0;
        for c in g.chars() {
            let l = c.len_utf8();
            if n + l > 14 {
                break;
            }
            c.encode_utf8(&mut s.bytes[n..]);
            n += l;
        }
        s.len = n as u8;
        s
    }

    /// Append a zero-width character (combining mark) if there is room.
    pub fn push_combining(&mut self, c: char) -> bool {
        let l = c.len_utf8();
        if self.width == 0 || self.len as usize + l > 14 {
            return false;
        }
        c.encode_utf8(&mut self.bytes[self.len as usize..]);
        self.len += l as u8;
        if c == '\u{FE0F}' && self.width == 1 {
            // emoji presentation: most terminals render it double width
            let first = self.as_str().chars().next().unwrap_or(' ');
            if (first as u32) >= 0x2000 {
                self.width = 2;
            }
        }
        true
    }

    #[inline]
    pub fn as_str(&self) -> &str {
        // SAFETY: bytes[..len] is always valid UTF-8 (only written via encode_utf8 at boundaries).
        unsafe { std::str::from_utf8_unchecked(&self.bytes[..self.len as usize]) }
    }
    #[inline]
    pub fn width(&self) -> usize {
        self.width as usize
    }
    #[inline]
    pub fn is_cont(&self) -> bool {
        self.width == 0
    }
    #[inline]
    pub fn is_blank(&self) -> bool {
        self.len == 1 && self.bytes[0] == b' '
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub struct Cell {
    pub sym: Sym,
    pub style: Style,
}

impl Cell {
    pub const BLANK: Cell = Cell { sym: Sym::BLANK, style: Style::DEFAULT };
    pub fn styled(style: Style) -> Cell {
        Cell { sym: Sym::BLANK, style }
    }
}

impl Default for Cell {
    fn default() -> Self {
        Cell::BLANK
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
}

impl Rect {
    pub const fn new(x: u16, y: u16, w: u16, h: u16) -> Rect {
        Rect { x, y, w, h }
    }
    #[inline]
    pub fn right(&self) -> u16 {
        self.x + self.w
    }
    #[inline]
    pub fn bottom(&self) -> u16 {
        self.y + self.h
    }
    #[inline]
    pub fn contains(&self, x: u16, y: u16) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }
    pub fn is_empty(&self) -> bool {
        self.w == 0 || self.h == 0
    }
    /// Shrink from the left/top by the given amounts (saturating).
    pub fn inset(&self, left: u16, top: u16, right: u16, bottom: u16) -> Rect {
        let x = self.x + left.min(self.w);
        let y = self.y + top.min(self.h);
        let w = self.w.saturating_sub(left + right);
        let h = self.h.saturating_sub(top + bottom);
        Rect { x, y, w, h }
    }
    pub fn row(&self, i: u16) -> Rect {
        Rect { x: self.x, y: self.y + i, w: self.w, h: if i < self.h { 1 } else { 0 } }
    }
}

#[derive(Clone, Debug)]
pub struct Buffer {
    pub width: u16,
    pub height: u16,
    pub cells: Vec<Cell>,
}

impl Buffer {
    pub fn new(width: u16, height: u16) -> Buffer {
        Buffer { width, height, cells: vec![Cell::BLANK; width as usize * height as usize] }
    }

    pub fn area(&self) -> Rect {
        Rect::new(0, 0, self.width, self.height)
    }

    pub fn resize(&mut self, width: u16, height: u16) {
        self.width = width;
        self.height = height;
        self.cells.clear();
        self.cells.resize(width as usize * height as usize, Cell::BLANK);
    }

    pub fn clear(&mut self, style: Style) {
        let c = Cell::styled(style);
        self.cells.iter_mut().for_each(|x| *x = c);
    }

    #[inline]
    fn idx(&self, x: u16, y: u16) -> usize {
        y as usize * self.width as usize + x as usize
    }

    #[inline]
    pub fn get(&self, x: u16, y: u16) -> &Cell {
        &self.cells[self.idx(x, y)]
    }

    #[inline]
    pub fn get_mut(&mut self, x: u16, y: u16) -> &mut Cell {
        let i = self.idx(x, y);
        &mut self.cells[i]
    }

    /// Place a symbol at (x, y) maintaining wide-character invariants. Returns the next x.
    /// `limit` is the exclusive right edge (wide symbols that don't fit become a blank).
    pub fn set(&mut self, x: u16, y: u16, sym: Sym, style: Style, limit: u16) -> u16 {
        if y >= self.height || x >= self.width || x >= limit {
            return x;
        }
        let limit = limit.min(self.width);
        let i = self.idx(x, y);
        // If we overwrite the right half of a wide char, blank its left half.
        if self.cells[i].sym.is_cont() && x > 0 {
            self.cells[i - 1].sym = Sym::BLANK;
        }
        // If we overwrite the left half of a wide char, blank its right half.
        if self.cells[i].sym.width() == 2 && x + 1 < self.width {
            self.cells[i + 1].sym = Sym::BLANK;
        }
        let w = sym.width();
        if w == 2 {
            if x + 1 >= limit {
                self.cells[i] = Cell { sym: Sym::BLANK, style };
                return x + 1;
            }
            if self.cells[i + 1].sym.width() == 2 && x + 2 < self.width {
                self.cells[i + 2].sym = Sym::BLANK;
            }
            self.cells[i] = Cell { sym, style };
            self.cells[i + 1] = Cell { sym: Sym::CONT, style };
            x + 2
        } else {
            self.cells[i] = Cell { sym, style };
            x + 1
        }
    }

    /// Draw a string clipped to `[x, limit)`. Tabs/controls are drawn as spaces. Returns next x.
    pub fn put_str(&mut self, mut x: u16, y: u16, s: &str, style: Style, limit: u16) -> u16 {
        if y >= self.height {
            return x;
        }
        let limit = limit.min(self.width);
        if s.is_ascii() {
            for b in s.bytes() {
                if x >= limit {
                    break;
                }
                let c = if b < 0x20 || b == 0x7F { ' ' } else { b as char };
                x = self.set(x, y, Sym::from_char(c), style, limit);
            }
            return x;
        }
        let mut i = 0;
        while i < s.len() && x < limit {
            let j = next_boundary(s, i);
            let g = &s[i..j];
            let c0 = g.chars().next().unwrap();
            let sym = if (c0 as u32) < 0x20 { Sym::BLANK } else { Sym::from_cluster(g) };
            if char_width(c0) == 0 && (c0 as u32) >= 0xA0 {
                // stray combining mark at start: attach to nothing, draw as blank-width-0 → skip
                i = j;
                continue;
            }
            x = self.set(x, y, sym, style, limit);
            i = j;
        }
        x
    }

    pub fn put_char(&mut self, x: u16, y: u16, c: char, style: Style) -> u16 {
        let lim = self.width;
        self.set(x, y, Sym::from_char(c), style, lim)
    }

    /// Fill a rectangle with a character and style.
    pub fn fill(&mut self, r: Rect, c: char, style: Style) {
        let sym = Sym::from_char(c);
        let x1 = r.right().min(self.width);
        let y1 = r.bottom().min(self.height);
        for y in r.y..y1 {
            let mut x = r.x;
            while x < x1 {
                x = self.set(x, y, sym, style, x1);
            }
        }
    }

    /// Change the background of every cell in `r` (keeps glyphs and foreground).
    pub fn set_bg(&mut self, r: Rect, bg: crate::style::Color) {
        let x1 = r.right().min(self.width);
        let y1 = r.bottom().min(self.height);
        for y in r.y..y1 {
            for x in r.x..x1 {
                let i = self.idx(x, y);
                self.cells[i].style.bg = bg;
            }
        }
    }

    /// Replace the whole style of every cell in `r`.
    pub fn set_style(&mut self, r: Rect, style: Style) {
        let x1 = r.right().min(self.width);
        let y1 = r.bottom().min(self.height);
        for y in r.y..y1 {
            for x in r.x..x1 {
                let i = self.idx(x, y);
                self.cells[i].style = style;
            }
        }
    }

    /// Row text (for tests / debugging).
    pub fn row_text(&self, y: u16) -> String {
        let mut s = String::new();
        for x in 0..self.width {
            let c = self.get(x, y);
            if !c.sym.is_cont() {
                s.push_str(c.sym.as_str());
            }
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wide_invariants() {
        let mut b = Buffer::new(6, 1);
        let x = b.put_str(0, 0, "a中b", Style::DEFAULT, 6);
        assert_eq!(x, 4);
        assert_eq!(b.row_text(0), "a中b  ");
        // overwrite right half of the wide char
        b.put_char(2, 0, 'x', Style::DEFAULT);
        assert_eq!(b.row_text(0), "a xb  ");
        // wide char that doesn't fit at the edge
        let x = b.put_str(5, 0, "中", Style::DEFAULT, 6);
        assert_eq!(x, 6);
        assert_eq!(b.row_text(0), "a xb  ");
    }

    #[test]
    fn combining_cluster() {
        let mut b = Buffer::new(4, 1);
        b.put_str(0, 0, "e\u{301}z", Style::DEFAULT, 4);
        assert_eq!(b.get(0, 0).sym.as_str(), "e\u{301}");
        assert_eq!(b.get(1, 0).sym.as_str(), "z");
    }
}
