//! Sentences → constructs → LaTeX prose (STEX-DESIGN §3, §9).
//!
//! [`split`] cuts the token stream into statements (sentence boundaries, feature 10).
//! [`Realizer`] recognizes each statement's construct (`let`, `assume`, `wts`, `qed`, `by THM`,
//! `case`, …), parses its clauses (logic connectives, quantifiers, mixed prose/math), and renders
//! them with the phrase bank. Along the way it records [`Fact`]s for the Lean translation.

use crate::lex::{K, Style, Tok};
use crate::lexicon;
use crate::math::{self, M, Parser, Step};
use crate::phrases::{Picker, Variance, fill};
use crate::theorems;

// ------------------------------------------------------------------------------------ statements

#[derive(Clone, Debug)]
pub struct Stmt {
    pub toks: Vec<Tok>,
    pub line: u32,
    /// The user's terminator (`.`, `?`, `!`), if any.
    pub term: Option<String>,
    /// A blank line precedes this statement.
    pub para: bool,
}

fn continues(cur: &[Tok], depth: i32, next: Option<&Tok>) -> bool {
    if depth > 0 {
        return true;
    }
    // the next line starts with a verb or connective: "A = {…}" ⏎ "is the set of all primes"
    if let Some(n) = next
        && n.kind == K::Word
        && !n.text.starts_with(char::is_uppercase)
        && (matches!(
            n.lower().as_str(),
            "is" | "are"
                | "was"
                | "were"
                | "has"
                | "have"
                | "equals"
                | "divides"
                | "lies"
                | "belongs to"
                | "holds"
                | "and"
                | "or"
                | "which"
                | "where"
                | "so that"
                | "such that"
                | "implies"
                | "iff"
                | "if and only if"
                | "with"
                | "w/"
        ) || n.lower().starts_with("is "))
    {
        return true;
    }
    let Some(last) = cur.last() else { return false };
    if last.kind == K::Sym
        && matches!(
            last.text.as_str(),
            "," | "="
                | "+"
                | "-"
                | "*"
                | "/"
                | "^"
                | "<"
                | ">"
                | "<="
                | ">="
                | "=="
                | "!="
                | ":="
                | "->"
                | "=>"
                | "<=>"
                | "&&"
                | "||"
        )
    {
        return true;
    }
    if last.kind == K::Word
        && matches!(
            last.lower().as_str(),
            "and"
                | "or"
                | "such that"
                | "s.t."
                | "st"
                | "with"
                | "w/"
                | "where"
                | "implies"
                | "iff"
                | "if"
                | "because"
                | "since"
                | "b/c"
                | "by"
                | "of"
                | "to"
                | "from"
                | "via"
                | "that"
                | "the"
                | "a"
                | "an"
                | "in"
                | "so"
                | "thus"
                | "hence"
                | "therefore"
                | "then"
                | "we have"
                | "we get"
                | "we obtain"
                | "gives"
                | "yields"
                | "namely"
                | "i.e."
                | "that is"
        )
    {
        return true;
    }
    let Some(n) = next else { return false };
    // "…, we have:" followed by a display block on the next line
    if last.is_sym(":") && matches!(n.kind, K::Math(_)) {
        return true;
    }
    match &n.kind {
        K::Word => matches!(
            n.lower().as_str(),
            "and" | "or" | "such that" | "s.t." | "st" | "where" | "with" | "w/" | "implies" | "iff"
        ),
        K::Sym => matches!(
            n.text.as_str(),
            "=" | "+" | "*" | "/" | "<" | ">" | "<=" | ">=" | "==" | "!=" | "=>" | "<=>" | "&&" | "||" | ","
        ),
        _ => false,
    }
}

/// Cut tokens into statements: `.`/`?`/prose `!` end a sentence, a newline ends one unless the line
/// clearly continues, a blank line starts a new paragraph.
pub fn split(toks: &[Tok]) -> Vec<Stmt> {
    let mut out = Vec::new();
    let mut cur: Vec<Tok> = Vec::new();
    let mut para = true;
    let mut depth = 0i32;
    let flush = |cur: &mut Vec<Tok>, term: Option<String>, para: &mut bool, out: &mut Vec<Stmt>| {
        if !cur.is_empty() {
            let line = cur[0].line;
            out.push(Stmt { toks: std::mem::take(cur), line, term, para: *para });
            *para = false;
        }
    };
    for (i, t) in toks.iter().enumerate() {
        let next = toks.get(i + 1);
        let ends_here = next.is_none_or(|n| matches!(n.kind, K::Newline | K::Para)) || t.sp_after;
        match &t.kind {
            K::Para => {
                flush(&mut cur, None, &mut para, &mut out);
                para = true;
                depth = 0;
            }
            K::Newline => {
                if cur.is_empty() || continues(&cur, depth, next) {
                    continue;
                }
                flush(&mut cur, None, &mut para, &mut out);
                depth = 0;
            }
            K::Sym if (t.text == "." || t.text == "?") && depth <= 0 && ends_here => {
                flush(&mut cur, Some(t.text.clone()), &mut para, &mut out);
            }
            K::Sym
                if t.text == "!"
                    && depth <= 0
                    && ends_here
                    && (t.sp_before || cur.last().is_some_and(|p| p.kind == K::Word && p.text.len() > 1)) =>
            {
                flush(&mut cur, Some("!".into()), &mut para, &mut out);
            }
            _ => {
                if t.kind == K::Sym {
                    match t.text.as_str() {
                        "(" | "[" | "{" => depth += 1,
                        ")" | "]" | "}" => depth -= 1,
                        _ => {}
                    }
                }
                cur.push(t.clone());
            }
        }
    }
    flush(&mut cur, None, &mut para, &mut out);
    out
}

// ------------------------------------------------------------------------------------ clauses

#[derive(Clone, Debug)]
pub enum Piece {
    Prose(String),
    Math(M),
    /// Already-rendered LaTeX (style blocks, raw latex spans).
    Tex(String),
    Punct(String),
}

#[derive(Clone, Debug)]
pub enum Clause {
    Math(M),
    Mixed(Vec<Piece>),
    And(Vec<Clause>),
    Or(Vec<Clause>),
    Xor(Box<Clause>, Box<Clause>),
    Nand(Box<Clause>, Box<Clause>),
    Nor(Box<Clause>, Box<Clause>),
    /// (premise, conclusion, written as "if … then …")
    Implies(Box<Clause>, Box<Clause>, bool),
    Iff(Box<Clause>, Box<Clause>),
    Not(Box<Clause>),
    Quant {
        q: Quant,
        vars: Box<Clause>,
        body: Option<Box<Clause>>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Quant {
    Exists,
    Unique,
    None,
    ForAll,
}

fn quant_word(w: &str) -> Option<Quant> {
    Some(match w {
        "exists" | "there exists" | "there exist" | "there is" | "there are" => Quant::Exists,
        "exists unique"
        | "there exists a unique"
        | "there is a unique"
        | "there exists unique"
        | "there is exactly one" => Quant::Unique,
        "there is no" | "there are no" | "there exist no" | "there exists no" | "there does not exist" => Quant::None,
        "for all" | "for every" | "for each" | "for any" | "forall" | "given any" => Quant::ForAll,
        _ => return None,
    })
}

fn is_sep(t: &Tok) -> bool {
    t.is_sym(",") || t.is_sym(":") || matches!(t.lower().as_str(), "such that" | "s.t." | "st" | "where" | "so that")
}

/// Where a quantifier's binder ends. Brackets nest (`c in (a, b)`); a comma before another bound
/// variable is a list comma (`integers m, n s.t.`); a colon before a signature belongs to the
/// binder (`g: Y to X`); `with`/`where`/`such that` separate the condition of ∃, while for ∀ they
/// restrict the binder up to the next comma ("for all n, k in N with k <= n, …").
fn binder_sep(rest: &[Tok], q: Quant) -> Option<usize> {
    let mut depth = 0i32;
    let mut restriction = None;
    for (i, t) in rest.iter().enumerate() {
        if t.kind == K::Sym {
            match t.text.as_str() {
                "(" | "[" | "{" => {
                    depth += 1;
                    continue;
                }
                ")" | "]" | "}" => {
                    depth -= 1;
                    continue;
                }
                _ => {}
            }
        }
        if depth != 0 || i == 0 {
            continue;
        }
        let l = t.lower();
        if t.kind == K::Word
            && matches!(
                l.as_str(),
                "such that" | "s.t." | "st" | "where" | "so that" | "with" | "for which" | "satisfying"
            )
        {
            if q == Quant::ForAll && restriction.is_none() {
                restriction = Some(i);
                continue;
            }
            if q != Quant::ForAll {
                // "there exist p, q with q != 0 such that …": `with` restricts, `such that` separates
                let later_such_that = matches!(l.as_str(), "with" | "where")
                    && rest[i + 1..]
                        .iter()
                        .any(|n| matches!(n.lower().as_str(), "such that" | "s.t." | "st" | "so that"));
                if later_such_that {
                    continue;
                }
                return Some(i);
            }
            continue;
        }
        if t.is_sym(":") {
            let signature = rest[i + 1..].iter().take(4).any(|n| n.is_kw("to") || n.is_sym("->") || n.is_sym("→"));
            if !signature {
                return Some(i);
            }
            continue;
        }
        if t.is_sym(",") {
            if restriction.is_none() && continues_binder(&rest[i + 1..]) {
                continue;
            }
            return Some(i);
        }
    }
    restriction
}

/// After a comma in a binder: another bound variable (`, n s.t.`, `, y in R`, `, b > 0`)?
fn continues_binder(t: &[Tok]) -> bool {
    let Some(v) = t.first() else { return false };
    let var_like = v.is_sym("...")
        || v.kind == K::Word
            && ((v.text.chars().count() == 1 && v.text.chars().all(char::is_alphabetic))
                || matches!(lexicon::get().lookup(&v.text), Some((lexicon::Kind::Greek, _))));
    if !var_like {
        return false;
    }
    let mut k = 1;
    while t.get(k).is_some_and(|n| n.is_sym("_") && !n.sp_before) {
        k += 2;
    }
    match t.get(k) {
        None => true,
        Some(n) => {
            is_sep(n)
                || n.is_kw("with")
                || n.is_kw("in")
                || n.is_kw("and")
                || n.is_kw("be")
                || matches!(n.lower().as_str(), "are" | "were" | "is" | "was" | "have" | "has")
                || matches!(n.text.as_str(), ">" | "<" | ">=" | "<=" | "!=" | "∈" | "≥" | "≤")
        }
    }
}

/// "a"/"an" used as an article: before a prose word or a number ("a largest prime", "a 2 by 2
/// board", "a triangle"), not before an operator or verb ("a divides b", "a is even", "a in A").
pub fn is_article_at(toks: &[Tok], i: usize) -> bool {
    let Some(t) = toks.get(i) else { return false };
    if t.kind != K::Word || !matches!(t.text.as_str(), "a" | "A" | "an" | "An") {
        return false;
    }
    let Some(n) = toks.get(i + 1) else { return false };
    if t.text.eq_ignore_ascii_case("an") {
        return n.kind == K::Word;
    }
    match n.kind {
        K::Num => true,
        K::Word => {
            let l = n.lower();
            if lexicon::is_link_verb(&l) || lexicon::is_prose_stop(&l) && !matches!(l.as_str(), "is" | "are" | "be") {
                return false;
            }
            if matches!(l.as_str(), "is" | "are" | "be" | "in" | "mod" | "choose" | "over" | "to") {
                return false;
            }
            match lexicon::get().lookup(&n.text) {
                Some((k, _)) => {
                    use lexicon::Kind::*;
                    // "a dot product", "a cross section": an operator word followed by a noun
                    let noun_after = toks.get(i + 2).is_some_and(|w| {
                        w.kind == K::Word
                            && w.text.chars().count() > 1
                            && !matches!(lexicon::get().lookup(&w.text), Some((Greek | Const | Set, _)))
                            && !lexicon::is_link_verb(&w.lower())
                    });
                    !matches!(k, Rel | Bin | Add | SetOp | Div | Post | Arrow | Accent) || noun_after
                }
                None => n.text.chars().all(|c| c.is_alphabetic() || c == '-'),
            }
        }
        _ => false,
    }
}

/// Index of the first top-level token satisfying `f`, scanning only before any quantifier
/// (whose body extends to the end of the clause).
fn find_top(toks: &[Tok], f: impl Fn(&Tok) -> bool) -> Option<usize> {
    let mut depth = 0;
    for (i, t) in toks.iter().enumerate() {
        if t.kind == K::Sym {
            match t.text.as_str() {
                "(" | "[" | "{" => depth += 1,
                ")" | "]" | "}" => depth -= 1,
                _ => {}
            }
        }
        if depth == 0 && i > 0 && t.kind == K::Word && quant_word(&t.lower()).is_some() {
            return None;
        }
        if depth == 0 && f(t) {
            return Some(i);
        }
    }
    None
}

fn split_all(toks: &[Tok], f: impl Fn(&Tok) -> bool + Copy) -> Vec<&[Tok]> {
    let mut parts = Vec::new();
    let mut rest = toks;
    while let Some(i) = find_top(rest, f) {
        if i == 0 || i + 1 >= rest.len() {
            break;
        }
        parts.push(&rest[..i]);
        rest = &rest[i + 1..];
    }
    parts.push(rest);
    parts
}

/// Split on top-level "and", except the "and" of "between a and b".
fn split_and(toks: &[Tok]) -> Vec<&[Tok]> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut depth = 0;
    let mut between = false;
    for (i, t) in toks.iter().enumerate() {
        if t.kind == K::Sym {
            match t.text.as_str() {
                "(" | "[" | "{" => depth += 1,
                ")" | "]" | "}" => depth -= 1,
                _ => {}
            }
        }
        if depth != 0 {
            continue;
        }
        if t.kind == K::Word && i > 0 && quant_word(&t.lower()).is_some() {
            break; // a quantifier's body extends to the end
        }
        if t.kind == K::Word && t.lower().contains("between") {
            between = true;
            continue;
        }
        if kw(t, &["and", "&&"]) {
            if between {
                between = false;
                continue;
            }
            if i > start && i + 1 < toks.len() {
                parts.push(&toks[start..i]);
                start = i + 1;
            }
        }
    }
    parts.push(&toks[start..]);
    parts
}

fn strip_that(toks: &[Tok]) -> &[Tok] {
    match toks.first() {
        Some(t) if t.is_kw("that") || t.is_sym(",") || t.is_sym(":") => &toks[1..],
        _ => toks,
    }
}

fn kw(t: &Tok, words: &[&str]) -> bool {
    (t.kind == K::Word && words.contains(&t.lower().as_str())) || (t.kind == K::Sym && words.contains(&t.text.as_str()))
}

pub fn parse_clause(toks: &[Tok]) -> Clause {
    let toks = strip_that(toks);
    if toks.is_empty() {
        return Clause::Mixed(Vec::new());
    }
    // a lone explicit $…$
    if toks.len() == 1
        && let K::Math(inner) = &toks[0].kind
        && let Some(m) = explicit(inner)
    {
        return Clause::Math(m);
    }
    if let Some(q) = (toks[0].kind == K::Word).then(|| quant_word(&toks[0].lower())).flatten() {
        let mut rest = &toks[1..];
        if rest.first().is_some_and(|t| t.is_sym(",")) {
            rest = &rest[1..];
        }
        if q == Quant::Exists && rest.first().is_some_and(|t| t.is_sym("!") && !t.sp_before) {
            rest = &rest[1..];
        }
        let qw = toks[0].lower();
        // "there are 10 choose 3 = 120 subsets", "there are only finitely many primes": a count,
        // not a variable binding
        if matches!(qw.as_str(), "there are" | "there is")
            && rest.first().is_some_and(|t| {
                t.kind == K::Num
                    || t.is_sym("(")
                    || matches!(
                        t.lower().as_str(),
                        "only"
                            | "infinitely"
                            | "finitely"
                            | "many"
                            | "several"
                            | "at"
                            | "more"
                            | "fewer"
                            | "no"
                            | "two"
                            | "three"
                            | "four"
                            | "five"
                            | "ten"
                    )
            })
        {
            return Clause::Mixed(segment(toks));
        }
        // "for all but finitely many n", "for all sufficiently large n": idioms, kept as written
        if q == Quant::ForAll
            && rest.first().is_some_and(|t| matches!(t.lower().as_str(), "but" | "sufficiently" | "large" | "small"))
        {
            return Clause::Mixed(segment(toks));
        }
        // binder: up to a separator, or the longest math expression ("for each x P(x)")
        let nested = rest.iter().position(|t| t.kind == K::Word && quant_word(&t.lower()).is_some());
        let sep = binder_sep(rest, q);
        let (vars, body) = match (sep, nested) {
            (_, Some(n)) if n > 0 && sep.is_none_or(|s| n < s) => {
                // "for all eps > 0 exists delta > 0 such that …": the body is the inner quantifier
                let v = &rest[..n];
                let v = if v.last().is_some_and(|t| t.is_sym(",")) { &v[..v.len() - 1] } else { v };
                (v, &rest[n..])
            }
            (Some(i), _) if i > 0 => (&rest[..i], &rest[i + 1..]),
            // "there is a largest prime": "a" is an article, the binder is the whole noun phrase
            _ if is_article_at(rest, 0) => (rest, &rest[rest.len()..]),
            _ => {
                let mut p = Parser::new(rest, 0, false).no_spaced_product();
                match p.expr() {
                    Some(_) if p.i > 0 => (&rest[..p.i], &rest[p.i..]),
                    _ => (rest, &rest[rest.len()..]),
                }
            }
        };
        // "there is a subtle point here, and …": an article phrase that names no variable
        if is_article_at(rest, 0) && !segment(vars).iter().any(|p| matches!(p, Piece::Math(_))) {
            return Clause::Mixed(segment(toks));
        }
        let vars_c = parse_clause(vars);
        // "there is a unique solution": no variable and no condition → just prose
        let starts_math = matches!(&vars_c, Clause::Math(_))
            || matches!(&vars_c, Clause::Mixed(p) if matches!(p.first(), Some(Piece::Math(_))));
        if body.is_empty() && !starts_math {
            return Clause::Mixed(segment(toks));
        }
        let body = if body.is_empty() { None } else { Some(Box::new(parse_clause(body))) };
        return Clause::Quant { q, vars: Box::new(vars_c), body };
    }
    if let Some(i) = find_top(toks, |t| kw(t, &["iff", "if and only if", "<=>", "<->"])) {
        return Clause::Iff(Box::new(parse_clause(&toks[..i])), Box::new(parse_clause(&toks[i + 1..])));
    }
    if toks[0].is_kw("if")
        && let Some(i) = {
            // the comma before the consequence, not one inside a list ("if p_1, ..., p_k were …")
            let then_at = find_top(&toks[1..], |t| t.is_kw("then"));
            let mut depth = 0i32;
            let mut comma_at = None;
            for (k, t) in toks[1..].iter().enumerate() {
                match t.text.as_str() {
                    "(" | "[" | "{" if t.kind == K::Sym => depth += 1,
                    ")" | "]" | "}" if t.kind == K::Sym => depth -= 1,
                    "," if depth == 0 && !continues_binder(&toks[k + 2..]) => {
                        comma_at = Some(k);
                        break;
                    }
                    _ => {}
                }
            }
            match (then_at, comma_at) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            }
        }
    {
        let (c, q) = (&toks[1..i + 1], &toks[i + 2..]);
        let q = if q.first().is_some_and(|t| t.is_kw("then")) { &q[1..] } else { q };
        return Clause::Implies(Box::new(parse_clause(c)), Box::new(parse_clause(q)), true);
    }
    if let Some(i) = find_top(toks, |t| kw(t, &["implies", "=>"]))
        && i > 0
    {
        return Clause::Implies(Box::new(parse_clause(&toks[..i])), Box::new(parse_clause(&toks[i + 1..])), false);
    }
    for (words, ctor) in [(&["xor", "^^"][..], 0u8), (&["nor", "!||"][..], 1), (&["nand", "!&&"][..], 2)] {
        if let Some(i) = find_top(toks, |t| kw(t, words))
            && i > 0
            // "P xor Q holds …": two atoms inside a sentence, not two statements (segment handles it)
            && !(i == 1 && is_bare_var(&toks[..1]) && toks.len() > 3 && is_bare_var(&toks[2..3]))
        {
            let (a, b) = (Box::new(parse_clause(&toks[..i])), Box::new(parse_clause(&toks[i + 1..])));
            return match ctor {
                0 => Clause::Xor(a, b),
                1 => Clause::Nor(a, b),
                _ => Clause::Nand(a, b),
            };
        }
    }
    let ors = split_all(toks, |t| kw(t, &["or", "||"]));
    if ors.len() > 1 {
        return Clause::Or(ors.into_iter().map(parse_clause).collect());
    }
    let ands = split_and(toks);
    if ands.len() > 1 {
        return Clause::And(ands.into_iter().map(parse_clause).collect());
    }
    if toks[0].is_kw("not") || toks[0].is_kw("it is not the case that") {
        let inner = parse_clause(&toks[1..]);
        return match inner {
            Clause::Math(m) => Clause::Math(math::negate(m)),
            other => Clause::Not(Box::new(other)),
        };
    }
    let pieces = segment(toks);
    if pieces.len() == 1
        && let Piece::Math(m) = &pieces[0]
    {
        return Clause::Math(m.clone());
    }
    Clause::Mixed(pieces)
}

