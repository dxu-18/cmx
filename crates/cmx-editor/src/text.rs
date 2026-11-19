//! Text storage: a vector of lines (no trailing '\n'), positions as (line, byte column).

use cmx_tui::unicode::{cluster_width, next_boundary, prev_boundary};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Default, Hash)]
pub struct Pos {
    pub line: usize,
    pub col: usize,
}

impl Pos {
    pub const fn new(line: usize, col: usize) -> Pos {
        Pos { line, col }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LineEnding {
    #[default]
    Lf,
    CrLf,
}

#[derive(Clone, Debug)]
pub struct Text {
    lines: Vec<String>,
}

/// Character classes used for word motions.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CharClass {
    Space,
    Word,
    Punct,
}

pub fn char_class(c: char) -> CharClass {
    if c.is_whitespace() {
        CharClass::Space
    } else if c.is_alphanumeric() || c == '_' {
        CharClass::Word
    } else {
        CharClass::Punct
    }
}

impl Default for Text {
    fn default() -> Self {
        Text { lines: vec![String::new()] }
    }
}

impl Text {
    pub fn new() -> Text {
        Text::default()
    }

    pub fn parse(s: &str) -> (Text, LineEnding) {
        let crlf = s.contains("\r\n");
        let lines: Vec<String> = if crlf {
            s.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l).to_string()).collect()
        } else {
            s.split('\n').map(|l| l.to_string()).collect()
        };
        (Text { lines }, if crlf { LineEnding::CrLf } else { LineEnding::Lf })
    }

    pub fn to_string_with(&self, le: LineEnding) -> String {
        self.lines.join(if le == LineEnding::CrLf { "\r\n" } else { "\n" })
    }

    #[inline]
    pub fn len_lines(&self) -> usize {
        self.lines.len()
    }

    #[inline]
    pub fn line(&self, i: usize) -> &str {
        self.lines.get(i).map(|s| s.as_str()).unwrap_or("")
    }

    #[inline]
    pub fn line_len(&self, i: usize) -> usize {
        self.lines.get(i).map(|s| s.len()).unwrap_or(0)
    }

    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    pub fn end(&self) -> Pos {
        let l = self.lines.len() - 1;
        Pos::new(l, self.lines[l].len())
    }

    pub fn total_bytes(&self) -> usize {
        self.lines.iter().map(|l| l.len() + 1).sum::<usize>().saturating_sub(1)
    }

    /// Clamp to a valid position on a char boundary.
    pub fn clamp(&self, p: Pos) -> Pos {
        let line = p.line.min(self.lines.len() - 1);
        let s = &self.lines[line];
        let mut col = p.col.min(s.len());
        while col > 0 && !s.is_char_boundary(col) {
            col -= 1;
        }
        Pos::new(line, col)
    }

    /// Insert `s` at `p`; returns the position right after the inserted text.
    pub fn insert(&mut self, p: Pos, s: &str) -> Pos {
        let p = self.clamp(p);
        if !s.contains('\n') {
            self.lines[p.line].insert_str(p.col, s);
            return Pos::new(p.line, p.col + s.len());
        }
        let tail = self.lines[p.line].split_off(p.col);
        let mut parts = s.split('\n');
        let first = parts.next().unwrap_or("");
        self.lines[p.line].push_str(first);
        let mut new_lines: Vec<String> = parts.map(|x| x.to_string()).collect();
        let last = new_lines.last_mut().expect("contains newline");
        let end_col = last.len();
        last.push_str(&tail);
        let n = new_lines.len();
        let at = p.line + 1;
        self.lines.splice(at..at, new_lines);
        Pos::new(p.line + n, end_col)
    }

    /// Remove `[a, b)`; returns the removed text.
    pub fn remove(&mut self, a: Pos, b: Pos) -> String {
        let (a, b) = if a <= b { (a, b) } else { (b, a) };
        let a = self.clamp(a);
        let b = self.clamp(b);
        if a == b {
            return String::new();
        }
        if a.line == b.line {
            return self.lines[a.line].drain(a.col..b.col).collect();
        }
        let mut removed = String::new();
        let tail = self.lines[b.line][b.col..].to_string();
        removed.push_str(&self.lines[a.line][a.col..]);
        for l in &self.lines[a.line + 1..b.line] {
            removed.push('\n');
            removed.push_str(l);
        }
        removed.push('\n');
        removed.push_str(&self.lines[b.line][..b.col]);
        self.lines[a.line].truncate(a.col);
        self.lines[a.line].push_str(&tail);
        self.lines.drain(a.line + 1..=b.line);
        removed
    }

    pub fn slice(&self, a: Pos, b: Pos) -> String {
        let (a, b) = if a <= b { (a, b) } else { (b, a) };
        let a = self.clamp(a);
        let b = self.clamp(b);
        if a.line == b.line {
            return self.lines[a.line][a.col..b.col].to_string();
        }
        let mut s = String::new();
        s.push_str(&self.lines[a.line][a.col..]);
        for l in &self.lines[a.line + 1..b.line] {
            s.push('\n');
            s.push_str(l);
        }
        s.push('\n');
        s.push_str(&self.lines[b.line][..b.col]);
        s
    }

    /// Character at `p`, or '\n' at end of line (None at end of document).
    pub fn char_at(&self, p: Pos) -> Option<char> {
        let s = self.line(p.line);
        if p.col < s.len() {
            s[p.col..].chars().next()
        } else if p.line + 1 < self.lines.len() {
            Some('\n')
        } else {
            None
        }
    }

    /// Next grapheme position (wrapping to the next line).
    pub fn next_pos(&self, p: Pos) -> Pos {
        let s = self.line(p.line);
        if p.col < s.len() {
            Pos::new(p.line, next_boundary(s, p.col))
        } else if p.line + 1 < self.lines.len() {
            Pos::new(p.line + 1, 0)
        } else {
            p
        }
    }

    /// Previous grapheme position (wrapping to the previous line end).
    pub fn prev_pos(&self, p: Pos) -> Pos {
        if p.col > 0 {
            Pos::new(p.line, prev_boundary(self.line(p.line), p.col))
        } else if p.line > 0 {
            Pos::new(p.line - 1, self.line_len(p.line - 1))
        } else {
            p
        }
    }

    /// Display column of byte `col` in `line`, expanding tabs.
    pub fn display_col(&self, line: usize, col: usize, tab: usize) -> usize {
        display_width_upto(self.line(line), col, tab)
    }

    /// Byte column closest to display column `dcol` (never splitting a grapheme).
    pub fn col_for_display(&self, line: usize, dcol: usize, tab: usize) -> usize {
        let s = self.line(line);
        let mut w = 0;
        let mut i = 0;
        while i < s.len() {
            let j = next_boundary(s, i);
            let cw = grapheme_width(&s[i..j], w, tab);
            if w + cw > dcol {
                // pick the nearer edge
                return if dcol - w > cw / 2 && cw > 1 { j } else { i };
            }
            w += cw;
            i = j;
        }
        s.len()
    }

    pub fn first_non_blank(&self, line: usize) -> usize {
        let s = self.line(line);
        s.len() - s.trim_start().len()
    }

    pub fn indent_of(&self, line: usize) -> &str {
        let s = self.line(line);
        &s[..self.first_non_blank(line)]
    }

    /// VS Code-style word motion right: skip spaces, then one run of the same class.
    pub fn word_right(&self, p: Pos) -> Pos {
        let s = self.line(p.line);
        if p.col >= s.len() {
            return self.next_pos(p);
        }
        let mut i = p.col;
        let mut it = s[i..].chars().peekable();
        while let Some(&c) = it.peek() {
            if char_class(c) != CharClass::Space {
                break;
            }
            i += c.len_utf8();
            it.next();
        }
        if let Some(&c) = it.peek() {
            let cls = char_class(c);
            while let Some(&c) = it.peek() {
                if char_class(c) != cls {
                    break;
                }
                i += c.len_utf8();
                it.next();
            }
        }
        Pos::new(p.line, i)
    }

    /// VS Code-style word motion left.
    pub fn word_left(&self, p: Pos) -> Pos {
        if p.col == 0 {
            return self.prev_pos(p);
        }
        let s = &self.line(p.line)[..p.col];
        let mut i = p.col;
        let mut it = s.chars().rev().peekable();
        while let Some(&c) = it.peek() {
            if char_class(c) != CharClass::Space {
                break;
            }
            i -= c.len_utf8();
            it.next();
        }
        if let Some(&c) = it.peek() {
            let cls = char_class(c);
            while let Some(&c) = it.peek() {
                if char_class(c) != cls {
                    break;
                }
                i -= c.len_utf8();
                it.next();
            }
        }
        Pos::new(p.line, i)
    }

    /// The word (Word-class run) touching `p`, if any.
    pub fn word_at(&self, p: Pos) -> Option<(Pos, Pos)> {
        let s = self.line(p.line);
        if s.is_empty() {
            return None;
        }
        let col = p.col.min(s.len());
        let at = s[col..].chars().next();
        let before = s[..col].chars().next_back();
        let cls = match (at, before) {
            (Some(c), _) if char_class(c) != CharClass::Space => char_class(c),
            (_, Some(c)) if char_class(c) != CharClass::Space => char_class(c),
            (Some(_), _) => CharClass::Space,
            _ => return None,
        };
        let mut a = col;
        while let Some(c) = s[..a].chars().next_back() {
            if char_class(c) != cls {
                break;
            }
            a -= c.len_utf8();
        }
        let mut b = col;
        while let Some(c) = s[b..].chars().next() {
            if char_class(c) != cls {
                break;
            }
            b += c.len_utf8();
        }
        if a == b {
            return None;
        }
        Some((Pos::new(p.line, a), Pos::new(p.line, b)))
    }

    /// Find `needle` starting at `from` (forward) or before `from` (backward), wrapping around.
    pub fn find(&self, needle: &str, from: Pos, forward: bool, case_sensitive: bool) -> Option<(Pos, Pos)> {
        if needle.is_empty() || needle.contains('\n') {
            return self.find_multiline(needle, from, forward, case_sensitive);
        }
        let n = self.lines.len();
        let needle_l = if case_sensitive { needle.to_string() } else { needle.to_ascii_lowercase() };
        let hay = |l: usize| -> std::borrow::Cow<'_, str> {
            if case_sensitive {
                std::borrow::Cow::Borrowed(self.line(l))
            } else {
                std::borrow::Cow::Owned(lower_same_len(self.line(l)))
            }
        };
        if forward {
            for k in 0..n {
                let l = (from.line + k) % n;
                let h = hay(l);
                let off = if k == 0 { from.col.min(h.len()) } else { 0 };
                if let Some(i) = h[off..].find(&needle_l) {
                    let c = off + i;
                    return Some((Pos::new(l, c), Pos::new(l, c + needle.len())));
                }
            }
            // wrapped all the way around: the part of the start line before `from`
            let h = hay(from.line);
            let end = (from.col + needle_l.len()).min(h.len());
            let end = (0..=end).rev().find(|&e| h.is_char_boundary(e)).unwrap_or(0);
            if let Some(c) = h[..end].find(&needle_l) {
                return Some((Pos::new(from.line, c), Pos::new(from.line, c + needle.len())));
            }
        } else {
            for k in 0..=n {
                let l = (from.line + n * 2 - k) % n;
                let h = hay(l);
                let end = if k == 0 { from.col.min(h.len()) } else { h.len() };
                if let Some(c) = h[..end].rfind(&needle_l) {
                    return Some((Pos::new(l, c), Pos::new(l, c + needle.len())));
                }
            }
        }
        None
    }

    fn find_multiline(&self, needle: &str, from: Pos, forward: bool, case_sensitive: bool) -> Option<(Pos, Pos)> {
        if needle.is_empty() {
            return None;
        }
        let all = self.to_string_with(LineEnding::Lf);
        let (h, nd) = if case_sensitive {
            (all.clone(), needle.to_string())
        } else {
            (lower_same_len(&all), needle.to_ascii_lowercase())
        };
        let off = self.offset_of(from);
        let hit = if forward {
            h[off.min(h.len())..].find(&nd).map(|i| i + off).or_else(|| h.find(&nd))
        } else {
            h[..off.min(h.len())].rfind(&nd).or_else(|| h.rfind(&nd))
        }?;
        Some((self.pos_of(hit), self.pos_of(hit + nd.len())))
    }

    /// Position of the bracket matching the one at `p`.
    pub fn match_bracket(&self, p: Pos) -> Option<Pos> {
        let t = self;
        let c = t.char_at(p)?;
        let (open, close, fwd) = match c {
            '(' => ('(', ')', true),
            '[' => ('[', ']', true),
            '{' => ('{', '}', true),
            ')' => ('(', ')', false),
            ']' => ('[', ']', false),
            '}' => ('{', '}', false),
            _ => return None,
        };
        let mut depth = 0i32;
        let mut q = p;
        let mut steps = 0;
        loop {
            let ch = t.char_at(q);
            if ch == Some(open) {
                depth += if fwd { 1 } else { -1 };
            } else if ch == Some(close) {
                depth += if fwd { -1 } else { 1 };
            }
            if depth == 0 {
                return Some(q);
            }
            let nq = if fwd { t.next_pos(q) } else { t.prev_pos(q) };
            if nq == q {
                return None;
            }
            q = nq;
            steps += 1;
            if steps > 200_000 {
                return None;
            }
        }
    }

    /// Byte offset of a position in the '\n'-joined text.
    pub fn offset_of(&self, p: Pos) -> usize {
        self.lines[..p.line.min(self.lines.len())].iter().map(|l| l.len() + 1).sum::<usize>() + p.col
    }

    pub fn pos_of(&self, mut off: usize) -> Pos {
        for (i, l) in self.lines.iter().enumerate() {
            if off <= l.len() {
                return Pos::new(i, off);
            }
            off -= l.len() + 1;
        }
        self.end()
    }

    /// All matches of `needle` on lines `[l0, l1)` (for highlighting).
    pub fn matches_in(&self, needle: &str, l0: usize, l1: usize, case_sensitive: bool, out: &mut Vec<(Pos, Pos)>) {
        if needle.is_empty() || needle.contains('\n') {
            return;
        }
        let nd = if case_sensitive { needle.to_string() } else { needle.to_ascii_lowercase() };
        for l in l0..l1.min(self.lines.len()) {
            let h = if case_sensitive { self.line(l).to_string() } else { lower_same_len(self.line(l)) };
            let mut s = 0;
            while let Some(i) = h[s..].find(&nd) {
                let c = s + i;
                out.push((Pos::new(l, c), Pos::new(l, c + nd.len())));
                s = c + nd.len().max(1);
                if s > h.len() {
                    break;
                }
            }
        }
    }
}

