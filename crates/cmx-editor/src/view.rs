//! Per-tab view state (cursor, selection, scroll) and the editing operations that need both the
//! document and the view (`Ed`).

use crate::document::Document;
use crate::history::{Coalesce, Edit, Sel, end_of};
use crate::search::Query;
use crate::text::{CharClass, Pos, Text, char_class, grapheme_width};
use cmx_tui::unicode::next_boundary;

/// How the anchor..cursor range is interpreted (vim visual modes are inclusive / linewise).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum SelKind {
    #[default]
    Exclusive,
    Inclusive,
    Line,
}

#[derive(Clone, Debug, Default)]
pub struct View {
    pub sel_kind: SelKind,
    pub cursor: Pos,
    pub anchor: Option<Pos>,
    /// Sticky display column for vertical movement.
    pub want_col: Option<usize>,
    pub scroll_y: usize,
    pub scroll_x: usize,
    /// Text area size, updated by the renderer.
    pub height: usize,
    pub width: usize,
    /// Secondary cursors (cursor, anchor); the primary one is `cursor`/`anchor`.
    pub extra: Vec<Sel>,
    /// Soft wrap: long lines continue on the next screen rows (no horizontal scrolling).
    pub wrap: bool,
    /// With wrap on: the first visible row is wrap segment `scroll_sub` of line `scroll_y`.
    pub scroll_sub: usize,
}

impl View {
    pub fn sel(&self) -> Sel {
        (self.cursor, self.anchor)
    }
    pub fn set_sel(&mut self, s: Sel) {
        self.cursor = s.0;
        self.anchor = s.1;
        self.want_col = None;
    }
    /// Ordered, non-empty selection.
    pub fn selection(&self) -> Option<(Pos, Pos)> {
        let a = self.anchor?;
        if a == self.cursor {
            return None;
        }
        Some(if a < self.cursor { (a, self.cursor) } else { (self.cursor, a) })
    }
    pub fn has_selection(&self) -> bool {
        self.selection().is_some()
    }
    /// Selection as an exclusive range, honouring vim visual modes.
    pub fn selection_in(&self, t: &Text) -> Option<(Pos, Pos)> {
        match self.sel_kind {
            SelKind::Exclusive => self.selection(),
            SelKind::Inclusive => {
                let a = self.anchor?;
                let (s, e) = if a <= self.cursor { (a, self.cursor) } else { (self.cursor, a) };
                Some((s, t.next_pos(e)))
            }
            SelKind::Line => {
                let a = self.anchor?;
                let (s, e) = if a <= self.cursor { (a, self.cursor) } else { (self.cursor, a) };
                let end = if e.line + 1 < t.len_lines() { Pos::new(e.line + 1, 0) } else { t.end() };
                Some((Pos::new(s.line, 0), end))
            }
        }
    }
    pub fn clear_selection(&mut self) {
        self.anchor = None;
    }

    /// Every selection (primary included), in document order, as ordered ranges.
    pub fn all_ranges(&self) -> Vec<(Pos, Pos)> {
        let mut v: Vec<(Pos, Pos)> = std::iter::once((self.cursor, self.anchor))
            .chain(self.extra.iter().copied())
            .map(|(c, a)| match a {
                Some(a) if a < c => (a, c),
                Some(a) => (c, a),
                None => (c, c),
            })
            .collect();
        v.sort();
        v
    }
}

/// Soft-wrap segments of a line for a text area `w` columns wide: `(byte, display column)` of each
/// segment start. Breaks after whitespace when a word would overflow, else mid-word.
pub fn wrap_starts(s: &str, w: usize, tab: usize) -> Vec<(usize, usize)> {
    let w = w.max(2);
    let mut out = vec![(0, 0)];
    let (mut seg_b, mut seg_dc) = (0usize, 0usize);
    let mut brk: Option<(usize, usize)> = None;
    let (mut dc, mut i) = (0usize, 0usize);
    while i < s.len() {
        let j = next_boundary(s, i);
        let g = &s[i..j];
        let gw = grapheme_width(g, dc, tab);
        if dc + gw - seg_dc > w && i > seg_b {
            let (nb, ndc) = match brk {
                Some(b) if b.0 > seg_b && b.0 <= i => b,
                _ => (i, dc),
            };
            out.push((nb, ndc));
            seg_b = nb;
            seg_dc = ndc;
            brk = None;
        }
        dc += gw;
        i = j;
        if g == " " || g == "\t" {
            brk = Some((i, dc));
        }
    }
    out
}

/// Where position `p` ends up after `edits` (applied in order).
pub fn map_pos(mut p: Pos, edits: &[Edit]) -> Pos {
    for e in edits {
        match e {
            Edit::Insert { at, text } => {
                if p >= *at {
                    let end = end_of(*at, text);
                    p = if p.line == at.line {
                        Pos::new(end.line, end.col + (p.col - at.col))
                    } else {
                        Pos::new(p.line + (end.line - at.line), p.col)
                    };
                }
            }
            Edit::Remove { at, text } => {
                let b = end_of(*at, text);
                if p >= b {
                    p = if p.line == b.line {
                        Pos::new(at.line, at.col + (p.col - b.col))
                    } else {
                        Pos::new(p.line - (b.line - at.line), p.col)
                    };
                } else if p > *at {
                    p = *at;
                }
            }
        }
    }
    p
}

pub struct Tab {
    pub doc: Document,
    pub view: View,
    /// Preview tabs (opened by browsing the explorer) are replaced by the next preview.
    pub preview: bool,
}

impl Tab {
    pub fn new(doc: Document) -> Tab {
        let view = View { wrap: doc.is_prose(), ..View::default() };
        Tab { doc, view, preview: false }
    }
    pub fn ed(&mut self) -> Ed<'_> {
        Ed { doc: &mut self.doc, v: &mut self.view }
    }
}

const PAIRS: [(char, char); 6] = [('(', ')'), ('[', ']'), ('{', '}'), ('"', '"'), ('\'', '\''), ('`', '`')];

pub struct Ed<'a> {
    pub doc: &'a mut Document,
    pub v: &'a mut View,
}

impl<'a> Ed<'a> {
    fn tw(&self) -> usize {
        self.doc.tab_width
    }

    // ------------------------------------------------------------------ movement

    pub fn move_to(&mut self, p: Pos, select: bool) {
        let p = self.doc.text.clamp(p);
        if select {
            if self.v.anchor.is_none() {
                self.v.anchor = Some(self.v.cursor);
            }
        } else {
            self.v.anchor = None;
        }
        self.v.cursor = p;
        self.v.want_col = None;
        self.doc.history.seal();
    }

    pub fn left(&mut self, select: bool) {
        if !select && let Some((a, _)) = self.v.selection() {
            return self.move_to(a, false);
        }
        let p = self.doc.text.prev_pos(self.v.cursor);
        self.move_to(p, select);
    }

    pub fn right(&mut self, select: bool) {
        if !select && let Some((_, b)) = self.v.selection() {
            return self.move_to(b, false);
        }
        let p = self.doc.text.next_pos(self.v.cursor);
        self.move_to(p, select);
    }

    pub fn vertical(&mut self, delta: isize, select: bool) {
        let t = &self.doc.text;
        let cur = self.v.cursor;
        let want = self.v.want_col.unwrap_or_else(|| t.display_col(cur.line, cur.col, self.doc.tab_width));
        let last = t.len_lines() - 1;
        let target = if delta < 0 {
            if cur.line == 0 {
                self.move_to(Pos::new(0, 0), select);
                return;
            }
            cur.line.saturating_sub((-delta) as usize)
        } else {
            if cur.line == last {
                let e = t.end();
                self.move_to(e, select);
                return;
            }
            (cur.line + delta as usize).min(last)
        };
        let col = t.col_for_display(target, want, self.doc.tab_width);
        self.move_to(Pos::new(target, col), select);
        self.v.want_col = Some(want);
    }

