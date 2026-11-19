//! Quick open (fuzzy file finder) and command palette overlay.

use crate::keymap::{Cmd, PALETTE};
use crate::prompt::LineInput;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PaletteMode {
    Files,
    Commands,
}

pub struct Palette {
    pub input: LineInput,
    pub files: Vec<String>,
    /// (score, index into the current source, match positions)
    pub results: Vec<(i64, usize, Vec<usize>)>,
    pub selected: usize,
    pub scroll: usize,
}

pub enum Picked {
    File(String),
    Command(Cmd),
    GotoLine(usize),
}

/// Subsequence fuzzy match. Returns (score, matched byte positions) or None.
pub fn fuzzy(pattern: &str, cand: &str) -> Option<(i64, Vec<usize>)> {
    if pattern.is_empty() {
        return Some((0, Vec::new()));
    }
    let pat: Vec<char> = pattern.chars().filter(|c| !c.is_whitespace()).map(|c| c.to_ascii_lowercase()).collect();
    let mut pos = Vec::with_capacity(pat.len());
    let mut score: i64 = 0;
    let mut pi = 0;
    let mut prev_match: Option<usize> = None;
    let base_start = cand.rfind('/').map(|i| i + 1).unwrap_or(0);
    let mut prev_char: Option<char> = None;
    for (i, c) in cand.char_indices() {
        if pi == pat.len() {
            break;
        }
        if c.to_ascii_lowercase() == pat[pi] {
            let mut s = 1;
            if prev_match.is_some_and(|p| p + 1 == i || cand[p..i].chars().count() == 1) {
                s += 8; // consecutive
            }
            match prev_char {
                None => s += 10,
                Some(p) if p == '/' || p == '_' || p == '-' || p == '.' || p == ' ' => s += 9,
                Some(p) if p.is_lowercase() && c.is_uppercase() => s += 7,
                _ => {}
            }
            if i >= base_start {
                s += 4; // in the file name
            }
            score += s;
            pos.push(i);
            prev_match = Some(i);
            pi += 1;
        }
        prev_char = Some(c);
    }
    if pi < pat.len() {
        return None;
    }
    score -= (cand.len() as i64) / 8;
    Some((score, pos))
}

impl Palette {
    pub fn new(files: Vec<String>, commands: bool) -> Palette {
        let mut p = Palette {
            input: LineInput::with(if commands { ">" } else { "" }),
            files,
            results: Vec::new(),
            selected: 0,
            scroll: 0,
        };
        p.update();
        p
    }

    pub fn mode(&self) -> PaletteMode {
        if self.input.text.starts_with('>') { PaletteMode::Commands } else { PaletteMode::Files }
    }

    pub fn label(&self, idx: usize) -> &str {
        match self.mode() {
            PaletteMode::Files => &self.files[idx],
            PaletteMode::Commands => PALETTE[idx].0,
        }
    }

    pub fn update(&mut self) {
        self.results.clear();
        self.selected = 0;
        self.scroll = 0;
        let text = self.input.text.clone();
        match self.mode() {
            PaletteMode::Commands => {
                let q = text[1..].trim();
                for (i, (label, _)) in PALETTE.iter().enumerate() {
                    if let Some((s, m)) = fuzzy(q, label) {
                        self.results.push((s, i, m));
                    }
                }
                if !q.is_empty() {
                    self.results.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
                }
            }
            PaletteMode::Files => {
                if text.starts_with(':') {
                    return;
                }
                for (i, f) in self.files.iter().enumerate() {
                    if let Some((s, m)) = fuzzy(&text, f) {
                        self.results.push((s, i, m));
                    }
                }
                if !text.is_empty() {
                    self.results.sort_by(|a, b| b.0.cmp(&a.0).then(self.files[a.1].len().cmp(&self.files[b.1].len())));
                }
                self.results.truncate(500);
            }
        }
    }

    pub fn move_sel(&mut self, d: isize, height: usize) {
        if self.results.is_empty() {
            return;
        }
        let n = self.results.len() as isize;
        self.selected = ((self.selected as isize + d).rem_euclid(n)) as usize;
        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if self.selected >= self.scroll + height {
            self.scroll = self.selected + 1 - height;
        }
    }

    pub fn picked(&self) -> Option<Picked> {
        let t = self.input.text.trim();
        if let Some(rest) = t.strip_prefix(':') {
            return rest.trim().parse::<usize>().ok().map(Picked::GotoLine);
        }
        let (_, idx, _) = self.results.get(self.selected)?;
        Some(match self.mode() {
            PaletteMode::Files => Picked::File(self.files[*idx].clone()),
            PaletteMode::Commands => Picked::Command(PALETTE[*idx].1),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy_ranks_basename() {
        let a = fuzzy("main", "src/main.rs").unwrap().0;
        let b = fuzzy("main", "src/maintenance/x.rs").unwrap().0;
        assert!(a > b);
        assert!(fuzzy("xyz", "src/main.rs").is_none());
        let p = Palette::new(vec!["a/b.rs".into(), "c/main.rs".into(), "README.md".into()], false);
        assert_eq!(p.results.len(), 3);
    }
}
