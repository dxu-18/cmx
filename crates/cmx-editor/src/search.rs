//! Search queries: literal with smart case (the default), or a regex (`regex.rs`), optionally
//! whole-word. Find prompt toggles: Alt+R regex, Alt+C match case, Alt+W whole word.

use crate::regex::{self, Regex};
use crate::text::{Pos, Text};

#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct Flags {
    pub regex: bool,
    /// Match case exactly (otherwise smart case: case-sensitive only if the query has a capital).
    pub case: bool,
    pub word: bool,
}

impl Flags {
    /// Short indicator for the prompt label, e.g. ` [.* Aa]`.
    pub fn badge(&self) -> String {
        let mut v = Vec::new();
        if self.regex {
            v.push(".*");
        }
        if self.case {
            v.push("Aa");
        }
        if self.word {
            v.push("\\b");
        }
        if v.is_empty() { String::new() } else { format!(" [{}]", v.join(" ")) }
    }
}

pub enum Query {
    Lit { needle: String, case: bool },
    Re(Regex),
}

impl Query {
    pub fn new(q: &str, f: Flags) -> Result<Query, String> {
        let case = f.case || q.chars().any(|c| c.is_uppercase());
        if f.regex {
            let pat = if f.word { format!(r"\b(?:{q})\b") } else { q.to_string() };
            Regex::new(&pat, !case).map(Query::Re)
        } else if f.word {
            Regex::new(&format!(r"\b{}\b", regex::escape(q)), !case).map(Query::Re)
        } else {
            Ok(Query::Lit { needle: q.to_string(), case })
        }
    }

    /// Plain literal, smart case (vim `*`, Ctrl+D).
    pub fn literal(q: &str, case: bool) -> Query {
        Query::Lit { needle: q.to_string(), case }
    }

    pub fn is_regex(&self) -> bool {
        matches!(self, Query::Re(_))
    }

    /// Next match after `from` (forward) or before it (backward), wrapping around the document.
    /// Empty regex matches are skipped (they can't be selected or stepped through).
    pub fn find(&self, t: &Text, from: Pos, forward: bool) -> Option<(Pos, Pos)> {
        match self {
            Query::Lit { needle, case } => t.find(needle, from, forward, *case),
            Query::Re(re) => {
                let n = t.len_lines();
                let line_hits = |l: usize| re.find_all(t.line(l));
                if forward {
                    for k in 0..=n {
                        let l = (from.line + k) % n;
                        let hits = line_hits(l);
                        let hit = match k {
                            0 => hits.into_iter().find(|&(a, _)| a >= from.col),
                            k if k == n => hits.into_iter().find(|&(a, _)| a < from.col),
                            _ => hits.into_iter().next(),
                        };
                        if let Some((a, b)) = hit {
                            return Some((Pos::new(l, a), Pos::new(l, b)));
                        }
                    }
                } else {
                    for k in 0..=n {
                        let l = (from.line + n * 2 - k) % n;
                        let hits = line_hits(l);
                        let hit = match k {
                            0 => hits.into_iter().rfind(|&(a, _)| a < from.col),
                            _ => hits.into_iter().next_back(),
                        };
                        if let Some((a, b)) = hit {
                            return Some((Pos::new(l, a), Pos::new(l, b)));
                        }
                    }
                }
                None
            }
        }
    }

    /// All matches on lines `[l0, l1)`.
    pub fn matches(&self, t: &Text, l0: usize, l1: usize, out: &mut Vec<(Pos, Pos)>) {
        match self {
            Query::Lit { needle, case } => t.matches_in(needle, l0, l1, *case, out),
            Query::Re(re) => {
                for l in l0..l1.min(t.len_lines()) {
                    for (a, b) in re.find_all(t.line(l)) {
                        out.push((Pos::new(l, a), Pos::new(l, b)));
                    }
                }
            }
        }
    }

    /// Does `[a, b)` match the query exactly (the current selection, for interactive replace)?
    pub fn matches_range(&self, t: &Text, a: Pos, b: Pos) -> bool {
        match self {
            Query::Lit { needle, case } => {
                let s = t.slice(a, b);
                if *case { s == *needle } else { s.eq_ignore_ascii_case(needle) }
            }
            Query::Re(re) => {
                a.line == b.line && re.find_all(t.line(a.line)).iter().any(|&(x, y)| x == a.col && y == b.col)
            }
        }
    }

    /// The replacement for the match `[a, b)`: `rep` itself, or with `$1`/`\1` groups expanded.
    /// `vim` also expands `&` to the whole match.
    pub fn replacement(&self, t: &Text, a: Pos, b: Pos, rep: &str, vim: bool) -> String {
        match self {
            Query::Lit { .. } => rep.to_string(),
            Query::Re(re) => {
                let line = t.line(a.line);
                // re-run from the match start to get its groups
                match re.captures_at(line, a.col) {
                    Some(c) if c[0] == Some(a.col) && c[1] == Some(b.col) => regex::expand(rep, &c, line, vim),
                    _ => rep.to_string(),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regex_find_wraps_both_ways() {
        let (t, _) = Text::parse("a1 b2\nc3\nd");
        let q = Query::new(r"[a-z]\d", Flags { regex: true, ..Default::default() }).unwrap();
        assert_eq!(q.find(&t, Pos::new(0, 1), true), Some((Pos::new(0, 3), Pos::new(0, 5))));
        assert_eq!(q.find(&t, Pos::new(1, 2), true), Some((Pos::new(0, 0), Pos::new(0, 2))));
        assert_eq!(q.find(&t, Pos::new(0, 3), false), Some((Pos::new(0, 0), Pos::new(0, 2))));
        assert_eq!(q.find(&t, Pos::new(0, 0), false), Some((Pos::new(1, 0), Pos::new(1, 2))));
        let mut v = Vec::new();
        q.matches(&t, 0, 3, &mut v);
        assert_eq!(v.len(), 3);
    }

    #[test]
    fn whole_word_case_and_replacement() {
        let (t, _) = Text::parse("cat concat Cat");
        let q = Query::new("cat", Flags { word: true, ..Default::default() }).unwrap();
        let mut v = Vec::new();
        q.matches(&t, 0, 1, &mut v);
        assert_eq!(v, vec![(Pos::new(0, 0), Pos::new(0, 3)), (Pos::new(0, 11), Pos::new(0, 14))]);
        let q = Query::new("cat", Flags { word: true, case: true, ..Default::default() }).unwrap();
        v.clear();
        q.matches(&t, 0, 1, &mut v);
        assert_eq!(v.len(), 1);
        let (t, _) = Text::parse("f(x, y)");
        let q = Query::new(r"(\w), (\w)", Flags { regex: true, ..Default::default() }).unwrap();
        let (a, b) = q.find(&t, Pos::new(0, 0), true).unwrap();
        assert!(q.matches_range(&t, a, b));
        assert_eq!(q.replacement(&t, a, b, "$2, $1", false), "y, x");
        assert!(Query::new("(", Flags { regex: true, ..Default::default() }).is_err());
    }
}
