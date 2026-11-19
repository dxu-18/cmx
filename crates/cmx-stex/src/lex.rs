//! Lexer: source text → tokens, keeping line numbers and whether each token is glued to its
//! neighbours (no whitespace in between). STEX-DESIGN §3.1.
//!
//! - `//` starts a comment to end of line (not after `:`, so URLs survive).
//! - `$…$` is an explicit math span (single dollars only; `$$` is an error).
//! - `latex{…}` is raw LaTeX. `bold{…}`/`bf{…}`, `italic{…}`/`it{…}` and `underline{…}`/`ul{…}`
//!   are style blocks. `bold`/`italic`/`underline` followed by a plain word style that word.
//! - Sigils `*A &A ~A #A @A`, forced capitals `_word` / `__word`, prose escape `-keyword`.
//! - Abbreviations (`s.t.`, `i.e.`, `b/c`, `w/`, …) and multi-word phrases (`such that`,
//!   `there exists`, `real numbers`, …) become single word tokens.

use std::fmt;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Style {
    Bold,
    Italic,
    Underline,
}

#[derive(Clone, PartialEq, Debug)]
pub enum K {
    /// A word (letters, digits after letters, `'`), or a merged abbreviation / phrase (lowercased
    /// canonical form for phrases; original text otherwise).
    Word,
    Num,
    Sym,
    /// Explicit `$…$`.
    Math(Vec<Tok>),
    /// `latex{…}` raw content.
    Latex(String),
    /// `bold{…}` etc.
    Block(Style, Vec<Tok>),
    /// `bold word`: style applied to the next word (stored in `text`).
    StyleWord(Style),
    /// `*A` `&A` `~A` `#A` `@A` (sigil char, letters in `text`).
    Sigil(char),
    /// `_word` (capitalize) and `__word` (all caps).
    Cap,
    Upper,
    /// `-keyword`: the keyword as literal prose.
    Escape,
    Newline,
    /// One or more blank lines.
    Para,
}

#[derive(Clone)]
pub struct Tok {
    pub kind: K,
    pub text: String,
    pub line: u32,
    /// Whitespace (or start of input) immediately before / after this token.
    pub sp_before: bool,
    pub sp_after: bool,
    /// Byte range in the lexed source (for the editor's highlighter); not part of equality.
    pub span: (usize, usize),
}

impl PartialEq for Tok {
    fn eq(&self, o: &Tok) -> bool {
        self.kind == o.kind
            && self.text == o.text
            && self.line == o.line
            && self.sp_before == o.sp_before
            && self.sp_after == o.sp_after
    }
}

impl fmt::Debug for Tok {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            K::Word | K::Num | K::Sym => write!(f, "{:?}", self.text),
            K::Math(t) => write!(f, "${t:?}$"),
            other => write!(f, "{other:?}({:?})", self.text),
        }
    }
}