    pub fn up(&mut self, select: bool) {
        if !select && let Some((a, _)) = self.v.selection() {
            self.move_to(a, false);
        }
        if self.v.wrap { self.vertical_visual(-1, select) } else { self.vertical(-1, select) }
    }

    pub fn down(&mut self, select: bool) {
        if !select && let Some((_, b)) = self.v.selection() {
            self.move_to(b, false);
        }
        if self.v.wrap { self.vertical_visual(1, select) } else { self.vertical(1, select) }
    }

    pub fn page(&mut self, down: bool, select: bool) {
        let h = self.v.height.max(2) as isize - 1;
        if self.v.wrap {
            self.vertical_visual(if down { h } else { -h }, select);
            let rows = if down { h } else { -h };
            self.scroll(rows);
            return;
        }
        let old = self.v.cursor.line;
        self.vertical(if down { h } else { -h }, select);
        let moved = self.v.cursor.line as isize - old as isize;
        self.v.scroll_y = (self.v.scroll_y as isize + moved).max(0) as usize;
    }

    // ------------------------------------------------------------------ soft wrap

    /// Wrap segments of line `l` in this view (one segment when wrap is off).
    pub fn segments(&self, l: usize) -> Vec<(usize, usize)> {
        if !self.v.wrap {
            return vec![(0, 0)];
        }
        wrap_starts(self.doc.text.line(l), self.v.width, self.doc.tab_width)
    }

    /// Index of the wrap segment holding `p`.
    fn seg_index(segs: &[(usize, usize)], col: usize) -> usize {
        segs.iter().rposition(|s| s.0 <= col).unwrap_or(0)
    }

    /// Up/down by screen rows when wrapping (the sticky column is relative to the row start).
    pub fn vertical_visual(&mut self, delta: isize, select: bool) {
        let tw = self.doc.tab_width;
        let cur = self.v.cursor;
        let segs = self.segments(cur.line);
        let k = Self::seg_index(&segs, cur.col);
        let want = self.v.want_col.unwrap_or_else(|| self.doc.text.display_col(cur.line, cur.col, tw) - segs[k].1);
        let (mut line, mut seg, mut segs) = (cur.line, k, segs);
        let last = self.doc.text.len_lines() - 1;
        for _ in 0..delta.unsigned_abs() {
            if delta < 0 {
                if seg > 0 {
                    seg -= 1;
                } else if line > 0 {
                    line -= 1;
                    segs = self.segments(line);
                    seg = segs.len() - 1;
                } else {
                    self.move_to(Pos::new(0, 0), select);
                    return;
                }
            } else if seg + 1 < segs.len() {
                seg += 1;
            } else if line < last {
                line += 1;
                segs = self.segments(line);
                seg = 0;
            } else {
                let e = self.doc.text.end();
                self.move_to(e, select);
                return;
            }
        }
        let mut col = self.doc.text.col_for_display(line, segs[seg].1 + want, tw);
        if let Some(next) = segs.get(seg + 1)
            && col >= next.0
        {
            col = self.doc.text.prev_pos(Pos::new(line, next.0)).col.max(segs[seg].0);
        }
        self.move_to(Pos::new(line, col), select);
        self.v.want_col = Some(want);
    }

    /// Move the first visible row by `rows` screen rows (wrap mode).
    fn scroll_rows(&mut self, rows: isize) {
        let last = self.doc.text.len_lines() - 1;
        let mut segs_top = self.segments(self.v.scroll_y).len();
        self.v.scroll_sub = self.v.scroll_sub.min(segs_top - 1);
        for _ in 0..rows.unsigned_abs() {
            if rows > 0 {
                if self.v.scroll_sub + 1 < segs_top {
                    self.v.scroll_sub += 1;
                } else if self.v.scroll_y < last {
                    self.v.scroll_y += 1;
                    self.v.scroll_sub = 0;
                    segs_top = self.segments(self.v.scroll_y).len();
                } else {
                    break;
                }
            } else if self.v.scroll_sub > 0 {
                self.v.scroll_sub -= 1;
            } else if self.v.scroll_y > 0 {
                self.v.scroll_y -= 1;
                segs_top = self.segments(self.v.scroll_y).len();
                self.v.scroll_sub = segs_top - 1;
            } else {
                break;
            }
        }
    }

    /// Smart home: first non-blank, then column 0.
    pub fn home(&mut self, select: bool) {
        let l = self.v.cursor.line;
        let fnb = self.doc.text.first_non_blank(l);
        let col = if self.v.cursor.col == fnb { 0 } else { fnb };
        self.move_to(Pos::new(l, col), select);
    }

    pub fn end(&mut self, select: bool) {
        let l = self.v.cursor.line;
        let len = self.doc.text.line_len(l);
        self.move_to(Pos::new(l, len), select);
    }

    pub fn doc_start(&mut self, select: bool) {
        self.move_to(Pos::new(0, 0), select);
    }

    pub fn doc_end(&mut self, select: bool) {
        let e = self.doc.text.end();
        self.move_to(e, select);
    }

    pub fn word_left(&mut self, select: bool) {
        let p = self.doc.text.word_left(self.v.cursor);
        self.move_to(p, select);
    }

    pub fn word_right(&mut self, select: bool) {
        let p = self.doc.text.word_right(self.v.cursor);
        self.move_to(p, select);
    }

    pub fn goto_line(&mut self, line: usize) {
        let l = line.min(self.doc.text.len_lines() - 1);
        let c = self.doc.text.first_non_blank(l);
        self.move_to(Pos::new(l, c), false);
        self.center();
    }

    pub fn center(&mut self) {
        self.v.scroll_y = self.v.cursor.line.saturating_sub(self.v.height / 2);
    }

    // ------------------------------------------------------------------ multiple cursors

    pub fn multi(&self) -> bool {
        !self.v.extra.is_empty()
    }

    /// Drop the secondary cursors (Esc).
    pub fn collapse(&mut self) -> bool {
        let had = self.multi();
        self.v.extra.clear();
        had
    }

    /// Run a single-cursor edit at every cursor, as one undo step. Cursors are processed from the
    /// last to the first so earlier positions stay valid; positions already processed are then
    /// shifted through the edits each later step makes. `f` gets the cursor's index in document
    /// order.
    pub fn each<F: FnMut(&mut Self, usize)>(&mut self, kind: Coalesce, ch: Option<char>, mut f: F) {
        if self.v.extra.is_empty() {
            return f(self, 0);
        }
        let mut all: Vec<(Sel, bool)> = self.v.extra.iter().map(|&s| (s, false)).collect();
        all.push(((self.v.cursor, self.v.anchor), true));
        let start = |s: &Sel| s.1.map_or(s.0, |a| a.min(s.0));
        all.sort_by_key(|(s, _)| start(s));
        let n = all.len();
        let before = self.v.sel();
        self.doc.begin(before, kind, ch);
        let mut done: Vec<(Sel, bool)> = Vec::new();
        for (k, (s, primary)) in all.into_iter().enumerate().rev() {
            self.v.cursor = s.0;
            self.v.anchor = s.1;
            self.v.want_col = None;
            self.doc.log = Some(Vec::new());
            f(self, k);
            let edits = self.doc.log.take().unwrap_or_default();
            for (d, _) in done.iter_mut() {
                d.0 = map_pos(d.0, &edits);
                d.1 = d.1.map(|p| map_pos(p, &edits));
            }
            done.push(((self.v.cursor, self.v.anchor), primary));
        }
        debug_assert_eq!(done.len(), n);
        self.set_cursors(done);
        let after = self.v.sel();
        self.doc.end(after);
    }

