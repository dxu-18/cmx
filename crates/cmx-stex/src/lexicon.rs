//! Word → symbol library (`data/symbols.tsv`) and the keyword / English word lists the segmenter
//! uses. The data file is embedded and parsed once (~200 µs).

use std::collections::HashMap;
use std::sync::OnceLock;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Greek,
    Const,
    Set,
    /// Operator name; argument optional (`sin x`, `sin(x)`).
    Func,
    /// Operator name that is also an English word: math only with a glued `(` (`rank(A)`).
    FuncP,
    Rel,
    /// Multiplicative operator (`times`, `otimes`).
    Bin,
    /// Additive operator (`plus`, `minus`, `pm`).
    Add,
    /// Set operator level (∪ ∩ ∖ △).
    SetOp,
    /// "divided by" → fraction.
    Div,
    Post,
    /// `x hat` / `hat x` → `\hat{x}`.
    Accent,
    Arrow,
    Big,
}

pub struct Lexicon {
    map: HashMap<String, (Kind, String)>,
    multi: Vec<String>,
}

static LEX: OnceLock<Lexicon> = OnceLock::new();

pub fn get() -> &'static Lexicon {
    LEX.get_or_init(|| {
        let mut map = HashMap::new();
        let mut multi = Vec::new();
        for line in include_str!("../data/symbols.tsv").lines() {
            if line.starts_with('#') || line.trim().is_empty() {
                continue;
            }
            let mut f = line.split('\t');
            let (Some(w), Some(k), Some(tex)) = (f.next(), f.next(), f.next()) else { continue };
            let kind = match k {
                "greek" => Kind::Greek,
                "const" => Kind::Const,
                "set" => Kind::Set,
                "func" => Kind::Func,
                "funcp" => Kind::FuncP,
                "rel" => Kind::Rel,
                "bin" => Kind::Bin,
                "add" => Kind::Add,
                "setop" => Kind::SetOp,
                "div" => Kind::Div,
                "accent" => Kind::Accent,
                "post" => Kind::Post,
                "arrow" => Kind::Arrow,
                "big" => Kind::Big,
                _ => continue,
            };
            if w.contains(' ') {
                multi.push(w.to_string());
            }
            map.insert(w.to_string(), (kind, tex.to_string()));
        }
        Lexicon { map, multi }
    })
}

impl Lexicon {
    /// Exact match first (so `Gamma` ≠ `gamma`, `E` is expectation only when capital), then lowercase.
    pub fn lookup(&self, w: &str) -> Option<(Kind, &str)> {
        if let Some((k, t)) = self.map.get(w) {
            return Some((*k, t));
        }
        let l = w.to_ascii_lowercase();
        // single capital letters are variables unless listed exactly
        if w.len() == 1 {
            return None;
        }
        self.map.get(&l).map(|(k, t)| (*k, t.as_str()))
    }

    pub fn multiword(&self) -> &[String] {
        &self.multi
    }
}

/// Structural stex keywords (for `-keyword` escapes and for stopping math runs).
pub fn is_keyword(w: &str) -> bool {
    const KW: &[&str] = &[
        "let",
        "fix",
        "assume",
        "suppose",
        "afsoc",
        "aftsoc",
        "wts",
        "qed",
        "contradiction",
        "contra",
        "then",
        "thus",
        "hence",
        "so",
        "therefore",
        "define",
        "via",
        "by",
        "because",
        "since",
        "b/c",
        "i.e.",
        "ie",
        "note",
        "wlog",
        "case",
        "proof",
        "claim",
        "lemma",
        "proposition",
        "theorem",
        "corollary",
        "observation",
        "remark",
        "fact",
        "sublemma",
        "aside",
        "exists",
        "forall",
        "and",
        "or",
        "not",
        "xor",
        "nand",
        "nor",
        "implies",
        "iff",
        "if",
        "with",
        "w/",
        "where",
        "such",
        "st",
        "s.t.",
        "sum",
        "prod",
        "product",
        "integral",
        "integrate",
        "limit",
        "lim",
        "derivative",
        "sqrt",
        "choose",
        "mod",
        "over",
        "bold",
        "italic",
        "underline",
        "latex",
        "in",
        "to",
        "if and only if",
        "only if",
    ];
    KW.contains(&w) || get().lookup(w).is_some()
}

/// Words after which a following single letter is certainly a variable ("let a be …", "for all a").
pub fn declares(w: &str) -> bool {
    matches!(
        w,
        "let"
            | "fix"
            | "assume"
            | "suppose"
            | "afsoc"
            | "aftsoc"
            | "define"
            | "exists"
            | "there exists"
            | "there exist"
            | "there is"
            | "there are"
            | "for all"
            | "for every"
            | "for each"
            | "for any"
            | "for some"
            | "some"
            | "any"
            | "every"
            | "each"
            | "all"
            | "forall"
            | "choose"
            | "pick"
            | "take"
            | "consider"
            | "then"
            | "wts"
            | "and"
            | "or"
            | "of"
            | "with"
            | "where"
            | "such that"
            | "s.t."
            | "st"
            | "w/"
            | "by"
            | "since"
            | "because"
            | "b/c"
            | "so"
            | "thus"
            | "hence"
            | "therefore"
            | "that"
            | "if"
            | "implies"
            | "iff"
    )
}

