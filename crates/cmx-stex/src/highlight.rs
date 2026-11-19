//! Syntax roles for the editor's stex highlighting, computed with the compiler's own lexer.
//!
//! Highlighting with the real lexer (multi-word phrase merging, abbreviations, `$…$`, style
//! blocks) means the colors are exactly what the compiler sees: every alias of a construct gets
//! the same role (`such that`, `s.t.`, `st`, `st.` are all [`Role::Quantifier`]); a phrase is
//! colored only once it is complete ("such" alone is prose until "that" follows, "there" until
//! "is"/"exists"); and a word that is a keyword only in a phrase ("is" alone) stays prose.

use crate::lex::{self, K, Tok};
use crate::lexicon;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// Statement keywords: let, assume, wts, afsoc, qed, case, step, define, note, wlog, …
    Structure,
    /// proof, lemma, theorem, claim, …
    Label,
    /// exists / there is / for all / such that / s.t. / with … (binders and their separators)
    Quantifier,
    /// and, or, not, implies, iff, xor, then, thus, hence, so, therefore, …
    Logic,
    /// by, because, since, b/c, i.e., via
    Reason,
    /// Words the compiler turns into symbols (union, choose, sqrt, alpha, integers, divides, …)
    MathWord,
    /// Explicit `$…$`, `latex{…}`, sigils
    Math,
    Number,
    /// = < <= == != + - * / ^ | …
    Operator,
    /// bold / italic / underline blocks and words
    Style,
    Comment,
}

fn word_role(w: &str) -> Option<Role> {
    let l = w.to_lowercase();
    let l = l.as_str();
    if matches!(
        l,
        "proof"
            | "claim"
            | "lemma"
            | "proposition"
            | "theorem"
            | "corollary"
            | "observation"
            | "remark"
            | "fact"
            | "sublemma"
            | "aside"
            | "definition"
            | "example"
    ) {
        return Some(Role::Label);
    }
    if matches!(
        l,
        "let"
            | "fix"
            | "assume"
            | "suppose"
            | "assuming"
            | "afsoc"
            | "aftsoc"
            | "contradiction"
            | "contra"
            | "this is a contradiction"
            | "wts"
            | "want to show"
            | "we want to show"
            | "we need to show"
            | "qed"
            | "define"
            | "case"
            | "subcase"
            | "sub-case"
            | "step"
            | "stage"
            | "choose"
            | "pick"
            | "take"
            | "note"
            | "note that"
            | "observe"
            | "notice"
            | "recall"
            | "wlog"
            | "without loss of generality"
            | "suppose not"
            | "assume not"
            | "suppose otherwise"
            | "assume otherwise"
            | "assume the contrary"
            | "suppose the contrary"
            | "for the sake of contradiction"
            | "for contradiction"
            | "base case"
            | "inductive step"
            | "induction step"
            | "inductive hypothesis"
            | "induction hypothesis"
            | "in either case"
            | "in both cases"
            | "in all cases"
            | "in every case"
            | "in any case"
            | "either way"
            | "in total"
            | "all in all"
            | "in all"
            | "altogether"
    ) || crate::proof::is_exhaustive_keyword(l)
    {
        return Some(Role::Structure);
    }
    if matches!(
        l,
        "exists"
            | "there exists"
            | "there exist"
            | "there is"
            | "there are"
            | "there exists a unique"
            | "there is a unique"
            | "there is exactly one"
            | "there exists unique"
            | "exists unique"
            | "there does not exist"
            | "there is no"
            | "there are no"
            | "there exist no"
            | "there exists no"
            | "for all"
            | "for every"
            | "for each"
            | "for any"
            | "for some"
            | "forall"
            | "such that"
            | "so that"
            | "s.t."
            | "st"
            | "w/"
            | "with"
            | "where"
    ) {
        return Some(Role::Quantifier);
    }
    if matches!(
        l,
        "and"
            | "or"
            | "not"
            | "implies"
            | "iff"
            | "if and only if"
            | "only if"
            | "if"
            | "xor"
            | "nand"
            | "nor"
            | "then"
            | "thus"
            | "hence"
            | "so"
            | "therefore"
            | "it follows that"
            | "consequently"
            | "we have"
    ) {
        return Some(Role::Logic);
    }
    if matches!(l, "by" | "because" | "since" | "b/c" | "via" | "i.e." | "ie" | "that is" | "in other words") {
        return Some(Role::Reason);
    }
    // math words the compiler knows (single letters are variables, left uncolored)
    if w.chars().count() > 1
        && (lexicon::get().lookup(w).is_some()
            || matches!(
                l,
                "sum"
                    | "prod"
                    | "product"
                    | "integral"
                    | "integrate"
                    | "limit"
                    | "lim"
                    | "derivative"
                    | "sqrt"
                    | "choose"
                    | "mod"
            ))
    {
        return Some(Role::MathWord);
    }
    None
}

fn tok_role(t: &Tok) -> Option<Role> {
    match &t.kind {
        K::Math(_) | K::Latex(_) | K::Sigil(_) => Some(Role::Math),
        K::Block(..) | K::StyleWord(_) => Some(Role::Style),
        K::Num => Some(Role::Number),
        K::Sym => match t.text.as_str() {
            "," | "." | ";" | ":" | "?" | "!" | "(" | ")" | "[" | "]" | "{" | "}" => None,
            _ => Some(Role::Operator),
        },
        K::Word => word_role(&t.text),
        _ => None,
    }
}

