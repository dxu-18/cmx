//! Undo/redo: transactions of primitive edits, with typing coalescing and save-point tracking.

use crate::text::{Pos, Text};
use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
pub enum Edit {
    Insert { at: Pos, text: String },
    Remove { at: Pos, text: String },
}

/// Cursor + anchor snapshot.
pub type Sel = (Pos, Option<Pos>);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Coalesce {
    /// Never merge with neighbours.
    None,
    /// Typing characters.
    Typing,
    /// Backspace/delete of single characters.
    Deleting,
}

#[derive(Clone, Debug)]
struct Txn {
    id: u64,
    edits: Vec<Edit>,
    before: Sel,
    after: Sel,
    kind: Coalesce,
    at: Instant,
    /// Last char typed (for word-boundary coalescing breaks).
    last_char: Option<char>,
}

#[derive(Default)]
pub struct History {
    undo: Vec<Txn>,
    redo: Vec<Txn>,
    open: Option<Txn>,
    next_id: u64,
    /// Force the next transaction not to coalesce (cursor moved, saved, ...).
    seal: bool,
    /// Nested `begin`s inside an open transaction (multi-cursor edits run each cursor's own
    /// transaction inside one outer transaction: a single undo step).
    depth: usize,
}

const COALESCE_WINDOW: Duration = Duration::from_millis(1500);

/// Position right after `text` inserted at `at`.
pub fn end_of(at: Pos, text: &str) -> Pos {
    match text.rfind('\n') {
        Some(i) => Pos::new(at.line + text.matches('\n').count(), text.len() - i - 1),
        None => Pos::new(at.line, at.col + text.len()),
    }
}

fn apply(text: &mut Text, e: &Edit, inverse: bool) {
    match (e, inverse) {
        (Edit::Insert { at, text: s }, false) | (Edit::Remove { at, text: s }, true) => {
            text.insert(*at, s);
        }
        (Edit::Insert { at, text: s }, true) | (Edit::Remove { at, text: s }, false) => {
            text.remove(*at, end_of(*at, s));
        }
    }
}

impl History {
    pub fn new() -> History {
        History { next_id: 1, ..Default::default() }
    }

    /// Version id of the current state (0 = pristine). Compare with a saved value for dirtiness.
    pub fn version(&self) -> u64 {
        self.undo.last().map(|t| t.id).unwrap_or(0)
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Prevent the next edit from merging into the previous transaction.
    pub fn seal(&mut self) {
        self.seal = true;
    }

    /// Start a transaction. May reopen the previous one to coalesce typing runs.
    pub fn begin(&mut self, sel: Sel, kind: Coalesce, ch: Option<char>) {
        if self.open.is_some() {
            self.depth += 1;
            return;
        }
        let id = self.next_id;
        self.next_id += 1;
        if !self.seal
            && kind != Coalesce::None
            && let Some(last) = self.undo.last()
        {
            let breaks_word =
                matches!(ch, Some(c) if c.is_whitespace()) && !matches!(last.last_char, Some(c) if c.is_whitespace());
            if last.kind == kind
                && last.after == sel
                && last.at.elapsed() < COALESCE_WINDOW
                && !(kind == Coalesce::Typing && (breaks_word || ch == Some('\n')))
            {
                let mut t = self.undo.pop().unwrap();
                t.id = id; // contents change → new version id
                t.at = Instant::now();
                t.last_char = ch;
                self.open = Some(t);
                return;
            }
        }
        self.seal = false;
        self.open =
            Some(Txn { id, edits: Vec::new(), before: sel, after: sel, kind, at: Instant::now(), last_char: ch });
    }

    pub fn record(&mut self, e: Edit) {
        if let Some(t) = self.open.as_mut() {
            // merge adjacent typing inserts for compactness
            if let (Some(Edit::Insert { at, text }), Edit::Insert { at: at2, text: t2 }) = (t.edits.last_mut(), &e)
                && end_of(*at, text) == *at2
            {
                text.push_str(t2);
                return;
            }
            t.edits.push(e);
        }
    }

    /// Close the transaction (dropped if it contains no edits).
    pub fn end(&mut self, sel: Sel) {
        if self.depth > 0 {
            self.depth -= 1;
            return;
        }
        if let Some(mut t) = self.open.take() {
            if t.edits.is_empty() {
                return;
            }
            t.after = sel;
            self.undo.push(t);
            self.redo.clear();
            if self.undo.len() > 10_000 {
                self.undo.remove(0);
            }
        }
    }

    pub fn undo(&mut self, text: &mut Text) -> Option<Sel> {
        self.open = None;
        self.depth = 0;
        let t = self.undo.pop()?;
        for e in t.edits.iter().rev() {
            apply(text, e, true);
        }
        let sel = t.before;
        self.redo.push(t);
        self.seal = true;
        Some(sel)
    }

    pub fn redo(&mut self, text: &mut Text) -> Option<Sel> {
        self.open = None;
        self.depth = 0;
        let t = self.redo.pop()?;
        for e in &t.edits {
            apply(text, e, false);
        }
        let sel = t.after;
        self.undo.push(t);
        self.seal = true;
        Some(sel)
    }

    /// Smallest line touched by the most recent undo/redo (for highlight invalidation).
    pub fn last_touched_line(&self) -> usize {
        let f = |t: &Txn| {
            t.edits
                .iter()
                .map(|e| match e {
                    Edit::Insert { at, .. } | Edit::Remove { at, .. } => at.line,
                })
                .min()
                .unwrap_or(0)
        };
        self.redo.last().map(f).into_iter().chain(self.undo.last().map(f)).min().unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undo_redo_and_coalesce() {
        let mut text = Text::new();
        let mut h = History::new();
        let mut cur = Pos::new(0, 0);
        for c in "ab cd".chars() {
            h.begin((cur, None), Coalesce::Typing, Some(c));
            let s = c.to_string();
            h.record(Edit::Insert { at: cur, text: s.clone() });
            cur = text.insert(cur, &s);
            h.end((cur, None));
        }
        assert_eq!(text.line(0), "ab cd");
        // "ab" and " cd" are two transactions (space breaks the word)
        h.undo(&mut text);
        assert_eq!(text.line(0), "ab");
        h.undo(&mut text);
        assert_eq!(text.line(0), "");
        assert!(h.undo(&mut text).is_none());
        h.redo(&mut text);
        h.redo(&mut text);
        assert_eq!(text.line(0), "ab cd");
        let v = h.version();
        assert_ne!(v, 0);
    }

    #[test]
    fn multiline_remove_undo() {
        let (mut text, _) = Text::parse("one\ntwo\nthree");
        let mut h = History::new();
        h.begin((Pos::new(0, 0), None), Coalesce::None, None);
        let r = text.remove(Pos::new(0, 1), Pos::new(2, 2));
        h.record(Edit::Remove { at: Pos::new(0, 1), text: r });
        h.end((Pos::new(0, 1), None));
        assert_eq!(text.to_string_with(Default::default()), "oree");
        h.undo(&mut text);
        assert_eq!(text.to_string_with(Default::default()), "one\ntwo\nthree");
    }
}
