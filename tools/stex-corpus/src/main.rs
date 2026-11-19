//! stex-corpus: phrase-frequency analysis of mathematical proofs (RESEARCH R6).
//!
//! Usage: `stex-corpus GROUP=DIR [GROUP=DIR ...] [--symbols symbols.tsv] [--out report.tsv]`
//!
//! Every `\begin{proof}…\end{proof}` (LaTeX) and `<proof>…</proof>` (PreTeXt) under each DIR is
//! extracted. Math, references and markup are replaced by placeholders (`math`, `ref`), the text is
//! split into sentences, and two things are counted per group:
//! - **probes**: the phrasings in the stex phrase bank (and the spec's lists), as sentence starts
//!   or anywhere in a sentence;
//! - **sentence starts**: the most common first 1–4 words, to discover phrasings we lack.
//!
//! Rates are per 10,000 proof sentences. The combined rate is the *mean over groups*, so a huge
//! source (the Stacks project) doesn't drown the others. Only aggregate counts leave the tool.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    /// Sentence starts with the phrase.
    Start,
    /// Occurrences anywhere (word-bounded).
    Any,
}

struct Probe {
    role: &'static str,
    phrase: &'static str,
    mode: Mode,
    /// Sentence must also contain this later (e.g. "if" … "then").
    and_contains: Option<&'static str>,
}

const fn s(role: &'static str, phrase: &'static str) -> Probe {
    Probe { role, phrase, mode: Mode::Start, and_contains: None }
}
const fn a(role: &'static str, phrase: &'static str) -> Probe {
    Probe { role, phrase, mode: Mode::Any, and_contains: None }
}
const fn sc(role: &'static str, phrase: &'static str, then: &'static str) -> Probe {
    Probe { role, phrase, mode: Mode::Start, and_contains: Some(then) }
}