impl Tok {
    pub fn is_word(&self) -> bool {
        self.kind == K::Word
    }
    pub fn is_sym(&self, s: &str) -> bool {
        self.kind == K::Sym && self.text == s
    }
    /// Case-insensitive word comparison.
    pub fn is_kw(&self, w: &str) -> bool {
        self.kind == K::Word && self.text.eq_ignore_ascii_case(w)
    }
    pub fn lower(&self) -> String {
        self.text.to_ascii_lowercase()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LexError {
    pub line: u32,
    pub msg: String,
}

/// Abbreviations matched case-insensitively at a word boundary → canonical text. Longest first.
const ABBREV: &[(&str, &str)] = &[
    ("w.l.o.g.", "wlog"),
    ("w.r.t.", "wrt"),
    ("s.t.", "s.t."),
    ("i.e.", "i.e."),
    ("e.g.", "e.g."),
    ("a.e.", "a.e."),
    ("a.s.", "a.s."),
    ("resp.", "resp."),
    ("etc.", "etc."),
    ("cf.", "cf."),
    ("s.t", "s.t."),
    ("i.e", "i.e."),
    ("e.g", "e.g."),
    ("ie.", "i.e."),
    ("st.", "s.t."),
    ("b/c", "b/c"),
    ("w/o", "w/o"),
    ("w/", "w/"),
];

/// Multi-word phrases merged into one word token (lowercase canonical form). Longest first is
/// ensured at runtime.
const PHRASES: &[&str] = &[
    "such that",
    "so that",
    "there exists",
    "there exist",
    "there is",
    "there are",
    "there exists a unique",
    "there is a unique",
    "there is exactly one",
    "there exists unique",
    "exists unique",
    "there does not exist",
    "there is no",
    "there are no",
    "there exist no",
    "there exists no",
    "for all",
    "for every",
    "for each",
    "for any",
    "for some",
    "if and only if",
    "only if",
    "empty set",
    "real numbers",
    "natural numbers",
    "complex numbers",
    "rational numbers",
    "positive integers",
    "the integers",
    "the reals",
    "the naturals",
    "the rationals",
    "the real numbers",
    "the natural numbers",
    "the complex numbers",
    "the rational numbers",
    "symmetric difference",
    "symmetrical difference",
    "power set",
    "with respect to",
    "is an element of",
    "is not an element of",
    "is a subset of",
    "is a proper subset of",
    "is not equal to",
    "is equal to",
    "not equal to",
    "equal to",
    "is less than or equal to",
    "is greater than or equal to",
    "less than or equal to",
    "greater than or equal to",
    "is less than",
    "is greater than",
    "less than",
    "greater than",
    "is at most",
    "is at least",
    "is in",
    "is not in",
    "belongs to",
    "is divisible by",
    "approaches",
    "tends to",
    "goes to",
    "up to",
    "the sake of contradiction",
    "for the sake of contradiction",
    "for contradiction",
    "in either case",
    "in both cases",
    "in all cases",
    "in every case",
    "in any case",
    "either way",
    // exhaustiveness of a case analysis (STEX-DESIGN §14): before the cases or after them
    "cases exhaustive",
    "exhaustive cases",
    "cases are exhaustive",
    "the cases are exhaustive",
    "these cases are exhaustive",
    "the following cases are exhaustive",
    "the cases exhaust all possibilities",
    "these cases exhaust all possibilities",
    "the cases cover all possibilities",
    "these cases cover all possibilities",
    "the cases cover every possibility",
    "these cases cover every possibility",
    "since the cases are exhaustive",
    "since these cases are exhaustive",
    "as the cases are exhaustive",
    "as these cases are exhaustive",
    "because the cases are exhaustive",
    "because these cases are exhaustive",
    "in all exhaustive cases",
    "in every exhaustive case",
    "in each exhaustive case",
    "in exhaustive cases",
    "in both exhaustive cases",
    "in either exhaustive case",
    // announcing a case analysis: "case on k" (STEX-DESIGN §14)
    "case on",
    "cases on",
    "casing on",
    "case analysis on",
    "case split on",
    "split into cases on",
    "we case on",
    "by cases on",
    "proceed by cases on",
    "proceed by case analysis on",
    "we proceed by cases on",
    "we proceed by case analysis on",
    "base case",
    "inductive step",
    "induction step",
    "inductive hypothesis",
    "induction hypothesis",
    "we want to show",
    "want to show",
    "we need to show",
    "we have",
    "it follows that",
    "this is a contradiction",
    "in other words",
    "that is",
    "note that",
    "without loss of generality",
    "is between",
    "is strictly between",
    "strictly between",
    "lies between",
    "lies strictly between",
    "in total",
    "all in all",
    "in all",
    "suppose not",
    "assume not",
    "suppose otherwise",
    "assume otherwise",
    "assume the contrary",
    "suppose the contrary",
];

pub fn lex(src: &str) -> Result<Vec<Tok>, LexError> {
    let mut lx = Lexer { s: src.as_bytes(), src, i: 0, line: 1, toks: Vec::new() };
    lx.run(false)?;
    let mut toks = lx.toks;
    merge_phrases(&mut toks);
    Ok(toks)
}

/// Lex the inside of a `$…$` or style block (no newline/paragraph structure beyond Newline tokens).
fn lex_inner(src: &str, line: u32) -> Result<Vec<Tok>, LexError> {
    let mut lx = Lexer { s: src.as_bytes(), src, i: 0, line, toks: Vec::new() };
    lx.run(true)?;
    let mut toks = lx.toks;
    toks.retain(|t| !matches!(t.kind, K::Newline | K::Para));
    merge_phrases(&mut toks);
    Ok(toks)
}

struct Lexer<'a> {
    s: &'a [u8],
    src: &'a str,
    i: usize,
    line: u32,
    toks: Vec<Tok>,
}

const SYMS: &[&str] = &[
    "<=>", "<->", "|->", "!&&", "!||", "!==", ":=", "==", "!=", "<=", ">=", "->", "=>", "<-", "&&", "||", "^^", "+-",
    "-+", "...",
];

impl Lexer<'_> {
    fn peek(&self, k: usize) -> u8 {
        *self.s.get(self.i + k).unwrap_or(&0)
    }