    /// A cursor movement at every cursor.
    pub fn each_move<F: FnMut(&mut Self)>(&mut self, mut f: F) {
        if self.v.extra.is_empty() {
            return f(self);
        }
        let mut done = Vec::new();
        let all: Vec<(Sel, bool)> = std::iter::once(((self.v.cursor, self.v.anchor), true))
            .chain(self.v.extra.iter().map(|&s| (s, false)))
            .collect();
        for (s, primary) in all {
            self.v.cursor = s.0;
            self.v.anchor = s.1;
            self.v.want_col = None;
            f(self);
            done.push(((self.v.cursor, self.v.anchor), primary));
        }
        self.set_cursors(done);
    }

    /// Install cursors, clamped, with duplicates and overlapping selections merged.
    fn set_cursors(&mut self, sels: Vec<(Sel, bool)>) {
        let t = &self.doc.text;
        let mut v: Vec<(Pos, Pos, bool, bool)> = sels
            .into_iter()
            .map(|((c, a), primary)| {
                let c = t.clamp(c);
                let a = a.map(|a| t.clamp(a)).filter(|a| *a != c);
                match a {
                    Some(a) if a < c => (a, c, false, primary),
                    Some(a) => (c, a, true, primary),
                    None => (c, c, false, primary),
                }
            })
            .collect();
        v.sort_by_key(|x| (x.0, x.1));
        let mut merged: Vec<(Pos, Pos, bool, bool)> = Vec::new();
        for x in v {
            match merged.last_mut() {
                Some(m) if x.0 < m.1 || (x.0 == m.1 && (x.0 == x.1 || m.0 == m.1)) => {
                    m.1 = m.1.max(x.1);
                    m.3 |= x.3;
                }
                _ => merged.push(x),
            }
        }
        self.v.extra.clear();
        let mut primary_set = false;
        for (s, e, reversed, primary) in merged {
            let sel = if s == e {
                (s, None)
            } else if reversed {
                (s, Some(e))
            } else {
                (e, Some(s))
            };
            if primary && !primary_set {
                self.v.cursor = sel.0;
                self.v.anchor = sel.1;
                primary_set = true;
            } else {
                self.v.extra.push(sel);
            }
        }
        if !primary_set && let Some(s) = self.v.extra.pop() {
            self.v.cursor = s.0;
            self.v.anchor = s.1;
        }
    }

    /// Add a cursor on the line above / below the topmost / bottommost cursor (Ctrl+Alt+↑/↓).
    pub fn add_cursor_vertical(&mut self, down: bool) {
        let tw = self.doc.tab_width;
        let all = self.v.all_ranges();
        let edge = if down { all.last().map(|r| r.1) } else { all.first().map(|r| r.0) };
        let Some(edge) = edge else { return };
        let line = if down {
            if edge.line + 1 >= self.doc.text.len_lines() {
                return;
            }
            edge.line + 1
        } else {
            let Some(l) = edge.line.checked_sub(1) else { return };
            l
        };
        let want =
            self.v.want_col.unwrap_or_else(|| self.doc.text.display_col(self.v.cursor.line, self.v.cursor.col, tw));
        let col = self.doc.text.col_for_display(line, want, tw);
        let primary = (self.v.cursor, self.v.anchor);
        let mut sels: Vec<(Sel, bool)> = self.v.extra.iter().map(|&s| (s, false)).collect();
        sels.push((primary, false));
        sels.push(((Pos::new(line, col), None), true));
        self.set_cursors(sels);
        self.v.want_col = Some(want);
    }

    /// Select every occurrence of the selection (or the word under the cursor), Ctrl+F2.
    pub fn select_all_occurrences(&mut self) -> usize {
        if !self.v.has_selection() {
            let c = self.v.cursor;
            self.select_word_at(c);
        }
        let Some((a, b)) = self.v.selection() else { return 0 };
        let needle = self.doc.text.slice(a, b);
        let mut hits = Vec::new();
        Query::literal(&needle, true).matches(&self.doc.text, 0, self.doc.text.len_lines(), &mut hits);
        if needle.contains('\n') || hits.is_empty() {
            return 1;
        }
        let n = hits.len();
        let sels = hits.into_iter().map(|(x, y)| ((y, Some(x)), x == a)).collect();
        self.set_cursors(sels);
        n
    }

    /// Add a cursor at `p` (Alt+click); clicking an existing cursor removes it.
    pub fn toggle_cursor_at(&mut self, p: Pos) {
        let p = self.doc.text.clamp(p);
        if let Some(i) = self.v.extra.iter().position(|s| s.0 == p && s.1.is_none()) {
            self.v.extra.remove(i);
            return;
        }
        if self.v.cursor == p && self.v.anchor.is_none() {
            if let Some(s) = self.v.extra.pop() {
                self.v.cursor = s.0;
                self.v.anchor = s.1;
            }
            return;
        }
        let mut sels: Vec<(Sel, bool)> = self.v.extra.iter().map(|&s| (s, false)).collect();
        sels.push(((self.v.cursor, self.v.anchor), false));
        sels.push(((p, None), true));
        self.set_cursors(sels);
    }