/// Parse an explicit `$…$` token list completely.
pub fn explicit(inner: &[Tok]) -> Option<M> {
    let mut p = Parser::new(inner, 0, true);
    let m = p.expr()?;
    if p.i == inner.len() { Some(m) } else { None }
}

/// A `$…$` that doesn't parse as one formula: the parts that parse stay math and only the tokens
/// that don't become `\text{}` (one bad token must not turn a whole formula into plain text).
pub fn explicit_lenient(inner: &[Tok]) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut words: Vec<String> = Vec::new();
    let flush = |words: &mut Vec<String>, out: &mut Vec<String>| {
        if !words.is_empty() {
            out.push(format!("\\text{{{}}}", math::escape_text(&words.join(" "))));
            words.clear();
        }
    };
    let mut i = 0;
    while i < inner.len() {
        let mut p = Parser::new(inner, i, true);
        match p.expr() {
            Some(m) if p.i > i => {
                flush(&mut words, &mut out);
                out.push(math::tex(&m));
                i = p.i;
            }
            _ => {
                words.push(inner[i].text.clone());
                i += 1;
            }
        }
    }
    flush(&mut words, &mut out);
    out.join(" ")
}

/// Prose/math segmentation of a token run (STEX-DESIGN §9).
pub fn segment(toks: &[Tok]) -> Vec<Piece> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < toks.len() {
        let t = &toks[i];
        match &t.kind {
            K::Math(inner) => {
                match explicit(inner) {
                    Some(m) => out.push(Piece::Math(m)),
                    None => out.push(Piece::Tex(format!("${}$", explicit_lenient(inner)))),
                }
                i += 1;
                continue;
            }
            K::Latex(raw) => {
                out.push(Piece::Tex(if raw.starts_with("\\begin{") { raw.clone() } else { format!("${raw}$") }));
                i += 1;
                continue;
            }
            K::Block(style, inner) => {
                // a single word in a style block is prose even if it is one letter (bf{b})
                let body = if inner.len() == 1 && inner[0].kind == K::Word {
                    math::escape_text(&inner[0].text)
                } else {
                    join(&segment(inner))
                };
                out.push(Piece::Tex(styled(*style, &body)));
                i += 1;
                continue;
            }
            K::StyleWord(style) => {
                if let Some(n) = toks.get(i + 1) {
                    let body = join(&segment(std::slice::from_ref(n)));
                    out.push(Piece::Tex(styled(*style, &body)));
                    i += 2;
                } else {
                    out.push(Piece::Prose(t.text.clone()));
                    i += 1;
                }
                continue;
            }
            K::Cap => {
                let mut c = t.text.chars();
                let s = c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default();
                out.push(Piece::Prose(s));
                i += 1;
                continue;
            }
            K::Upper => {
                out.push(Piece::Prose(t.text.to_uppercase()));
                i += 1;
                continue;
            }
            K::Escape => {
                out.push(Piece::Prose(t.text.clone()));
                i += 1;
                continue;
            }
            K::Newline | K::Para => {
                i += 1;
                continue;
            }
            _ => {}
        }
        // "AM-GM", "4-digit", "sigma-algebra", "L-shaped": one hyphenated word, not a subtraction
        if let Some((piece, end)) = hyphen_compound(toks, i) {
            out.push(piece);
            i = end;
            continue;
        }
        // "…, by CRT there is …": a theorem cited mid-sentence reads by its name
        if t.is_kw("by")
            && let Some((id, n)) = theorems::match_at(&toks[i + 1..])
        {
            out.push(Piece::Prose(format!("by {}", theorems::get(id).name)));
            i += 1 + n;
            continue;
        }
        // "the derivative of f with respect to x is …": the noun phrase stays prose
        if t.is_kw("derivative") && i > 0 && kw(&toks[i - 1], &["the", "a", "its"]) {
            out.push(Piece::Prose(t.text.clone()));
            i += 1;
            continue;
        }
        // "P xor Q holds when …": the connective between two atoms is one formula
        if i + 2 < toks.len()
            && let Some(op) = match toks[i + 1].lower().as_str() {
                "xor" | "^^" => Some("\\oplus"),
                "nand" | "!&&" => Some("\\uparrow"),
                "nor" | "!||" => Some("\\downarrow"),
                _ => None,
            }
            && is_bare_var(&toks[i..i + 1])
            && is_bare_var(&toks[i + 2..i + 3])
        {
            out.push(Piece::Math(M::Bin(
                op.into(),
                Box::new(M::Var(t.text.clone())),
                Box::new(M::Var(toks[i + 2].text.clone())),
            )));
            i += 3;
            continue;
        }
        // "a largest prime", "a 2 by 2 board": an article, not a variable named a
        if is_article_at(toks, i) {
            out.push(prose_tok(t));
            i += 1;
            continue;
        }
        // try a math run here
        let mut p = Parser::new(toks, i, false);
        if let Some(m) = p.expr()
            && p.i > i
            && plausible(&m, toks, i, p.i)
        {
            // "functions from A to B", "f maps R to R": `to` is a preposition here, not an arrow
            if i > 0
                && toks[i - 1].kind == K::Word
                && matches!(toks[i - 1].lower().as_str(), "from" | "maps" | "map" | "sends" | "send" | "takes")
                && let M::Chain(first, steps) = &m
                && steps.len() == 1
                && steps[0].op == "\\to"
                && toks[i..p.i].iter().any(|t| t.is_kw("to"))
            {
                let set = |x: &M| match x {
                    M::Var(v) => lexicon::number_set(v).map(|s| M::Sym(s.into())).unwrap_or_else(|| x.clone()),
                    other => other.clone(),
                };
                let rhs_is_set = matches!(&steps[0].rhs, M::Sym(s) if s.starts_with("\\mathbb"));
                out.push(Piece::Math(if rhs_is_set { set(first) } else { (**first).clone() }));
                out.push(Piece::Prose("to".into()));
                out.push(Piece::Math(steps[0].rhs.clone()));
                i = p.i;
                continue;
            }
            // "p, q in Z": a run of bare variables before a relation is one expression
            if (matches!(m, M::Var(_)) || matches!(&m, M::Sub(b, _) if matches!(**b, M::Var(_))))
                && let Some((merged, end)) = var_list(toks, p.i, m.clone())
            {
                out.push(Piece::Math(merged));
                i = end;
                continue;
            } // ordinal suffix glued to the math: "n-th", "(k+1)st"
            let mut j = p.i;
            let mut suffix = String::new();
            if toks.get(j).is_some_and(|t| t.is_sym("-") && !t.sp_before)
                && toks.get(j + 1).is_some_and(|t| !t.sp_before && math::is_ordinal_suffix(&t.text))
            {
                suffix = format!("-{}", toks[j + 1].text);
                j += 2;
            } else if toks.get(j).is_some_and(|t| t.kind == K::Word && !t.sp_before && math::is_ordinal_suffix(&t.text))
            {
                suffix = toks[j].text.clone();
                j += 1;
            }
            if suffix.is_empty() {
                out.push(Piece::Math(m));
            } else {
                out.push(Piece::Tex(format!("${}${suffix}", math::tex(&m))));
            }
            i = j;
            continue;
        }
        // "nth", "kth", "ith": a variable with an ordinal suffix
        if t.kind == K::Word
            && t.text.len() == 3
            && t.text.as_bytes()[0].is_ascii_alphabetic()
            && math::is_ordinal_suffix(&t.text[1..])
            && !matches!(t.lower().as_str(), "and" | "end" | "art" | "est" | "ast" | "ard")
        {
            out.push(Piece::Tex(format!("${}${}", &t.text[..1], &t.text[1..])));
            i += 1;
            continue;
        }
        out.push(prose_tok(t));
        i += 1;
    }
    out
}

/// After a bare variable at `i - 1`: `, y, z in S` → `x, y, z ∈ S`. Only when the list ends in a
/// relation (so prose lists like "a, b and c" are untouched).
fn var_list(toks: &[Tok], mut i: usize, first: M) -> Option<(M, usize)> {
    let is_item =
        |m: &M| matches!(m, M::Var(_)) || matches!(m, M::Sub(b, _) if matches!(**b, M::Var(_))) || math::is_dots(m);
    let mut vars = vec![first];
    loop {
        let more = toks.get(i).is_some_and(|t| t.is_sym(","))
            && toks.get(i + 1).is_some_and(|t| t.kind == K::Word || t.is_sym("..."));
        if !more {
            // "say p_1, p_2, ..., p_k": a list on its own (with an ellipsis or 3+ items)
            if vars.len() >= 3 || vars.len() >= 2 && vars.iter().any(math::is_dots) {
                return Some((M::Seq(vars), i));
            }
            return None;
        }
        let mut p = Parser::new(toks, i + 1, false);
        let Some(m) = p.expr() else { return finish_list(vars, i) };
        match m {
            v if is_item(&v) => {
                vars.push(v);
                i = p.i;
            }
            M::Chain(first, steps) if is_item(&first) => {
                vars.push(*first);
                return Some((M::Chain(Box::new(M::Seq(vars)), steps), p.i));
            }
            _ => return finish_list(vars, i),
        }
    }
}

fn finish_list(vars: Vec<M>, i: usize) -> Option<(M, usize)> {
    if vars.len() >= 3 || vars.len() >= 2 && vars.iter().any(math::is_dots) { Some((M::Seq(vars), i)) } else { None }
}

fn plausible(m: &M, toks: &[Tok], start: usize, end: usize) -> bool {
    match m {
        M::Var(v) => {
            let single = v.chars().count() == 1;
            if !single {
                return true; // glued multi-letter product (xy=1)
            }
            let next_prose = toks.get(end).is_some_and(|n| {
                n.kind == K::Word && !lexicon::is_link_verb(&n.lower()) && lexicon::get().lookup(&n.text).is_none()
            });
            // "a"/"A" before a prose noun is an article, "I" a pronoun, even after "choose"/"with"
            // ("choose a committee"); before a verb or operator they are variables ("let a be …")
            if end == start + 1 && is_article_at(toks, start) {
                return false;
            }
            !(v == "I" && next_prose)
        }
        M::Num(n) => {
            if n.contains('.') {
                return true;
            }
            // "2 steps", "case 3", "lemma 2" stay text; "is 0", "equals 1" are math
            // ("0 or 1 upon division by 4": a preposition after the number is not a counted noun)
            let counts_noun = toks.get(end).is_some_and(|t| {
                t.kind == K::Word
                    && !lexicon::is_link_verb(&t.lower())
                    && !lexicon::is_prose_stop(&t.lower())
                    && !matches!(
                        t.lower().as_str(),
                        "upon"
                            | "when"
                            | "whenever"
                            | "after"
                            | "before"
                            | "unless"
                            | "until"
                            | "as"
                            | "than"
                            | "modulo"
                    )
            });
            let numbered = start > 0
                && matches!(
                    toks[start - 1].lower().as_str(),
                    "case"
                        | "lemma"
                        | "theorem"
                        | "step"
                        | "part"
                        | "chapter"
                        | "section"
                        | "page"
                        | "line"
                        | "claim"
                        | "equation"
                        | "figure"
                        | "table"
                        | "exercise"
                        | "problem"
                        | "question"
                );
            !counts_noun && !numbered
        }
        M::Sym(_)
            if end == start + 1
                && matches!(lexicon::get().lookup(&toks[start].text), Some((lexicon::Kind::Set, _))) =>
        {
            // "x, y be real numbers", "over the reals": a lone set word reads as prose
            false
        }
        M::Sym(s) => {
            // lone constants: only unmistakable ones
            s.starts_with("\\mathbb")
                || s.starts_with("\\mathcal")
                || s.starts_with("\\mathscr")
                || s.starts_with("\\mathfrak")
                || s.starts_with("\\mathtt")
                || matches!(
                    lexicon::get().lookup(&toks[start].text),
                    Some((lexicon::Kind::Greek | lexicon::Kind::Set, _))
                )
                || matches!(s.as_str(), "\\pi" | "\\infty" | "\\varnothing" | "\\aleph_0")
        }
        M::Text(_) => false,
        _ => true,
    }
}

fn prose_tok(t: &Tok) -> Piece {
    match &t.kind {
        K::Sym => match t.text.as_str() {
            "," | ";" | ":" | "." | "?" | "!" | ")" | "]" => Piece::Punct(t.text.clone()),
            "(" | "[" => Piece::Prose(t.text.clone()),
            "&&" => Piece::Prose("and".into()),
            "||" => Piece::Prose("or".into()),
            "=>" => Piece::Prose("implies".into()),
            "<=>" | "<->" => Piece::Prose("if and only if".into()),
            "==" => Piece::Tex("$\\equiv$".into()),
            "=" => Piece::Tex("$=$".into()),
            "<" | ">" => Piece::Tex(format!("${}$", t.text)),
            "<=" => Piece::Tex("$\\leq$".into()),
            ">=" => Piece::Tex("$\\geq$".into()),
            "!=" => Piece::Tex("$\\neq$".into()),
            other => Piece::Prose(math::escape_text(other)),
        },
        K::Num => Piece::Prose(t.text.clone()),
        _ => {
            let canon = match t.lower().as_str() {
                "s.t." | "st" => "such that".to_string(),
                "w/" => "with".into(),
                "w/o" => "without".into(),
                "b/c" => "because".into(),
                "wrt" => "with respect to".into(),
                "wlog" => "without loss of generality".into(),
                "iff" => "if and only if".into(),
                "i.e." | "ie" => "that is,".into(),
                _ => math::escape_text(&t.text),
            };
            Piece::Prose(canon)
        }
    }
}