    fn ws_before(&self, start: usize) -> bool {
        start == 0 || matches!(self.s[start - 1], b' ' | b'\t' | b'\n' | b'\r')
    }

    fn push(&mut self, kind: K, text: String, start: usize) {
        let sp_before = self.ws_before(start)
            || self.toks.last().is_some_and(|t| matches!(t.kind, K::Newline | K::Para))
            || self.toks.is_empty();
        if let Some(prev) = self.toks.last_mut()
            && sp_before
        {
            prev.sp_after = true;
        }
        let span = (start, self.i.max(start));
        self.toks.push(Tok { kind, text, line: self.line, sp_before, sp_after: false, span });
    }

    fn finish_last(&mut self) {
        let i = self.i;
        if let Some(t) = self.toks.last_mut()
            && i > t.span.0
        {
            t.span.1 = i;
        }
        let end_ws = self.i >= self.s.len() || matches!(self.peek(0), b' ' | b'\t' | b'\n' | b'\r');
        if let Some(t) = self.toks.last_mut()
            && end_ws
        {
            t.sp_after = true;
        }
    }

    /// Byte index of the `}` matching the `{` at `open`.
    fn match_brace(&self, open: usize) -> Option<usize> {
        let mut depth = 0i32;
        let mut j = open;
        while j < self.s.len() {
            match self.s[j] {
                b'\\' => j += 1,
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(j);
                    }
                }
                _ => {}
            }
            j += 1;
        }
        None
    }

    fn run(&mut self, inner: bool) -> Result<(), LexError> {
        while self.i < self.s.len() {
            let c = self.peek(0);
            let start = self.i;
            match c {
                b'\n' => {
                    self.line += 1;
                    self.i += 1;
                    let mut blank = false;
                    loop {
                        let mut j = self.i;
                        while j < self.s.len() && matches!(self.s[j], b' ' | b'\t' | b'\r') {
                            j += 1;
                        }
                        if j < self.s.len() && self.s[j] == b'\n' {
                            blank = true;
                            self.line += 1;
                            self.i = j + 1;
                        } else {
                            break;
                        }
                    }
                    if !inner {
                        if let Some(t) = self.toks.last_mut() {
                            t.sp_after = true;
                        }
                        let kind = if blank { K::Para } else { K::Newline };
                        if !self.toks.last().is_some_and(|t| t.kind == K::Para) {
                            self.toks.push(Tok {
                                kind,
                                text: String::new(),
                                line: self.line,
                                sp_before: true,
                                sp_after: true,
                                span: (start, start),
                            });
                        }
                    }
                }
                b' ' | b'\t' | b'\r' => self.i += 1,
                b'/' if self.peek(1) == b'/' && (start == 0 || self.s[start - 1] != b':') => {
                    while self.i < self.s.len() && self.s[self.i] != b'\n' {
                        self.i += 1;
                    }
                }
                b'$' => {
                    if self.peek(1) == b'$' {
                        return Err(LexError {
                            line: self.line,
                            msg:
                                "`$$` is not stex syntax: use single `$…$` (display vs inline is decided automatically)"
                                    .into(),
                        });
                    }
                    let Some(rel) = self.src[start + 1..].find('$') else {
                        return Err(LexError { line: self.line, msg: "unclosed `$`".into() });
                    };
                    let inner_src = &self.src[start + 1..start + 1 + rel];
                    let toks = lex_inner(inner_src, self.line)?;
                    let line0 = self.line;
                    self.line += inner_src.matches('\n').count() as u32;
                    self.i = start + rel + 2;
                    self.push(K::Math(toks), inner_src.to_string(), start);
                    if let Some(t) = self.toks.last_mut() {
                        t.line = line0;
                    }
                    self.finish_last();
                }
                b'0'..=b'9' => {
                    let mut j = self.i;
                    while j < self.s.len() && self.s[j].is_ascii_digit() {
                        j += 1;
                    }
                    if j + 1 < self.s.len() && self.s[j] == b'.' && self.s[j + 1].is_ascii_digit() {
                        j += 1;
                        while j < self.s.len() && self.s[j].is_ascii_digit() {
                            j += 1;
                        }
                    }
                    // ordinals "1st", "2nd", "23rd", "4th" are prose words
                    let rest = &self.src[j..];
                    let suffix = ["st", "nd", "rd", "th"].iter().find(|suf| {
                        rest.get(..2).is_some_and(|r| r.eq_ignore_ascii_case(suf))
                            && !rest.as_bytes().get(2).is_some_and(|b| b.is_ascii_alphanumeric())
                    });
                    if let Some(suf) = suffix {
                        self.i = j + suf.len();
                        self.push(K::Word, self.src[start..self.i].to_string(), start);
                        self.finish_last();
                        continue;
                    }
                    self.i = j;
                    self.push(K::Num, self.src[start..j].to_string(), start);
                    self.finish_last();
                }
                _ if c.is_ascii_alphabetic() || c >= 0x80 => self.word(start, inner)?,
                b'_' if self.ws_before(start) && (self.peek(1).is_ascii_alphabetic() || self.peek(1) == b'_') => {
                    let upper = self.peek(1) == b'_';
                    let ws = if upper { start + 2 } else { start + 1 };
                    let mut j = ws;
                    while j < self.s.len() && (self.s[j].is_ascii_alphanumeric() || self.s[j] == b'\'') {
                        j += 1;
                    }
                    if j == ws {
                        self.i = start + 1;
                        self.push(K::Sym, "_".into(), start);
                    } else {
                        self.i = j;
                        self.push(if upper { K::Upper } else { K::Cap }, self.src[ws..j].to_string(), start);
                    }
                    self.finish_last();
                }
                b'*' | b'&' | b'~' | b'#' | b'@'
                    if self.peek(1).is_ascii_alphabetic()
                        && (self.ws_before(start)
                            || matches!(self.s[start - 1], b'(' | b'[' | b'{' | b',' | b'$' | b'=' | b'|')) =>
                {
                    let mut j = start + 1;
                    while j < self.s.len() && self.s[j].is_ascii_alphabetic() {
                        j += 1;
                    }
                    self.i = j;
                    self.push(K::Sigil(c as char), self.src[start + 1..j].to_string(), start);
                    self.finish_last();
                }
                _ => {
                    let rest = &self.src[start..];
                    let sym = SYMS.iter().find(|s| rest.starts_with(**s)).copied();
                    // `||` glued to an operand is a pair of bars (`||v||`, `||x| - |y||`); spaced, it
                    // is logical or (`x > 0 || x < 1`)
                    if sym == Some("||") {
                        let after = self.s.get(start + 2).copied().unwrap_or(b' ');
                        let glued_left = !self.ws_before(start) && self.s[start - 1] != b'(';
                        let glued_right = !matches!(after, b' ' | b'\t' | b'\n' | b'\r');
                        if glued_left || glued_right {
                            for k in 0..2 {
                                self.i = start + k + 1;
                                self.push(K::Sym, "|".into(), start + k);
                                self.finish_last();
                            }
                            continue;
                        }
                    }
                    let len = sym.map(str::len).unwrap_or_else(|| rest.chars().next().map_or(1, char::len_utf8));
                    self.i = start + len;
                    self.push(K::Sym, rest[..len].to_string(), start);
                    self.finish_last();
                }
            }
        }
        Ok(())
    }

    fn word(&mut self, start: usize, inner: bool) -> Result<(), LexError> {
        let rest = &self.src[start..];
        // abbreviations only start a word in prose position ("a+b/c" is a fraction, not "because")
        let boundary_before = self.ws_before(start) || matches!(self.s[start - 1], b'(' | b'[' | b',');
        if boundary_before {
            for (ab, canon) in ABBREV {
                if rest.get(..ab.len()).is_some_and(|r| r.eq_ignore_ascii_case(ab)) {
                    let next = rest.as_bytes().get(ab.len()).copied().unwrap_or(b' ');
                    // must end at a word boundary ("st." needs a following space so "st.x" isn't eaten)
                    // "a + b/c = d": between arithmetic operators b/c is a fraction, not "because"
                    let between_ops = ab.contains('/') && {
                        let prev = self.s[..start].iter().rev().find(|b| !b.is_ascii_whitespace()).copied();
                        let after = rest[ab.len()..].trim_start().as_bytes().first().copied();
                        matches!(prev, Some(b'+' | b'-' | b'*' | b'=' | b'<' | b'>' | b'^' | b'/'))
                            || matches!(after, Some(b'+' | b'-' | b'*' | b'=' | b'<' | b'>' | b'^' | b')'))
                    };
                    if !between_ops && !next.is_ascii_alphanumeric() && (!ab.ends_with('.') || next != b'.') {
                        self.i = start + ab.len();
                        self.push(K::Word, canon.to_string(), start);
                        self.finish_last();
                        return Ok(());
                    }
                }
            }
        }
        let bytes = self.s;
        // A non-letter Unicode character (≤ ∈ ∀ ∑ ² ...) is a token of its own: "x≤5" is x, ≤, 5.
        let first = rest.chars().next().unwrap_or(' ');
        let mut j = start + first.len_utf8();
        if first.is_alphabetic() || first.is_ascii_digit() {
            let base = j;
            for (k, ch) in self.src[base..].char_indices() {
                let at = base + k;
                let word_char = ch.is_alphabetic() || ch.is_ascii_digit();
                let apostrophe = ch == '\'' && bytes.get(at + 1).is_some_and(|b| b.is_ascii_alphabetic());
                if !(word_char || apostrophe) {
                    j = at;
                    break;
                }
                j = at + ch.len_utf8();
            }
        }
        // keep a trailing digit run attached (x1, a2) but split "2x" handled by Num lexing
        let w = &self.src[start..j];
        self.i = j;
        let lower = w.to_ascii_lowercase();
        // style / raw blocks: keyword, optional spaces, `{`
        let mut k = j;
        while k < bytes.len() && (bytes[k] == b' ' || bytes[k] == b'\t') {
            k += 1;
        }
        let style = match lower.as_str() {
            "bold" | "bf" => Some(Style::Bold),
            "italic" | "it" => Some(Style::Italic),
            "underline" | "ul" => Some(Style::Underline),
            _ => None,
        };
        if k < bytes.len() && bytes[k] == b'{' && (lower == "latex" || style.is_some()) {
            let Some(close) = self.match_brace(k) else {
                return Err(LexError { line: self.line, msg: format!("unclosed `{{` after `{w}`") });
            };
            let body = &self.src[k + 1..close];
            let line0 = self.line;
            let kind = if lower == "latex" {
                K::Latex(body.trim().to_string())
            } else {
                K::Block(style.expect("checked"), lex_inner(body, self.line)?)
            };
            self.line += body.matches('\n').count() as u32;
            self.i = close + 1;
            self.push(kind, w.to_string(), start);
            if let Some(t) = self.toks.last_mut() {
                t.line = line0;
            }
            self.finish_last();
            return Ok(());
        }
        // `bold word` / `italic word` / `underline word` (long forms only: "it" is English)
        if matches!(lower.as_str(), "bold" | "italic" | "underline")
            && k > j
            && k < bytes.len()
            && bytes[k].is_ascii_alphabetic()
            && !inner
        {
            self.push(K::StyleWord(style.expect("checked")), w.to_string(), start);
            self.finish_last();
            return Ok(());
        }
        // `-keyword` → prose escape, only in prose position: after a prose word or at the start of a
        // line ("we use -exists"), never after an operator or operand ("x >= -pi", "x -pi").
        if start > 0 && self.s[start - 1] == b'-' && self.toks.last().is_some_and(|t| t.is_sym("-") && t.sp_before) {
            let n = self.toks.len();
            let prose_pos = n < 2 || {
                let p = &self.toks[n - 2];
                matches!(p.kind, K::Newline | K::Para)
                    || p.kind == K::Word
                        && p.text.chars().count() > 1
                        && crate::lexicon::get().lookup(&p.text).is_none()
            };
            if prose_pos && crate::lexicon::is_keyword(&lower) {
                self.toks.pop();
                self.push(K::Escape, w.to_string(), start - 1);
                self.finish_last();
                return Ok(());
            }
        }
        self.push(K::Word, w.to_string(), start);
        self.finish_last();
        Ok(())
    }
}