/// Phrasings to measure: the stex phrase bank, the spec's lists (prompt.txt), and common extras.
const PROBES: &[Probe] = &[
    // declaring objects
    s("let", "let"),
    s("let", "fix"),
    s("let", "take"),
    s("let", "choose"),
    s("let", "pick"),
    s("let", "consider"),
    s("let", "given"),
    s("let", "given any"),
    s("let", "start with"),
    sc("let", "let", "be arbitrary"),
    sc("let", "let", "be given"),
    s("let", "fix an arbitrary"),
    s("let", "fix any"),
    s("let", "select"),
    // assumptions
    s("assume", "suppose that"),
    s("assume", "suppose"),
    s("assume", "assume that"),
    s("assume", "assume"),
    s("assume", "assuming"),
    s("assume", "if"),
    s("assume", "now suppose"),
    s("assume", "now assume"),
    s("assume", "we assume"),
    s("assume", "let us assume"),
    // proof by contradiction
    s("afsoc", "suppose for contradiction"),
    s("afsoc", "suppose for the sake of contradiction"),
    s("afsoc", "assume for the sake of contradiction"),
    s("afsoc", "assume for contradiction"),
    s("afsoc", "assume to the contrary"),
    s("afsoc", "suppose to the contrary"),
    s("afsoc", "suppose on the contrary"),
    s("afsoc", "assume on the contrary"),
    s("afsoc", "assume toward a contradiction"),
    s("afsoc", "assume towards a contradiction"),
    s("afsoc", "seeking a contradiction"),
    s("afsoc", "suppose not"),
    s("afsoc", "assume otherwise"),
    s("afsoc", "assume the contrary"),
    s("afsoc", "arguing by contradiction"),
    s("afsoc", "by way of contradiction"),
    s("afsoc", "for the sake of contradiction"),
    s("afsoc", "if not"),
    s("afsoc", "assume not"),
    // reaching a contradiction
    a("contradiction", "this is a contradiction"),
    a("contradiction", "which is a contradiction"),
    a("contradiction", "a contradiction"),
    a("contradiction", "contradiction"),
    a("contradiction", "this contradicts"),
    a("contradiction", "which contradicts"),
    a("contradiction", "contradicting"),
    a("contradiction", "which is absurd"),
    a("contradiction", "which is impossible"),
    a("contradiction", "this is impossible"),
    a("contradiction", "this cannot happen"),
    a("contradiction", "contrary to"),
    a("contradiction", "absurd"),
    a("contradiction", "impossible"),
    a("contradiction", "contradicts our assumption"),
    a("contradiction", "contradicts the assumption"),
    // closing a contradiction
    s("contradiction.close", "hence our assumption"),
    s("contradiction.close", "so our assumption"),
    s("contradiction.close", "therefore our assumption"),
    s("contradiction.close", "thus our assumption"),
    s("contradiction.close", "this contradiction"),
    s("contradiction.close", "hence no such"),
    a("contradiction.close", "the assumption was false"),
    a("contradiction.close", "must be false"),
    // stating the goal
    s("goal", "we show that"),
    s("goal", "we want to show"),
    s("goal", "we need to show"),
    s("goal", "we must show"),
    s("goal", "we wish to prove"),
    s("goal", "we wish to show"),
    s("goal", "our goal is to show"),
    s("goal", "we aim to show"),
    s("goal", "we claim that"),
    s("goal", "we will prove"),
    s("goal", "we will show"),
    s("goal", "we prove that"),
    s("goal", "we prove"),
    s("goal", "we have to show"),
    s("goal", "it is required to show"),
    s("goal", "to prove"),
    s("goal", "to show"),
    s("goal", "we first show"),
    s("goal", "we now show"),
    s("goal", "let us show"),
    s("goal", "let us prove"),
    s("goal", "we check that"),
    s("goal", "we verify that"),
    s("goal", "claim"),
    // reductions
    s("suffices", "it suffices to show"),
    s("suffices", "it suffices to prove"),
    s("suffices", "it is enough to show"),
    s("suffices", "it is enough to prove"),
    s("suffices", "it remains to show"),
    s("suffices", "it remains to prove"),
    s("suffices", "we need only show"),
    s("suffices", "we only need to show"),
    s("suffices", "it suffices to"),
    s("suffices", "it is sufficient to"),
    // closing
    a("qed", "as required"),
    a("qed", "as desired"),
    a("qed", "as claimed"),
    a("qed", "as was to be shown"),
    a("qed", "and we are done"),
    a("qed", "we are done"),
    a("qed", "this completes the proof"),
    a("qed", "which completes the proof"),
    a("qed", "this proves the"),
    a("qed", "this proves that"),
    a("qed", "which proves the"),
    a("qed", "this finishes the proof"),
    a("qed", "this concludes the proof"),
    a("qed", "the proof is complete"),
    a("qed", "this shows the"),
    a("qed", "proving the"),
    a("qed", "as wanted"),
    a("qed", "as asserted"),
    // drawing conclusions (sentence starts)
    s("then", "then"),
    s("then", "thus"),
    s("then", "hence"),
    s("then", "therefore"),
    s("then", "so"),
    s("then", "it follows that"),
    s("then", "consequently"),
    s("then", "we obtain"),
    s("then", "this shows that"),
    s("then", "we conclude that"),
    s("then", "we deduce that"),
    s("then", "we see that"),
    s("then", "this implies"),
    s("then", "this implies that"),
    s("then", "we get"),
    s("then", "we find"),
    s("then", "this gives"),
    s("then", "this yields"),
    s("then", "as a result"),
    s("then", "which means"),
    s("then", "in particular"),
    s("then", "we have"),
    s("then", "accordingly"),
    s("then", "it follows"),
    s("then", "this means that"),
    s("then", "we infer that"),
    s("then", "this proves"),
    s("then", "whence"),
    s("then", "now"),
    s("then", "but"),
    s("then", "since"),
    s("then", "by"),
    // implication connectives
    a("implies", "implies"),
    a("implies", "implies that"),
    sc("implies", "if", "then"),
    a("implies", "whenever"),
    a("implies", "only if"),
    a("implies", "follows from"),
    a("implies", "gives"),
    a("implies", "yields"),
    a("implies", "ensures"),
    a("implies", "guarantees"),
    a("implies", "provided that"),
    a("implies", "is implied by"),
    a("implies", "leads to"),
    a("implies", "means that"),
    a("implies", "is a consequence of"),
    a("implies", "entails"),
    a("implies", "forces"),
    // equivalence
    a("iff", "if and only if"),
    a("iff", "iff"),
    a("iff", "precisely when"),
    a("iff", "exactly when"),
    a("iff", "is equivalent to"),
    a("iff", "just in case"),
    a("iff", "necessary and sufficient"),
    a("iff", "equivalently"),
    // quantifiers
    a("exists", "there exists"),
    a("exists", "there exist"),
    a("exists", "there is"),
    a("exists", "there are"),
    a("exists", "for some"),
    a("exists", "we can find"),
    a("exists", "one can find"),
    a("exists", "we may choose"),
    a("exists", "can be chosen"),
    a("exists", "there must be"),
    a("exists", "at least one"),
    a("exists", "such that"),
    a("exists.unique", "there exists a unique"),
    a("exists.unique", "there is a unique"),
    a("exists.unique", "there is exactly one"),
    a("exists.unique", "unique"),
    a("exists.none", "there is no"),
    a("exists.none", "there are no"),
    a("exists.none", "there does not exist"),
    a("forall", "for all"),
    a("forall", "for every"),
    a("forall", "for each"),
    a("forall", "for any"),
    a("forall", "for arbitrary"),
    a("forall", "given any"),
    // justification
    a("because", "because"),
    a("because", "since"),
    a("because", ", as"),
    a("because", "due to"),
    a("because", "given that"),
    a("because", "by virtue of"),
    a("because", "owing to"),
    a("because", "in view of"),
    s("because.lead", "since"),
    s("because.lead", "because"),
    s("because.lead", "as"),
    s("because.lead", "given that"),
    s("because.lead", "in view of"),
    s("by.before", "by"),
    s("by.before", "applying"),
    s("by.before", "using"),
    s("by.before", "from"),
    s("by.before", "it follows from"),
    s("by.before", "in view of"),
    s("by.before", "according to"),
    s("by.before", "by the"),
    a("by.after", "by"),
    a("by.after", "by the"),
    a("by.after", "using"),
    a("by.after", "by virtue of"),
    // restating
    s("ie", "that is"),
    s("ie", "in other words"),
    s("ie", "namely"),
    s("ie", "equivalently"),
    s("ie", "explicitly"),
    a("ie.mid", "that is"),
    a("ie.mid", "i.e."),
    a("ie.mid", "in other words"),
    a("ie.mid", "namely"),
    // pointing out
    s("note", "note that"),
    s("note", "observe that"),
    s("note", "notice that"),
    s("note", "recall that"),
    s("note", "we note that"),
    s("note", "we observe that"),
    s("note", "it is worth noting"),
    s("note", "keep in mind"),
    s("note", "remark that"),
    s("note", "note"),
    s("note", "observe"),
    s("note", "recall"),
    // wlog
    s("wlog", "without loss of generality"),
    s("wlog", "wlog"),
    s("wlog", "we may assume"),
    s("wlog", "we can assume"),
    s("wlog", "we may and do assume"),
    s("wlog", "by symmetry"),
    s("wlog", "it is harmless to assume"),
    s("wlog", "after relabeling"),
    s("wlog", "by relabeling"),
    // defining
    s("define", "define"),
    s("define", "we define"),
    s("define", "set"),
    s("define", "put"),
    s("define", "denote by"),
    s("define", "write"),
    s("define", "we write"),
    s("define", "call"),
    s("define", "let"),
    // cases
    s("case", "case"),
    s("case", "suppose first that"),
    s("case", "first suppose"),
    s("case", "first assume"),
    s("case", "now suppose"),
    s("case", "in the first case"),
    s("case", "in the second case"),
    s("case", "on the other hand"),
    s("case", "otherwise"),
    s("case", "next consider"),
    s("cases.close", "in either case"),
    s("cases.close", "in both cases"),
    s("cases.close", "in all cases"),
    s("cases.close", "in every case"),
    s("cases.close", "either way"),
    s("cases.close", "in any case"),
    s("cases.close", "whichever case"),
    // similarly / obviousness / signposting
    s("similarly", "similarly"),
    s("similarly", "likewise"),
    s("similarly", "analogously"),
    s("similarly", "in the same way"),
    s("similarly", "by the same argument"),
    s("similarly", "the same argument shows"),
    s("similarly", "by the same reasoning"),
    s("clearly", "clearly"),
    s("clearly", "obviously"),
    s("clearly", "evidently"),
    s("clearly", "trivially"),
    s("clearly", "of course"),
    s("clearly", "it is clear that"),
    s("clearly", "it is easy to see"),
    s("clearly", "it is obvious that"),
    s("signpost", "first"),
    s("signpost", "next"),
    s("signpost", "finally"),
    s("signpost", "we begin by"),
    s("signpost", "we start with"),
    s("signpost", "now"),
    // statement prefixes
    s("statement", "we have"),
    s("statement", "math"),
    s("statement", "we compute"),
    s("statement", "we get"),
    // induction
    s("induction", "by induction"),
    s("induction", "by the induction hypothesis"),
    s("induction", "by the inductive hypothesis"),
    s("induction", "for the base case"),
    s("induction", "for the inductive step"),
    s("induction", "we proceed by induction"),
    s("induction", "we use induction"),
    a("induction", "induction on"),
];