fn styled(s: Style, body: &str) -> String {
    match s {
        Style::Bold => format!("\\textbf{{{body}}}"),
        Style::Italic => format!("\\textit{{{body}}}"),
        Style::Underline => format!("\\underline{{{body}}}"),
    }
}

/// Join pieces with spaces, no space before closing punctuation or after an opening bracket.
pub fn join(pieces: &[Piece]) -> String {
    let mut s = String::new();
    let mut prev_open = true;
    for p in pieces {
        let (text, punct) = match p {
            Piece::Prose(t) => (t.clone(), false),
            Piece::Math(m) => (format!("${}$", math::tex(m)), false),
            Piece::Tex(t) => (t.clone(), false),
            Piece::Punct(t) => (t.clone(), true),
        };
        if !punct && !prev_open && !s.is_empty() {
            s.push(' ');
        }
        prev_open = matches!(p, Piece::Prose(t) if t == "(" || t == "[");
        s.push_str(&text);
    }
    s
}

// ------------------------------------------------------------------------------------ facts (Lean)

#[derive(Clone, Debug)]
pub enum Fact {
    Decl {
        var: String,
        ty: Option<String>,
        line: u32,
    },
    Hyp {
        m: M,
        line: u32,
    },
    Goal {
        m: M,
        line: u32,
    },
    Have {
        m: M,
        line: u32,
        thm: Option<usize>,
    },
    Afsoc {
        m: M,
        line: u32,
    },
    Contra {
        line: u32,
    },
    Qed {
        m: Option<M>,
        line: u32,
    },
    Def {
        lhs: M,
        rhs: M,
        sig: Option<M>,
        line: u32,
    },
    /// `choose x = v`: x is introduced with value v (a witness when the goal is ∃ x).
    Witness {
        var: String,
        value: M,
        line: u32,
    },
    /// Start of one `case` (its condition, if any) at nesting `depth` (1 = top level); the steps
    /// that follow belong to it.
    Case {
        cond: Option<M>,
        line: u32,
        depth: usize,
    },
    /// The case analysis at `depth` ended (the next fact, if a `Have`, is what the cases prove).
    EndCases {
        line: u32,
        depth: usize,
    },
    /// "We proceed by induction on n" (`strong` for strong/complete induction).
    Induction {
        var: String,
        strong: bool,
        line: u32,
    },
    /// The base case (its condition, e.g. `n = 0`); following facts belong to it.
    Base {
        cond: Option<M>,
        line: u32,
    },
    /// The inductive step; `var` is the step variable the proof uses (k in "assume P(k)").
    Step {
        var: Option<String>,
        line: u32,
    },
    Skip {
        line: u32,
        text: String,
    },
}

/// Predicates written in words: "is even", "is not prime", "is irrational" … (Lean only).
const PREDICATES: &[&str] = &[
    "even",
    "odd",
    "prime",
    "positive",
    "negative",
    "nonnegative",
    "non-negative",
    "nonpositive",
    "non-positive",
    "nonzero",
    "non-zero",
    "zero",
    "irrational",
    "rational",
    "square",
    "perfect square",
    "composite",
];

fn word_pred(subject: &M, words: &[String]) -> Option<M> {
    let mut w: Vec<&str> = words.iter().map(String::as_str).filter(|w| !matches!(*w, "a" | "an")).collect();
    let neg = w.first() == Some(&"not");
    if neg {
        w.remove(0);
    }
    let pred = w.join(" ");
    let pred = pred.trim_end_matches(" number").trim_end_matches(" integer").to_string();
    if !PREDICATES.contains(&pred.as_str()) {
        return None;
    }
    let m = M::Pred(pred, Box::new(subject.clone()));
    Some(if neg { M::Not(Box::new(m)) } else { m })
}

/// "x, y be real numbers", "p be a prime", "n be a positive integer" → (vars, type, predicates).
fn typed_decl(c: &Clause) -> Option<(Vec<String>, &'static str, Vec<&'static str>)> {
    let Clause::Mixed(p) = c else { return None };
    let Some(Piece::Math(subject)) = p.first() else { return None };
    let vars = vars_of(subject);
    if vars.is_empty() || !p[1..].iter().all(|x| matches!(x, Piece::Prose(_))) {
        return None;
    }
    let words: Vec<String> = p[1..]
        .iter()
        .filter_map(|x| match x {
            Piece::Prose(t) => Some(t.to_lowercase()),
            _ => None,
        })
        .flat_map(|t| t.split(' ').map(String::from).collect::<Vec<_>>())
        .collect();
    if !matches!(words.first().map(String::as_str), Some("be" | "is" | "are")) {
        return None;
    }
    let mut preds = Vec::new();
    let mut ty = None;
    for w in &words[1..] {
        match w.as_str() {
            "a" | "an" | "two" | "some" | "arbitrary" | "distinct" | "given" | "fixed" | "number" | "numbers" => {}
            "integer" | "integers" => ty = Some("\\mathbb{Z}"),
            "natural" | "naturals" => ty = Some("\\mathbb{N}"),
            "real" | "reals" => ty = Some("\\mathbb{R}"),
            "rational" | "rationals" => ty = Some("\\mathbb{Q}"),
            "complex" => ty = Some("\\mathbb{C}"),
            "prime" | "primes" => {
                ty = ty.or(Some("\\mathbb{N}"));
                preds.push("prime");
            }
            "positive" => preds.push("positive"),
            "negative" => preds.push("negative"),
            "nonnegative" | "non-negative" => preds.push("nonnegative"),
            "nonzero" | "non-zero" => preds.push("nonzero"),
            "even" => preds.push("even"),
            "odd" => preds.push("odd"),
            _ => return None,
        }
    }
    Some((vars, ty?, preds))
}

/// First math piece of a clause (for variable discovery in prose-heavy statements).
fn first_math(c: &Clause) -> Option<M> {
    match c {
        Clause::Mixed(p) => p.iter().find_map(|x| match x {
            Piece::Math(m) => Some(m.clone()),
            _ => None,
        }),
        _ => None,
    }
}

fn collect_vars(m: &M, out: &mut Vec<String>) {
    match m {
        M::Var(v) => {
            if !out.contains(v) {
                out.push(v.clone());
            }
        }
        M::Bin(_, a, b)
        | M::Implicit(a, b)
        | M::Frac(a, b)
        | M::Pow(a, b)
        | M::Sub(a, b)
        | M::Binom(a, b)
        | M::Mod(a, b) => {
            collect_vars(a, out);
            collect_vars(b, out);
        }
        M::Chain(a, steps) => {
            collect_vars(a, out);
            for s in steps {
                collect_vars(&s.rhs, out);
            }
        }
        M::Call(_, args, _) | M::Seq(args) | M::Set(args) => {
            for a in args {
                collect_vars(a, out);
            }
        }
        M::Neg(a) | M::Group(a, _) | M::Not(a) | M::Pred(_, a) | M::Post(_, a) | M::Abs(a) | M::Func(_, a) => {
            collect_vars(a, out)
        }
        M::Big { lower, upper, body, .. } => {
            // the index (`i` in "sum from i=1 to k") is bound: not a free variable
            let bound = match lower.as_deref() {
                Some(M::Chain(v, _)) => vars_of(v),
                _ => Vec::new(),
            };
            let mut inner = Vec::new();
            for x in [lower, upper, body].into_iter().flatten() {
                collect_vars(x, &mut inner);
            }
            for v in inner {
                if !bound.contains(&v) && !out.contains(&v) {
                    out.push(v);
                }
            }
        }
        _ => {}
    }
}

/// A claim as prose: predicates read as words ("$n$ is odd"), everything else as math.
fn claim_text(m: &M) -> String {
    match m {
        M::Pred(p, a) => format!("${}$ is {p}", math::tex(a)),
        M::Bin(op, a, b)
            if (op == "\\lor" || op == "\\land") && (matches!(**a, M::Pred(..)) || matches!(**b, M::Pred(..))) =>
        {
            let word = if op == "\\lor" { "or" } else { "and" };
            format!("{} {word} {}", claim_text(a), claim_text(b))
        }
        // a negated ∃ goal: "for every $n \in \mathbb{N}$, $n^2 \leq 50$", not a ∀ formula
        M::Quant { q, vars, body: Some(b) } if q == "\\forall" || q == "\\exists" => {
            if q == "\\forall" {
                format!("for every ${}$, {}", math::tex(vars), claim_text(b))
            } else {
                format!("there exists ${}$ such that {}", math::tex(vars), claim_text(b))
            }
        }
        _ => format!("${}$", math::tex(m)),
    }
}

/// Equations asserted inside a prose clause whose variables are all declared: math pieces of
/// mixed prose, conjuncts, and the count of "there are C(5,2) = 10 ways …".
fn closed_chains(c: &Clause, declared: &[String], out: &mut Vec<M>) {
    fn take(m: &M, declared: &[String], out: &mut Vec<M>) {
        if let M::Chain(_, steps) = m
            && steps.iter().all(|s| !matches!(s.op.as_str(), ":" | ":=" | "\\to" | "\\mapsto"))
        {
            let mut vars = Vec::new();
            collect_vars(m, &mut vars);
            if vars.iter().all(|v| declared.contains(v)) {
                out.push(m.clone());
            }
        }
    }
    match c {
        Clause::Mixed(pieces) => {
            for p in pieces {
                if let Piece::Math(m) = p {
                    take(m, declared, out);
                }
            }
        }
        Clause::Quant { q: Quant::Exists, vars, body } => {
            if let Clause::Math(m) = &**vars {
                take(m, declared, out);
            } else {
                closed_chains(vars, declared, out);
            }
            if let Some(b) = body
                && matches!(&**b, Clause::Mixed(_))
            {
                closed_chains(b, declared, out);
            }
        }
        Clause::And(parts) => {
            for p in parts {
                match p {
                    Clause::Math(m) => take(m, declared, out),
                    other => closed_chains(other, declared, out),
                }
            }
        }
        _ => {}
    }
}

/// "we proceed by induction on n", "by strong induction on k", "induction on n", "we induct on n".
fn induction_spec(toks: &[Tok]) -> Option<(String, bool)> {
    let words: Vec<String> = toks.iter().map(|t| t.lower()).collect();
    let at = words.iter().position(|w| w == "induction" || w == "induct")?;
    if at > 6 || words.get(at + 1).map(String::as_str) != Some("on") {
        return None;
    }
    let var = toks.get(at + 2)?;
    if var.kind != K::Word || toks.len() > at + 3 {
        return None;
    }
    let lead_ok = words[..at].iter().all(|w| {
        matches!(
            w.as_str(),
            "we" | "proceed"
                | "proceeding"
                | "by"
                | "use"
                | "using"
                | "argue"
                | "will"
                | "prove"
                | "this"
                | "it"
                | "strong"
                | "complete"
                | "mathematical"
                | "the"
                | "claim"
                | "statement"
                | "proof"
        )
    });
    if !lead_ok {
        return None;
    }
    let strong = words[..at].iter().any(|w| w == "strong" || w == "complete");
    Some((var.text.clone(), strong))
}

pub fn clause_math(c: &Clause) -> Option<M> {
    // "a is divisible by b" (prose in LaTeX) → b ∣ a for Lean
    if let Clause::Mixed(p) = c
        && let [Piece::Math(a), Piece::Prose(w), Piece::Math(b)] = &p[..]
        && matches!(w.to_lowercase().as_str(), "is divisible by" | "divisible by")
    {
        let a = match a {
            M::Chain(_, steps) => steps.last().map(|s| s.rhs.clone())?,
            other => other.clone(),
        };
        return Some(M::Chain(Box::new(b.clone()), vec![math::Step { op: "\\mid".into(), rhs: a, tag: None, end: 0 }]));
    }
    // "n is even", "p is not prime", "x is a positive integer" → predicates for Lean
    if let Clause::Mixed(p) = c
        && let Some(Piece::Math(subject)) = p.first()
        && p.len() >= 3
        && matches!(p.get(1), Some(Piece::Prose(w)) if w == "is")
        && p[1..].iter().all(|x| matches!(x, Piece::Prose(_)))
    {
        let words: Vec<String> = p[2..]
            .iter()
            .filter_map(|x| match x {
                Piece::Prose(t) => Some(t.to_lowercase()),
                _ => None,
            })
            .flat_map(|t| t.split(' ').map(String::from).collect::<Vec<_>>())
            .collect();
        return match subject {
            M::Chain(_, steps) if !steps.is_empty() => {
                let last = steps.last().map(|s| s.rhs.clone())?;
                let pred = word_pred(&last, &words)?;
                Some(M::Bin("\\land".into(), Box::new(subject.clone()), Box::new(pred)))
            }
            _ => word_pred(subject, &words),
        };
    }
    Some(match c {
        Clause::Math(m) => m.clone(),
        Clause::And(v) => fold(v, "\\land")?,
        Clause::Or(v) => fold(v, "\\lor")?,
        Clause::Implies(a, b, _) => chain(clause_math(a)?, "\\implies", clause_math(b)?),
        Clause::Iff(a, b) => chain(clause_math(a)?, "\\iff", clause_math(b)?),
        Clause::Not(a) => math::negate(clause_math(a)?),
        Clause::Quant { q, vars, body } => M::Quant {
            q: match q {
                Quant::Exists => "\\exists",
                Quant::Unique => "\\exists!",
                Quant::None => "\\nexists",
                Quant::ForAll => "\\forall",
            }
            .into(),
            vars: Box::new(clause_math(vars)?),
            body: match body {
                Some(b) => Some(Box::new(clause_math(b)?)),
                None => None,
            },
        },
        _ => return None,
    })
}

fn fold(v: &[Clause], op: &str) -> Option<M> {
    let mut it = v.iter();
    let mut acc = clause_math(it.next()?)?;
    for c in it {
        acc = M::Bin(op.into(), Box::new(acc), Box::new(clause_math(c)?));
    }
    Some(acc)
}

fn chain(a: M, op: &str, b: M) -> M {
    M::Chain(Box::new(a), vec![Step { op: op.into(), rhs: b, tag: None, end: 0 }])
}

// ------------------------------------------------------------------------------------ realizer

/// One unit of output inside a paragraph.
#[derive(Clone, Debug, PartialEq)]
pub enum Out {
    Sentence(String),
    Display(String),
    /// A nested list (sub-cases, steps inside a case …): items of outputs.
    List(Vec<Vec<Out>>),
    /// A paragraph break inside a list item (a blank line within one case).
    Break,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    Para(Vec<Out>),
    /// `case` items: each is a list of outputs.
    Items(Vec<Vec<Out>>),
}

enum Just {
    Thm(usize),
    Text(Vec<Piece>),
    Reason(Vec<Tok>, String),
}

pub struct Realizer<'p> {
    pick: &'p mut Picker,
    goals: Vec<(String, Option<M>)>,
    afsoc: Option<String>,
    pending_label: Option<String>,
    /// Rendering the top-level clause of a sentence (quantifiers may use sentence-initial forms
    /// like "We have P for some x"); embedded clauses use the `.inner` roles.
    top: bool,
    /// A claim-like label ended its line ("lemma 2."): the next statement is its claim.
    claim_next: bool,
    pub facts: Vec<Fact>,
    pub theorems: Vec<usize>,
    pub blocks: Vec<Block>,
    cur: Vec<Out>,
    /// Open lists (cases, steps), innermost last.
    lists: Vec<Level>,
    /// Inside an induction proof: which part we are in, and the induction variable.
    induction: Option<(IndPhase, String)>,
    /// Index of the current inductive step's `Step` fact (to record its variable).
    step_fact: Option<usize>,
    line: u32,
    /// All top-level statements and the index of the current one (for look-ahead: does a blank
    /// line end a case, which cases follow an "exhaustive" opener).
    all: Vec<Stmt>,
    pos: usize,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum ListKind {
    Cases,
    Steps,
}

struct Level {
    kind: ListKind,
    depth: usize,
    items: Vec<Vec<Out>>,
    /// Per item: its label ("1", "2a", auto-numbered when the user gave none).
    labels: Vec<String>,
    /// A list of `subcase`s.
    sub: bool,
}