/// Verbs/links that follow a variable ("a is even", "A has …"), used to tell the variable `a`
/// from the article "a".
pub fn is_link_verb(w: &str) -> bool {
    matches!(
        w,
        "is" | "are"
            | "be"
            | "was"
            | "were"
            | "has"
            | "have"
            | "satisfies"
            | "satisfy"
            | "must"
            | "can"
            | "cannot"
            | "does"
            | "do"
            | "will"
            | "would"
            | "should"
            | "may"
            | "might"
            | "and"
            | "or"
            | "divides"
            | "equals"
            | "is in"
            | "is not"
            | "is a subset of"
            | "is an element of"
            | "is equal to"
            | "is less than"
            | "is greater than"
            | "is at most"
            | "is at least"
            | "is divisible by"
            | "belongs to"
            | "as"
            | "if"
            | "iff"
            | "implies"
            | "then"
            | "such that"
            | "s.t."
            | "with"
            | "w/"
            | "for"
            | "by"
            | "since"
            | "because"
    )
}

/// Words that are always prose inside a math run in prose mode (they stop the run).
pub fn is_prose_stop(w: &str) -> bool {
    matches!(
        w,
        "and"
            | "or"
            | "not"
            | "implies"
            | "iff"
            | "then"
            | "if"
            | "by"
            | "because"
            | "since"
            | "b/c"
            | "such that"
            | "s.t."
            | "st"
            | "with"
            | "w/"
            | "where"
            | "for"
            | "for all"
            | "for every"
            | "for each"
            | "for any"
            | "for some"
            | "so"
            | "thus"
            | "hence"
            | "therefore"
            | "via"
            | "i.e."
            | "ie"
            | "xor"
            | "nand"
            | "nor"
            | "is"
            | "are"
            | "be"
    )
}

/// The capital letters that name number sets in set position (`x in Z`, `f: N to N`).
pub fn number_set(c: &str) -> Option<&'static str> {
    Some(match c {
        "N" => "\\mathbb{N}",
        "Z" => "\\mathbb{Z}",
        "Q" => "\\mathbb{Q}",
        "R" => "\\mathbb{R}",
        "C" => "\\mathbb{C}",
        "F" => "\\mathbb{F}",
        "field" => "\\mathbb{F}",
        _ => return None,
    })
}

/// Short English words that are never multi-letter variables in explicit math (they become `\text{}`).
pub fn is_english(w: &str) -> bool {
    matches!(
        w.to_ascii_lowercase().as_str(),
        "a" | "an"
            | "the"
            | "is"
            | "are"
            | "be"
            | "of"
            | "for"
            | "all"
            | "if"
            | "in"
            | "on"
            | "at"
            | "to"
            | "by"
            | "we"
            | "so"
            | "as"
            | "it"
            | "no"
            | "not"
            | "and"
            | "or"
            | "but"
            | "some"
            | "any"
            | "each"
            | "every"
            | "has"
            | "have"
            | "with"
            | "where"
            | "when"
            | "then"
            | "this"
            | "that"
            | "odd"
            | "even"
            | "prime"
            | "else"
            | "otherwise"
            | "since"
            | "because"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookups() {
        let l = get();
        assert_eq!(l.lookup("alpha"), Some((Kind::Greek, "\\alpha")));
        assert_eq!(l.lookup("Gamma"), Some((Kind::Greek, "\\Gamma")));
        assert_eq!(l.lookup("integers"), Some((Kind::Set, "\\mathbb{Z}")));
        assert_eq!(l.lookup("empty set"), Some((Kind::Const, "\\varnothing")));
        assert_eq!(l.lookup("union"), Some((Kind::SetOp, "\\cup")));
        assert_eq!(l.lookup("is isomorphic to"), Some((Kind::Rel, "\\cong")));
        assert_eq!(l.lookup("is not isomorphic to"), Some((Kind::Rel, "\\ncong")));
        assert_eq!(l.lookup("≤"), Some((Kind::Rel, "\\leq")));
        assert_eq!(l.lookup("rank"), Some((Kind::FuncP, "\\operatorname{rank}")));
        assert!(l.multiword().len() > 500);
        assert_eq!(l.lookup("x"), None);
        assert!(l.multiword().iter().any(|w| w == "real numbers"));
        assert!(is_keyword("exists") && is_keyword("alpha") && !is_keyword("prime"));
    }
}