#[derive(Default)]
struct Group {
    files: usize,
    proofs: usize,
    sentences: usize,
    words: usize,
    probes: Vec<u64>,
    starts: HashMap<String, u64>,
    symbols: HashMap<String, u64>,
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out);
        } else if matches!(p.extension().and_then(|e| e.to_str()), Some("tex" | "ptx" | "xml")) {
            out.push(p);
        }
    }
}

/// Bodies between `open` and `close` markers, allowing nesting.
fn between(src: &str, open: &str, close: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = src;
    while let Some(i) = rest.find(open) {
        let body_start = i + open.len();
        let mut depth = 1;
        let mut j = body_start;
        let bytes = rest.as_bytes();
        let mut end = None;
        while j < rest.len() {
            if rest[j..].starts_with(open) {
                depth += 1;
                j += open.len();
                continue;
            }
            if rest[j..].starts_with(close) {
                depth -= 1;
                if depth == 0 {
                    end = Some(j);
                    break;
                }
                j += close.len();
                continue;
            }
            j += 1;
            while j < rest.len() && !rest.is_char_boundary(j) {
                j += 1;
            }
            let _ = bytes;
        }
        match end {
            Some(e) => {
                out.push(rest[body_start..e].to_string());
                rest = &rest[e + close.len()..];
            }
            None => break,
        }
    }
    out
}