/// Lowercase only ASCII (keeps byte offsets identical to the original).
pub fn lower_same_len(s: &str) -> String {
    s.to_ascii_lowercase()
}

#[inline]
pub fn grapheme_width(g: &str, at_col: usize, tab: usize) -> usize {
    let b = g.as_bytes()[0];
    if b == b'\t' {
        tab - (at_col % tab)
    } else if b < 0x20 || b == 0x7f {
        2 // shown as ^X
    } else if b < 0x80 {
        1
    } else {
        cluster_width(g).max(1)
    }
}

/// Display width of `s[..upto]`.
pub fn display_width_upto(s: &str, upto: usize, tab: usize) -> usize {
    let upto = upto.min(s.len());
    let mut w = 0;
    let mut i = 0;
    while i < upto {
        let j = next_boundary(s, i);
        w += grapheme_width(&s[i..j], w, tab);
        i = j;
    }
    w
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_remove_roundtrip() {
        let (mut t, le) = Text::parse("hello\nworld\n");
        assert_eq!(le, LineEnding::Lf);
        assert_eq!(t.len_lines(), 3);
        let e = t.insert(Pos::new(0, 5), " there\nbig");
        assert_eq!(e, Pos::new(1, 3));
        assert_eq!(t.to_string_with(LineEnding::Lf), "hello there\nbig\nworld\n");
        let r = t.remove(Pos::new(0, 5), Pos::new(1, 3));
        assert_eq!(r, " there\nbig");
        assert_eq!(t.to_string_with(LineEnding::Lf), "hello\nworld\n");
        assert_eq!(t.slice(Pos::new(0, 3), Pos::new(1, 2)), "lo\nwo");
    }

    #[test]
    fn crlf() {
        let (t, le) = Text::parse("a\r\nb\r\n");
        assert_eq!(le, LineEnding::CrLf);
        assert_eq!(t.line(0), "a");
        assert_eq!(t.to_string_with(le), "a\r\nb\r\n");
    }

    #[test]
    fn words_and_display() {
        let (t, _) = Text::parse("  foo.bar(baz)\tx");
        assert_eq!(t.word_right(Pos::new(0, 0)), Pos::new(0, 5));
        assert_eq!(t.word_right(Pos::new(0, 5)), Pos::new(0, 6));
        assert_eq!(t.word_left(Pos::new(0, 9)), Pos::new(0, 6));
        assert_eq!(t.display_col(0, 15, 4), 16);
        assert_eq!(t.col_for_display(0, 16, 4), 15);
        assert_eq!(t.word_at(Pos::new(0, 7)), Some((Pos::new(0, 6), Pos::new(0, 9))));
    }

    #[test]
    fn find_wraps() {
        let (t, _) = Text::parse("abc\nxbc\nabc");
        assert_eq!(t.find("bc", Pos::new(1, 2), true, true), Some((Pos::new(2, 1), Pos::new(2, 3))));
        assert_eq!(t.find("ab", Pos::new(2, 1), true, true), Some((Pos::new(0, 0), Pos::new(0, 2))));
        assert_eq!(t.find("bc", Pos::new(1, 0), false, true), Some((Pos::new(0, 1), Pos::new(0, 3))));
        assert_eq!(t.find("XB", Pos::new(0, 0), true, false), Some((Pos::new(1, 0), Pos::new(1, 2))));
        assert_eq!(t.find("c\nx", Pos::new(0, 0), true, true), Some((Pos::new(0, 2), Pos::new(1, 1))));
    }
}