    /// Text of every selection (or line, when a cursor has none) joined by newlines, for copy.
    pub fn copy_multi(&self) -> String {
        let t = &self.doc.text;
        let mut v: Vec<Sel> =
            std::iter::once((self.v.cursor, self.v.anchor)).chain(self.v.extra.iter().copied()).collect();
        v.sort_by_key(|s| s.1.map_or(s.0, |a| a.min(s.0)));
        v.iter()
            .map(|&(c, a)| match a {
                Some(a) if a != c => t.slice(a.min(c), a.max(c)),
                _ => t.line(c.line).to_string(),
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Paste at every cursor: one line each when the clipboard has exactly one line per cursor
    /// (VS Code), otherwise the whole text at each.
    pub fn paste_multi(&mut self, text: &str) {
        let n = 1 + self.v.extra.len();
        let lines: Vec<String> = text.strip_suffix('\n').unwrap_or(text).split('\n').map(String::from).collect();
        let distribute = lines.len() == n && n > 1;
        self.each(Coalesce::None, None, |e, k| {
            let s = if distribute { lines[k].as_str() } else { text };
            e.insert_str(s);
        });
    }

    // ------------------------------------------------------------------ selection

    pub fn select_all(&mut self) {
        self.v.anchor = Some(Pos::new(0, 0));
        self.v.cursor = self.doc.text.end();
        self.v.want_col = None;
    }

    /// Select the current line (repeat to extend to following lines), VS Code Ctrl+L.
    pub fn select_line(&mut self) {
        let last = self.doc.text.len_lines() - 1;
        let (start, end_line) = match self.v.selection() {
            Some((a, b)) => (Pos::new(a.line, 0), if b.col == 0 { b.line } else { b.line + 1 }),
            None => (Pos::new(self.v.cursor.line, 0), self.v.cursor.line + 1),
        };
        let end = if end_line > last { self.doc.text.end() } else { Pos::new(end_line, 0) };
        self.v.anchor = Some(start);
        self.v.cursor = end;
    }

    pub fn select_word_at(&mut self, p: Pos) {
        match self.doc.text.word_at(p) {
            Some((a, b)) => {
                self.v.anchor = Some(a);
                self.v.cursor = b;
            }
            None => self.move_to(p, false),
        }
    }

    pub fn select_lines_at(&mut self, line: usize) {
        let last = self.doc.text.len_lines() - 1;
        self.v.anchor = Some(Pos::new(line, 0));
        self.v.cursor = if line < last { Pos::new(line + 1, 0) } else { self.doc.text.end() };
    }

    /// Lines covered by the selection (or the cursor line).
    pub fn selected_lines(&self) -> (usize, usize) {
        match self.v.selection() {
            Some((a, b)) => (a.line, if b.col == 0 && b.line > a.line { b.line - 1 } else { b.line }),
            None => (self.v.cursor.line, self.v.cursor.line),
        }
    }

    // ------------------------------------------------------------------ editing

    /// Run `f` as one undoable transaction.
    pub fn transact<F: FnOnce(&mut Self)>(&mut self, kind: Coalesce, ch: Option<char>, f: F) {
        let sel = self.v.sel();
        self.doc.begin(sel, kind, ch);
        f(self);
        let sel = self.v.sel();
        self.doc.end(sel);
        self.v.want_col = None;
    }

    fn delete_selection_raw(&mut self) -> bool {
        if let Some((a, b)) = self.v.selection() {
            self.doc.remove(a, b);
            self.v.cursor = a;
            self.v.anchor = None;
            true
        } else {
            self.v.anchor = None;
            false
        }
    }

    /// Replace the selection (if any) with `s`.
    pub fn insert_str(&mut self, s: &str) {
        self.transact(Coalesce::None, None, |e| {
            e.delete_selection_raw();
            let c = e.v.cursor;
            e.v.cursor = e.doc.insert(c, s);
        });
    }

    pub fn type_char(&mut self, c: char, autopair: bool) {
        let next = self.doc.text.char_at(self.v.cursor);
        // surround selection with a pair
        if autopair
            && let Some((a, b)) = self.v.selection()
            && let Some(&(o, cl)) = PAIRS.iter().find(|(o, _)| *o == c)
        {
            self.transact(Coalesce::None, None, |e| {
                e.doc.insert(b, &cl.to_string());
                e.doc.insert(a, &o.to_string());
                let b2 = if a.line == b.line { Pos::new(b.line, b.col + o.len_utf8()) } else { b };
                e.v.anchor = Some(Pos::new(a.line, a.col + o.len_utf8()));
                e.v.cursor = b2;
            });
            return;
        }
        // type over a closing char that we auto-inserted
        if autopair && !self.v.has_selection() && next == Some(c) && PAIRS.iter().any(|(_, cl)| *cl == c) {
            let p = self.doc.text.next_pos(self.v.cursor);
            self.v.cursor = p;
            return;
        }
        let pair = if autopair && !self.v.has_selection() {
            PAIRS.iter().find(|(o, _)| *o == c).and_then(|&(o, cl)| {
                let next_ok = match next {
                    None | Some('\n') => true,
                    Some(n) => n.is_whitespace() || ")]};,".contains(n),
                };
                if !next_ok {
                    return None;
                }
                if o == cl {
                    let prev = self.doc.text.line(self.v.cursor.line)[..self.v.cursor.col].chars().next_back();
                    if matches!(prev, Some(p) if p.is_alphanumeric() || p == o) {
                        return None;
                    }
                    if o == '\'' && matches!(self.doc.lang, crate::highlight::Lang::Rust) {
                        return None; // lifetimes
                    }
                }
                Some(cl)
            })
        } else {
            None
        };
        let mut buf = [0u8; 4];
        let s = c.encode_utf8(&mut buf).to_string();
        self.transact(Coalesce::Typing, Some(c), |e| {
            e.delete_selection_raw();
            let at = e.v.cursor;
            let end = e.doc.insert(at, &s);
            if let Some(cl) = pair {
                e.doc.insert(end, &cl.to_string());
            }
            e.v.cursor = end;
        });
        // outdent a closing brace typed on an otherwise blank line
        if matches!(c, '}' | ']' | ')') {
            let l = self.v.cursor.line;
            let line = self.doc.text.line(l).to_string();
            if line.trim() == c.to_string()
                && l > 0
                && let Some(open_line) = self.find_open_bracket_line(Pos::new(l, self.v.cursor.col - 1))
            {
                let ind = self.doc.text.indent_of(open_line).to_string();
                let cur_ind = self.doc.text.indent_of(l).to_string();
                if ind != cur_ind {
                    self.transact(Coalesce::None, None, |e| {
                        e.doc.remove(Pos::new(l, 0), Pos::new(l, cur_ind.len()));
                        e.doc.insert(Pos::new(l, 0), &ind);
                        e.v.cursor = Pos::new(l, ind.len() + 1);
                    });
                }
            }
        }
    }

    fn find_open_bracket_line(&self, close_at: Pos) -> Option<usize> {
        self.matching_bracket_from(close_at).map(|p| p.line)
    }

    pub fn newline(&mut self) {
        let cur = self.v.selection().map(|(a, _)| a).unwrap_or(self.v.cursor);
        let line = self.doc.text.line(cur.line);
        let indent: String = line[..self.doc.text.first_non_blank(cur.line).min(cur.col)].to_string();
        let before = line[..cur.col].trim_end();
        let after_ch = line[cur.col..].trim_start().chars().next();
        let opens = before.ends_with('{')
            || before.ends_with('[')
            || before.ends_with('(')
            || (before.ends_with(':') && matches!(self.doc.lang, crate::highlight::Lang::Python));
        let unit = self.doc.indent_unit();
        let between_pair = opens && matches!(after_ch, Some('}' | ']' | ')'));
        self.transact(Coalesce::None, Some('\n'), |e| {
            e.delete_selection_raw();
            let at = e.v.cursor;
            // drop trailing whitespace before the cursor on the split line
            let ln = e.doc.text.line(at.line).to_string();
            let trimmed = ln[..at.col].trim_end_matches([' ', '\t']).len();
            let at = if trimmed < at.col && trimmed >= indent.len() {
                e.doc.remove(Pos::new(at.line, trimmed), at);
                Pos::new(at.line, trimmed)
            } else {
                at
            };
            if between_pair {
                let s = format!("\n{indent}{unit}\n{indent}");
                e.doc.insert(at, &s);
                e.v.cursor = Pos::new(at.line + 1, indent.len() + unit.len());
            } else {
                let extra = if opens { unit.as_str() } else { "" };
                let s = format!("\n{indent}{extra}");
                let end = e.doc.insert(at, &s);
                // remove leading whitespace of the moved tail
                let tail = e.doc.text.line(end.line).to_string();
                let lead = tail[end.col..].len() - tail[end.col..].trim_start_matches([' ', '\t']).len();
                if lead > 0 {
                    e.doc.remove(end, Pos::new(end.line, end.col + lead));
                }
                e.v.cursor = end;
            }
        });
    }

    pub fn backspace(&mut self) {
        if self.v.has_selection() {
            return self.transact(Coalesce::None, None, |e| {
                e.delete_selection_raw();
            });
        }
        let cur = self.v.cursor;
        if cur == Pos::new(0, 0) {
            return;
        }
        let line = self.doc.text.line(cur.line);
        // delete an empty auto-pair
        let prev = line[..cur.col].chars().next_back();
        let next = line[cur.col..].chars().next();
        let pair_del = matches!((prev, next), (Some(a), Some(b)) if PAIRS.contains(&(a, b)));
        // indentation-aware: in leading whitespace of spaces, remove to previous indent stop
        let in_indent = cur.col > 0 && line[..cur.col].bytes().all(|b| b == b' ') && !self.doc.use_tabs;
        let start = if in_indent {
            let tw = self.doc.tab_width;
            let target = (cur.col - 1) / tw * tw;
            Pos::new(cur.line, target)
        } else {
            self.doc.text.prev_pos(cur)
        };
        self.transact(Coalesce::Deleting, None, |e| {
            let end = if pair_del { e.doc.text.next_pos(cur) } else { cur };
            e.doc.remove(start, end);
            e.v.cursor = start;
            e.v.anchor = None;
        });
    }

    pub fn delete(&mut self) {
        if self.v.has_selection() {
            return self.transact(Coalesce::None, None, |e| {
                e.delete_selection_raw();
            });
        }
        let cur = self.v.cursor;
        let next = self.doc.text.next_pos(cur);
        if next == cur {
            return;
        }
        self.transact(Coalesce::Deleting, None, |e| {
            e.doc.remove(cur, next);
            e.v.anchor = None;
        });
    }

    pub fn delete_word_left(&mut self) {
        if self.v.has_selection() {
            return self.backspace();
        }
        let cur = self.v.cursor;
        let start = self.doc.text.word_left(cur);
        self.transact(Coalesce::None, None, |e| {
            e.doc.remove(start, cur);
            e.v.cursor = start;
        });
    }

    pub fn delete_word_right(&mut self) {
        if self.v.has_selection() {
            return self.delete();
        }
        let cur = self.v.cursor;
        let end = self.doc.text.word_right(cur);
        self.transact(Coalesce::None, None, |e| {
            e.doc.remove(cur, end);
        });
    }

    pub fn delete_to_line_start(&mut self) {
        let cur = self.v.cursor;
        let start = Pos::new(cur.line, self.doc.text.first_non_blank(cur.line).min(cur.col));
        let start = if start == cur { Pos::new(cur.line, 0) } else { start };
        self.transact(Coalesce::None, None, |e| {
            e.doc.remove(start, cur);
            e.v.cursor = start;
        });
    }

    /// Ctrl+Delete: delete from the cursor to the end of the line (the line break if already there).
    pub fn delete_to_line_end(&mut self) {
        if self.v.has_selection() {
            return self.delete();
        }
        let cur = self.v.cursor;
        let len = self.doc.text.line_len(cur.line);
        let end = if cur.col < len { Pos::new(cur.line, len) } else { self.doc.text.next_pos(cur) };
        if end == cur {
            return;
        }
        self.transact(Coalesce::None, None, |e| {
            e.doc.remove(cur, end);
        });
    }

    /// Tab key: indent selected lines, or insert indentation at the cursor.
    pub fn tab(&mut self) {
        if let Some((a, b)) = self.v.selection()
            && a.line != b.line
        {
            return self.indent_lines(true);
        }
        let unit = if self.doc.use_tabs {
            "\t".to_string()
        } else {
            let tw = self.doc.tab_width;
            let dc = self.doc.text.display_col(self.v.cursor.line, self.v.cursor.col, tw);
            " ".repeat(tw - dc % tw)
        };
        self.insert_str(&unit);
    }

    pub fn indent_lines(&mut self, indent: bool) {
        let (l0, l1) = self.selected_lines();
        let unit = self.doc.indent_unit();
        let tw = self.doc.tab_width;
        let sel = self.v.sel();
        self.transact(Coalesce::None, None, |e| {
            let mut deltas: Vec<(usize, isize)> = Vec::new();
            for l in l0..=l1 {
                if indent {
                    if !e.doc.text.line(l).is_empty() {
                        e.doc.insert(Pos::new(l, 0), &unit);
                        deltas.push((l, unit.len() as isize));
                    }
                } else {
                    let line = e.doc.text.line(l);
                    let n = if line.starts_with('\t') {
                        1
                    } else {
                        line.bytes().take(tw).take_while(|&b| b == b' ').count()
                    };
                    if n > 0 {
                        e.doc.remove(Pos::new(l, 0), Pos::new(l, n));
                        deltas.push((l, -(n as isize)));
                    }
                }
            }
            let adj = |p: Pos| -> Pos {
                match deltas.iter().find(|(l, _)| *l == p.line) {
                    Some((_, d)) => Pos::new(p.line, (p.col as isize + d).max(0) as usize),
                    None => p,
                }
            };
            e.v.cursor = e.doc.text.clamp(adj(sel.0));
            e.v.anchor = sel.1.map(|a| e.doc.text.clamp(adj(a)));
        });
    }

    // ------------------------------------------------------------------ clipboard

    /// Text to copy: the selection, or the whole current line (linewise) when nothing is selected.
    pub fn copy_text(&self) -> (String, bool) {
        match self.v.selection() {
            Some((a, b)) => (self.doc.text.slice(a, b), false),
            None => (format!("{}\n", self.doc.text.line(self.v.cursor.line)), true),
        }
    }

    pub fn cut(&mut self) -> (String, bool) {
        let (s, linewise) = self.copy_text();
        if linewise {
            self.delete_lines();
        } else {
            self.transact(Coalesce::None, None, |e| {
                e.delete_selection_raw();
            });
        }
        (s, linewise)
    }

    pub fn paste(&mut self, text: &str, linewise: bool) {
        if linewise && !self.v.has_selection() {
            let l = self.v.cursor.line;
            let col = self.v.cursor.col;
            self.transact(Coalesce::None, None, |e| {
                e.doc.insert(Pos::new(l, 0), text);
                let added = text.matches('\n').count();
                e.v.cursor = Pos::new(l + added, col);
            });
        } else {
            self.insert_str(text);
        }
    }

    // ------------------------------------------------------------------ line operations

    pub fn delete_lines(&mut self) {
        let (l0, l1) = self.selected_lines();
        let last = self.doc.text.len_lines() - 1;
        let col = self.v.cursor.col;
        self.transact(Coalesce::None, None, |e| {
            let (a, b) = if l1 < last {
                (Pos::new(l0, 0), Pos::new(l1 + 1, 0))
            } else if l0 > 0 {
                (Pos::new(l0 - 1, e.doc.text.line_len(l0 - 1)), Pos::new(l1, e.doc.text.line_len(l1)))
            } else {
                (Pos::new(0, 0), Pos::new(l1, e.doc.text.line_len(l1)))
            };
            e.doc.remove(a, b);
            let l = l0.min(e.doc.text.len_lines() - 1);
            e.v.cursor = e.doc.text.clamp(Pos::new(l, col));
            e.v.anchor = None;
        });
    }

    pub fn move_lines(&mut self, up: bool) {
        let (l0, l1) = self.selected_lines();
        let last = self.doc.text.len_lines() - 1;
        if (up && l0 == 0) || (!up && l1 == last) {
            return;
        }
        let sel = self.v.sel();
        self.transact(Coalesce::None, None, |e| {
            if up {
                let moved = e.doc.text.line(l0 - 1).to_string();
                e.doc.remove(Pos::new(l0 - 1, 0), Pos::new(l0, 0));
                e.doc.insert(Pos::new(l1 - 1, e.doc.text.line_len(l1 - 1)), &format!("\n{moved}"));
            } else {
                let moved = e.doc.text.line(l1 + 1).to_string();
                e.doc.remove(Pos::new(l1, e.doc.text.line_len(l1)), Pos::new(l1 + 1, moved.len()));
                e.doc.insert(Pos::new(l0, 0), &format!("{moved}\n"));
            }
            let d: isize = if up { -1 } else { 1 };
            e.v.cursor = Pos::new((sel.0.line as isize + d) as usize, sel.0.col);
            e.v.anchor = sel.1.map(|p| Pos::new((p.line as isize + d) as usize, p.col));
        });
    }

    pub fn copy_lines(&mut self, up: bool) {
        let (l0, l1) = self.selected_lines();
        let block: Vec<String> = (l0..=l1).map(|l| self.doc.text.line(l).to_string()).collect();
        let block = block.join("\n");
        let n = l1 - l0 + 1;
        let sel = self.v.sel();
        self.transact(Coalesce::None, None, |e| {
            let at = Pos::new(l1, e.doc.text.line_len(l1));
            e.doc.insert(at, &format!("\n{block}"));
            if !up {
                e.v.cursor = Pos::new(sel.0.line + n, sel.0.col);
                e.v.anchor = sel.1.map(|p| Pos::new(p.line + n, p.col));
            }
        });
    }

    pub fn insert_line(&mut self, below: bool) {
        let l = self.v.cursor.line;
        let indent = self.doc.text.indent_of(l).to_string();
        self.transact(Coalesce::None, None, |e| {
            if below {
                let at = Pos::new(l, e.doc.text.line_len(l));
                e.v.cursor = e.doc.insert(at, &format!("\n{indent}"));
            } else {
                e.doc.insert(Pos::new(l, 0), &format!("{indent}\n"));
                e.v.cursor = Pos::new(l, indent.len());
            }
            e.v.anchor = None;
        });
    }

    pub fn toggle_comment(&mut self) -> bool {
        let Some(tok) = self.doc.lang.comment_token() else { return false };
        let (l0, l1) = self.selected_lines();
        let lines: Vec<usize> = (l0..=l1).filter(|&l| !self.doc.text.line(l).trim().is_empty()).collect();
        if lines.is_empty() {
            return true;
        }
        let all_commented = lines.iter().all(|&l| self.doc.text.line(l).trim_start().starts_with(tok));
        let min_indent = lines.iter().map(|&l| self.doc.text.first_non_blank(l)).min().unwrap_or(0);
        let sel = self.v.sel();
        self.transact(Coalesce::None, None, |e| {
            let mut deltas = Vec::new();
            for &l in &lines {
                if all_commented {
                    let fnb = e.doc.text.first_non_blank(l);
                    let rest = &e.doc.text.line(l)[fnb + tok.len()..];
                    let n = tok.len() + usize::from(rest.starts_with(' '));
                    e.doc.remove(Pos::new(l, fnb), Pos::new(l, fnb + n));
                    deltas.push((l, fnb, -(n as isize)));
                } else {
                    let s = format!("{tok} ");
                    e.doc.insert(Pos::new(l, min_indent), &s);
                    deltas.push((l, min_indent, s.len() as isize));
                }
            }
            let adj = |p: Pos| match deltas.iter().find(|(l, _, _)| *l == p.line) {
                Some(&(_, at, d)) if p.col >= at => Pos::new(p.line, (p.col as isize + d).max(at as isize) as usize),
                _ => p,
            };
            e.v.cursor = e.doc.text.clamp(adj(sel.0));
            e.v.anchor = sel.1.map(|a| e.doc.text.clamp(adj(a)));
        });
        true
    }

    // ------------------------------------------------------------------ history

    pub fn undo(&mut self) -> bool {
        match self.doc.undo() {
            Some(s) => {
                self.v.set_sel((self.doc.text.clamp(s.0), s.1.map(|a| self.doc.text.clamp(a))));
                true
            }
            None => false,
        }
    }

    pub fn redo(&mut self) -> bool {
        match self.doc.redo() {
            Some(s) => {
                self.v.set_sel((self.doc.text.clamp(s.0), s.1.map(|a| self.doc.text.clamp(a))));
                true
            }
            None => false,
        }
    }

    // ------------------------------------------------------------------ search

    pub fn find(&mut self, needle: &str, forward: bool, case: bool) -> bool {
        self.find_q(&Query::literal(needle, case), forward)
    }

    /// Select the next (previous) match of `q` after (before) the cursor / selection.
    pub fn find_q(&mut self, q: &Query, forward: bool) -> bool {
        let from = if forward {
            self.v.selection().map(|(_, b)| b).unwrap_or(self.v.cursor)
        } else {
            self.v.selection().map(|(a, _)| a).unwrap_or(self.v.cursor)
        };
        match q.find(&self.doc.text, from, forward) {
            Some((a, b)) => {
                self.v.anchor = Some(a);
                self.v.cursor = b;
                self.v.want_col = None;
                true
            }
            None => false,
        }
    }

    /// Ctrl+D: select the word under the cursor; with a selection, add a cursor at the next
    /// occurrence (VS Code). Returns false when there is nothing to select or add.
    pub fn select_next_occurrence(&mut self) -> bool {
        match self.v.selection() {
            None => {
                let c = self.v.cursor;
                self.select_word_at(c);
                self.v.has_selection()
            }
            Some((a, b)) => {
                let needle = self.doc.text.slice(a, b);
                let q = Query::literal(&needle, true);
                // continue after the last occurrence already selected
                let mut from = b;
                for &(c, an) in &self.v.extra {
                    if let Some(an) = an {
                        let e = c.max(an);
                        if e > from {
                            from = e;
                        }
                    }
                }
                let Some((na, nb)) = q.find(&self.doc.text, from, true) else { return false };
                let taken = |p: Pos| p == a || self.v.extra.iter().any(|&(c, an)| an.is_some_and(|an| c.min(an) == p));
                if taken(na) {
                    return false; // every occurrence is selected
                }
                let primary = (self.v.cursor, self.v.anchor);
                self.v.extra.push(primary);
                self.v.anchor = Some(na);
                self.v.cursor = nb;
                self.v.want_col = None;
                true
            }
        }
    }

    pub fn replace_selection_if_match(&mut self, q: &Query, rep: &str) -> bool {
        if let Some((a, b)) = self.v.selection()
            && q.matches_range(&self.doc.text, a, b)
        {
            let r = q.replacement(&self.doc.text, a, b, rep, false);
            self.insert_str(&r);
            return true;
        }
        false
    }

    /// Replace every match of `q` in lines `[l0, l1)` (`first_only`: the first match per line,
    /// vim `:s` without `g`). Regex replacements expand `$1`/`\1` (and `&` for vim).
    pub fn replace_in(&mut self, q: &Query, rep: &str, l0: usize, l1: usize, first_only: bool, vim: bool) -> usize {
        let mut hits = Vec::new();
        match q {
            Query::Lit { needle, case } if needle.contains('\n') => {
                let mut from = Pos::new(l0, 0);
                let mut seen = std::collections::HashSet::new();
                while let Some((a, b)) = self.doc.text.find(needle, from, true, *case) {
                    if !seen.insert(a) || a < from || a.line >= l1 {
                        break;
                    }
                    hits.push((a, b));
                    from = b;
                }
            }
            _ => q.matches(&self.doc.text, l0, l1, &mut hits),
        }
        hits.retain(|(a, b)| a != b);
        if first_only {
            let mut seen = std::collections::HashSet::new();
            hits.retain(|(a, _)| seen.insert(a.line));
        }
        let count = hits.len();
        if count == 0 {
            return 0;
        }
        let reps: Vec<String> = hits.iter().map(|&(a, b)| q.replacement(&self.doc.text, a, b, rep, vim)).collect();
        self.transact(Coalesce::None, None, |e| {
            for (k, &(a, b)) in hits.iter().enumerate().rev() {
                e.doc.remove(a, b);
                e.doc.insert(a, &reps[k]);
            }
            let c = e.v.cursor;
            e.v.cursor = e.doc.text.clamp(c);
            e.v.anchor = None;
        });
        count
    }

    pub fn replace_all(&mut self, q: &Query, rep: &str) -> usize {
        let n = self.doc.text.len_lines();
        self.replace_in(q, rep, 0, n, false, false)
    }

    // ------------------------------------------------------------------ brackets

    fn matching_bracket_from(&self, p: Pos) -> Option<Pos> {
        self.doc.text.match_bracket(p)
    }

    /// Bracket pair touching the cursor (at the cursor, or just before it).
    pub fn matching_bracket(&self) -> Option<(Pos, Pos)> {
        let cur = self.v.cursor;
        if let Some(m) = self.matching_bracket_from(cur) {
            return Some((cur, m));
        }
        if cur.col > 0 {
            let p = self.doc.text.prev_pos(cur);
            if let Some(m) = self.matching_bracket_from(p) {
                return Some((p, m));
            }
        }
        None
    }

    pub fn jump_to_bracket(&mut self) {
        if let Some((_, m)) = self.matching_bracket() {
            self.move_to(m, false);
        }
    }

    // ------------------------------------------------------------------ viewport

    pub fn ensure_visible(&mut self) {
        if self.v.wrap {
            self.v.scroll_x = 0;
            let h = self.v.height.max(1);
            let cur = self.v.cursor;
            let k = Self::seg_index(&self.segments(cur.line), cur.col);
            let top = (self.v.scroll_y, self.v.scroll_sub);
            if (cur.line, k) < top {
                self.v.scroll_y = cur.line;
                self.v.scroll_sub = k;
                return;
            }
            // rows from the top of the view to the cursor row
            let mut rows = 0usize;
            let mut l = self.v.scroll_y;
            let mut sub = self.v.scroll_sub;
            while l < cur.line && rows < h {
                rows += self.segments(l).len() - sub;
                sub = 0;
                l += 1;
            }
            rows = rows + k - sub.min(k);
            if rows >= h {
                self.scroll_rows((rows + 1 - h) as isize);
            }
            return;
        }
        let h = self.v.height.max(1);
        let w = self.v.width.max(1);
        let margin = 0;
        let line = self.v.cursor.line;
        if line < self.v.scroll_y + margin {
            self.v.scroll_y = line.saturating_sub(margin);
        } else if line + margin >= self.v.scroll_y + h {
            self.v.scroll_y = (line + margin + 1).saturating_sub(h);
        }
        let max_scroll = self.doc.text.len_lines().saturating_sub(1);
        self.v.scroll_y = self.v.scroll_y.min(max_scroll);
        let dc = self.doc.text.display_col(line, self.v.cursor.col, self.tw());
        if dc < self.v.scroll_x {
            self.v.scroll_x = dc.saturating_sub(w / 4);
        } else if dc >= self.v.scroll_x + w {
            self.v.scroll_x = dc + 1 - w + w / 4;
        }
    }

    /// Document position under a text-area cell (row, col relative to the text area).
    pub fn pos_at(&self, row: usize, col: usize) -> Pos {
        let t = &self.doc.text;
        if self.v.wrap {
            let (mut l, mut sub, mut r) = (self.v.scroll_y, self.v.scroll_sub, row);
            loop {
                let segs = self.segments(l);
                let sub0 = sub.min(segs.len() - 1);
                if r < segs.len() - sub0 || l + 1 >= t.len_lines() {
                    let k = (sub0 + r).min(segs.len() - 1);
                    let mut c = t.col_for_display(l, segs[k].1 + col, self.tw());
                    if let Some(next) = segs.get(k + 1)
                        && c >= next.0
                    {
                        c = t.prev_pos(Pos::new(l, next.0)).col.max(segs[k].0);
                    }
                    return Pos::new(l, c);
                }
                r -= segs.len() - sub0;
                sub = 0;
                l += 1;
            }
        }
        let line = (self.v.scroll_y + row).min(t.len_lines() - 1);
        let c = t.col_for_display(line, self.v.scroll_x + col, self.tw());
        Pos::new(line, c)
    }

    pub fn scroll(&mut self, lines: isize) {
        if self.v.wrap {
            return self.scroll_rows(lines);
        }
        let max = self.doc.text.len_lines().saturating_sub(1) as isize;
        self.v.scroll_y = (self.v.scroll_y as isize + lines).clamp(0, max) as usize;
    }

    /// Word class of the char at the cursor (for vim / double-click helpers).
    pub fn class_at(&self, p: Pos) -> CharClass {
        self.doc.text.char_at(p).map(char_class).unwrap_or(CharClass::Space)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::Text;

    fn tab(s: &str) -> Tab {
        let mut d = Document::untitled(1);
        d.text = Text::parse(s).0;
        let mut t = Tab::new(d);
        t.view.height = 20;
        t.view.width = 80;
        t
    }

    fn content(t: &Tab) -> String {
        t.doc.text.to_string_with(Default::default())
    }

    #[test]
    fn typing_autopair_and_undo() {
        let mut t = tab("");
        for c in "f(x".chars() {
            t.ed().type_char(c, true);
        }
        assert_eq!(content(&t), "f(x)");
        t.ed().type_char(')', true);
        assert_eq!(content(&t), "f(x)");
        assert_eq!(t.view.cursor, Pos::new(0, 4));
        assert!(t.ed().undo());
        assert_eq!(content(&t), "");
    }

    #[test]
    fn newline_indent_between_braces() {
        let mut t = tab("fn x() {}");
        t.view.cursor = Pos::new(0, 8);
        t.ed().newline();
        assert_eq!(content(&t), "fn x() {\n    \n}");
        assert_eq!(t.view.cursor, Pos::new(1, 4));
    }

    #[test]
    fn line_ops() {
        let mut t = tab("a\nb\nc");
        t.view.cursor = Pos::new(1, 0);
        t.ed().move_lines(true);
        assert_eq!(content(&t), "b\na\nc");
        assert_eq!(t.view.cursor.line, 0);
        t.ed().copy_lines(false);
        assert_eq!(content(&t), "b\nb\na\nc");
        t.ed().delete_lines();
        assert_eq!(content(&t), "b\na\nc");
        t.view.cursor = Pos::new(2, 0);
        t.ed().delete_lines();
        assert_eq!(content(&t), "b\na");
    }

    #[test]
    fn comment_toggle_and_indent() {
        let mut t = tab("x\n  y");
        t.doc.lang = crate::highlight::Lang::Rust;
        t.ed().select_all();
        t.ed().toggle_comment();
        assert_eq!(content(&t), "// x\n//   y");
        t.ed().select_all();
        t.ed().toggle_comment();
        assert_eq!(content(&t), "x\n  y");
        t.ed().select_all();
        t.ed().indent_lines(true);
        assert_eq!(content(&t), "    x\n      y");
        t.ed().indent_lines(false);
        assert_eq!(content(&t), "x\n  y");
    }

    #[test]
    fn replace_all_and_next_occurrence() {
        let mut t = tab("foo bar foo\nfoo");
        assert_eq!(t.ed().replace_all(&Query::literal("foo", true), "baz"), 3);
        assert_eq!(content(&t), "baz bar baz\nbaz");
        t.view.cursor = Pos::new(0, 1);
        assert!(t.ed().select_next_occurrence());
        assert_eq!(t.view.selection(), Some((Pos::new(0, 0), Pos::new(0, 3))));
        assert!(t.ed().select_next_occurrence());
        assert_eq!(t.view.selection(), Some((Pos::new(0, 8), Pos::new(0, 11))));
    }

    #[test]
    fn backspace_pairs_and_indent() {
        let mut t = tab("()");
        t.view.cursor = Pos::new(0, 1);
        t.ed().backspace();
        assert_eq!(content(&t), "");
        let mut t = tab("        x");
        t.view.cursor = Pos::new(0, 8);
        t.ed().backspace();
        assert_eq!(content(&t), "    x");
    }

    #[test]
    fn cut_whole_line_and_paste_linewise() {
        let mut t = tab("one\ntwo");
        let (s, lw) = t.ed().cut();
        assert_eq!((s.as_str(), lw), ("one\n", true));
        assert_eq!(content(&t), "two");
        t.ed().paste("one\n", true);
        assert_eq!(content(&t), "one\ntwo");
    }

    #[test]
    fn multi_cursor_typing_is_one_undo_step() {
        let mut t = tab("ab\ncd\nef");
        t.view.cursor = Pos::new(0, 1);
        t.ed().add_cursor_vertical(true);
        t.ed().add_cursor_vertical(true);
        assert_eq!(t.view.extra.len(), 2);
        for c in "XY".chars() {
            t.ed().each(Coalesce::Typing, Some(c), |e, _| e.type_char(c, true));
        }
        assert_eq!(content(&t), "aXYb\ncXYd\neXYf");
        assert_eq!(t.view.cursor, Pos::new(2, 3));
        t.ed().each(Coalesce::Deleting, None, |e, _| e.backspace());
        assert_eq!(content(&t), "aXb\ncXd\neXf");
        assert!(t.ed().undo());
        assert_eq!(content(&t), "aXYb\ncXYd\neXYf");
        assert!(t.ed().undo());
        assert_eq!(content(&t), "ab\ncd\nef");
    }

    #[test]
    fn multi_cursor_same_line_and_newlines() {
        let mut t = tab("a,b,c");
        t.view.cursor = Pos::new(0, 1);
        t.ed().toggle_cursor_at(Pos::new(0, 3));
        assert_eq!(t.view.extra.len(), 1);
        t.ed().each(Coalesce::None, Some('\n'), |e, _| e.newline());
        assert_eq!(content(&t), "a\n,b\n,c");
        let all = t.view.all_ranges();
        assert_eq!(all, vec![(Pos::new(1, 0), Pos::new(1, 0)), (Pos::new(2, 0), Pos::new(2, 0))]);
        // moving both to line start merges nothing; Esc collapses
        t.ed().each_move(|e| e.left(false));
        assert_eq!(t.view.all_ranges().len(), 2);
        assert!(t.ed().collapse());
        assert!(t.view.extra.is_empty());
    }

    #[test]
    fn next_and_all_occurrences_paste_and_copy() {
        let mut t = tab("foo x foo y foo");
        t.view.cursor = Pos::new(0, 1);
        assert!(t.ed().select_next_occurrence()); // word
        assert!(t.ed().select_next_occurrence()); // + 2nd
        assert!(t.ed().select_next_occurrence()); // + 3rd
        assert!(!t.ed().select_next_occurrence()); // all selected
        assert_eq!(t.view.extra.len(), 2);
        assert_eq!(t.ed().copy_multi(), "foo\nfoo\nfoo");
        t.ed().paste_multi("1\n2\n3\n");
        assert_eq!(content(&t), "1 x 2 y 3");
        let mut t = tab("ab ab\nab");
        t.view.cursor = Pos::new(0, 0);
        assert_eq!(t.ed().select_all_occurrences(), 3);
        t.ed().each(Coalesce::None, None, |e, _| e.insert_str("Z"));
        assert_eq!(content(&t), "Z Z\nZ");
        // the same text pasted at each cursor when the line count differs
        t.ed().paste_multi("q");
        assert_eq!(content(&t), "Zq Zq\nZq");
    }

    #[test]
    fn map_pos_through_edits() {
        let e = vec![Edit::Insert { at: Pos::new(0, 1), text: "xy\nz".into() }];
        assert_eq!(map_pos(Pos::new(0, 3), &e), Pos::new(1, 3));
        assert_eq!(map_pos(Pos::new(2, 0), &e), Pos::new(3, 0));
        assert_eq!(map_pos(Pos::new(0, 0), &e), Pos::new(0, 0));
        let e = vec![Edit::Remove { at: Pos::new(0, 1), text: "ab\ncd".into() }];
        assert_eq!(map_pos(Pos::new(1, 5), &e), Pos::new(0, 4));
        assert_eq!(map_pos(Pos::new(1, 0), &e), Pos::new(0, 1));
        assert_eq!(map_pos(Pos::new(3, 2), &e), Pos::new(2, 2));
    }

    #[test]
    fn soft_wrap_segments_and_movement() {
        assert_eq!(wrap_starts("aaaa bbbb cccc", 10, 4), vec![(0, 0), (10, 10)]);
        assert_eq!(wrap_starts("aaaa bbbb cccc", 6, 4), vec![(0, 0), (5, 5), (10, 10)]);
        assert_eq!(wrap_starts("abcdefghij", 4, 4), vec![(0, 0), (4, 4), (8, 8)]);
        assert_eq!(wrap_starts("", 4, 4), vec![(0, 0)]);
        let mut t = tab("one two three four five\nsix");
        t.view.wrap = true;
        t.view.width = 10;
        t.view.height = 2;
        // rows: "one two " | "three " | "four five" | "six"
        t.view.cursor = Pos::new(0, 1);
        t.ed().down(false);
        assert_eq!(t.view.cursor, Pos::new(0, 9));
        t.ed().down(false);
        assert_eq!(t.view.cursor, Pos::new(0, 15));
        t.ed().ensure_visible();
        assert_eq!((t.view.scroll_y, t.view.scroll_sub), (0, 1));
        t.ed().down(false);
        assert_eq!(t.view.cursor, Pos::new(1, 1));
        t.ed().ensure_visible();
        assert_eq!((t.view.scroll_y, t.view.scroll_sub), (0, 2));
        assert_eq!(t.ed().pos_at(0, 2), Pos::new(0, 16));
        assert_eq!(t.ed().pos_at(1, 0), Pos::new(1, 0));
        t.ed().up(false);
        t.ed().up(false);
        assert_eq!(t.view.cursor, Pos::new(0, 9));
        t.ed().ensure_visible();
        assert_eq!((t.view.scroll_y, t.view.scroll_sub), (0, 1));
        t.ed().scroll(-5);
        assert_eq!((t.view.scroll_y, t.view.scroll_sub), (0, 0));
    }

    #[test]
    fn reload_is_undoable() {
        let dir = std::env::temp_dir().join(format!("cmx-reload-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("f.txt");
        std::fs::write(&p, "old\n").unwrap();
        let mut d = Document::open(&p).unwrap();
        assert!(d.is_prose());
        std::fs::write(&p, "new text\n").unwrap();
        assert!(d.reload().unwrap());
        assert_eq!(d.text.line(0), "new text");
        assert!(!d.is_dirty());
        d.undo();
        assert_eq!(d.text.line(0), "old");
        assert!(!d.reload().unwrap() || d.text.line(0) == "new text");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