/// Replace every `start … end` span with `with`.
fn replace_spans(s: &str, start: &str, end: &str, with: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find(start) {
        out.push_str(&rest[..i]);
        let after = &rest[i + start.len()..];
        match after.find(end) {
            Some(j) => {
                out.push_str(with);
                rest = &after[j + end.len()..];
            }
            None => {
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

fn strip_comments(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for line in s.lines() {
        let b = line.as_bytes();
        let mut cut = line.len();
        for i in 0..b.len() {
            if b[i] == b'%' && (i == 0 || b[i - 1] != b'\\') {
                cut = i;
                break;
            }
        }
        out.push_str(&line[..cut]);
        out.push('\n');
    }
    out
}

const MATH_ENVS: &[&str] = &[
    "equation",
    "align",
    "gather",
    "multline",
    "eqnarray",
    "displaymath",
    "alignat",
    "flalign",
    "split",
    "tikzcd",
    "CD",
    "array",
    "matrix",
    "pmatrix",
    "bmatrix",
    "cases",
    "xy",
    "tikzpicture",
    "diagram",
    "math",
];

fn clean_latex(s: &str) -> String {
    let mut t = strip_comments(s);
    for env in MATH_ENVS {
        for star in ["*", ""] {
            t = replace_spans(&t, &format!("\\begin{{{env}{star}}}"), &format!("\\end{{{env}{star}}}"), " math ");
        }
    }
    t = replace_spans(&t, "$$", "$$", " math ");
    t = replace_spans(&t, "\\[", "\\]", " math ");
    t = replace_spans(&t, "\\(", "\\)", " math ");
    // inline $…$ (skip escaped \$)
    let mut out = String::with_capacity(t.len());
    let mut in_math = false;
    let mut prev = ' ';
    for c in t.chars() {
        if c == '$' && prev != '\\' {
            if !in_math {
                out.push_str(" math ");
            }
            in_math = !in_math;
        } else if !in_math {
            out.push(c);
        }
        prev = c;
    }
    // commands
    let mut res = String::with_capacity(out.len());
    let chars: Vec<char> = out.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\\' {
            let mut j = i + 1;
            while j < chars.len() && (chars[j].is_ascii_alphabetic() || chars[j] == '@') {
                j += 1;
            }
            let name: String = chars[i + 1..j].iter().collect();
            if j < chars.len() && chars[j] == '*' {
                j += 1;
            }
            let lower = name.to_ascii_lowercase();
            let skip_arg = lower.ends_with("ref") || lower.starts_with("cite") || lower == "label" || lower == "index";
            if lower == "item" {
                res.push_str(". ");
                i = j;
                continue;
            }
            if skip_arg {
                // optional [..] and {..}
                while j < chars.len() && (chars[j] == '[' || chars[j] == '{') {
                    let close = if chars[j] == '[' { ']' } else { '}' };
                    let mut d = 0;
                    while j < chars.len() {
                        if chars[j] == chars[j.min(chars.len() - 1)] && (chars[j] == '[' || chars[j] == '{') {
                            d += 1;
                        } else if chars[j] == close {
                            d -= 1;
                            if d == 0 {
                                j += 1;
                                break;
                            }
                        }
                        j += 1;
                    }
                }
                if lower != "label" && lower != "index" {
                    res.push_str(" ref ");
                }
                i = j;
                continue;
            }
            if name.is_empty() {
                // \\ , \, \; \  etc.
                res.push(' ');
                i = (i + 2).min(chars.len());
                continue;
            }
            res.push(' ');
            i = j;
            continue;
        }
        res.push(match c {
            '{' | '}' | '~' => ' ',
            _ => c,
        });
        i += 1;
    }
    res
}

fn clean_ptx(s: &str) -> String {
    let mut t = s.to_string();
    for tag in ["me", "men", "md", "mdn", "m"] {
        t = replace_spans(&t, &format!("<{tag}>"), &format!("</{tag}>"), " math ");
    }
    // remaining tags
    let mut out = String::with_capacity(t.len());
    let mut in_tag = false;
    let mut tag = String::new();
    for c in t.chars() {
        if c == '<' {
            in_tag = true;
            tag.clear();
        } else if in_tag && c == '>' {
            in_tag = false;
            let n = tag.trim_start_matches('/').split_whitespace().next().unwrap_or("");
            if n == "xref" {
                out.push_str(" ref ");
            } else if n == "li" || n == "p" {
                out.push_str(". ");
            } else {
                out.push(' ');
            }
        } else if in_tag {
            tag.push(c);
        } else {
            out.push(c);
        }
    }
    out.replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&")
}

const ABBREV: &[&str] =
    &["i.e", "e.g", "cf", "resp", "etc", "thm", "lem", "prop", "cor", "def", "sec", "eq", "fig", "vs", "al", "ex"];

fn sentences(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let chars: Vec<char> = text.chars().collect();
    for i in 0..chars.len() {
        let c = chars[i];
        cur.push(c);
        if matches!(c, '.' | '?' | '!') && chars.get(i + 1).is_none_or(|n| n.is_whitespace()) {
            let word = cur.trim_end_matches(['.', '?', '!']).rsplit(|c: char| c.is_whitespace()).next().unwrap_or("");
            if c == '.' && ABBREV.contains(&word.to_ascii_lowercase().as_str()) {
                continue;
            }
            out.push(std::mem::take(&mut cur));
        }
    }
    out.push(cur);
    out.into_iter().filter_map(|s| normalize(&s)).collect()
}

/// Lowercase, drop punctuation except word-internal apostrophes and ", as", collapse whitespace,
/// collapse runs of `math`, strip leading enumerators like "(1)" / "(a)".
fn normalize(s: &str) -> Option<String> {
    let lower = s.to_lowercase();
    let mut t = String::with_capacity(lower.len());
    for c in lower.chars() {
        if c.is_alphanumeric() || c == '\'' || c == '-' {
            t.push(c);
        } else if c == ',' {
            t.push_str(" , ");
        } else {
            t.push(' ');
        }
    }
    let mut words: Vec<&str> = Vec::new();
    for w in t.split_whitespace() {
        if w == "math" && words.last() == Some(&"math") {
            continue;
        }
        words.push(w);
    }
    // leading enumerators / refs
    while let Some(f) = words.first() {
        if f.len() <= 3
            && (f.chars().all(|c| c.is_ascii_digit()) || matches!(*f, "a" | "b" | "c" | "i" | "ii" | "iii" | "iv"))
            && words.len() > 3
            && words[1] != ","
        {
            // "(1) Suppose": the parenthesis was already removed; only strip when followed by a capital word originally (approximation: keep "a" as article)
            if *f == "a" {
                break;
            }
            words.remove(0);
        } else if *f == "," || *f == "ref" && words.len() > 2 && words[1] == "," {
            words.remove(0);
        } else {
            break;
        }
    }
    if words.len() < 2 {
        return None;
    }
    Some(words.join(" "))
}

fn count_probe(p: &Probe, sent: &str) -> u64 {
    let phrase = p.phrase.replace(',', " ,").replace(". ", " ").trim_end_matches('.').to_string();
    let phrase = phrase.split_whitespace().collect::<Vec<_>>().join(" ");
    match p.mode {
        Mode::Start => {
            let ok = sent == phrase || sent.starts_with(&format!("{phrase} "));
            let ok = ok && p.and_contains.is_none_or(|t| sent[phrase.len()..].contains(&format!(" {t} ")));
            ok as u64
        }
        Mode::Any => {
            let hay = format!(" {sent} ");
            hay.matches(&format!(" {phrase} ")).count() as u64
        }
    }
}

/// `weights --report R --map M`: phrase-map.txt + frequency report → phrases.txt (stdout).
///
/// weight = corpus rate × simplicity (1 / (1 + 0.2 · extra words beyond the role's shortest
/// template)); the `fixed` template is raised to ≥ 50% of the role maximum; `same` copies the previous
/// weight; everything is scaled so the role maximum is 100, rounded, with a floor of 1.
fn weights(report: &str, map: &str) -> String {
    let mut rates: HashMap<String, f64> = HashMap::new();
    let mut header = Vec::new();
    for line in report.lines() {
        if line.starts_with("# group") {
            header.push(line.trim_start_matches("# ").to_string());
            continue;
        }
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() >= 5
            && let Ok(mean) = f[f.len() - 1].parse::<f64>()
        {
            rates.insert(format!("{}/{}", f[0], f[1]), mean);
        }
    }
    let words = |t: &str| {
        t.split_whitespace()
            .filter(|w| !(w.starts_with('{') && w.trim_end_matches([',', ':', ';', '.']).ends_with('}')))
            .count()
    };
    let mut out = String::new();
    out.push_str("# stex phrase bank (STEX-DESIGN §5). GENERATED by `stex-corpus weights` from\n");
    out.push_str(
        "# tools/stex-corpus/phrase-map.txt and a frequency table of real proofs. Edit the map, not this file.\n",
    );
    out.push_str("# Corpus (2026-11-13; per-group rates averaged so no source dominates):\n");
    for h in &header {
        out.push_str(&format!("#   {h}\n"));
    }
    out.push_str(
        "# Weight = corpus rate x simplicity (-20% per extra word); the first (--fixed) template is kept at\n",
    );
    out.push_str("# >= 50% of the role's top weight; scaled to 100 per role; floor 1 so --varied/--random reach every phrasing.\n");
    out.push_str("#   [role]\n#   weight<TAB>template      Slots: {P} {Q} {X} {T} {A} {N}\n");
    let mut role: Option<String> = None;
    let mut rows: Vec<(String, f64, bool, bool)> = Vec::new(); // template, raw, fixed, same
    let flush = |role: &Option<String>, rows: &mut Vec<(String, f64, bool, bool)>, out: &mut String| {
        let Some(r) = role else { return };
        if rows.is_empty() {
            return;
        }
        let minw = rows.iter().map(|(t, ..)| words(t)).min().unwrap_or(1);
        let mut raw: Vec<f64> =
            rows.iter().map(|(t, rate, ..)| rate / (1.0 + 0.2 * (words(t) - minw) as f64)).collect();
        let max0 = raw.iter().cloned().fold(0.0, f64::max);
        for (k, (_, _, fixed, _)) in rows.iter().enumerate() {
            if *fixed {
                raw[k] = if max0 > 0.0 { raw[k].max(0.5 * max0) } else { 1.0 };
            }
        }
        for k in 1..rows.len() {
            if rows[k].3 {
                raw[k] = raw[k - 1];
            }
        }
        let max = raw.iter().cloned().fold(0.0, f64::max).max(1e-9);
        out.push_str(&format!("\n[{r}]\n"));
        for (k, (t, ..)) in rows.iter().enumerate() {
            let w = ((100.0 * raw[k] / max).round() as i64).max(1);
            out.push_str(&format!("{w}\t{t}\n"));
        }
        rows.clear();
    };
    for line in map.lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        if let Some(r) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            flush(&role, &mut rows, &mut out);
            role = Some(r.to_string());
            continue;
        }
        let f: Vec<&str> = line.split('\t').collect();
        let template = f[0].to_string();
        let ev = f.get(1).copied().unwrap_or("-");
        let flags = f.get(2).copied().unwrap_or("");
        let rate = if let Some(n) = ev.strip_prefix('=') {
            n.parse().unwrap_or(0.0)
        } else if ev == "-" {
            0.0
        } else {
            ev.split('+').map(|k| *rates.get(k).unwrap_or_else(|| panic!("no report row {k}"))).sum()
        };
        rows.push((template, rate, flags.contains("fixed"), flags.contains("same")));
    }
    flush(&role, &mut rows, &mut out);
    out
}

