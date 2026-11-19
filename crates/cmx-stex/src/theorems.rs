//! Theorem alias library (`data/theorems.tsv`, STEX-DESIGN §4). `by CRT`, `by the chinese remainder
//! thm` and `by Sun Tzu's theorem` all resolve to the same entry, which carries the prose name, the
//! Lean identifier for `stex check`, and a plain-text statement.

use crate::lex::{K, Tok};
use std::collections::HashMap;
use std::sync::OnceLock;

pub struct Theorem {
    pub name: &'static str,
    /// Mathlib identifier, if any.
    pub lean: Option<&'static str>,
    pub statement: &'static str,
}

struct Lib {
    list: Vec<Theorem>,
    by_alias: HashMap<String, usize>,
    max_words: usize,
}

static LIB: OnceLock<Lib> = OnceLock::new();

fn lib() -> &'static Lib {
    LIB.get_or_init(|| {
        let mut list = Vec::new();
        let mut by_alias = HashMap::new();
        let mut max_words = 1;
        for line in include_str!("../data/theorems.tsv").lines() {
            if line.starts_with('#') || line.trim().is_empty() {
                continue;
            }
            let f: Vec<&'static str> = line.split('\t').collect();
            if f.len() < 4 {
                continue;
            }
            let id = list.len();
            list.push(Theorem { name: f[0], lean: if f[1] == "-" { None } else { Some(f[1]) }, statement: f[3] });
            for alias in f[2].split('|').chain(std::iter::once(f[0])) {
                let n = normalize(alias);
                max_words = max_words.max(n.split(' ').count());
                by_alias.entry(n).or_insert(id);
            }
        }
        Lib { list, by_alias, max_words }
    })
}

/// Lowercase, drop apostrophes and a leading "the", hyphens/dashes → spaces, "thm" → "theorem".
pub fn normalize(s: &str) -> String {
    let s: String = s
        .chars()
        .filter(|c| !matches!(c, '\'' | '’'))
        .map(|c| if matches!(c, '-' | '–' | '—') { ' ' } else { c.to_ascii_lowercase() })
        .collect();
    let s = s.replace('ö', "o").replace('é', "e").replace('ô', "o");
    let words: Vec<&str> = s.split_whitespace().map(|w| if w == "thm" { "theorem" } else { w }).collect();
    let words = if words.first() == Some(&"the") { &words[1..] } else { &words[..] };
    words.join(" ")
}

pub fn get(id: usize) -> &'static Theorem {
    &lib().list[id]
}

/// Longest theorem alias at the start of `toks` (the words after `by`). Returns (id, tokens used).
pub fn match_at(toks: &[Tok]) -> Option<(usize, usize)> {
    let l = lib();
    let mut best = None;
    let mut text = String::new();
    for (k, t) in toks.iter().enumerate() {
        if k >= l.max_words + 3 {
            break;
        }
        match &t.kind {
            K::Word => {}
            K::Sym if t.text == "-" || t.text == "'" => {}
            _ => break,
        }
        if t.sp_before && !text.is_empty() {
            text.push(' ');
        }
        text.push_str(&t.text);
        if let Some(&id) = l.by_alias.get(&normalize(&text)) {
            best = Some((id, k + 1));
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lex::lex;

    #[test]
    fn aliases() {
        for src in ["CRT", "the Chinese Remainder Theorem", "chinese remainder thm", "Sun Tzu's theorem"] {
            let t = lex(src).unwrap();
            let (id, n) = match_at(&t).unwrap_or_else(|| panic!("{src}"));
            assert_eq!(get(id).name, "the Chinese Remainder Theorem", "{src}");
            assert_eq!(n, t.len(), "{src}");
        }
        let t = lex("Cauchy-Schwarz we get").unwrap();
        let (id, n) = match_at(&t).unwrap();
        assert_eq!(get(id).name, "the Cauchy–Schwarz inequality");
        assert_eq!(n, 3);
        assert!(match_at(&lex("subtraction").unwrap()).is_none());
    }
}