/// Multi-word phrases indexed by their lowercase first word, longest first.
fn phrase_index() -> &'static std::collections::HashMap<String, Vec<Vec<String>>> {
    static IDX: std::sync::OnceLock<std::collections::HashMap<String, Vec<Vec<String>>>> = std::sync::OnceLock::new();
    IDX.get_or_init(|| {
        let mut m: std::collections::HashMap<String, Vec<Vec<String>>> = std::collections::HashMap::new();
        for p in PHRASES.iter().copied().chain(crate::lexicon::get().multiword().iter().map(String::as_str)) {
            let words: Vec<String> = p.split(' ').map(|w| w.to_lowercase()).collect();
            m.entry(words[0].clone()).or_default().push(words);
        }
        for v in m.values_mut() {
            v.sort_by_key(|p| std::cmp::Reverse(p.len()));
            v.dedup();
        }
        m
    })
}

/// Merge multi-word phrases (case-insensitive, words separated only by spaces) into one token.
fn merge_phrases(toks: &mut Vec<Tok>) {
    let idx = phrase_index();
    let mut out: Vec<Tok> = Vec::with_capacity(toks.len());
    let mut i = 0;
    while i < toks.len() {
        let mut merged = false;
        if toks[i].kind == K::Word
            && let Some(cands) = idx.get(&toks[i].text.to_lowercase())
        {
            for ph in cands {
                let n = ph.len();
                if i + n <= toks.len()
                    && (1..n).all(|k| toks[i + k].kind == K::Word && toks[i + k].text.to_lowercase() == ph[k])
                    && (1..n).all(|k| toks[i + k].sp_before)
                {
                    let mut t = toks[i].clone();
                    t.text = ph.join(" ");
                    t.sp_after = toks[i + n - 1].sp_after;
                    t.span.1 = toks[i + n - 1].span.1;
                    // keep the capitalization hint of the first word for prose output
                    if toks[i].text.starts_with(|c: char| c.is_uppercase()) {
                        let mut c = t.text.chars();
                        let first = c.next().map(|f| f.to_uppercase().collect::<String>()).unwrap_or_default();
                        t.text = first + c.as_str();
                    }
                    out.push(t);
                    i += n;
                    merged = true;
                    break;
                }
            }
        }
        if !merged {
            out.push(toks[i].clone());
            i += 1;
        }
    }
    *toks = out;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(src: &str) -> Vec<String> {
        lex(src).unwrap().iter().map(|t| format!("{t:?}")).collect()
    }

    #[test]
    fn basics() {
        assert_eq!(texts("let x = 5"), ["\"let\"", "\"x\"", "\"=\"", "\"5\""]);
        assert_eq!(texts("x^20+30 choose 4"), ["\"x\"", "\"^\"", "\"20\"", "\"+\"", "\"30\"", "\"choose\"", "\"4\""]);
        assert_eq!(texts("a <= b != c <=> d"), ["\"a\"", "\"<=\"", "\"b\"", "\"!=\"", "\"c\"", "\"<=>\"", "\"d\""]);
        assert_eq!(texts("x s.t. y"), ["\"x\"", "\"s.t.\"", "\"y\""]);
        assert_eq!(texts("x st. y"), ["\"x\"", "\"s.t.\"", "\"y\""]);
        assert_eq!(texts("such that there exists"), ["\"such that\"", "\"there exists\""]);
        assert_eq!(texts("3.14 x // comment"), ["\"3.14\"", "\"x\""]);
        assert_eq!(
            texts("see https://x.org"),
            ["\"see\"", "\"https\"", "\":\"", "\"/\"", "\"/\"", "\"x\"", "\".\"", "\"org\""]
        );
    }

    #[test]
    fn blocks_and_sigils() {
        let t = lex("bold {hi there} *A &B latex{\\exists} _cap __up").unwrap();
        assert!(matches!(t[0].kind, K::Block(Style::Bold, _)));
        assert_eq!(t[1].kind, K::Sigil('*'));
        assert_eq!(t[2].kind, K::Sigil('&'));
        assert_eq!(t[3].kind, K::Latex("\\exists".into()));
        assert_eq!((t[4].kind.clone(), t[4].text.as_str()), (K::Cap, "cap"));
        assert_eq!((t[5].kind.clone(), t[5].text.as_str()), (K::Upper, "up"));
        let t = lex("2*A").unwrap();
        assert_eq!(t.len(), 3, "2*A is multiplication");
    }

    #[test]
    fn math_spans_and_errors() {
        let t = lex("so $x = 5$ ok").unwrap();
        assert!(matches!(&t[1].kind, K::Math(v) if v.len() == 3));
        assert!(lex("$$x$$").is_err());
        assert!(lex("$x").is_err());
    }

    #[test]
    fn newlines_and_paragraphs() {
        let t = lex("a\nb\n\n\nc").unwrap();
        let kinds: Vec<_> = t.iter().map(|t| t.kind.clone()).collect();
        assert_eq!(kinds, [K::Word, K::Newline, K::Word, K::Para, K::Word]);
        assert_eq!(t[4].line, 5);
    }

    #[test]
    fn escape_keyword() {
        let t = lex("we use -exists here, x = -y").unwrap();
        assert_eq!(t[2].kind, K::Escape);
        assert!(t.iter().any(|t| t.is_sym("-")), "x = -y stays a minus");
    }
}