/// `symbols --base B`: expand symbol-base.txt into symbols.tsv (stdout). `+phrase` entries also yield
/// "is phrase" → latex and "is not phrase" / "isn't phrase" → negated latex. The first definition
/// of a word wins; later duplicates are reported on stderr.
fn symbols(base: &str) -> String {
    let mut out = String::new();
    out.push_str("# stex word → LaTeX symbol library (spec feature 12). GENERATED by `stex-corpus symbols` from\n");
    out.push_str("# tools/stex-corpus/symbol-base.txt. Edit the base file, not this one.\n");
    out.push_str("# word(s)<TAB>kind<TAB>latex   (multi-word entries are merged into one token by the lexer)\n");
    let mut seen: HashMap<String, String> = HashMap::new();
    let mut n = 0;
    let mut add = |word: &str, kind: &str, tex: &str, out: &mut String| {
        let key = word.to_string();
        if let Some(prev) = seen.get(&key) {
            if prev != &format!("{kind}\t{tex}") {
                eprintln!("duplicate word {word:?}: keeping {prev:?}, dropping {kind}\t{tex}");
            }
            return;
        }
        seen.insert(key, format!("{kind}\t{tex}"));
        out.push_str(&format!("{word}\t{kind}\t{tex}\n"));
        n += 1;
    };
    for line in base.lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() < 3 {
            continue;
        }
        let (kind, tex, words) = (f[0], f[1], f[2]);
        let neg = f.get(3).copied();
        for w in words.split('|') {
            match w.strip_prefix('+') {
                Some(phrase) => {
                    add(phrase, kind, tex, &mut out);
                    add(&format!("is {phrase}"), kind, tex, &mut out);
                    if let Some(neg) = neg {
                        add(&format!("is not {phrase}"), kind, neg, &mut out);
                        add(&format!("isn't {phrase}"), kind, neg, &mut out);
                        add(&format!("not {phrase}"), kind, neg, &mut out);
                    }
                }
                None => add(w, kind, tex, &mut out),
            }
        }
    }
    eprintln!("{n} entries");
    out
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("symbols") {
        let base = args.iter().position(|a| a == "--base").and_then(|i| args.get(i + 1)).expect("--base");
        print!("{}", symbols(&fs::read_to_string(base).expect("base")));
        return;
    }
    if args.first().map(String::as_str) == Some("weights") {
        let get = |flag: &str| args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).expect(flag);
        let report = fs::read_to_string(get("--report")).expect("report");
        let map = fs::read_to_string(get("--map")).expect("map");
        print!("{}", weights(&report, &map));
        return;
    }
    let mut groups: Vec<(String, PathBuf)> = Vec::new();
    let mut out_path = None;
    let mut symbols_path = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--out" => out_path = it.next().cloned(),
            "--symbols" => symbols_path = it.next().cloned(),
            other => {
                let (g, d) = other.split_once('=').expect("GROUP=DIR");
                groups.push((g.to_string(), PathBuf::from(d)));
            }
        }
    }
    let symbol_words: Vec<String> = symbols_path
        .map(|p| {
            fs::read_to_string(p)
                .unwrap_or_default()
                .lines()
                .filter(|l| !l.starts_with('#') && !l.is_empty())
                .filter_map(|l| l.split('\t').next())
                .map(|w| w.to_lowercase())
                .filter(|w| w.chars().count() > 3)
                .collect()
        })
        .unwrap_or_default();
    let mut names: Vec<String> = Vec::new();
    let mut stats: HashMap<String, Group> = HashMap::new();
    for (g, dir) in &groups {
        if !names.contains(g) {
            names.push(g.clone());
        }
        let st =
            stats.entry(g.clone()).or_insert_with(|| Group { probes: vec![0; PROBES.len()], ..Default::default() });
        let mut files = Vec::new();
        walk(dir, &mut files);
        for f in files {
            let Ok(bytes) = fs::read(&f) else { continue };
            let src = String::from_utf8_lossy(&bytes);
            let is_ptx = f.extension().is_some_and(|e| e != "tex");
            let proofs = if is_ptx {
                between(&src, "<proof>", "</proof>")
            } else {
                between(&src, "\\begin{proof}", "\\end{proof}")
            };
            if proofs.is_empty() {
                continue;
            }
            st.files += 1;
            for p in proofs {
                st.proofs += 1;
                let text = if is_ptx { clean_ptx(&p) } else { clean_latex(&p) };
                for sent in sentences(&text) {
                    st.sentences += 1;
                    let words: Vec<&str> = sent.split(' ').filter(|w| *w != ",").collect();
                    st.words += words.len();
                    for n in 1..=4.min(words.len()) {
                        *st.starts.entry(words[..n].join(" ")).or_default() += 1;
                    }
                    for (k, pr) in PROBES.iter().enumerate() {
                        st.probes[k] += count_probe(pr, &sent);
                    }
                    if !symbol_words.is_empty() {
                        let hay = format!(" {sent} ");
                        for w in &symbol_words {
                            if hay.contains(&format!(" {w} ")) {
                                *st.symbols.entry(w.clone()).or_default() += 1;
                            }
                        }
                    }
                }
            }
        }
    }
    let rate = |g: &Group, c: u64| if g.sentences == 0 { 0.0 } else { c as f64 * 10_000.0 / g.sentences as f64 };
    let mut report = String::new();
    report
        .push_str("# stex proof-phrase frequency table (generated by tools/stex-corpus; see claude/RESEARCH.md R6)\n");
    for n in &names {
        let g = &stats[n];
        report.push_str(&format!(
            "# group {n}: {} files, {} proofs, {} sentences, {} words\n",
            g.files, g.proofs, g.sentences, g.words
        ));
    }
    report.push_str("# rates are per 10,000 proof sentences; `mean` is the unweighted mean over groups\n");
    report.push_str(&format!(
        "role\tphrase\tmode\t{}\tmean\n",
        names.iter().map(|n| format!("{n}_count\t{n}_rate")).collect::<Vec<_>>().join("\t")
    ));
    for (k, p) in PROBES.iter().enumerate() {
        let mut cols = Vec::new();
        let mut mean = 0.0;
        for n in &names {
            let g = &stats[n];
            let r = rate(g, g.probes[k]);
            mean += r / names.len() as f64;
            cols.push(format!("{}\t{r:.1}", g.probes[k]));
        }
        let mode = match (p.mode, p.and_contains) {
            (Mode::Start, Some(t)) => format!("start+{t}"),
            (Mode::Start, None) => "start".into(),
            (Mode::Any, _) => "any".into(),
        };
        report.push_str(&format!("{}\t{}\t{mode}\t{}\t{mean:.1}\n", p.role, p.phrase, cols.join("\t")));
    }
    // top sentence starts (combined by mean rate)
    let mut all: HashMap<&str, f64> = HashMap::new();
    for n in &names {
        let g = &stats[n];
        for (k, c) in &g.starts {
            *all.entry(k.as_str()).or_default() += rate(g, *c) / names.len() as f64;
        }
    }
    let mut v: Vec<(&str, f64)> = all.into_iter().filter(|(k, _)| k.split(' ').count() >= 2).collect();
    v.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap().then(a.0.cmp(b.0)));
    report.push_str("\n# top sentence starts (2-4 words), mean rate per 10k sentences\n");
    for (k, r) in v.iter().take(300) {
        report.push_str(&format!("start\t{k}\t{r:.1}\n"));
    }
    if !symbol_words.is_empty() {
        let mut sv: HashMap<&str, f64> = HashMap::new();
        for n in &names {
            let g = &stats[n];
            for (k, c) in &g.symbols {
                *sv.entry(k.as_str()).or_default() += rate(g, *c) / names.len() as f64;
            }
        }
        let mut sv: Vec<(&str, f64)> = sv.into_iter().collect();
        sv.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap().then(a.0.cmp(b.0)));
        report
            .push_str("\n# symbol-library words used in proof prose (sentences containing them), mean rate per 10k\n");
        for (k, r) in sv.iter().take(400) {
            report.push_str(&format!("symword\t{k}\t{r:.2}\n"));
        }
    }
    match out_path {
        Some(p) => fs::write(&p, &report).expect("write report"),
        None => print!("{report}"),
    }
}