impl Level {
    fn new(kind: ListKind, depth: usize) -> Level {
        Level { kind, depth, items: Vec::new(), labels: Vec::new(), sub: false }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum IndPhase {
    Intro,
    Base,
    Step,
}

/// Words that continue an expression or a clause: after them a would-be case label is part of
/// the condition ("case k is odd", "case n even", "case x in A").
fn continues_condition(t: &Tok) -> bool {
    match t.kind {
        K::Sym => !matches!(t.text.as_str(), "(" | "[" | ":" | "." | "," | ")"),
        K::Word => {
            let w = t.lower();
            w.chars().count() > 1 || matches!(w.as_str(), "o")
        }
        _ => false,
    }
}

/// The explicit label after `case`, if the user wrote one, and where the condition starts.
/// A label is a number ("1", "2a", "1.2"), a parenthesized tag ("(i)"), or a short name that is
/// clearly not the start of the condition: "case k: …", "case k n = 1" (a new expression follows),
/// but not "case k is odd" / "case n = 0" / "case n even" (k and n are the condition's subject).
fn case_label(toks: &[Tok]) -> (Option<String>, usize) {
    let Some(first) = toks.first() else { return (None, 0) };
    // "(i)", "(a)", "(2)"
    if first.is_sym("(")
        && let (Some(x), Some(close)) = (toks.get(1), toks.get(2))
        && close.is_sym(")")
        && matches!(x.kind, K::Num | K::Word)
        && x.text.chars().count() <= 4
        && toks.get(3).is_none_or(|t| t.sp_before || kw(t, &[":", ".", ","]))
    {
        return (Some(format!("({})", x.text)), 3);
    }
    if !matches!(first.kind, K::Num | K::Word) {
        return (None, 0);
    }
    let mut label = first.text.clone();
    let mut i = 1;
    while let Some(n) = toks.get(i) {
        if !n.sp_before && matches!(n.kind, K::Word | K::Num) {
            label.push_str(&n.text);
            i += 1;
        } else {
            break;
        }
    }
    let next = toks.get(i);
    // "case 1", "case k:", "case 2a." / "case 1 if …"
    if next.is_none_or(|t| kw(t, &[":", ".", ",", "if", "suppose", "assume", "when", "where"])) {
        let word = first.kind == K::Word;
        // a lone long word is a label only before ':' ("case one: …"), never a keyword phrase
        if word && label.chars().count() > 3 && !next.is_some_and(|t| t.is_sym(":")) {
            return (None, 0);
        }
        return (Some(label), i);
    }
    let next = next.expect("checked");
    let numeric = first.kind == K::Num && label.starts_with(|c: char| c.is_ascii_digit());
    if numeric {
        // "case 1 n = 0", "case 2 both are odd" — but "case 0 < x", "case 2x = 4", "case 3 | n"
        if continues_condition(next) && !(next.kind == K::Word && !is_verbish(next)) {
            return (None, 0);
        }
        return (Some(label), i);
    }
    // a short name: "case k n = 1", "case i x < 0"
    let short = label.chars().count() == 1 || is_roman(&label);
    let starts_expr = match next.kind {
        K::Math(_) | K::Sigil(_) | K::Latex(_) | K::Num => true,
        K::Word => next.text.chars().count() == 1 && next.text.chars().all(|c| c.is_ascii_alphabetic()),
        K::Sym => next.is_sym("(") && next.sp_before,
        _ => false,
    };
    if short && starts_expr {
        return (Some(label), i);
    }
    (None, 0)
}

/// Words that make "case 2 is …" a condition about the number 2 rather than a label.
fn is_verbish(t: &Tok) -> bool {
    matches!(
        t.lower().as_str(),
        "is" | "are"
            | "divides"
            | "does"
            | "do"
            | "has"
            | "have"
            | "and"
            | "or"
            | "in"
            | "is in"
            | "is not in"
            | "mod"
            | "less than"
            | "greater than"
            | "equal to"
            | "is less than"
            | "is greater than"
            | "is equal to"
            | "is at most"
            | "is at least"
            | "is divisible by"
            | "belongs to"
            | "times"
            | "plus"
            | "minus"
    )
}

fn is_roman(s: &str) -> bool {
    matches!(s, "i" | "ii" | "iii" | "iv" | "v" | "vi" | "vii" | "viii" | "ix" | "x")
}

/// `case …` after the keyword: (explicit label, condition, body after a top-level ':').
fn case_parts(toks: &[Tok]) -> (Option<String>, &[Tok], &[Tok]) {
    let (label, i) = case_label(toks);
    let mut cond = &toks[i..];
    while cond.first().is_some_and(|t| kw(t, &[":", ".", ",", "if", "suppose", "assume", "when", "where"])) {
        cond = &cond[1..];
    }
    // "case 1 n = 0: then …": the part after a top-level ':' is the case body
    let mut body: &[Tok] = &[];
    if let Some(c) = find_top(cond, |t| t.is_sym(":"))
        && c + 1 < cond.len()
    {
        body = &cond[c + 1..];
        cond = &cond[..c];
    }
    while cond.last().is_some_and(|t| t.is_sym(":") || t.is_sym(".")) {
        cond = &cond[..cond.len() - 1];
    }
    (label, cond, body)
}

/// The first label of a list ("1", "a", "i", "(a)", "1a"'s parent restarting): a case analysis
/// starting over.
fn is_first_label(l: &str) -> bool {
    let l = l.trim_start_matches('(').trim_end_matches([')', '.']);
    matches!(l, "1" | "a" | "A" | "i" | "I" | "one")
}

/// "cases exhaustive", "exhaustive", "the cases are exhaustive" …: before the cases (an opener)
/// or right after them.
const EXH_WORDS: &[&str] = &[
    "exhaustive",
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
];

/// "case on k": announces a case analysis on k.
const CASE_ON: &[&str] = &[
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
];

/// The case-analysis keyword families: "exhaustive" and "case on" (for highlighting).
pub fn is_exhaustive_keyword(w: &str) -> bool {
    EXH_WORDS.contains(&w) || EXH_CLOSERS.contains(&w) || CASE_ON.contains(&w)
}

fn is_exh_opener_word(w: &str) -> bool {
    EXH_WORDS.contains(&w)
}

/// "since k is odd or even, exhaustive cases": the reason comes first. Returns the index of the
/// "exhaustive" keyword.
fn lead_reason_exh(toks: &[Tok]) -> Option<usize> {
    if !toks.first().is_some_and(|t| kw(t, &["b/c", "because", "since", "as"])) {
        return None;
    }
    let c = find_top(&toks[1..], |t| t.is_sym(","))? + 1;
    toks.get(c + 1).filter(|t| t.kind == K::Word && is_exh_opener_word(&t.lower())).map(|_| c + 1)
}

/// Closing transitions that state what holds in every case: "in all exhaustive cases, P",
/// "since the cases are exhaustive, P".
const EXH_CLOSERS: &[&str] = &[
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
];

const CASE_CLOSERS: &[&str] =
    &["in either case", "in both cases", "in all cases", "in every case", "in any case", "either way"];

#[derive(Clone, Copy, PartialEq, Debug)]
enum ParaFate {
    /// A new paragraph inside the current list item.
    Stay,
    /// Ends the innermost case analysis; continues the enclosing item (a step).
    LeaveCases,
    /// Ends every open list.
    Close,
}

/// The user's justification for exhaustiveness: "b/c R" (the word kept for --fixed; `true` when it
/// came before the keyword) or "by T".
enum Why {
    Because(String, &'static str, bool),
    By(String),
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum ExhRole {
    /// Before a case analysis.
    Open,
    /// Before the sub-cases of the current case.
    Nested,
    /// After the cases.
    End,
}

/// What a top-level statement is, for deciding where a blank line ends a case.
#[derive(Clone, Copy, PartialEq, Debug)]
enum SKind {
    Case {
        sub: bool,
    },
    Step,
    /// "in either case", "in total" … and the "exhaustive" family.
    Closer,
    Exh,
    /// "case on k": starts a case analysis.
    CaseOn,
    /// Ends any case analysis: qed, a label, a new goal.
    End,
    Other,
}

fn stmt_kind(toks: &[Tok]) -> SKind {
    let Some(f) = toks.first() else { return SKind::Other };
    if toks.len() > 1 && toks.last().is_some_and(|t| t.is_kw("qed")) {
        return SKind::End;
    }
    if f.kind != K::Word {
        return SKind::Other;
    }
    if lead_reason_exh(toks).is_some() {
        return SKind::Exh;
    }
    let w = f.lower();
    match w.as_str() {
        _ if CASE_ON.contains(&w.as_str()) => SKind::CaseOn,
        "case" => SKind::Case { sub: false },
        "subcase" | "sub-case" => SKind::Case { sub: true },
        "step" | "stage" => SKind::Step,
        "in total" | "altogether" | "overall" | "in all" | "all in all" => SKind::Closer,
        "qed" | "proof" | "claim" | "lemma" | "proposition" | "theorem" | "corollary" | "observation" | "remark"
        | "fact" | "sublemma" | "aside" | "base case" | "inductive step" | "induction step" | "definition"
        | "example" | "wts" | "want to show" | "we want to show" | "we need to show" => SKind::End,
        _ if CASE_CLOSERS.contains(&w.as_str()) || EXH_CLOSERS.contains(&w.as_str()) => SKind::Closer,
        _ if EXH_WORDS.contains(&w.as_str()) => SKind::Exh,
        _ => SKind::Other,
    }
}

/// Nesting depth of a case/step label: "1" → 1, "1a" / "1.2" → 2, "1.2.3" / "1a.i" → 3.
fn label_depth(label: &str) -> usize {
    let mut n = 0;
    let mut prev: Option<bool> = None;
    for c in label.chars() {
        if c == '.' || c == '(' || c == ')' {
            prev = None;
            continue;
        }
        let digit = c.is_ascii_digit();
        if prev != Some(digit) {
            n += 1;
        }
        prev = Some(digit);
    }
    n.max(1)
}

fn capitalize(s: &str) -> String {
    // skip leading LaTeX commands like \textbf{ ; never touch math
    let mut out = String::with_capacity(s.len());
    let mut done = false;
    let mut in_cmd = false;
    for c in s.chars() {
        if !done {
            if c == '$' {
                done = true;
            } else if c == '\\' {
                in_cmd = true;
            } else if in_cmd && (c == '{' || c == ' ') {
                in_cmd = false;
            } else if !in_cmd && c.is_alphabetic() {
                out.extend(c.to_uppercase());
                done = true;
                continue;
            }
        }
        out.push(c);
    }
    out
}

fn is_prop(c: &Clause) -> bool {
    match c {
        Clause::Math(M::Chain(..) | M::Quant { .. } | M::Not(_)) => true,
        Clause::Quant { .. } => true,
        Clause::Not(a) => is_prop(a),
        Clause::And(v) | Clause::Or(v) => v.iter().all(is_prop),
        _ => false,
    }
}

fn list_join(items: &[String], conj: &str) -> String {
    match items.len() {
        0 => String::new(),
        1 => items[0].clone(),
        2 => format!("{} {conj} {}", items[0], items[1]),
        n => format!("{}, {conj} {}", items[..n - 1].join(", "), items[n - 1]),
    }
}

impl<'p> Realizer<'p> {
    pub fn new(pick: &'p mut Picker) -> Realizer<'p> {
        Realizer {
            pick,
            goals: Vec::new(),
            afsoc: None,
            pending_label: None,
            top: false,
            claim_next: false,
            facts: Vec::new(),
            theorems: Vec::new(),
            blocks: Vec::new(),
            cur: Vec::new(),
            lists: Vec::new(),
            induction: None,
            step_fact: None,
            line: 0,
            all: Vec::new(),
            pos: 0,
        }
    }

    /// Realize every statement in order (with look-ahead available).
    pub fn run(&mut self, stmts: &[Stmt]) {
        self.all = stmts.to_vec();
        for (i, st) in stmts.iter().enumerate() {
            self.pos = i;
            self.statement(st);
        }
    }

    fn fixed(&self) -> bool {
        self.pick.variance == Variance::Fixed
    }

    fn phrase(&mut self, role: &'static str, slots: &[(char, &str)]) -> String {
        let avail: Vec<char> = slots.iter().map(|(c, _)| *c).collect();
        let t = self.pick.pick(role, &avail);
        fill(t, slots)
    }

    pub fn clause(&mut self, c: &Clause) -> String {
        let top = std::mem::replace(&mut self.top, false);
        match c {
            Clause::Math(m) => format!("${}$", math::tex(m)),
            Clause::Mixed(p) => join(p),
            Clause::And(v) => {
                let parts: Vec<String> = v.iter().map(|c| self.clause(c)).collect();
                if parts.len() == 2 && v.iter().all(is_prop) {
                    self.phrase("and", &[('P', &parts[0]), ('Q', &parts[1])])
                } else if v.iter().all(is_prop) && !parts.iter().any(|p| p.ends_with(',')) {
                    list_join(&parts, "and")
                } else {
                    // prose parts keep the user's own "and"s ("p, and p | n since p | a and a | n")
                    parts.join(" and ")
                }
            }
            Clause::Or(v) => {
                let parts: Vec<String> = v.iter().map(|c| self.clause(c)).collect();
                if parts.len() == 2 && v.iter().all(is_prop) {
                    self.phrase("or", &[('P', &parts[0]), ('Q', &parts[1])])
                } else {
                    list_join(&parts, "or")
                }
            }
            Clause::Xor(a, b) => self.pair("xor", a, b),
            Clause::Nand(a, b) => self.pair("nand", a, b),
            Clause::Nor(a, b) => self.pair("nor", a, b),
            Clause::Implies(a, b, if_form) => {
                if is_prop(a) && is_prop(b) {
                    self.pair(if *if_form { "implies.if" } else { "implies" }, a, b)
                } else {
                    let (p, q) = (self.clause(a), self.clause(b));
                    if *if_form { format!("if {p}, then {q}") } else { format!("{p} implies {q}") }
                }
            }
            Clause::Iff(a, b) => {
                if is_prop(a) && is_prop(b) {
                    self.pair("iff", a, b)
                } else {
                    let (p, q) = (self.clause(a), self.clause(b));
                    format!("{p} if and only if {q}")
                }
            }
            Clause::Not(a) => {
                let p = self.clause(a);
                if is_prop(a) { self.phrase("not", &[('P', &p)]) } else { format!("not {p}") }
            }
            Clause::Quant { q, vars, body } => {
                let x = self.clause(vars);
                match (q, body) {
                    (Quant::ForAll, Some(b)) => {
                        let p = self.clause(b);
                        let s = self.phrase(if top { "forall" } else { "forall.inner" }, &[('X', &x), ('P', &p)]);
                        if plural_binder(&x) {
                            s.replacen("for every ", "for all ", 1)
                                .replacen("For every ", "For all ", 1)
                                .replacen("for each ", "for all ", 1)
                                .replacen("For each ", "For all ", 1)
                        } else {
                            s
                        }
                    }
                    (Quant::ForAll, None) => format!("for every {x}"),
                    (Quant::Exists, Some(b)) => {
                        let p = self.clause(b);
                        // "there is some m with …" → "there exists m such that …"
                        let x = x.strip_prefix("some ").unwrap_or(&x).to_string();
                        let s = self.phrase(if top { "exists" } else { "exists.inner" }, &[('X', &x), ('P', &p)]);
                        agree(s, plural_binder(&x))
                    }
                    (Quant::Exists, None) => {
                        let s = self.phrase("exists.bare", &[('X', &x)]);
                        agree(s, plural_binder(&x))
                    }
                    (Quant::Unique, Some(b)) => {
                        let p = self.clause(b);
                        self.phrase("exists.unique", &[('X', &x), ('P', &p)])
                    }
                    (Quant::Unique, None) => self.phrase("exists.unique.bare", &[('X', &x)]),
                    (Quant::None, Some(b)) => {
                        let p = self.clause(b);
                        // "there does not exist an integer n" → "there is no integer n"
                        let x = x.strip_prefix("an ").or_else(|| x.strip_prefix("a ")).unwrap_or(&x).to_string();
                        let s = self.phrase("exists.none", &[('X', &x), ('P', &p)]);
                        agree(s, plural_binder(&x))
                    }
                    (Quant::None, None) => {
                        let s = self.phrase("exists.none.bare", &[('X', &x)]);
                        agree(s, plural_binder(&x))
                    }
                }
            }
        }
    }

    fn pair(&mut self, role: &'static str, a: &Clause, b: &Clause) -> String {
        let (p, q) = (self.clause(a), self.clause(b));
        self.phrase(role, &[('P', &p), ('Q', &q)])
    }

    // -------------------------------------------------------------- output plumbing

    fn emit(&mut self, s: String, term: &Option<String>) {
        let mut s = capitalize(s.trim());
        if let Some(l) = self.pending_label.take() {
            s = format!("{l} {s}");
        }
        let end = s.trim_end().chars().last();
        if !matches!(end, Some('.' | '?' | '!' | ':')) {
            s.push_str(term.as_deref().unwrap_or("."));
        }
        self.sink().push(Out::Sentence(s));
    }

    /// Where output goes: the current list item, or the paragraph.
    fn sink(&mut self) -> &mut Vec<Out> {
        if self.lists.last().is_some_and(|l| !l.items.is_empty()) {
            return self.lists.last_mut().and_then(|l| l.items.last_mut()).expect("checked");
        }
        &mut self.cur
    }

    /// Start a new item of a `kind` list at label depth `depth`, closing or opening levels as needed.
    fn open_item(&mut self, kind: ListKind, depth: usize) {
        loop {
            let (k, d) = match self.lists.last() {
                Some(l) => (l.kind, l.depth),
                None => {
                    // "proof" on its own line before "case 1 …": the label stays above the list
                    if let Some(l) = self.pending_label.take() {
                        self.cur.push(Out::Sentence(l));
                    }
                    if !self.cur.is_empty() {
                        self.blocks.push(Block::Para(std::mem::take(&mut self.cur)));
                    }
                    self.push_level(kind, depth);
                    break;
                }
            };
            if k == kind && d == depth {
                break; // a sibling
            }
            let enclosing = self.lists.iter().any(|l| l.kind == kind && l.depth >= depth);
            if (k == kind && d > depth) || (k != kind && enclosing) {
                self.close_level();
                continue;
            }
            // deeper than anything open (a sub-case), or a different kind of list inside an item
            self.push_level(kind, depth);
            break;
        }
        let l = self.lists.last_mut().expect("just ensured");
        l.items.push(Vec::new());
        l.labels.push(String::new());
    }

    fn push_level(&mut self, kind: ListKind, depth: usize) {
        self.lists.push(Level::new(kind, depth));
    }

    /// Where a statement that follows a blank line goes while a list is open (see `statement`).
    fn para_fate(&self, st: &Stmt) -> ParaFate {
        let innermost_cases = self.lists.last().is_some_and(|l| l.kind == ListKind::Cases);
        let steps_outside = self.lists.iter().any(|l| l.kind == ListKind::Steps);
        match stmt_kind(&st.toks) {
            SKind::Case { .. } => {
                if self.restarts(&st.toks) {
                    ParaFate::Close
                } else {
                    ParaFate::Stay
                }
            }
            SKind::Step | SKind::Closer => ParaFate::Stay,
            SKind::Exh => match self.exh_role(self.pos) {
                ExhRole::Open => ParaFate::Close,
                ExhRole::Nested | ExhRole::End => ParaFate::Stay,
            },
            SKind::CaseOn => match self.case_on_role(self.pos) {
                ExhRole::Nested => ParaFate::Stay,
                _ => ParaFate::Close,
            },
            SKind::End => ParaFate::Close,
            SKind::Other => {
                for (j, s) in self.all.iter().enumerate().skip(self.pos + 1) {
                    match stmt_kind(&s.toks) {
                        SKind::Case { .. } => {
                            return if self.restarts(&s.toks) { ParaFate::Close } else { ParaFate::Stay };
                        }
                        SKind::Step => {
                            return if innermost_cases && steps_outside {
                                ParaFate::LeaveCases
                            } else {
                                ParaFate::Stay
                            };
                        }
                        SKind::Closer => return ParaFate::Stay,
                        SKind::CaseOn => {
                            return if self.case_on_role(j) == ExhRole::Open {
                                ParaFate::Close
                            } else {
                                ParaFate::Stay
                            };
                        }
                        SKind::Exh => {
                            return if self.exh_role(j) == ExhRole::Open { ParaFate::Close } else { ParaFate::Stay };
                        }
                        SKind::End => return ParaFate::Close,
                        SKind::Other => {}
                    }
                }
                ParaFate::Close
            }
        }
    }

    /// A blank line that stays inside the open list: a paragraph break in the current item.
    fn para_break(&mut self, st: &Stmt) {
        if let Some(l) = self.pending_label.take() {
            self.sink().push(Out::Sentence(l));
        }
        let prose = match stmt_kind(&st.toks) {
            SKind::Other => true,
            SKind::Exh => self.exh_role(self.pos) == ExhRole::Nested,
            SKind::CaseOn => self.case_on_role(self.pos) == ExhRole::Nested,
            _ => false,
        };
        if prose
            && let Some(item) = self.lists.last_mut().and_then(|l| l.items.last_mut())
            && item.last().is_some_and(|o| !matches!(o, Out::Break))
        {
            item.push(Out::Break);
        }
    }

    /// `case 1 …` while a case list with items is open at that depth: a new case analysis.
    fn restarts(&self, toks: &[Tok]) -> bool {
        let Some((label, ..)) = toks.get(1..).map(case_parts) else { return false };
        let Some(l) = label else { return false };
        let d = label_depth(&l);
        is_first_label(&l)
            && self.lists.iter().any(|x| x.kind == ListKind::Cases && x.depth == d && !x.items.is_empty())
    }

    /// Is the "exhaustive" statement at `self.all[j]` an opener (cases follow it) or a closer?
    fn exh_role(&self, j: usize) -> ExhRole {
        if let Some(t) = self.all.get(j).and_then(|s| s.toks.first())
            && EXH_CLOSERS.contains(&t.lower().as_str())
        {
            return ExhRole::End;
        }
        let Some(next) = self.all.get(j + 1) else { return ExhRole::End };
        let SKind::Case { sub } = stmt_kind(&next.toks) else { return ExhRole::End };
        let open = self.lists.iter().rev().find(|l| l.kind == ListKind::Cases).map(|l| l.depth);
        let Some(open) = open else { return ExhRole::Open };
        let depth = next.toks.get(1..).map(case_parts).and_then(|p| p.0).map_or(1, |l| label_depth(&l));
        if sub || depth > open {
            return ExhRole::Nested;
        }
        // a case at an open level follows: these cases end here, unless a new analysis starts
        let opener =
            self.all.get(j).and_then(|s| s.toks.first()).is_some_and(|t| t.is_kw("the following cases are exhaustive"));
        if opener || self.restarts(&next.toks) { ExhRole::Open } else { ExhRole::End }
    }

    /// "case on k" at `self.all[j]`: the start of a new case analysis, or of the sub-cases of
    /// the current case (sub-cases or deeper labels follow).
    fn case_on_role(&self, j: usize) -> ExhRole {
        match self.exh_role(j) {
            ExhRole::Nested => ExhRole::Nested,
            _ => ExhRole::Open,
        }
    }

    /// "case on k" / "cases on n mod 3" / "we proceed by cases on x": announce the case analysis
    /// ("We proceed by case analysis on $k$."). Only when the user writes it (STEX-DESIGN §14).
    fn case_on(&mut self, toks: &[Tok], term: &Option<String>) {
        let mut rest = &toks[1..];
        while rest.first().is_some_and(|t| t.is_sym(":") || t.is_sym(",")) {
            rest = &rest[1..];
        }
        // "case on k, exhaustive b/c …": the exhaustiveness part is its own sentence
        let mut exh: &[Tok] = &[];
        if let Some(c) = find_top(rest, |t| t.is_sym(",") || t.is_sym(";"))
            && rest.get(c + 1).is_some_and(|t| t.kind == K::Word && is_exh_opener_word(&t.lower()))
        {
            exh = &rest[c + 1..];
            rest = &rest[..c];
        }
        while rest.last().is_some_and(|t| t.is_sym(":") || t.is_sym(".")) {
            rest = &rest[..rest.len() - 1];
        }
        if self.case_on_role(self.pos) == ExhRole::Open {
            self.close_all_cases();
        }
        let s = if rest.is_empty() {
            self.phrase("cases.on.bare", &[])
        } else {
            let x = join(&segment(rest));
            self.phrase("cases.on", &[('X', &x)])
        };
        if exh.is_empty() {
            self.emit(s, term);
        } else {
            self.emit(s, &None);
            self.exhaustive(exh, None, term);
        }
    }

    /// The "exhaustive" family, written by the user (STEX-DESIGN §14). Nothing is ever derived:
    /// - before the cases: "cases exhaustive" → "The following cases are exhaustive.";
    ///   "exhaustive cases b/c R" → "The following cases are exhaustive, because R.";
    ///   "since R, exhaustive cases" → "Since R, the following cases are exhaustive." (`lead`)
    /// - after them: the same with "These cases are exhaustive", plus "In all exhaustive cases, P"
    ///   when the user states P ("exhaustive b/c R, so P") or writes the closer
    ///   ("in all exhaustive cases, P", "since the cases are exhaustive, P").
    fn exhaustive(&mut self, toks: &[Tok], lead: Option<(&[Tok], &str)>, term: &Option<String>) {
        let head = toks[0].lower();
        let mut rest = &toks[1..];
        while rest.first().is_some_and(|t| t.is_sym(",") || t.is_sym(":")) {
            rest = &rest[1..];
        }
        let closer = EXH_CLOSERS.contains(&head.as_str());
        // the user's reason ("b/c Q" / "by T") and what follows from the cases ("so P")
        let (reason, concl): (&[Tok], &[Tok]) = if closer {
            (&[], rest)
        } else {
            match find_top(rest, |t| kw(t, &["so", "hence", "thus", "therefore", "consequently", "it follows that"])) {
                Some(k) => {
                    let mut r = &rest[..k];
                    while r.last().is_some_and(|t| t.is_sym(",") || t.is_kw("and")) {
                        r = &r[..r.len() - 1];
                    }
                    (r, strip_that(&rest[k + 1..]))
                }
                None => (rest, &[]),
            }
        };
        // the reason word as written: --fixed keeps "because" / "since" / "as" (spec)
        let word_of = |w: &str| match w {
            "since" => "since",
            "as" => "as",
            _ => "because",
        };
        let why = match (lead, reason.first()) {
            (Some((r, w)), _) if !r.is_empty() => Some(Why::Because(self.clause(&parse_clause(r)), word_of(w), true)),
            (_, Some(t)) if t.is_kw("by") && reason.len() > 1 => Some(Why::By(join(&segment(&reason[1..])))),
            (_, Some(t)) if kw(t, &["b/c", "because", "since", "as", "for"]) && reason.len() > 1 => {
                let w = word_of(&t.lower());
                Some(Why::Because(self.clause(&parse_clause(&reason[1..])), w, false))
            }
            // "exhaustive: x is either even or odd" — the user's words are the reason
            (_, Some(_)) => Some(Why::Because(self.clause(&parse_clause(reason)), "since", false)),
            (_, None) => None,
        };
        let role = if closer { ExhRole::End } else { self.exh_role(self.pos) };
        let before = role != ExhRole::End;
        if role == ExhRole::Open && self.lists.iter().any(|l| l.kind == ListKind::Cases) {
            self.close_cases();
        }
        if !before && self.lists.iter().any(|l| l.kind == ListKind::Cases) {
            self.close_cases();
        }
        let (subject, bank, bank_by, bank_bare) = if before {
            ("the following cases", "cases.exhaustive.open", "cases.exhaustive.open.by", "cases.exhaustive.open.bare")
        } else {
            ("these cases", "cases.exhaustive", "cases.exhaustive.by", "cases.exhaustive.bare")
        };
        let p = (!concl.is_empty()).then(|| {
            let c = parse_clause(concl);
            self.fact_have(&c, None);
            self.clause(&c)
        });
        // the exhaustiveness sentence: always for the plain family, never for a closer phrase
        // (that one says it itself)
        if !closer {
            let s = match &why {
                Some(Why::Because(q, w, true)) => {
                    if self.fixed() {
                        format!("{} {q}, {subject} are exhaustive", capitalize(w))
                    } else {
                        self.phrase(bank, &[('R', q)])
                    }
                }
                Some(Why::Because(q, w, false)) => {
                    if self.fixed() {
                        format!("{} are exhaustive, {w} {q}", capitalize(subject))
                    } else {
                        self.phrase(bank, &[('R', q)])
                    }
                }
                Some(Why::By(t)) => self.phrase(bank_by, &[('T', t)]),
                None => self.phrase(bank_bare, &[]),
            };
            self.emit(s, &if p.is_some() { None } else { term.clone() });
        }
        if let Some(p) = p {
            let s = if closer && self.fixed() {
                // the user's own words: "In all exhaustive cases, …", "Since the cases are exhaustive, …"
                format!("{}, {p}", toks[0].text)
            } else if closer {
                self.phrase("cases.exhaustive.close.since", &[('P', &p)])
            } else {
                // "These cases are exhaustive, because R." was just said: no "since the cases are …"
                self.phrase("cases.exhaustive.close", &[('P', &p)])
            };
            self.emit(s, term);
        }
    }

    /// Close every open case list (and what is nested in them), keeping an enclosing step list.
    fn close_all_cases(&mut self) {
        if let Some(k) = self.lists.iter().position(|l| l.kind == ListKind::Cases) {
            while self.lists.len() > k {
                self.close_level();
            }
        }
    }

    /// Close lists up to and including the innermost case analysis ("exhaustive" after sub-cases).
    fn close_cases(&mut self) {
        while let Some(l) = self.lists.last() {
            let cases = l.kind == ListKind::Cases;
            self.close_level();
            if cases {
                break;
            }
        }
    }

    fn close_level(&mut self) {
        let Some(l) = self.lists.pop() else { return };
        if l.kind == ListKind::Cases {
            self.facts.push(Fact::EndCases { line: self.line, depth: l.depth });
        }
        if l.items.is_empty() {
            return;
        }
        if self.lists.is_empty() {
            if !self.cur.is_empty() {
                self.blocks.push(Block::Para(std::mem::take(&mut self.cur)));
            }
            self.blocks.push(Block::Items(l.items));
        } else {
            self.sink().push(Out::List(l.items));
        }
    }
    fn close_para(&mut self) {
        if let Some(l) = self.pending_label.take() {
            self.sink().push(Out::Sentence(l));
        }
        self.close_items();
        if !self.cur.is_empty() {
            self.blocks.push(Block::Para(std::mem::take(&mut self.cur)));
        }
    }

    /// `by T qed` / `since Q qed`: close the proof citing the reason.
    fn qed_with_reason(&mut self, lead: &[Tok]) {
        self.close_items();
        let goal = self.goals.pop();
        let reason = &lead[1..];
        let s = if lead[0].is_kw("by") {
            let (t, role) = match theorems::match_at(reason) {
                Some((id, n)) if n == reason.len() => {
                    if !self.theorems.contains(&id) {
                        self.theorems.push(id);
                    }
                    (theorems::get(id).name.to_string(), "by.before")
                }
                _ => (join(&segment(reason)), "by.text.before"),
            };
            match &goal {
                Some((p, _)) => self.phrase(role, &[('P', p), ('T', &t)]),
                None => format!("This completes the proof by {t}"),
            }
        } else {
            let q = self.clause(&parse_clause(reason));
            match &goal {
                Some((p, _)) => format!("Since {q}, {p}"),
                None => format!("Since {q}, this completes the proof"),
            }
        };
        self.facts.push(Fact::Qed { m: goal.and_then(|g| g.1), line: self.line });
        self.emit(s + ". \\hfill$\\square$", &Some(String::new()));
    }

    /// Append text to the most recent sentence (e.g. ", so" before a display).
    fn append_to_last(&mut self, text: &str) {
        if let Some(Out::Sentence(s)) = self.sink().last_mut() {
            s.push_str(text);
        }
    }
    fn push_out(&mut self, o: Out) {
        self.sink().push(o);
    }

    /// `choose x = 3`, `pick k in Z such that …`: introduce a witness or an object.
    fn choose(&mut self, word: &str, toks: &[Tok], term: &Option<String>) {
        let (body, just) = self.split_just(toks);
        let c = parse_clause(body);
        let x = self.clause(&c);
        match &c {
            Clause::Math(M::Chain(first, steps))
                if steps.len() == 1 && steps[0].op == "=" && matches!(**first, M::Var(_)) =>
            {
                self.facts.push(Fact::Witness { var: math::tex(first), value: steps[0].rhs.clone(), line: self.line });
            }
            other => self.let_facts(other),
        }
        let s = if self.fixed() {
            format!("{} {x}", capitalize(&word.to_lowercase()))
        } else {
            self.phrase("choose", &[('X', &x)])
        };
        let s = self.justify(s, just);
        self.emit(s, term);
    }
    fn close_items(&mut self) {
        while !self.lists.is_empty() {
            self.close_level();
        }
    }

    pub fn finish(mut self) -> (Vec<Block>, Vec<Fact>, Vec<usize>) {
        self.close_para();
        (self.blocks, self.facts, self.theorems)
    }

    // -------------------------------------------------------------- statements

    pub fn statement(&mut self, st: &Stmt) {
        self.line = st.line;
        if st.para {
            // a blank line inside a case analysis is a new paragraph of the current case when more
            // cases (or a closing "in all cases" / "exhaustive") follow; otherwise it ends the list
            match if self.lists.is_empty() { ParaFate::Close } else { self.para_fate(st) } {
                ParaFate::Close => self.close_para(),
                ParaFate::LeaveCases => {
                    self.close_cases();
                    if self.lists.is_empty() { self.close_para() } else { self.para_break(st) }
                }
                ParaFate::Stay => self.para_break(st),
            }
        }
        let toks = &st.toks[..];
        let Some(first) = toks.first() else { return };
        // --fixed / --natural: each sentence's phrasing depends only on its own text, so editing
        // one sentence never reshuffles the others
        let text: Vec<&str> = toks.iter().map(|t| t.text.as_str()).collect();
        self.pick.reseed_for(crate::phrases::fnv1a(text.join(" ").as_bytes()));
        // "…, so" + a `$…$` block on its own line: the sentence flows into the display
        if toks.len() > 1
            && let Some(last) = toks.last()
            && let K::Math(inner) = &last.kind
            && (last.line > toks[toks.len() - 2].line || last.text.contains('\n'))
        {
            // a trailing connective ("…, so" / "…, we have") is kept as prose but not parsed
            let mut lead_toks = &toks[..toks.len() - 1];
            let mut tail = String::new();
            if let Some(t) = lead_toks.last()
                && t.kind == K::Word
                && matches!(
                    t.lower().as_str(),
                    "so" | "thus"
                        | "hence"
                        | "therefore"
                        | "then"
                        | "we have"
                        | "we get"
                        | "we obtain"
                        | "gives"
                        | "yields"
                )
            {
                tail = t.text.clone();
                lead_toks = &lead_toks[..lead_toks.len() - 1];
                if lead_toks.last().is_some_and(|t| t.is_sym(",")) {
                    lead_toks = &lead_toks[..lead_toks.len() - 1];
                    tail = format!(", {tail}");
                } else {
                    tail = format!(" {tail}");
                }
            }
            if lead_toks.last().is_some_and(|t| t.is_sym(":")) {
                lead_toks = &lead_toks[..lead_toks.len() - 1];
                tail.push(':');
            }
            let lead = Stmt { toks: lead_toks.to_vec(), line: st.line, term: Some(String::new()), para: false };
            if !lead.toks.is_empty() {
                self.statement(&lead);
                self.append_to_last(&tail);
            } else if !tail.is_empty() {
                // just "then" / "so" before the display: keep it as the lead-in
                let word = tail.trim_start_matches(',').trim().to_string();
                self.emit(word, &Some(String::new()));
            }
            let d = display(inner, true, st.term.as_deref());
            self.push_out(Out::Display(d));
            if let Some(m) = explicit(&strip_tags(inner).0) {
                self.facts.push(Fact::Have { m, line: last.line, thm: None });
            }
            return;
        }
        // "by double containment qed", "x = 5 qed": the statement ends the proof
        if toks.len() > 1 && toks.last().is_some_and(|t| t.is_kw("qed")) {
            let lead = &toks[..toks.len() - 1];
            if matches!(first.lower().as_str(), "by" | "since" | "because" | "b/c") {
                return self.qed_with_reason(lead);
            }
            let sub = Stmt { toks: lead.to_vec(), line: st.line, term: Some(".".into()), para: false };
            self.statement(&sub);
            let q = Stmt { toks: vec![toks[toks.len() - 1].clone()], line: st.line, term: None, para: false };
            return self.statement(&q);
        }
        let h = if first.kind == K::Word { first.lower() } else { String::new() };
        if self.claim_next
            && matches!(h.as_str(), "proof" | "let" | "fix" | "assume" | "suppose" | "afsoc" | "wts" | "case")
        {
            self.claim_next = false;
        }
        if let Some((var, strong)) = induction_spec(toks) {
            return self.induction_stmt(&var, strong, &st.term);
        }
        // "since k is odd or even, exhaustive cases": the reason first
        if let Some(k) = lead_reason_exh(toks) {
            let word = toks[0].lower();
            return self.exhaustive(&toks[k..], Some((&toks[1..k - 1], word.as_str())), &st.term);
        }
        let rest = &toks[1..];
        match h.as_str() {
            "let" | "fix" => self.let_stmt(rest, &st.term),
            "choose" | "pick" | "take" | "select" => self.choose(&first.text, rest, &st.term),
            "assume" | "suppose" | "assuming" => {
                let r = strip_that(rest);
                if r.first().is_some_and(|t| {
                    matches!(
                        t.lower().as_str(),
                        "for the sake of contradiction" | "for contradiction" | "to the contrary"
                    )
                }) {
                    return self.afsoc(&r[1..], &st.term);
                }
                let c = parse_clause(r);
                let p = self.clause(&c);
                if self.phase() == Some(IndPhase::Step) {
                    // the inductive hypothesis is Lean's `ih`: only learn the step variable
                    self.learn_step_var(&c);
                } else {
                    self.fact_hyp(&c);
                }
                // "assume the claim for 2^k boards": an object, not a statement → "Assume …"
                let verb = |w: &str| {
                    matches!(
                        w,
                        "is" | "are"
                            | "be"
                            | "was"
                            | "were"
                            | "has"
                            | "have"
                            | "holds"
                            | "hold"
                            | "fails"
                            | "fail"
                            | "satisfies"
                            | "satisfy"
                            | "divides"
                            | "equals"
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
                            | "lies"
                            | "belongs to"
                    ) || w.starts_with("is ")
                };
                let verbless = r.first().is_some_and(|t| t.is_kw("the"))
                    && !r.iter().any(|t| t.kind == K::Word && verb(&t.lower()))
                    && !r
                        .iter()
                        .any(|t| matches!(t.kind, K::Sym) && matches!(t.text.as_str(), "=" | "<" | ">" | "<=" | ">="));
                let s = if verbless { format!("Assume {p}") } else { self.phrase("assume", &[('P', &p)]) };
                self.emit(s, &st.term);
            }
            "afsoc" | "aftsoc" => self.afsoc(rest, &st.term),
            "contradiction" | "contra" | "this is a contradiction" => {
                let r: &[Tok] = match rest.first() {
                    Some(t) if kw(t, &["with", "since", "because", "to", ":", ",", "of"]) => &rest[1..],
                    _ => rest,
                };
                let s = if r.is_empty() {
                    match self.afsoc.clone() {
                        Some(a) => self.phrase("contradiction", &[('A', &a)]),
                        None => self.phrase("contradiction", &[]),
                    }
                } else {
                    let p = self.clause(&parse_clause(r));
                    self.phrase("contradiction.with", &[('P', &p)])
                };
                self.facts.push(Fact::Contra { line: self.line });
                self.afsoc = None;
                self.emit(s, &st.term);
            }
            "wts" | "want to show" | "we want to show" | "we need to show" | "show" | "to show" => {
                let c = parse_clause(strip_that(rest));
                let p = self.clause(&c);
                let m = clause_math(&c);
                if matches!(self.phase(), Some(IndPhase::Base | IndPhase::Step)) {
                    // inside an induction each branch's goal comes from Lean's `induction`
                    if self.phase() == Some(IndPhase::Step) {
                        self.learn_step_var(&c);
                    }
                } else {
                    if let Some(m) = &m {
                        self.facts.push(Fact::Goal { m: m.clone(), line: self.line });
                    }
                    self.goals.push((p.clone(), m));
                }
                let s = self.phrase("goal", &[('P', &p)]);
                self.emit(s, &st.term);
            }
            "qed" => {
                // the proof's end also ends an open case analysis and any induction
                self.close_items();
                self.induction = None;
                let custom = (!rest.is_empty()).then(|| parse_clause(rest));
                let goal = match &custom {
                    Some(c) => Some((self.clause(c), clause_math(c))),
                    None => self.goals.pop(),
                };
                if custom.is_some() {
                    self.goals.pop();
                }
                // the last sentence already concluded the goal ("Hence n^2 is odd, as …"), or there
                // is no goal: the square ends that sentence instead of restating it
                let last = match self.sink().last() {
                    Some(Out::Sentence(s)) => Some(s.clone()),
                    _ => None,
                };
                let concluded = match (&goal, &last) {
                    (Some((p, _)), Some(l)) => {
                        let l = l.to_lowercase();
                        [
                            "hence ",
                            "thus",
                            "so ",
                            "therefore ",
                            "consequently",
                            // a case analysis' closing sentence: "In either case, P", "In all exhaustive cases, P"
                            "in either case",
                            "in both cases",
                            "in all ",
                            "in every ",
                            "in each ",
                            "in any case",
                            "either way",
                            "since the cases",
                            "as the cases",
                            "because the cases",
                        ]
                        .iter()
                        .any(|w| l.starts_with(w))
                            && l.contains(&p.to_lowercase())
                    }
                    _ => false,
                };
                self.facts.push(Fact::Qed { m: goal.clone().and_then(|g| g.1), line: self.line });
                if (goal.is_none() || concluded) && last.is_some() {
                    self.append_to_last(" \\hfill$\\square$");
                    return;
                }
                let s = match &goal {
                    Some((p, _)) => self.phrase("qed", &[('P', p)]) + ". \\hfill$\\square$",
                    None => "\\hfill$\\square$".to_string(),
                };
                self.emit(s, &Some(String::new()));
            }
            "then" | "thus" | "hence" | "so" | "therefore" | "it follows that" | "consequently" => {
                let r = strip_that(rest);
                self.conclude(&h, r, &st.term);
            }
            "define" => self.define(rest, &st.term),
            "i.e." | "ie" | "that is" | "in other words" => {
                let c = parse_clause(rest);
                let p = self.clause(&c);
                self.fact_have(&c, None);
                let s = self.phrase("ie", &[('P', &p)]);
                self.emit(s, &st.term);
            }
            "note" | "note that" | "observe" | "notice" | "recall" => {
                let c = parse_clause(strip_that(rest));
                let p = self.clause(&c);
                self.fact_have(&c, None);
                let s = self.phrase("note", &[('P', &p)]);
                self.emit(s, &st.term);
            }
            "wlog" | "without loss of generality" => {
                let mut r = rest;
                while let Some(t) = r.first() {
                    if kw(t, &[",", "assume", "suppose", "that", "we may assume"]) {
                        r = &r[1..];
                    } else {
                        break;
                    }
                }
                let c = parse_clause(r);
                let p = self.clause(&c);
                self.fact_hyp(&c);
                // "wlog we may write …": already a full sentence, no "assume that"
                let s = if r.first().is_some_and(|t| t.is_kw("we")) {
                    format!("Without loss of generality, {p}")
                } else {
                    self.phrase("wlog", &[('P', &p)])
                };
                self.emit(s, &st.term);
            }
            "case" => self.case(rest, false),
            w if EXH_WORDS.contains(&w) || EXH_CLOSERS.contains(&w) => self.exhaustive(toks, None, &st.term),
            w if CASE_ON.contains(&w) => self.case_on(toks, &st.term),
            "subcase" | "sub-case" => self.case(rest, true),
            "step" | "stage" => self.step(&first.text, rest, &st.term),
            "in total" | "altogether" | "overall" | "in all" | "all in all" => {
                self.close_items();
                self.plain(toks, &st.term);
            }
            "suppose not"
            | "assume not"
            | "suppose otherwise"
            | "assume otherwise"
            | "assume the contrary"
            | "suppose the contrary" => {
                // "suppose otherwise, and let S be …": the negated goal, then the rest
                let mut r = rest;
                while r.first().is_some_and(|t| t.is_sym(",") || t.is_kw("and")) {
                    r = &r[1..];
                }
                if r.is_empty() || !r[0].is_kw("let") && !r[0].is_kw("then") && !r[0].is_kw("so") {
                    return self.afsoc(rest, &st.term);
                }
                self.afsoc(&[], &Some(".".into()));
                let sub = Stmt { toks: r.to_vec(), line: st.line, term: st.term.clone(), para: false };
                self.statement(&sub);
            }
            "in either case" | "in both cases" | "in all cases" | "in every case" | "in any case" | "either way" => {
                self.close_all_cases();
                let c = parse_clause(strip_that(rest));
                let p = self.clause(&c);
                self.fact_have(&c, None);
                let s = if self.fixed() {
                    format!("{}, {p}", first.text)
                } else {
                    self.phrase("cases.close", &[('P', &p)])
                };
                self.emit(s, &st.term);
            }
            "proof" | "claim" | "lemma" | "proposition" | "theorem" | "corollary" | "observation" | "remark"
            | "fact" | "sublemma" | "aside" | "base case" | "inductive step" | "induction step" | "definition"
            | "example" => self.label(toks, st),
            _ if self.claim_next => {
                self.claim_next = false;
                let c = parse_clause(toks);
                let p = self.clause(&c);
                let m = clause_math(&c);
                if let Some(m) = &m {
                    self.facts.push(Fact::Goal { m: m.clone(), line: self.line });
                }
                self.goals.push((p.clone(), m));
                self.emit(p, &st.term);
            }
            _ => self.plain(toks, &st.term),
        }
    }

    fn afsoc(&mut self, rest: &[Tok], term: &Option<String>) {
        let r = strip_that(rest);
        let r = match r.first() {
            Some(t) if kw(t, &["for the sake of contradiction", "for contradiction", "to the contrary"]) => {
                strip_that(&r[1..])
            }
            _ => r,
        };
        let r = match r.first() {
            Some(t) if r.len() == 1 && kw(t, &["not", "otherwise", "the contrary", "the opposite"]) => &r[1..],
            _ => r,
        };
        if r.is_empty() {
            // "suppose not" / bare "afsoc": assume the negation of what we are proving
            let negated = self.goals.last().and_then(|(_, m)| m.clone()).map(math::negate_claim);
            let p = match &negated {
                Some(m) => claim_text(m),
                None => "the claim is false".to_string(),
            };
            match negated {
                Some(m) => self.facts.push(Fact::Afsoc { m, line: self.line }),
                None => self.facts.push(Fact::Skip { line: self.line, text: "(proof by contradiction)".into() }),
            }
            self.afsoc = Some(p.clone());
            let s = self.phrase("afsoc", &[('P', &p)]);
            return self.emit(s, term);
        }
        let c = parse_clause(r);
        let p = self.clause(&c);
        match clause_math(&c) {
            Some(m) => self.facts.push(Fact::Afsoc { m, line: self.line }),
            None => self.facts.push(Fact::Skip { line: self.line, text: format!("(proof by contradiction) {p}") }),
        }
        self.afsoc = Some(p.clone());
        let s = self.phrase("afsoc", &[('P', &p)]);
        self.emit(s, term);
    }

    fn phase(&self) -> Option<IndPhase> {
        self.induction.as_ref().map(|(p, _)| *p)
    }

    /// "we proceed by [strong] induction on n".
    fn induction_stmt(&mut self, var: &str, strong: bool, term: &Option<String>) {
        let x = format!("${var}$");
        self.facts.push(Fact::Induction { var: var.to_string(), strong, line: self.line });
        self.induction = Some((IndPhase::Intro, var.to_string()));
        self.step_fact = None;
        let s = self.phrase(if strong { "induction.strong" } else { "induction" }, &[('X', &x)]);
        self.emit(s, term);
    }

    /// `base case n = 0: …` / `inductive step: …`.
    fn induction_part(&mut self, head: &str, rest: &[Tok], st: &Stmt) {
        self.close_para();
        let var = self.induction.as_ref().map(|(_, v)| v.clone()).unwrap_or_else(|| "n".into());
        let mut rest = rest;
        while rest.first().is_some_and(|t| t.is_sym(":") || t.is_sym(".") || t.is_sym(",")) {
            rest = &rest[1..];
        }
        if head == "base case" {
            // optional condition before ':' ("base case n = 0: …"), or a bare value ("base case 0")
            let (cond, body) = match find_top(rest, |t| t.is_sym(":")) {
                Some(c) => (&rest[..c], &rest[c + 1..]),
                None if !rest.is_empty() && rest.len() <= 3 => (rest, &rest[rest.len()..]),
                None => (&rest[..0], rest),
            };
            let cm = if cond.is_empty() {
                None
            } else {
                match clause_math(&parse_clause(cond)) {
                    Some(M::Num(n)) => Some(M::Chain(
                        Box::new(M::Var(var.clone())),
                        vec![Step { op: "=".into(), rhs: M::Num(n), tag: None, end: 0 }],
                    )),
                    other => other,
                }
            };
            let tag = match &cm {
                Some(m) => format!("\\textit{{Base case}} (${}$).", math::tex(m)),
                None => "\\textit{Base case.}".to_string(),
            };
            self.facts.push(Fact::Base { cond: cm, line: self.line });
            self.induction = Some((IndPhase::Base, var));
            self.pending_label = Some(tag);
            if !body.is_empty() {
                let sub = Stmt { toks: body.to_vec(), line: st.line, term: st.term.clone(), para: false };
                self.statement(&sub);
            }
        } else {
            self.step_fact = Some(self.facts.len());
            self.facts.push(Fact::Step { var: None, line: self.line });
            self.induction = Some((IndPhase::Step, var));
            self.pending_label = Some("\\textit{Inductive step.}".into());
            if !rest.is_empty() {
                let sub = Stmt { toks: rest.to_vec(), line: st.line, term: st.term.clone(), para: false };
                self.statement(&sub);
            }
        }
    }

    /// Record the inductive step's variable (k in "assume P(k)") on the `Step` fact.
    fn learn_step_var(&mut self, c: &Clause) {
        let Some(i) = self.step_fact else { return };
        if matches!(self.facts.get(i), Some(Fact::Step { var: Some(_), .. })) {
            return;
        }
        let Some(m) = clause_math(c).or_else(|| first_math(c)) else { return };
        let ind = self.induction.as_ref().map(|(_, v)| v.clone()).unwrap_or_default();
        let mut vars = Vec::new();
        collect_vars(&m, &mut vars);
        let pick = vars.iter().find(|v| **v != ind && v.chars().count() == 1).or(vars.iter().find(|v| **v == ind));
        if let Some(v) = pick.cloned()
            && let Some(Fact::Step { var, .. }) = self.facts.get_mut(i)
        {
            *var = Some(v);
        }
    }

    /// `step 1: …`: one step of a counting or construction procedure.
    fn step(&mut self, word: &str, toks: &[Tok], term: &Option<String>) {
        let mut i = 0;
        let mut label = String::new();
        while let Some(t) = toks.get(i) {
            if matches!(t.kind, K::Num | K::Word) && (i == 0 || !t.sp_before) && t.text.len() <= 4 {
                label.push_str(&t.text);
                i += 1;
            } else {
                break;
            }
        }
        let mut body = &toks[i..];
        while body.first().is_some_and(|t| t.is_sym(":") || t.is_sym(".") || t.is_sym(",")) {
            body = &body[1..];
        }
        self.open_item(ListKind::Steps, label_depth(&label));
        let name = capitalize(&word.to_lowercase());
        let tag = if label.is_empty() { format!("{name}:") } else { format!("{name} {label}:") };
        if body.is_empty() {
            self.sink().push(Out::Sentence(tag));
            return;
        }
        self.pending_label = Some(tag);
        let sub = Stmt { toks: body.to_vec(), line: self.line, term: term.clone(), para: false };
        self.statement(&sub);
    }

    fn fact_hyp(&mut self, c: &Clause) {
        match clause_math(c) {
            Some(m) => self.facts.push(Fact::Hyp { m, line: self.line }),
            None => self.facts.push(Fact::Skip { line: self.line, text: self.plain_text(c) }),
        }
    }

    fn fact_have(&mut self, c: &Clause, thm: Option<usize>) {
        match clause_math(c) {
            Some(m) => self.facts.push(Fact::Have { m, line: self.line, thm }),
            None => {
                self.facts.push(Fact::Skip { line: self.line, text: self.plain_text(c) });
                // "there are 10·C(9,2) = 360 ways": equations inside prose are still claims to check,
                // when every variable in them is already declared (or there are none)
                let declared: Vec<String> = self
                    .facts
                    .iter()
                    .filter_map(|f| match f {
                        Fact::Decl { var, .. } => Some(var.clone()),
                        _ => None,
                    })
                    .collect();
                let mut found = Vec::new();
                closed_chains(c, &declared, &mut found);
                for m in found {
                    self.facts.push(Fact::Have { m, line: self.line, thm });
                }
            }
        }
    }

    /// "P, so Q, thus R": split a sentence at mid-sentence conclusion words. Each part is its own
    /// fact; the prose keeps the user's connectives. `None` when there is nothing to split.
    fn connected_parts(&mut self, toks: &[Tok], thm: Option<usize>) -> Option<String> {
        const CONN: &[&str] = &["so", "thus", "hence", "therefore", "consequently", "whence"];
        let mut cuts = Vec::new();
        let mut depth = 0;
        for (i, t) in toks.iter().enumerate() {
            if t.kind == K::Sym {
                match t.text.as_str() {
                    "(" | "[" | "{" => depth += 1,
                    ")" | "]" | "}" => depth -= 1,
                    _ => {}
                }
            }
            if depth == 0 && i > 0 && i + 1 < toks.len() && t.kind == K::Word && CONN.contains(&t.lower().as_str()) {
                cuts.push(i);
            }
        }
        if cuts.is_empty() {
            return None;
        }
        let mut out = String::new();
        let mut start = 0;
        for (k, &cut) in cuts.iter().chain(std::iter::once(&toks.len())).enumerate() {
            let mut part = &toks[start..cut];
            let comma = part.last().is_some_and(|t| t.is_sym(","));
            if comma {
                part = &part[..part.len() - 1];
            }
            let c = parse_clause(strip_that(part));
            let text = self.clause(&c);
            self.fact_have(&c, if k == cuts.len() { thm } else { None });
            out.push_str(&text);
            if cut < toks.len() {
                out.push_str(if comma { ", " } else { " " });
                out.push_str(&toks[cut].text);
                out.push(' ');
                start = cut + 1;
            }
        }
        Some(out)
    }

    fn plain_text(&self, c: &Clause) -> String {
        match c {
            Clause::Mixed(p) => join(p),
            _ => String::from("(statement)"),
        }
    }

    /// Split off a justification: `by THM …` / `… by THM`, `… because Q`, `since Q, P`.
    fn split_just<'t>(&mut self, toks: &'t [Tok]) -> (&'t [Tok], Option<(Just, bool)>) {
        if toks.is_empty() {
            return (toks, None);
        }
        if toks[0].is_kw("by") {
            if let Some((id, n)) = theorems::match_at(&toks[1..]) {
                return (strip_that(&toks[1 + n..]), Some((Just::Thm(id), true)));
            }
            // non-theorem lead: words up to the first comma
            if let Some(c) = toks.iter().position(|t| t.is_sym(",")) {
                return (&toks[c + 1..], Some((Just::Text(segment(&toks[1..c])), true)));
            }
        }
        if matches!(toks[0].lower().as_str(), "since" | "because" | "b/c" | "as")
            && let Some(c) = find_top(&toks[1..], |t| t.is_sym(",") || t.is_kw("then"))
        {
            let word = if toks[0].lower() == "since" { "since" } else { "because" };
            return (&toks[c + 2..], Some((Just::Reason(toks[1..c + 1].to_vec(), word.into()), true)));
        }
        // "ax + by = d": `by` between arithmetic operators is the product b·y, not a reason
        let arith = |t: Option<&Tok>| {
            t.is_some_and(|t| t.kind == K::Sym && matches!(t.text.as_str(), "+" | "-" | "*" | "/" | "=" | "<" | ">"))
        };
        let by_at = toks.iter().enumerate().position(|(k, t)| {
            t.is_kw("by") && !(k > 0 && arith(toks.get(k - 1)) && (arith(toks.get(k + 1)) || toks.get(k + 1).is_none()))
        });
        if let Some(i) = by_at.filter(|&i| find_top(&toks[..=i], |t| t.is_kw("by")).is_some())
            && i > 0
        {
            let tail = &toks[i + 1..];
            if let Some((id, n)) = theorems::match_at(tail)
                && tail[n..].iter().all(|t| t.kind == K::Sym && matches!(t.text.as_str(), "," | ";"))
            {
                let body = &toks[..i];
                let body = if body.last().is_some_and(|t| t.is_sym(",")) { &body[..body.len() - 1] } else { body };
                return (body, Some((Just::Thm(id), false)));
            }
            let body = &toks[..i];
            let body = if body.last().is_some_and(|t| t.is_sym(",")) { &body[..body.len() - 1] } else { body };
            return (body, Some((Just::Text(segment(tail)), false)));
        }
        if let Some(i) = find_top(toks, |t| matches!(t.lower().as_str(), "because" | "since" | "b/c"))
            && i > 0
        {
            let word = if toks[i].lower() == "since" { "since" } else { "because" };
            let body = &toks[..i];
            let body = if body.last().is_some_and(|t| t.is_sym(",")) { &body[..body.len() - 1] } else { body };
            return (body, Some((Just::Reason(toks[i + 1..].to_vec(), word.into()), false)));
        }
        (toks, None)
    }

    /// Render `body` with its justification.
    fn justify(&mut self, p: String, just: Option<(Just, bool)>) -> String {
        let Some((j, lead)) = just else { return p };
        match j {
            Just::Thm(id) => {
                if !self.theorems.contains(&id) {
                    self.theorems.push(id);
                }
                let t = theorems::get(id).name;
                self.by(&p, t, lead)
            }
            Just::Text(pieces) => {
                let t = join(&pieces);
                if p.is_empty() {
                    return format!("by {t}");
                }
                if lead {
                    self.phrase("by.text.before", &[('P', &p), ('T', &t)])
                } else {
                    self.phrase("by.text.after", &[('P', &p), ('T', &t)])
                }
            }
            Just::Reason(toks, word) => {
                let q = self.clause(&parse_clause(&toks));
                if lead {
                    if self.fixed() {
                        let w = if word == "since" { "Since" } else { "Because" };
                        format!("{w} {q}, {p}")
                    } else {
                        self.phrase("because.lead", &[('P', &p), ('Q', &q)])
                    }
                } else {
                    let role = if word == "since" { "since" } else { "because" };
                    self.phrase(role, &[('P', &p), ('Q', &q)])
                }
            }
        }
    }

    fn by(&mut self, p: &str, t: &str, lead: bool) -> String {
        if p.is_empty() {
            return format!("by {t}");
        }
        if lead {
            self.phrase("by.before", &[('P', p), ('T', t)])
        } else {
            self.phrase("by.after", &[('P', p), ('T', t)])
        }
    }

    fn just_thm(j: &Option<(Just, bool)>) -> Option<usize> {
        match j {
            Some((Just::Thm(id), _)) => Some(*id),
            _ => None,
        }
    }

    fn plain(&mut self, toks: &[Tok], term: &Option<String>) {
        // latex{\begin{tikzpicture}…} on its own: a centered display, not math
        if toks.len() == 1
            && let K::Latex(raw) = &toks[0].kind
            && raw.starts_with("\\begin{")
        {
            self.push_out(Out::Display(format!("\\begin{{center}}\n{raw}\n\\end{{center}}")));
            return;
        }
        // a standalone explicit $…$ is display math
        if toks.len() == 1
            && let K::Math(inner) = &toks[0].kind
        {
            let multiline = toks[0].text.contains('\n');
            let d = display(inner, multiline, term.as_deref());
            self.push_out(Out::Display(d));
            if let Some(m) = explicit(&strip_tags(inner).0) {
                self.facts.push(Fact::Have { m, line: self.line, thm: None });
            }
            return;
        }
        let (body, just) = self.split_just(toks);
        let thm = Self::just_thm(&just);
        // `P, i.e. Q`
        if let Some(i) = find_top(body, |t| matches!(t.lower().as_str(), "i.e." | "ie"))
            && i > 0
        {
            let a = &body[..i];
            let a = if a.last().is_some_and(|t| t.is_sym(",")) { &a[..a.len() - 1] } else { a };
            let (ca, cb) = (parse_clause(a), parse_clause(strip_that(&body[i + 1..])));
            let (p, q) = (self.clause(&ca), self.clause(&cb));
            self.fact_have(&ca, thm);
            let s = self.phrase("ie.mid", &[('P', &p), ('Q', &q)]);
            let s = self.justify(s, just);
            return self.emit(s, term);
        }
        if let Some(s) = self.connected_parts(body, thm) {
            let s = self.justify(s, just);
            return self.emit(s, term);
        }
        let c = parse_clause(body);
        // `X := Y` defines
        if let Clause::Math(M::Chain(_, steps)) = &c
            && steps.iter().any(|s| s.op == ":=")
        {
            let x = self.clause(&c);
            if let Clause::Math(M::Chain(lhs, steps)) = &c {
                self.facts.push(Fact::Def {
                    lhs: (**lhs).clone(),
                    rhs: steps[0].rhs.clone(),
                    sig: None,
                    line: self.line,
                });
            }
            let s = self.phrase("define.assign", &[('X', &x)]);
            return self.emit(s, term);
        }
        self.top = true;
        let p = self.clause(&c);
        self.top = false;
        self.fact_have(&c, thm);
        let p = if matches!(c, Clause::Math(_)) && just.is_none() { self.phrase("statement", &[('P', &p)]) } else { p };
        let s = self.justify(p, just);
        self.emit(s, term);
    }

    fn conclude(&mut self, word: &str, toks: &[Tok], term: &Option<String>) {
        let toks = if toks.first().is_some_and(|t| t.is_sym(",")) { &toks[1..] } else { toks };
        let (body, just) = self.split_just(toks);
        let thm = Self::just_thm(&just);
        let p = match self.connected_parts(body, thm) {
            Some(s) => s,
            None => {
                let c = parse_clause(body);
                let p = self.clause(&c);
                self.fact_have(&c, thm);
                p
            }
        };
        let lead = matches!(just, Some((_, true)));
        let s = if lead {
            // "then by CRT, P" → "Then, by …, P"
            let inner = self.justify(p, just);
            let conn =
                if self.fixed() { word.to_string() } else { self.phrase("then", &[('P', "")]).trim().to_string() };
            format!("{conn}, {}", lower_first(&inner))
        } else {
            let p = self.justify(p, just);
            if self.fixed() {
                let w = match word {
                    "consequently" => "consequently,",
                    other => other,
                };
                format!("{w} {p}")
            } else {
                self.phrase("then", &[('P', &p)])
            }
        };
        self.emit(s, term);
    }

    fn let_stmt(&mut self, toks: &[Tok], term: &Option<String>) {
        let (toks, just) = self.split_just(toks);
        // items: top-level commas, "and let", and "and" between bare variables. A comma inside a
        // condition ("with a, b > 0", "such that x, y > 0") belongs to the condition.
        let mut items: Vec<Vec<Tok>> = Vec::new();
        let mut cur: Vec<Tok> = Vec::new();
        let mut depth = 0;
        let mut i = 0;
        let mut in_cond = false;
        while i < toks.len() {
            let t = &toks[i];
            if t.kind == K::Sym {
                match t.text.as_str() {
                    "(" | "[" | "{" => depth += 1,
                    ")" | "]" | "}" => depth -= 1,
                    _ => {}
                }
            }
            if depth == 0 && kw(t, &["with", "w/", "where", "such that", "s.t.", "st", "so that", "satisfying"]) {
                in_cond = true;
            }
            let and_let = t.is_kw("and") && toks.get(i + 1).is_some_and(|n| n.is_kw("let"));
            // "x and y", "F_1 = 1 and F_2 = 1": "and" before another declared variable
            let next_declares = {
                let rest = &toks[(i + 1).min(toks.len())..];
                let n = if rest.len() >= 3 && rest[1].is_sym("_") && !rest[1].sp_before { 3 } else { 1 };
                // F_(n+1): a parenthesized subscript
                let n = if n == 3 && rest[2].is_sym("(") {
                    rest.iter().position(|t| t.is_sym(")")).map_or(n, |k| k + 1)
                } else {
                    n
                };
                rest.len() >= n
                    && is_bare_var(&rest[..1])
                    && rest.get(n).is_none_or(|f| {
                        f.is_kw("in")
                            || f.is_kw("be")
                            || f.is_kw("and")
                            || f.is_sym(",")
                            || matches!(f.text.as_str(), "=" | ":=" | "<" | ">" | "<=" | ">=" | "!=")
                    })
            };
            let and_vars = t.is_kw("and") && !in_cond && !cur.is_empty() && next_declares;
            if depth == 0 && ((t.is_sym(",") && !in_cond) || and_let || and_vars) {
                items.push(std::mem::take(&mut cur));
                in_cond = false;
                i += 1;
                continue;
            }
            if t.is_kw("let") && cur.is_empty() {
                i += 1;
                continue;
            }
            if t.is_kw("and") && cur.is_empty() {
                i += 1;
                continue;
            }
            cur.push(t.clone());
            i += 1;
        }
        items.push(cur);
        items.retain(|v| !v.is_empty());
        // "let x in Z, sum from i=1 to 4 = 30": an item that declares nothing is a condition on the
        // one before it (spec: the comma means "such that")
        let mut items2: Vec<Vec<Tok>> = Vec::new();
        for it in items {
            let declares = is_bare_var(&it[..1]) || matches!(it[0].kind, K::Sigil(_) | K::Math(_));
            if !declares && !items2.is_empty() && it[0].kind == K::Word && !lexicon::declares(&it[0].lower()) {
                let prev = items2.last_mut().expect("checked");
                prev.push(Tok {
                    kind: K::Word,
                    text: "such that".into(),
                    line: it[0].line,
                    sp_before: true,
                    sp_after: true,
                    span: (0, 0),
                });
                prev.extend(it);
            } else {
                items2.push(it);
            }
        }
        let items = items2;
        // merge bare variables into the next item: let x, y in Z → "x, y ∈ Z"
        let mut merged: Vec<(Vec<M>, Vec<Tok>)> = Vec::new();
        let mut vars: Vec<M> = Vec::new();
        for mut it in items {
            // "let A_1, A_2, ... be sets": the ellipsis joins the list, the rest is the item
            if it.len() > 1 && it[0].is_sym("...") {
                vars.push(M::Sym(math::DOTS.into()));
                it.remove(0);
            }
            if is_bare_var(&it) {
                if it[0].is_sym("...") {
                    vars.push(M::Sym(math::DOTS.into()));
                    continue;
                }
                let mut p = Parser::new(&it, 0, false);
                if let Some(m) = p.expr()
                    && p.i == it.len()
                {
                    vars.push(m);
                    continue;
                }
            }
            merged.push((std::mem::take(&mut vars), it));
        }
        if !vars.is_empty() {
            merged.push((vars, Vec::new()));
        }
        let mut rendered = Vec::new();
        for (k, (vars, it)) in merged.iter().enumerate() {
            let c = with_vars(vars, parse_clause(it));
            self.let_facts(&c);
            let x = self.clause(&c);
            let arbitrary = matches!(&c, Clause::Math(M::Chain(_, s)) if s.len() == 1 && s[0].op == "\\in")
                || matches!(&c, Clause::Math(M::Var(_) | M::Seq(_)));
            let role = if k == 0 { if arbitrary { "let.arbitrary" } else { "let" } } else { "let.more" };
            rendered.push(self.phrase(role, &[('X', &x)]));
        }
        let s = list_join(&rendered, "and");
        let s = self.justify(s, just);
        self.emit(s, term);
    }
    fn let_facts(&mut self, c: &Clause) {
        let line = self.line;
        if let Some((vars, ty, preds)) = typed_decl(c) {
            for v in &vars {
                self.facts.push(Fact::Decl { var: v.clone(), ty: Some(ty.to_string()), line });
                for p in &preds {
                    self.facts.push(Fact::Hyp { m: M::Pred(p.to_string(), Box::new(M::Var(v.clone()))), line });
                }
            }
            return;
        }
        match c {
            Clause::Math(M::Chain(first, steps)) if steps.len() == 1 && steps[0].op == "\\in" => {
                let ty = math::tex(&steps[0].rhs);
                for v in vars_of(first) {
                    self.facts.push(Fact::Decl { var: v, ty: Some(ty.clone()), line });
                }
            }
            Clause::Math(M::Var(v)) => self.facts.push(Fact::Decl { var: v.clone(), ty: None, line }),
            Clause::Math(M::Seq(vs)) => {
                for v in vs {
                    for name in vars_of(v) {
                        self.facts.push(Fact::Decl { var: name, ty: None, line });
                    }
                }
            }
            Clause::Math(m) => {
                if let M::Chain(first, _) = m {
                    for v in vars_of(first) {
                        self.facts.push(Fact::Decl { var: v, ty: None, line });
                    }
                }
                self.facts.push(Fact::Hyp { m: m.clone(), line });
            }
            other => self.fact_hyp(other),
        }
    }

    fn define(&mut self, toks: &[Tok], term: &Option<String>) {
        let via = find_top(toks, |t| t.is_kw("via") || t.is_kw("by") || t.is_kw("as"));
        let (x_toks, p_toks) = match via {
            Some(i) if i > 0 => (&toks[..i], Some(&toks[i + 1..])),
            _ => (toks, None),
        };
        let xc = parse_clause(x_toks);
        let x = self.clause(&xc);
        let s = match p_toks {
            Some(pt) => {
                let pc = parse_clause(pt);
                let p = self.clause(&pc);
                if let Clause::Math(M::Chain(lhs, steps)) = &pc
                    && steps.len() == 1
                {
                    self.facts.push(Fact::Def {
                        lhs: (**lhs).clone(),
                        rhs: steps[0].rhs.clone(),
                        sig: clause_math(&xc),
                        line: self.line,
                    });
                }
                self.phrase("define.via", &[('X', &x), ('P', &p)])
            }
            None => {
                if let Clause::Math(M::Chain(lhs, steps)) = &xc
                    && steps.len() == 1
                    && matches!(steps[0].op.as_str(), "=" | ":=")
                {
                    self.facts.push(Fact::Def {
                        lhs: (**lhs).clone(),
                        rhs: steps[0].rhs.clone(),
                        sig: None,
                        line: self.line,
                    });
                }
                self.phrase("define", &[('X', &x)])
            }
        };
        self.emit(s, term);
    }
    fn case(&mut self, toks: &[Tok], subcase: bool) {
        let (explicit, cond, body) = case_parts(toks);
        let innermost = self.lists.iter().rev().find(|l| l.kind == ListKind::Cases).map(|l| (l.depth, l.sub));
        let depth = match (&explicit, subcase) {
            // a sibling sub-case, or the first one inside the current case
            (_, true) => match innermost {
                Some((d, true)) => d,
                Some((d, false)) => d + 1,
                None => 1,
            },
            (Some(l), false) => label_depth(l),
            // unlabeled `case`: top level (sub-cases are `subcase` or deeper labels like "1a")
            (None, false) => 1,
        };
        self.open_item(ListKind::Cases, depth);
        // auto-numbering: "Case 2", "Subcase 1.2" (the enclosing case's label, then the position)
        let n = self.lists.last().map_or(1, |l| l.items.len());
        let label = explicit.unwrap_or_else(|| {
            let parent = self.lists[..self.lists.len().saturating_sub(1)]
                .iter()
                .rev()
                .find(|l| l.kind == ListKind::Cases)
                .and_then(|l| l.labels.last().cloned())
                .filter(|p| !p.is_empty());
            match parent {
                Some(p) if depth > 1 => format!("{p}.{n}"),
                _ => n.to_string(),
            }
        });
        // "case n even" / "case p not prime": the verb is implied
        let mut owned: Vec<Tok>;
        let mut cond = cond;
        if let [subject, rest @ ..] = cond
            && !rest.is_empty()
            && matches!(subject.kind, K::Word | K::Math(_) | K::Sigil(_))
            && subject.text.chars().count() <= 2
            && PREDICATES
                .contains(&rest.iter().map(|t| t.lower()).filter(|w| w != "not").collect::<Vec<_>>().join(" ").as_str())
        {
            let mut is = subject.clone();
            is.kind = K::Word;
            is.text = "is".into();
            is.sp_before = true;
            owned = vec![subject.clone(), is];
            owned.extend_from_slice(rest);
            cond = &owned;
        }
        let (c, p) = if cond.is_empty() {
            (None, String::new())
        } else {
            let c = parse_clause(cond);
            let p = self.clause(&c);
            (Some(c), p)
        };
        let m = c.as_ref().and_then(clause_math);
        if let Some(l) = self.lists.last_mut() {
            if subcase && l.items.len() == 1 {
                l.sub = true;
            }
            if let Some(x) = l.labels.last_mut() {
                *x = label.clone();
            }
        }
        self.facts.push(Fact::Case { cond: m, line: self.line, depth });
        let s = if c.is_none() {
            self.phrase(if subcase { "subcase.bare" } else { "case.bare" }, &[('N', &label)])
        } else {
            self.phrase(if subcase { "subcase" } else { "case" }, &[('N', &label), ('P', &p)])
        };
        self.emit(s, &None);
        if !body.is_empty() {
            let sub = Stmt { toks: body.to_vec(), line: self.line, term: None, para: false };
            self.statement(&sub);
        }
    }
    fn label(&mut self, toks: &[Tok], st: &Stmt) {
        let head = toks[0].lower();
        if matches!(head.as_str(), "base case" | "inductive step" | "induction step") {
            return self.induction_part(&head, &toks[1..], st);
        }
        let mut i = 1;
        let mut name = capitalize(&toks[0].text.to_lowercase());
        // number or short name: "lemma 2", "theorem 1.3"
        while let Some(t) = toks.get(i) {
            if t.kind == K::Num
                || (t.kind == K::Sym
                    && t.text == "."
                    && toks.get(i + 1).is_some_and(|n| n.kind == K::Num && !n.sp_before))
            {
                if t.kind == K::Num && t.sp_before {
                    name.push(' ');
                }
                name.push_str(&t.text);
                i += 1;
            } else {
                break;
            }
        }
        while toks.get(i).is_some_and(|t| t.is_sym(":") || t.is_sym(".")) {
            i += 1;
        }
        let rest = &toks[i..];
        // a labelled block (Theorem, Proof, …) starts its own paragraph
        self.close_para();
        let tag = format!("\\textit{{{name}.}}");
        let is_claim = !matches!(
            toks[0].lower().as_str(),
            "proof"
                | "remark"
                | "aside"
                | "observation"
                | "fact"
                | "base case"
                | "inductive step"
                | "induction step"
                | "example"
        );
        self.pending_label = Some(tag);
        if rest.is_empty() {
            // "Proof." alone still leads the next sentence; "Lemma 2." makes the next sentence its claim
            self.claim_next = is_claim;
            return;
        }
        // "lemma: let a, b in Z. if … then …": the setup sentence first; the claim is the next one
        if is_claim
            && rest[0].kind == K::Word
            && matches!(rest[0].lower().as_str(), "let" | "fix" | "assume" | "suppose")
        {
            let sub = Stmt { toks: rest.to_vec(), line: st.line, term: st.term.clone(), para: false };
            self.statement(&sub);
            self.claim_next = true;
            return;
        }
        if is_claim {
            let c = parse_clause(rest);
            let p = self.clause(&c);
            let m = clause_math(&c);
            if let Some(m) = &m {
                self.facts.push(Fact::Goal { m: m.clone(), line: self.line });
            }
            self.goals.push((p.clone(), m));
            self.emit(p, &st.term);
        } else {
            let sub = Stmt { toks: rest.to_vec(), line: st.line, term: st.term.clone(), para: false };
            self.statement(&sub);
        }
    }
}

fn lower_first(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) if f.is_uppercase() && !s.starts_with("By ") => f.to_lowercase().collect::<String>() + c.as_str(),
        Some('B') if s.starts_with("By ") => format!("b{}", &s[1..]),
        _ => s.to_string(),
    }
}

/// Prefix merged bare variables: `x`, `y in Z` → `x, y ∈ Z`.
fn with_vars(vars: &[M], c: Clause) -> Clause {
    if vars.is_empty() {
        return c;
    }
    let seq = |m: M| {
        let mut v = vars.to_vec();
        v.push(m);
        M::Seq(v)
    };
    match c {
        Clause::Math(M::Chain(first, steps)) => Clause::Math(M::Chain(Box::new(seq(*first)), steps)),
        Clause::Math(m) => Clause::Math(seq(m)),
        Clause::Mixed(mut p) => {
            if let Some(Piece::Math(m)) = p.first().cloned() {
                p[0] = Piece::Math(match m {
                    M::Chain(first, steps) => M::Chain(Box::new(seq(*first)), steps),
                    other => seq(other),
                });
            } else {
                p.insert(0, Piece::Math(M::Seq(vars.to_vec())));
            }
            Clause::Mixed(p)
        }
        // "let x, y in R s.t. … and …": the variables join the first part
        Clause::And(mut parts) if !parts.is_empty() => {
            let first = parts.remove(0);
            parts.insert(0, with_vars(vars, first));
            Clause::And(parts)
        }
        other => Clause::And(vec![Clause::Math(M::Seq(vars.to_vec())), other]),
    }
}

/// A single variable token (`x`, `alpha`, `a_1`, `...`), as in a declaration list.
fn is_bare_var(it: &[Tok]) -> bool {
    match it {
        [t] => {
            t.is_sym("...")
                || t.kind == K::Word
                    && ((t.text.chars().count() == 1 && t.text.chars().all(char::is_alphabetic))
                        || matches!(lexicon::get().lookup(&t.text), Some((lexicon::Kind::Greek, _))))
        }
        [v, u, s] => v.kind == K::Word && v.text.chars().count() == 1 && u.is_sym("_") && !u.sp_before && !s.sp_before,
        _ => false,
    }
}

/// Variable names in a binder expression (`x`, `x, y`).
pub fn vars_of(m: &M) -> Vec<String> {
    match m {
        M::Var(v) => vec![v.clone()],
        M::Seq(v) => v.iter().flat_map(vars_of).collect(),
        M::Sym(s) if s.starts_with('\\') && !s.starts_with("\\math") => vec![s.clone()],
        _ => Vec::new(),
    }
}

// ------------------------------------------------------------------------------------ display math

/// Remove `by …` tags from explicit display tokens: returns the remaining tokens and, for each tag,
/// the token index (in the remaining stream) where it was attached.
pub fn strip_tags(inner: &[Tok]) -> (Vec<Tok>, Vec<(usize, String)>) {
    let chain_op = |t: &Tok| {
        (t.kind == K::Sym && matches!(t.text.as_str(), "=" | "<" | ">" | "<=" | ">=" | "==" | "!=" | "=>" | "<=>"))
            || matches!(t.lower().as_str(), "implies" | "iff" | "then" | "if and only if")
    };
    let mut out = Vec::new();
    let mut tags = Vec::new();
    let mut i = 0;
    let mut depth = 0;
    while i < inner.len() {
        let t = &inner[i];
        if t.kind == K::Sym {
            match t.text.as_str() {
                "(" | "[" | "{" => depth += 1,
                ")" | "]" | "}" => depth -= 1,
                _ => {}
            }
        }
        if depth == 0 && t.is_kw("by") {
            let mut j = i + 1;
            let mut words = Vec::new();
            while j < inner.len() && !inner[j].is_sym(",") && !chain_op(&inner[j]) {
                words.push(inner[j].clone());
                j += 1;
            }
            let tag = match theorems::match_at(&words) {
                Some((id, n)) if n == words.len() => theorems::get(id).name.trim_start_matches("the ").to_string(),
                _ => glue_text(&words),
            };
            tags.push((out.len(), tag));
            if j < inner.len() && inner[j].is_sym(",") {
                j += 1;
            }
            i = j;
            continue;
        }
        out.push(t.clone());
        i += 1;
    }
    (out, tags)
}

/// Source text of tokens, with spaces only where the source had them ("non-contradiction").
fn glue_text(toks: &[Tok]) -> String {
    let mut s = String::new();
    for (k, t) in toks.iter().enumerate() {
        if k > 0 && t.sp_before {
            s.push(' ');
        }
        s.push_str(&t.text);
    }
    s
}

/// Render a standalone `$…$`: `align*` for long or tagged chains, `\[…\]` otherwise.
pub fn display(inner: &[Tok], multiline: bool, term: Option<&str>) -> String {
    let (toks, tags) = strip_tags(inner);
    let punct = match term {
        Some(".") => ".",
        Some(",") => ",",
        _ => "",
    };
    let Some(mut m) = explicit(&toks) else {
        return format!("\\[\n  {}{punct}\n\\]", explicit_lenient(&toks));
    };
    if let M::Chain(_, steps) = &mut m {
        for (pos, tag) in &tags {
            // attach to the step whose right side ends at (or just before) the tag
            if let Some(st) = steps.iter_mut().filter(|s| s.end <= *pos).last() {
                st.tag = Some(tag.clone());
            }
        }
    }
    let steps_n = if let M::Chain(_, s) = &m { s.len() } else { 0 };
    let tagged = !tags.is_empty();
    let long = math::width(&m) > 60;
    if let M::Chain(first, steps) = &m
        && steps_n >= 2
        && (tagged || long || multiline)
    {
        let mut s = String::from("\\begin{align*}\n");
        for (k, st) in steps.iter().enumerate() {
            s.push_str("  ");
            if k == 0 {
                s.push_str(&math::tex(first));
                s.push(' ');
            }
            s.push('&');
            s.push_str(&st.op);
            s.push(' ');
            let mut rhs = String::new();
            math::emit_rhs(&st.op, &st.rhs, &mut rhs);
            s.push_str(&rhs);
            if k + 1 == steps.len() {
                s.push_str(punct);
            }
            if let Some(t) = &st.tag {
                s.push_str(&format!(" \\tag*{{{}}}", math::escape_text(t)));
            }
            if k + 1 < steps.len() {
                s.push_str(" \\\\");
            }
            s.push('\n');
        }
        s.push_str("\\end{align*}");
        return s;
    }
    let tag = tags.last().map(|(_, t)| format!(" \\tag*{{{}}}", math::escape_text(t))).unwrap_or_default();
    if tagged {
        return format!("\\begin{{equation*}}\n  {}{punct}{tag}\n\\end{{equation*}}", math::tex(&m));
    }
    format!("\\[\n  {}{punct}\n\\]", math::tex(&m))
}

/// Subject–verb agreement of an existential with its binder: "there is $l$" / "there are integers
/// $m, n$", "there exists $x$" / "there exist $x, y$" (whichever phrasing the bank picked).
fn agree(s: String, plural: bool) -> String {
    let swaps: &[(&str, &str)] = if plural {
        &[("here exists ", "here exist "), ("here is ", "here are ")]
    } else {
        &[("here exist ", "here exists "), ("here are ", "here is ")]
    };
    // only the binder's verb (the first one), never a later "there are" inside the condition
    let at = |s: &str, pat: &str| s.find(pat);
    let first = swaps.iter().filter_map(|(a, b)| at(&s, a).map(|i| (i, *a, *b))).min_by_key(|x| x.0);
    match first {
        Some((i, a, b)) => format!("{}{}{}", &s[..i], b, &s[i + a.len()..]),
        None => s,
    }
}

/// Does a rendered binder name several things ("integers $m, n$", "$x$ and $y$", "sets $A$")?
/// Then "there exist" / "for all", not "there exists" / "for every".
fn plural_binder(x: &str) -> bool {
    let mut prose = String::new();
    let mut in_math = false;
    let mut math_commas = false;
    let mut depth = 0i32;
    for c in x.chars() {
        if c == '$' {
            in_math = !in_math;
            continue;
        }
        if in_math {
            match c {
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' => depth -= 1,
                ',' if depth == 0 => math_commas = true,
                _ => {}
            }
        } else {
            prose.push(c);
        }
    }
    const PLURALS: &[&str] = &[
        "integers",
        "reals",
        "numbers",
        "naturals",
        "rationals",
        "sets",
        "primes",
        "elements",
        "functions",
        "sequences",
        "points",
        "vectors",
        "matrices",
        "polynomials",
        "subsets",
        "values",
        "solutions",
        "pairs",
        "positive",
        "nonnegative",
    ];
    let words: Vec<&str> = prose.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
    math_commas && !words.iter().any(|w| *w == "a" || *w == "an")
        || words.iter().any(|w| PLURALS.contains(w) && *w != "positive" && *w != "nonnegative")
        || words.contains(&"and")
}

/// A hyphenated word starting at `i`: "4-digit", "AM-GM", "L-shaped", "sigma-algebra" (a Greek
/// letter before the hyphen is typeset: $\sigma$-algebra). Not "n-1" or "x-y" (subtraction) and not
/// ordinals ("n-th").
fn hyphen_compound(toks: &[Tok], i: usize) -> Option<(Piece, usize)> {
    let t = toks.get(i)?;
    if !matches!(t.kind, K::Word | K::Num) {
        return None;
    }
    let word_after = |k: usize| -> bool {
        let (Some(d), Some(w)) = (toks.get(k), toks.get(k + 1)) else { return false };
        d.is_sym("-")
            && !d.sp_before
            && !w.sp_before
            && w.kind == K::Word
            && w.text.chars().all(char::is_alphabetic)
            && !math::is_ordinal_suffix(&w.text)
            && (w.text.chars().count() >= 3
                || w.text.chars().count() >= 2 && t.text.chars().all(|c| c.is_ascii_uppercase()))
    };
    if !word_after(i + 1) {
        return None;
    }
    if t.kind == K::Word && t.text.chars().count() == 1 && t.text.chars().all(|c| c.is_ascii_lowercase()) {
        // "x-axis" is a word; but "x-intercept"? also a word. Single lowercase letters are fine.
    }
    let mut end = i + 1;
    let mut rest = String::new();
    while word_after(end) {
        rest.push('-');
        rest.push_str(&toks[end + 1].text);
        end += 2;
    }
    let piece = match lexicon::get().lookup(&t.text) {
        Some((lexicon::Kind::Greek, tex)) => Piece::Tex(format!("${tex}${rest}")),
        _ => Piece::Prose(format!("{}{rest}", t.text)),
    };
    Some((piece, end))
}