/// Roles for one line of stex: `(start, end, role)` byte ranges, plus whether a `$` display block
/// is still open at the end of the line (`in_display` says whether one was open at its start).
pub fn line_roles(line: &str, in_display: bool) -> (Vec<(usize, usize, Role)>, bool) {
    let mut out = Vec::new();
    let mut from = 0;
    if in_display {
        match line.find('$') {
            Some(p) => {
                out.push((0, p + 1, Role::Math));
                from = p + 1;
            }
            None => {
                if !line.is_empty() {
                    out.push((0, line.len(), Role::Math));
                }
                return (out, true);
            }
        }
    }
    // the rest of the line: a `//` comment and an unclosed `$` (a display starting here) end it
    let rest = &line[from..];
    let mut end = rest.len();
    let mut comment = None;
    let mut dollars = Vec::new();
    let b = rest.as_bytes();
    let mut k = 0;
    while k < b.len() {
        match b[k] {
            b'$' => dollars.push(k),
            b'/' if k + 1 < b.len() && b[k + 1] == b'/' && dollars.len() % 2 == 0 => {
                comment = Some(k);
                break;
            }
            _ => {}
        }
        k += 1;
    }
    if let Some(c) = comment {
        end = c;
    }
    let mut open_display = false;
    if dollars.len() % 2 == 1 {
        let last = *dollars.last().expect("odd count");
        end = end.min(last);
        open_display = true;
    }
    let body = &rest[..end];
    match lex::lex(body) {
        Ok(toks) => {
            for t in &toks {
                if let Some(r) = tok_role(t)
                    && t.span.1 > t.span.0
                {
                    out.push((from + t.span.0, from + t.span.1, r));
                }
            }
        }
        // mid-edit (`bold{` not closed yet): color what precedes the problem
        Err(_) => {
            if let Some(cut) = body.rfind(['{', '$'])
                && let Ok(toks) = lex::lex(&body[..cut])
            {
                for t in &toks {
                    if let Some(r) = tok_role(t)
                        && t.span.1 > t.span.0
                    {
                        out.push((from + t.span.0, from + t.span.1, r));
                    }
                }
            }
        }
    }
    if open_display {
        out.push((from + end, from + if let Some(c) = comment { c.max(end) } else { rest.len() }, Role::Math));
    }
    if let Some(c) = comment {
        out.push((from + c, line.len(), Role::Comment));
    }
    (out, open_display)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roles(line: &str) -> Vec<(String, Role)> {
        line_roles(line, false).0.into_iter().map(|(a, b, r)| (line[a..b].to_string(), r)).collect()
    }

    fn role_of(line: &str, word: &str) -> Option<Role> {
        roles(line).into_iter().find(|(w, _)| w == word).map(|(_, r)| r)
    }

    #[test]
    fn aliases_share_a_role_and_phrases_color_when_complete() {
        for sep in ["such that", "s.t.", "st", "st.", "s.t", "with", "w/"] {
            let line = format!("let x in Z {sep} x > 0");
            assert_eq!(role_of(&line, sep), Some(Role::Quantifier), "{line}: {:?}", roles(&line));
        }
        // "such" alone is not a keyword yet
        assert_eq!(role_of("let x in Z such", "such"), None);
        for q in ["exists", "there exists", "there is", "there are", "for all", "for every", "forall"] {
            let line = format!("{q} x in Z, x = x");
            assert_eq!(role_of(&line, q), Some(Role::Quantifier), "{line}");
        }
        // "is" on its own is prose; "is in" is a math word
        assert_eq!(role_of("x is even", "is"), None);
        assert_eq!(role_of("x is in A", "is in"), Some(Role::MathWord));
        for k in ["afsoc", "aftsoc", "wts", "let", "qed", "assume"] {
            assert_eq!(role_of(&format!("{k} x = 1"), k), Some(Role::Structure), "{k}");
        }
        for r in ["because", "since", "b/c", "by"] {
            assert_eq!(role_of(&format!("x = 1 {r} y = 1"), r), Some(Role::Reason), "{r}");
        }
        assert_eq!(role_of("x = 1 i.e. y = 1", "i.e."), Some(Role::Reason));
        assert_eq!(role_of("A union B", "union"), Some(Role::MathWord));
        assert_eq!(role_of("lemma: x = 1", "lemma"), Some(Role::Label));
        // case-analysis keywords: "case on", "exhaustive" and its phrasings
        for k in
            ["case on", "cases exhaustive", "exhaustive cases", "in all exhaustive cases", "we proceed by cases on"]
        {
            assert_eq!(role_of(&format!("{k} k"), k), Some(Role::Structure), "{k}");
        }
    }

    #[test]
    fn math_comments_and_displays() {
        let (r, open) = line_roles("let $x = 1$ // note", false);
        assert!(!open);
        assert!(r.contains(&(4, 11, Role::Math)), "{r:?}");
        assert!(r.contains(&(12, 19, Role::Comment)), "{r:?}");
        // a lone `$` opens a display that continues on the next lines
        let (r, open) = line_roles("$", false);
        assert!(open && r.iter().any(|x| x.2 == Role::Math));
        let (r, open) = line_roles("x = 9 implies x - 9 = 0", true);
        assert!(open && r == vec![(0, 23, Role::Math)]);
        let (_, open) = line_roles("$", true);
        assert!(!open);
        // unclosed style block while typing: no panic, earlier words still colored
        let (r, _) = line_roles("let x = 1 and bold{ half", false);
        assert!(r.iter().any(|x| x.2 == Role::Structure), "{r:?}");
    }
}
