//! Math expressions: tokens → [`M`] → LaTeX (STEX-DESIGN §3, §9).
//!
//! Two modes:
//! - **prose mode** (math runs found inside sentences): logic words (`and`, `or`, `implies`, …)
//!   and other prose words *end* the expression, so the sentence parser can turn them into prose.
//! - **explicit mode** (inside `$…$`): logic words and quantifiers become symbols, unknown words
//!   become `\text{…}`, and spaced juxtaposition is multiplication.
//!
//! Precedence, low → high: `iff`/`implies` chains < `or`/`xor`/`nor` < `and`/`nand` < `not` <
//! relation chains (`= < ∈ ⊆ ≡ → :` …, with `mod` after `≡` → `\pmod`) < set operators (∪ ∩ ∖ △)
//! < `+ - ± ∓` < `* / · × over choose mod %` and implicit multiplication < unary minus <
//! `^ _` < postfix (`!`, `'`, `%`, complement) and calls `f(x)`.

use crate::lex::{K, Tok};
use crate::lexicon::{self, Kind};

#[derive(Clone, Debug, PartialEq)]
pub enum M {
    Num(String),
    Var(String),
    Sym(String),
    Text(String),
    Raw(String),
    Neg(Box<M>),
    Pm(String, Box<M>),
    Bin(String, Box<M>, Box<M>),
    Implicit(Box<M>, Box<M>),
    Frac(Box<M>, Box<M>),
    Binom(Box<M>, Box<M>),
    Mod(Box<M>, Box<M>),
    Pow(Box<M>, Box<M>),
    Sub(Box<M>, Box<M>),
    Post(String, Box<M>),
    Call(Box<M>, Vec<M>, char),
    Func(String, Box<M>),
    Group(Box<M>, char),
    Abs(Box<M>),
    Norm(Box<M>),
    Floor(Box<M>, bool),
    Set(Vec<M>),
    SetB(Box<M>, Box<M>),
    Sqrt(Option<Box<M>>, Box<M>),
    /// `\hat{x}`, `\bar{x}`, ...
    Accent(String, Box<M>),
    /// A property stated in words ("n is even"): (Lean predicate, subject). Only produced for the
    /// Lean translation; rendered as `\operatorname{pred}(x)` if it ever reaches LaTeX.
    Pred(String, Box<M>),
    Big {
        op: String,
        lower: Option<Box<M>>,
        upper: Option<Box<M>>,
        body: Option<Box<M>>,
    },
    Int {
        lower: Option<Box<M>>,
        upper: Option<Box<M>>,
        body: Box<M>,
        var: String,
    },
    Lim {
        sub: Box<M>,
        body: Box<M>,
    },
    Deriv {
        var: String,
        body: Box<M>,
    },
    Chain(Box<M>, Vec<Step>),
    Not(Box<M>),
    Quant {
        q: String,
        vars: Box<M>,
        body: Option<Box<M>>,
    },
    Seq(Vec<M>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Step {
    pub op: String,
    pub rhs: M,
    /// `\tag*{…}` text from a `by …` annotation (display math only).
    pub tag: Option<String>,
    /// Token index just after this step's right-hand side (used to attach tags).
    pub end: usize,
}

pub fn is_logic_op(op: &str) -> bool {
    matches!(op, "\\implies" | "\\iff" | "\\impliedby")
}

/// Negated form of a relation operator (`not x = 5` → `x \neq 5`).
pub fn negate_op(op: &str) -> Option<&'static str> {
    Some(match op {
        "=" => "\\neq",
        "\\neq" => "=",
        "\\equiv" => "\\not\\equiv",
        "<" => "\\not<",
        ">" => "\\not>",
        "\\leq" => "\\not\\leq",
        "\\geq" => "\\not\\geq",
        "\\in" => "\\notin",
        "\\notin" => "\\in",
        "\\subseteq" => "\\not\\subseteq",
        "\\subsetneq" => "\\not\\subsetneq",
        "\\supseteq" => "\\not\\supseteq",
        "\\mid" => "\\nmid",
        "\\nmid" => "\\mid",
        "\\sim" => "\\nsim",
        "\\approx" => "\\not\\approx",
        "\\cong" => "\\ncong",
        _ => return None,
    })
}

/// Relation operator for a symbol token.
fn sym_rel(s: &str) -> Option<&'static str> {
    Some(match s {
        "=" => "=",
        "==" => "\\equiv",
        "!=" => "\\neq",
        "!==" => "\\not\\equiv",
        "<" => "<",
        ">" => ">",
        "<=" => "\\leq",
        ">=" => "\\geq",
        ":=" => ":=",
        "->" => "\\to",
        "|->" => "\\mapsto",
        "<-" => "\\leftarrow",
        "~" => "\\sim",
        "≤" => "\\leq",
        "≥" => "\\geq",
        "≠" => "\\neq",
        "∈" => "\\in",
        "⊆" => "\\subseteq",
        _ => return None,
    })
}

/// Relations whose right side is in "set position" (single capitals N Z Q R C F → blackboard).
fn set_position(op: &str) -> bool {
    matches!(op, "\\in" | "\\notin" | "\\subseteq" | "\\subsetneq" | "\\supseteq" | ":" | "\\to" | "\\not\\subseteq")
}

pub struct Parser<'a> {
    t: &'a [Tok],
    pub i: usize,
    explicit: bool,
    stops: Vec<String>,
    set_ctx: bool,
    /// Inside brackets: commas and stop words of outer contexts don't apply.
    depth: usize,
    /// Spaced juxtaposition may multiply in prose (`2 sqrt(x)`); off for quantifier binders
    /// ("for each x P(x)").
    spaced: bool,
}

impl<'a> Parser<'a> {
    pub fn new(t: &'a [Tok], start: usize, explicit: bool) -> Parser<'a> {
        Parser { t, i: start, explicit, stops: Vec::new(), set_ctx: false, depth: 0, spaced: true }
    }

    pub fn no_spaced_product(mut self) -> Self {
        self.spaced = false;
        self
    }

    pub fn with_stops(mut self, stops: &[&str]) -> Self {
        self.stops = stops.iter().map(|s| s.to_string()).collect();
        self
    }

    fn peek(&self) -> Option<&'a Tok> {
        self.t.get(self.i)
    }
    fn peek_at(&self, k: usize) -> Option<&'a Tok> {
        self.t.get(self.i + k)
    }
    fn at_word(&self, w: &str) -> bool {
        self.peek().is_some_and(|t| t.is_kw(w))
    }
    fn at_sym(&self, s: &str) -> bool {
        self.peek().is_some_and(|t| t.is_sym(s))
    }
    fn at_end(&self) -> bool {
        self.i >= self.t.len()
    }

    /// Stop words end an expression at operator positions.
    fn stopped(&self) -> bool {
        match self.peek() {
            None => true,
            Some(t) => {
                if matches!(t.kind, K::Newline | K::Para) {
                    return true;
                }
                if self.depth == 0 && t.kind == K::Word {
                    let l = t.lower();
                    if self.stops.contains(&l) {
                        return true;
                    }
                    if !self.explicit && lexicon::is_prose_stop(&l) {
                        return true;
                    }
                    if l == "by" {
                        return true;
                    }
                }
                self.depth == 0 && t.kind == K::Sym && self.stops.contains(&t.text)
            }
        }
    }

    /// Parse the longest expression from the current position.
    pub fn expr(&mut self) -> Option<M> {
        self.logic_chain()
    }

    // ------------------------------------------------------------------ logic (explicit mode)

    fn logic_op(&self) -> Option<&'static str> {
        if !self.explicit || self.stopped_hard() {
            return None;
        }
        let t = self.peek()?;
        match (&t.kind, t.lower().as_str()) {
            (K::Word, "implies" | "then") => Some("\\implies"),
            (K::Word, "iff" | "if and only if") => Some("\\iff"),
            (K::Sym, "=>") => Some("\\implies"),
            (K::Sym, "<=>" | "<->") => Some("\\iff"),
            _ => None,
        }
    }

    /// Like `stopped` but ignores prose stops (used where explicit-mode words are operators).
    fn stopped_hard(&self) -> bool {
        match self.peek() {
            None => true,
            Some(t) => {
                matches!(t.kind, K::Newline | K::Para)
                    || self.depth == 0
                        && (t.kind == K::Word || t.kind == K::Sym)
                        && self.stops.iter().any(|s| *s == t.lower())
            }
        }
    }

    fn logic_chain(&mut self) -> Option<M> {
        let first = self.or_expr()?;
        let mut steps = Vec::new();
        while let Some(op) = self.logic_op() {
            let save = self.i;
            self.i += 1;
            match self.or_expr() {
                Some(rhs) => steps.push(Step { op: op.into(), rhs, tag: None, end: self.i }),
                None => {
                    self.i = save;
                    break;
                }
            }
        }
        Some(if steps.is_empty() { first } else { M::Chain(Box::new(first), steps) })
    }

    fn binlogic(&mut self, words: &[(&str, &'static str)], next: fn(&mut Self) -> Option<M>) -> Option<M> {
        let mut lhs = next(self)?;
        loop {
            if !self.explicit || self.stopped_hard() {
                break;
            }
            let Some(t) = self.peek() else { break };
            let l = t.lower();
            let Some((_, op)) = words.iter().find(|(w, _)| *w == l) else { break };
            let save = self.i;
            self.i += 1;
            match next(self) {
                Some(rhs) => lhs = M::Bin(op.to_string(), Box::new(lhs), Box::new(rhs)),
                None => {
                    self.i = save;
                    break;
                }
            }
        }
        Some(lhs)
    }

    fn or_expr(&mut self) -> Option<M> {
        self.binlogic(
            &[
                ("or", "\\lor"),
                ("||", "\\lor"),
                ("xor", "\\oplus"),
                ("^^", "\\oplus"),
                ("nor", "\\downarrow"),
                ("!||", "\\downarrow"),
            ],
            Self::and_expr,
        )
    }

    fn and_expr(&mut self) -> Option<M> {
        self.binlogic(
            &[("and", "\\land"), ("&&", "\\land"), ("nand", "\\uparrow"), ("!&&", "\\uparrow")],
            Self::not_expr,
        )
    }

    fn not_expr(&mut self) -> Option<M> {
        let t = self.peek()?;
        let is_not_word = self.explicit && t.is_kw("not");
        // `!x = 5`: `!` glued to what follows
        let is_bang = t.is_sym("!") && !t.sp_after && !self.prev_is_operand();
        if is_not_word || is_bang {
            let save = self.i;
            self.i += 1;
            if let Some(inner) = self.not_expr() {
                return Some(negate(inner));
            }
            self.i = save;
            return None;
        }
        self.quant_expr()
    }

    fn prev_is_operand(&self) -> bool {
        self.i > 0
            && self.t.get(self.i - 1).is_some_and(|p| {
                !self.t[self.i].sp_before
                    && (matches!(p.kind, K::Num | K::Word) || p.is_sym(")") || p.is_sym("]") || p.is_sym("}"))
            })
    }

    fn quant_expr(&mut self) -> Option<M> {
        if self.explicit
            && let Some(t) = self.peek()
        {
            let q = match t.lower().as_str() {
                "exists" | "there exists" | "there exist" | "there is" | "there are" | "for some" => Some("\\exists"),
                "exists unique"
                | "there exists a unique"
                | "there is a unique"
                | "there exists unique"
                | "there is exactly one" => Some("\\exists!"),
                "forall" | "for all" | "for every" | "for each" | "for any" => Some("\\forall"),
                "there does not exist" | "there is no" => Some("\\nexists"),
                _ => None,
            };
            let q = q.or(if t.is_sym("exists!") { Some("\\exists!") } else { None });
            if let Some(q) = q {
                let save = self.i;
                self.i += 1;
                if self.at_sym("!") && !self.t[self.i].sp_before {
                    self.i += 1;
                }
                let q = if q == "\\exists" && self.t.get(save + 1).is_some_and(|n| n.is_sym("!") && !n.sp_before) {
                    "\\exists!"
                } else {
                    q
                };
                let old = std::mem::take(&mut self.stops);
                self.stops = old.clone();
                self.stops.extend(["such that", "s.t.", "st", ",", "with", "where", ":"].map(String::from));
                let vars = self.rel_chain();
                self.stops = old;
                let Some(vars) = vars else {
                    self.i = save;
                    return None;
                };
                let mut body = None;
                if let Some(n) = self.peek()
                    && (n.is_sym(",")
                        || n.is_sym(":")
                        || matches!(n.lower().as_str(), "such that" | "s.t." | "st" | "with" | "where"))
                {
                    let s2 = self.i;
                    self.i += 1;
                    match self.logic_chain() {
                        Some(b) => body = Some(Box::new(b)),
                        None => self.i = s2,
                    }
                }
                return Some(M::Quant { q: q.into(), vars: Box::new(vars), body });
            }
        }
        self.rel_chain()
    }

    // ------------------------------------------------------------------ relations

    /// Relation at the cursor: (latex op, tokens consumed). Handles `not =`, `not in`, words.
    fn rel_op(&self) -> Option<(String, usize)> {
        if self.stopped_hard() {
            return None;
        }
        let t = self.peek()?;
        let mut neg = false;
        let mut k = 0;
        let mut tok = t;
        if t.is_kw("not") {
            neg = true;
            k = 1;
            tok = self.peek_at(1)?;
        }
        let op: String = match &tok.kind {
            K::Sym if tok.text == "|" && tok.sp_before && tok.sp_after => "\\mid".into(),
            K::Sym if tok.text == ":" && !neg => ":".into(),
            K::Sym => sym_rel(&tok.text)?.into(),
            K::Word => match lexicon::get().lookup(&tok.text) {
                Some((Kind::Rel | Kind::Arrow, tex)) => tex.to_string(),
                _ => return None,
            },
            _ => return None,
        };
        if neg {
            return Some((negate_op(&op)?.to_string(), k + 1));
        }
        // the word "to" is an arrow only when it isn't a stop word here (sum/integral bounds); the
        // `|` / `:` of a set-builder `{x | …}` is a separator, not "divides"
        if self.stops.iter().any(|s| *s == tok.lower() || *s == tok.text) {
            return None;
        }
        Some((op, k + 1))
    }

    fn rel_chain(&mut self) -> Option<M> {
        let first = self.setop()?;
        // "x is between a and b" → a ≤ x ≤ b ("strictly between" → a < x < b)
        if let Some(t) = self.peek()
            && t.kind == K::Word
            && let l = t.lower()
            && matches!(
                l.as_str(),
                "between"
                    | "is between"
                    | "lies between"
                    | "strictly between"
                    | "is strictly between"
                    | "lies strictly between"
            )
        {
            let save = self.i;
            let op = if l.contains("strictly") { "<" } else { "\\leq" };
            self.i += 1;
            let low = self.scoped(&["and"], |p| p.setop());
            if let Some(low) = low
                && self.at_word("and")
            {
                self.i += 1;
                if let Some(high) = self.setop() {
                    let end = self.i;
                    return Some(M::Chain(
                        Box::new(low),
                        vec![
                            Step { op: op.into(), rhs: first, tag: None, end },
                            Step { op: op.into(), rhs: high, tag: None, end },
                        ],
                    ));
                }
            }
            self.i = save;
        }
        let mut steps: Vec<Step> = Vec::new();
        while let Some((op, n)) = self.rel_op() {
            // "x is divisible by 7" stays prose in prose mode (Lean still reads it as 7 ∣ x)
            if !self.explicit && op.contains("divisible") {
                break;
            }
            let save = self.i;
            self.i += n;
            // "g: B to C": after a set variable, C is a set variable too, not ℂ
            let lhs_var = steps.last().map(|s| &s.rhs).unwrap_or(&first);
            if op == "\\to"
                && matches!(lhs_var, M::Var(v) if v.len() == 1 && v.chars().all(|c| c.is_ascii_uppercase()) && lexicon::number_set(v).is_none())
                && self.peek().is_some_and(|t| t.text == "C")
            {
                declare("C");
            }
            let old = self.set_ctx;
            self.set_ctx = set_position(&op);
            // "x_1 + x_2 = 10 in nonnegative integers": a prose "in" after an equation
            if op == "\\in"
                && !self.explicit
                && steps.iter().any(|s| s.op == "=")
                && self.peek().is_some_and(|t| {
                    t.kind == K::Word && t.text.contains(' ')
                        || t.text.len() > 1 && t.kind == K::Word && lexicon::number_set(&t.text).is_none()
                })
            {
                self.i = save;
                self.set_ctx = old;
                break;
            }
            let rhs = self.setop();
            self.set_ctx = old;
            // "n == sum … of a_i mod 9": the modulus belongs to the congruence
            let rhs = if op == "\\equiv" { rhs.map(lift_mod) } else { rhs };
            match rhs {
                Some(rhs) => steps.push(Step { op, rhs, tag: None, end: self.i }),
                None => {
                    self.i = save;
                    break;
                }
            }
        }
        Some(if steps.is_empty() { first } else { M::Chain(Box::new(first), steps) })
    }

    fn setop(&mut self) -> Option<M> {
        let mut lhs = self.additive()?;
        loop {
            if self.stopped() {
                break;
            }
            let Some(t) = self.peek() else { break };
            // `\` is always set difference (`A\B`, `A \ B`); raw LaTeX goes through `latex{…}`
            let op: String = match &t.kind {
                K::Word => match lexicon::get().lookup(&t.text) {
                    Some((Kind::SetOp, tex)) => tex.to_string(),
                    _ => break,
                },
                K::Sym if t.text == "\\" => "\\setminus".into(),
                _ => break,
            };
            let save = self.i;
            self.i += 1;
            match self.additive() {
                Some(rhs) => lhs = M::Bin(op, Box::new(lhs), Box::new(rhs)),
                None => {
                    self.i = save;
                    break;
                }
            }
        }
        Some(lhs)
    }

    fn additive(&mut self) -> Option<M> {
        let mut lhs = self.multiplicative()?;
        loop {
            if self.stopped() {
                break;
            }
            let Some(t) = self.peek() else { break };
            let op = match t.text.as_str() {
                "+" if t.kind == K::Sym => "+",
                // "n-th", "(k+1)-st": an ordinal, not a subtraction
                "-" if t.kind == K::Sym
                    && !t.sp_before
                    && self.peek_at(1).is_some_and(|n| !n.sp_before && is_ordinal_suffix(&n.text)) =>
                {
                    break;
                }
                "-" if t.kind == K::Sym => "-",
                "+-" | "±" if t.kind == K::Sym => "\\pm",
                "-+" | "∓" if t.kind == K::Sym => "\\mp",
                _ if t.kind == K::Word => match lexicon::get().lookup(&t.text) {
                    Some((Kind::Add, tex)) => tex,
                    _ => break,
                },
                _ => break,
            };
            let save = self.i;
            self.i += 1;
            match self.multiplicative() {
                Some(rhs) => lhs = M::Bin(op.to_string(), Box::new(lhs), Box::new(rhs)),
                None => {
                    self.i = save;
                    break;
                }
            }
        }
        Some(lhs)
    }

    fn multiplicative(&mut self) -> Option<M> {
        let mut lhs = self.unary()?;
        loop {
            if self.stopped() {
                break;
            }
            let Some(t) = self.peek() else { break };
            enum Op {
                B(String),
                Frac,
                Binom,
                Mod,
                Implicit,
            }
            let op = match (&t.kind, t.lower().as_str()) {
                (K::Sym, "*" | "·" | "⋅") => Op::B("\\cdot".into()),
                (K::Sym, "/") => Op::Frac,
                (K::Sym, "%") if t.sp_before && t.sp_after => Op::Mod,
                (K::Word, "over") => Op::Frac,
                (K::Word, "choose") => Op::Binom,
                (K::Word, "mod") => Op::Mod,
                // `g o f`: composition (a variable named "o" between two functions is unheard of)
                (K::Word, "o" | "circ") if t.sp_before && t.sp_after && composable(&lhs) => Op::B("\\circ".into()),
                (K::Word, _) if matches!(lexicon::get().lookup(&t.text), Some((Kind::Bin | Kind::Div, _))) => {
                    match lexicon::get().lookup(&t.text) {
                        Some((Kind::Bin, tex)) => Op::B(tex.to_string()),
                        _ => Op::Frac,
                    }
                }
                _ if self.implicit_ok(&lhs) => Op::Implicit,
                _ => break,
            };
            let save = self.i;
            if !matches!(op, Op::Implicit) {
                self.i += 1;
            }
            let rhs = if matches!(op, Op::Mod) {
                // `x == 5 mod 6`: modulus binds like a factor
                self.unary()
            } else {
                self.unary()
            };
            match rhs {
                Some(rhs) => {
                    lhs = match op {
                        Op::B(tex) => M::Bin(tex, Box::new(lhs), Box::new(rhs)),
                        Op::Frac => M::Frac(Box::new(strip_parens(lhs)), Box::new(strip_parens(rhs))),
                        Op::Binom => M::Binom(Box::new(strip_parens(lhs)), Box::new(strip_parens(rhs))),
                        Op::Mod => M::Mod(Box::new(lhs), Box::new(rhs)),
                        Op::Implicit => M::Implicit(Box::new(lhs), Box::new(rhs)),
                    }
                }
                None => {
                    self.i = save;
                    break;
                }
            }
        }
        Some(lhs)
    }

    /// Juxtaposition is multiplication when the next atom is glued (`2x`, `(a)(b)`), anywhere in
    /// explicit mode, and across a space in prose when both sides are unmistakably math
    /// (`2 sqrt(ab)`, `a_i 10^i`, `e^a e^b`, `(1+x)^k (1+x)`, `|A| |B|`, `p_1 p_2 ... p_k`).
    fn implicit_ok(&self, lhs: &M) -> bool {
        let Some(t) = self.peek() else { return false };
        // after an ellipsis any factor continues the product: p_1 p_2 ... P
        let after_dots = {
            let mut r = lhs;
            while let M::Implicit(_, b) | M::Bin(_, _, b) = r {
                r = b;
            }
            is_dots(r)
        };
        if after_dots
            && !self.explicit
            && t.kind == K::Word
            && t.text.chars().count() == 1
            && t.text.chars().all(char::is_alphabetic)
        {
            return true;
        }
        // a set word ("distinct primes p_i") never multiplies
        let set_word = matches!(lhs, M::Sym(s) if s.starts_with("\\mathbb"));
        if self.spaced
            && !self.explicit
            && t.sp_before
            && !set_word
            && !matches!(lhs, M::Text(_))
            && self.math_certain_at(self.i)
        {
            return true;
        }
        if t.kind == K::Word && !t.sp_before && is_ordinal_suffix(&t.text) {
            return false; // "(k+1)th"
        }
        if t.kind == K::Sym && !matches!(t.text.as_str(), "(" | "[") {
            return false;
        }
        if t.kind == K::Sym && t.text == "(" && t.sp_before && !self.explicit {
            return false;
        }
        if t.kind == K::Word {
            let l = t.lower();
            if lexicon::is_keyword(&l)
                && lexicon::get().lookup(&t.text).is_none_or(|(k, _)| {
                    matches!(k, Kind::Rel | Kind::Arrow | Kind::Bin | Kind::Add | Kind::SetOp | Kind::Div | Kind::Post)
                })
            {
                return false;
            }
            if self.stops.contains(&l) || is_d_var(&t.text).is_some() && self.stops.iter().any(|s| s == "dx") {
                return false;
            }
        }
        if !matches!(t.kind, K::Num | K::Word | K::Sym | K::Sigil(_) | K::Latex(_)) {
            return false;
        }
        let glued = !t.sp_before;
        glued || (self.explicit && t.kind != K::Num)
    }

    /// Does the token at `i` start an atom that can only be math (not an English word)?
    fn math_certain_at(&self, i: usize) -> bool {
        let Some(t) = self.t.get(i) else { return false };
        let glued = |k: usize, s: &[&str]| {
            self.t.get(k).is_some_and(|n| !n.sp_before && n.kind == K::Sym && s.contains(&n.text.as_str()))
        };
        match &t.kind {
            K::Sym => {
                matches!(t.text.as_str(), "..." | "…" | "(")
                    || (t.text == "|" && self.t.get(i + 1).is_some_and(|n| !n.sp_before && n.kind != K::Sym))
            }
            K::Num => glued(i + 1, &["^", "!"]),
            K::Sigil(_) | K::Latex(_) => true,
            K::Word => {
                let w = t.text.as_str();
                let l = t.lower();
                if matches!(l.as_str(), "sqrt" | "cbrt") {
                    return true;
                }
                if matches!(l.as_str(), "sum" | "prod") {
                    return self.t.get(i + 1).is_some_and(|n| n.is_kw("from") || n.is_sym("_"));
                }
                if lexicon::is_keyword(&l) && lexicon::get().lookup(w).is_none() {
                    return false;
                }
                match lexicon::get().lookup(w) {
                    Some((Kind::Greek, _)) => true,
                    Some((Kind::Func, _)) => glued(i + 1, &["("]),
                    Some((Kind::Big, _)) => true,
                    Some(_) => false,
                    None => {
                        if l.len() == 3 && l.ends_with("rt") && glued(i + 1, &["("]) {
                            return true;
                        }
                        let single = w.chars().count() == 1 && w.chars().all(|c| c.is_ascii_alphabetic());
                        // x_1, e^x, f(x), z bar
                        single
                            && (glued(i + 1, &["_", "^", "("])
                                || self.t.get(i + 1).is_some_and(|n| {
                                    n.sp_before && matches!(lexicon::get().lookup(&n.text), Some((Kind::Accent, _)))
                                }))
                    }
                }
            }
            _ => false,
        }
    }

    fn unary(&mut self) -> Option<M> {
        let t = self.peek()?;
        if t.kind == K::Sym {
            match t.text.as_str() {
                "-" => {
                    let save = self.i;
                    self.i += 1;
                    if let Some(x) = self.unary() {
                        return Some(M::Neg(Box::new(x)));
                    }
                    self.i = save;
                    return None;
                }
                "+-" | "-+" | "±" | "∓" => {
                    let op = if matches!(t.text.as_str(), "+-" | "±") { "\\pm" } else { "\\mp" };
                    let save = self.i;
                    self.i += 1;
                    if let Some(x) = self.unary() {
                        return Some(M::Pm(op.into(), Box::new(x)));
                    }
                    self.i = save;
                    return None;
                }
                _ => {}
            }
        }
        let base = self.atom()?;
        Some(self.postfix(base))
    }

    fn postfix(&mut self, mut base: M) -> M {
        while let Some(t) = self.peek() {
            match (&t.kind, t.text.as_str()) {
                (K::Sym, "^") | (K::Sym, "_") => {
                    let sup = t.text == "^";
                    let save = self.i;
                    self.i += 1;
                    let e = if self.at_sym("{") {
                        self.brace_group()
                    } else if self.at_sym("-") {
                        self.i += 1;
                        self.pow_atom().map(|a| M::Neg(Box::new(a)))
                    } else {
                        self.pow_atom()
                    };
                    match e {
                        Some(e) => {
                            let e = strip_parens(e);
                            base = if sup {
                                M::Pow(Box::new(base), Box::new(e))
                            } else {
                                M::Sub(Box::new(base), Box::new(e))
                            }
                        }
                        None => {
                            self.i = save;
                            break;
                        }
                    }
                }
                (K::Sym, "!") if !t.sp_before && !self.peek_at(1).is_some_and(|n| n.is_sym("=") && !n.sp_before) => {
                    self.i += 1;
                    base = M::Post("!".into(), Box::new(base));
                }
                (K::Sym, "'") if !t.sp_before => {
                    self.i += 1;
                    base = M::Post("'".into(), Box::new(base));
                }
                (K::Sym, "%") if !t.sp_before => {
                    self.i += 1;
                    base = M::Post("\\%".into(), Box::new(base));
                }
                (K::Word, _) if t.sp_before && matches!(lexicon::get().lookup(&t.text), Some((Kind::Accent, _))) => {
                    let Some((_, tex)) = lexicon::get().lookup(&t.text) else { break };
                    self.i += 1;
                    base = M::Accent(tex.to_string(), Box::new(base));
                }
                (K::Word, _) if matches!(lexicon::get().lookup(&t.text), Some((Kind::Post, _))) => {
                    let Some((_, tex)) = lexicon::get().lookup(&t.text) else { break };
                    self.i += 1;
                    base = M::Post(tex.to_string(), Box::new(base));
                }
                _ => break,
            }
        }
        base
    }

    /// Operand of `^` / `_`: an atom with its own postfix `!` but no further `^`.
    fn pow_atom(&mut self) -> Option<M> {
        self.atom()
    }

    /// `{ … }` after `^`/`_`: grouping, not a set.
    fn brace_group(&mut self) -> Option<M> {
        let save = self.i;
        self.i += 1;
        self.depth += 1;
        let e = self.logic_chain();
        self.depth -= 1;
        if e.is_some() && self.at_sym("}") {
            self.i += 1;
            return e;
        }
        self.i = save;
        None
    }

    // ------------------------------------------------------------------ atoms

    fn atom(&mut self) -> Option<M> {
        let t = self.peek()?;
        match &t.kind {
            K::Num => {
                self.i += 1;
                // `3rt(5)`, `7rt 6`: n-th root
                if let Some(n) = self.peek()
                    && n.is_kw("rt")
                    && !n.sp_before
                {
                    self.i += 1;
                    let arg = self.root_arg()?;
                    return Some(M::Sqrt(Some(Box::new(M::Num(t.text.clone()))), Box::new(arg)));
                }
                Some(M::Num(t.text.clone()))
            }
            K::Sigil(c) => {
                self.i += 1;
                let cmd = match c {
                    '*' => "mathbb",
                    '&' => "mathcal",
                    '~' => "mathscr",
                    '#' => "mathtt",
                    _ => "mathfrak",
                };
                Some(M::Sym(format!("\\{cmd}{{{}}}", t.text)))
            }
            K::Latex(raw) => {
                self.i += 1;
                Some(M::Raw(raw.clone()))
            }
            K::Math(inner) if self.depth > 0 || !self.explicit => {
                // an explicit span inside a prose run: parse it explicitly
                let mut p = Parser::new(inner, 0, true);
                let m = p.expr()?;
                if p.i < inner.len() {
                    return None;
                }
                self.i += 1;
                Some(m)
            }
            K::Sym => self.sym_atom(),
            K::Word => self.word_atom(),
            _ => None,
        }
    }

    fn sym_atom(&mut self) -> Option<M> {
        let t = self.peek()?;
        match t.text.as_str() {
            "(" | "[" => {
                let close = if t.text == "(" { ")" } else { "]" };
                let save = self.i;
                self.i += 1;
                let items = self.list_until(close)?;
                if !self.at_sym(close) {
                    self.i = save;
                    return None;
                }
                self.i += 1;
                let br = t.text.chars().next().unwrap_or('(');
                let inner = if items.len() == 1 { items.into_iter().next()? } else { M::Seq(items) };
                Some(M::Group(Box::new(inner), br))
            }
            "{" => {
                let save = self.i;
                self.i += 1;
                if self.at_sym("}") {
                    self.i += 1;
                    return Some(M::Set(Vec::new()));
                }
                self.depth += 1;
                let old = std::mem::take(&mut self.stops);
                self.stops = vec!["|".into(), ":".into(), ",".into()];
                let first = self.logic_chain();
                self.stops = old;
                self.depth -= 1;
                let first = match first {
                    Some(f) => f,
                    None => {
                        self.i = save;
                        return None;
                    }
                };
                if self.at_sym("|") || self.at_sym(":") {
                    self.i += 1;
                    self.depth += 1;
                    let cond = self.logic_chain();
                    self.depth -= 1;
                    // {x in Z | x is even}: the rest of the condition in words
                    let cond = cond.map(|c| {
                        let mut words = Vec::new();
                        let mut k = self.i;
                        while let Some(t) = self.t.get(k) {
                            if t.kind != K::Word || t.is_sym("}") {
                                break;
                            }
                            words.push(t.text.clone());
                            k += 1;
                        }
                        if !words.is_empty() && self.t.get(k).is_some_and(|t| t.is_sym("}")) {
                            self.i = k;
                            M::Implicit(Box::new(c), Box::new(M::Text(words.join(" "))))
                        } else {
                            c
                        }
                    });
                    if let Some(c) = cond
                        && self.at_sym("}")
                    {
                        self.i += 1;
                        return Some(M::SetB(Box::new(first), Box::new(c)));
                    }
                    self.i = save;
                    return None;
                }
                let mut items = vec![first];
                while self.at_sym(",") {
                    self.i += 1;
                    self.depth += 1;
                    let e = self.logic_chain();
                    self.depth -= 1;
                    match e {
                        Some(e) => items.push(e),
                        None => {
                            self.i = save;
                            return None;
                        }
                    }
                }
                if self.at_sym("}") {
                    self.i += 1;
                    return Some(M::Set(items));
                }
                self.i = save;
                None
            }
            "|" if self.peek_at(1).is_some_and(|n| n.is_sym("|") && !n.sp_before) => {
                // `||v||` (the lexer splits glued `||`): a norm, else nested absolute values
                let save = self.i;
                self.i += 2;
                self.depth += 1;
                let old = std::mem::take(&mut self.stops);
                self.stops = vec!["|".into()];
                let inner = self.setop();
                self.stops = old;
                self.depth -= 1;
                if let Some(inner) = inner
                    && self.at_sym("|")
                    && self.peek_at(1).is_some_and(|n| n.is_sym("|") && !n.sp_before)
                {
                    self.i += 2;
                    return Some(M::Norm(Box::new(inner)));
                }
                self.i = save;
                self.abs_atom()
            }
            "|" | "||" => self.abs_atom(),
            "∞" => {
                self.i += 1;
                Some(M::Sym("\\infty".into()))
            }
            "..." | "…" => {
                self.i += 1;
                Some(M::Sym(DOTS.into()))
            }
            "∅" => {
                self.i += 1;
                Some(M::Sym("\\varnothing".into()))
            }
            _ => None,
        }
    }

    /// `|x|` (or `||v||` lexed as one token).
    fn abs_atom(&mut self) -> Option<M> {
        let t = self.peek()?;
        let norm = t.text == "||";
        // `||` with nothing inside is "or"; `|` needs at least one token before the closing pipe
        let save = self.i;
        self.i += 1;
        self.depth += 1;
        let old = std::mem::take(&mut self.stops);
        self.stops = vec![t.text.clone()];
        let inner = self.setop();
        self.stops = old;
        self.depth -= 1;
        if let Some(inner) = inner
            && self.at_sym(&t.text)
        {
            self.i += 1;
            return Some(if norm { M::Norm(Box::new(inner)) } else { M::Abs(Box::new(inner)) });
        }
        self.i = save;
        None
    }

    /// Comma-separated expressions until `close` (not consumed).
    fn list_until(&mut self, close: &str) -> Option<Vec<M>> {
        self.depth += 1;
        let old = std::mem::take(&mut self.stops);
        self.stops = vec![",".into(), close.into()];
        let mut items = Vec::new();
        let r = loop {
            match self.logic_chain() {
                Some(e) => items.push(e),
                None => break None,
            }
            if self.at_sym(",") {
                self.i += 1;
                continue;
            }
            break Some(());
        };
        self.stops = old;
        self.depth -= 1;
        r.map(|_| items)
    }

    fn word_atom(&mut self) -> Option<M> {
        let t = self.peek()?;
        let w = t.text.as_str();
        let l = t.lower();
        let glued_paren = self.peek_at(1).is_some_and(|n| n.is_sym("(") && !n.sp_before);
        // stex constructs
        match l.as_str() {
            "sqrt" => {
                self.i += 1;
                let arg = self.root_arg()?;
                return Some(M::Sqrt(None, Box::new(arg)));
            }
            "cbrt" => {
                self.i += 1;
                let arg = self.root_arg()?;
                return Some(M::Sqrt(Some(Box::new(M::Num("3".into()))), Box::new(arg)));
            }
            "sum" | "summation" | "prod" | "product" | "bigcup" | "bigcap" => return self.big_op(),
            "union" | "intersection" | "intersect"
                if self
                    .peek_at(1)
                    .is_some_and(|n| n.is_kw("from") || n.is_kw("over") || (n.is_sym("_") && !n.sp_before)) =>
            {
                return self.big_op();
            }
            "comp" | "complement" if glued_paren => {
                // comp(A) = A^c
                self.i += 1;
                let Some(M::Group(inner, _)) = self.sym_atom() else { return None };
                return Some(M::Post("^{c}".into(), inner));
            }
            "integral" | "integrate" | "int" => return self.integral(),
            "lim" | "limit" => return self.limit(),
            "derivative" => return self.derivative(),
            "abs" | "floor" | "ceil" | "norm" if glued_paren => {
                self.i += 1;
                let Some(M::Group(inner, _)) = self.sym_atom() else { return None };
                return Some(match l.as_str() {
                    "abs" => M::Abs(inner),
                    "norm" => M::Norm(inner),
                    "floor" => M::Floor(inner, true),
                    _ => M::Floor(inner, false),
                });
            }
            "d" if self.peek_at(1).is_some_and(|n| n.is_sym("/") && !n.sp_before)
                && self.peek_at(2).is_some_and(|n| !n.sp_before && is_d_var(&n.text).is_some()) =>
            {
                let var = is_d_var(&self.t[self.i + 2].text)?.to_string();
                self.i += 3;
                let body = self.unary()?;
                return Some(M::Deriv { var, body: Box::new(body) });
            }
            _ => {}
        }
        // n-th root written as one word: nrt(x)
        if l.len() == 3
            && l.ends_with("rt")
            && l.as_bytes()[0].is_ascii_alphabetic()
            && !matches!(l.as_str(), "wrt" | "art")
        {
            self.i += 1;
            let arg = self.root_arg()?;
            return Some(M::Sqrt(Some(Box::new(M::Var(w[..1].to_string()))), Box::new(arg)));
        }
        // number sets in set position: x in Z, f: N to N
        let prev_set_word = self.i > 0
            && self.depth == 0
            && self
                .t
                .get(self.i - 1)
                .is_some_and(|p| p.kind == K::Word && matches!(p.lower().as_str(), "in" | "over" | "on"))
            && self.peek_at(1).is_some_and(|n| n.is_sym("^") && !n.sp_before);
        if (self.set_ctx || prev_set_word)
            && !glued_paren
            && !is_declared(w)
            && let Some(bb) = lexicon::number_set(w)
        {
            self.i += 1;
            return Some(M::Sym(bb.into()));
        }
        if let Some((kind, tex)) = lexicon::get().lookup(w) {
            // symbol names that are everyday nouns ("a perfect square", "a triangle") are symbols
            // only inside $…$
            if kind == Kind::Const && !self.explicit && is_english_noun_symbol(&l) {
                return None;
            }
            match kind {
                Kind::Greek | Kind::Const | Kind::Set => {
                    self.i += 1;
                    let base = M::Sym(tex.to_string());
                    return Some(self.maybe_call(base));
                }
                Kind::Func => {
                    self.i += 1;
                    // id_X: a named map with a subscript
                    if self.at_sym("_") && !self.t[self.i].sp_before {
                        return Some(M::Raw(tex.to_string()));
                    }
                    if glued_paren {
                        return Some(self.maybe_call(M::Raw(tex.to_string())));
                    }
                    if !self.stopped() {
                        let save = self.i;
                        if let Some(arg) = self.unary() {
                            return Some(M::Func(tex.to_string(), Box::new(arg)));
                        }
                        self.i = save;
                    }
                    if self.explicit {
                        return Some(M::Raw(tex.to_string()));
                    }
                    self.i -= 1;
                    return None;
                }
                Kind::FuncP => {
                    // English-word operator names are math only as `rank(A)` or `id_X`
                    let glued_sub = self.peek_at(1).is_some_and(|n| n.is_sym("_") && !n.sp_before);
                    if glued_sub {
                        self.i += 1;
                        return Some(M::Raw(tex.to_string()));
                    }
                    if !glued_paren {
                        return None;
                    }
                    self.i += 1;
                    return Some(self.maybe_call(M::Raw(tex.to_string())));
                }
                Kind::Accent => {
                    // `hat x` → \hat{x}
                    let save = self.i;
                    self.i += 1;
                    if !self.stopped()
                        && let Some(a) = self.atom()
                    {
                        let a = self.postfix(a);
                        return Some(M::Accent(tex.to_string(), Box::new(strip_parens(a))));
                    }
                    self.i = save;
                    return None;
                }
                Kind::Big => {
                    return self.big_op();
                }
                _ => return None,
            }
        }
        // "ax + by = d": a two-letter keyword between arithmetic operators is a product of variables
        let arith =
            |t: Option<&Tok>, ops: &[&str]| t.is_some_and(|t| t.kind == K::Sym && ops.contains(&t.text.as_str()));
        if w.len() == 2
            && w.chars().all(|c| c.is_ascii_lowercase())
            && arith(self.i.checked_sub(1).and_then(|p| self.t.get(p)), &["+", "-", "*", "/", "="])
            && (arith(self.peek_at(1), &["=", "+", "-", "<", ">", "<=", ">=", ")", "/", "*"])
                || self.peek_at(1).is_none())
        {
            self.i += 1;
            return Some(M::Var(w.to_string()));
        }
        if lexicon::is_keyword(&l) || (!self.explicit && lexicon::is_prose_stop(&l)) {
            return None;
        }
        let chars: Vec<char> = w.chars().collect();
        // a named object with a subscript: id_X, rk_A
        if chars.len() > 1
            && chars.len() <= 4
            && chars.iter().all(|c| c.is_ascii_alphabetic())
            && !lexicon::is_english(w)
            && self.peek_at(1).is_some_and(|n| n.is_sym("_") && !n.sp_before)
        {
            self.i += 1;
            return Some(M::Raw(format!("\\mathrm{{{w}}}")));
        }
        let ident =
            chars[0].is_ascii_alphabetic() && (chars.len() == 1 || chars[1..].iter().all(|c| c.is_ascii_digit()));
        if ident {
            self.i += 1;
            let name = if chars.len() == 1 { w.to_string() } else { format!("{}_{{{}}}", chars[0], &w[1..]) };
            return Some(self.maybe_call(M::Var(name)));
        }
        // multi-letter words
        if glued_paren && w.chars().all(|c| c.is_ascii_alphabetic()) {
            self.i += 1;
            return Some(self.maybe_call(M::Raw(format!("\\operatorname{{{w}}}"))));
        }
        if self.explicit {
            self.i += 1;
            if w.len() <= 3 && !lexicon::is_english(w) && w.chars().all(|c| c.is_ascii_alphabetic()) {
                return Some(M::Var(w.to_string()));
            }
            // merge consecutive prose words into one \text{}
            let mut words = vec![w.to_string()];
            while let Some(n) = self.peek() {
                let nl = n.lower();
                if n.kind == K::Word
                    && lexicon::get().lookup(&n.text).is_none()
                    && !lexicon::is_keyword(&nl)
                    && n.text.chars().count() > 1
                    && !self.stops.contains(&nl)
                {
                    words.push(n.text.clone());
                    self.i += 1;
                } else {
                    break;
                }
            }
            return Some(M::Text(words.join(" ")));
        }
        // prose mode: a multi-letter word glued to math (`xy=1`, `ab+c`) is a product of variables
        let glued_math = !t.sp_after
            && self
                .peek_at(1)
                .is_some_and(|n| n.kind == K::Sym && !matches!(n.text.as_str(), "," | "." | ";" | ":" | "!" | "?"))
            || self.i > 0
                && !t.sp_before
                && self.t.get(self.i - 1).is_some_and(|p| p.kind == K::Sym || p.kind == K::Num);
        // … or one set off by spaces between arithmetic operators (`a^2 + ab + b^2`, `x = ab`)
        let op = |t: Option<&Tok>| {
            t.is_some_and(|t| {
                t.kind == K::Sym
                    && (matches!(t.text.as_str(), "+" | "-" | "*" | "/" | "=" | "<" | ">" | "<=" | ">=" | "!=" | "^")
                        // `ab | c` divides; in "we show |x - y| = …" the bar opens an absolute value
                        || t.text == "|" && t.sp_before && t.sp_after)
            })
        };
        let spaced_math = op(self.i.checked_sub(1).and_then(|p| self.t.get(p)))
            || op(self.peek_at(1))
            // "kl in Z": only before a set ("born in the" stays prose)
            || self.peek_at(1).is_some_and(|n| n.is_kw("in") || n.is_kw("divides"))
                && self.peek_at(2).is_some_and(|s| {
                    s.kind == K::Sigil('*')
                        || matches!(s.kind, K::Sigil(_))
                        || s.text.len() == 1 && s.text.chars().all(|c| c.is_ascii_uppercase())
                        || s.kind == K::Num
                });
        // "xy is odd" after "let x, y ∈ ℤ": a short word spelled only with declared one-letter
        // variables (and not an English word) is their product
        let n = w.chars().count();
        if (2..=3).contains(&n)
            && w.chars().all(|c| c.is_ascii_lowercase() && is_declared_var(c))
            && !lexicon::is_english(w)
            && !lexicon::is_keyword(w)
            && !lexicon::is_prose_stop(w)
            && !self.stops.iter().any(|s| s.eq_ignore_ascii_case(w))
        {
            self.i += 1;
            let mut vars = w.chars().map(|c| M::Var(c.to_string()));
            let first = vars.next().expect("two or more letters");
            return Some(vars.fold(first, |a, b| M::Implicit(Box::new(a), Box::new(b))));
        }
        if (glued_math || spaced_math)
            && w.len() <= 4
            && w.chars().all(|c| c.is_ascii_alphabetic())
            && !lexicon::is_english(w)
            && !lexicon::is_keyword(&w.to_ascii_lowercase())
            && !self.stops.iter().any(|s| s.eq_ignore_ascii_case(w))
        {
            self.i += 1;
            return Some(M::Var(w.to_string()));
        }
        None
    }

    /// `f(x, y)` when a glued `(` follows.
    fn maybe_call(&mut self, f: M) -> M {
        if self.at_sym("(")
            && !self.t[self.i].sp_before
            && let save = self.i
            && let Some(M::Group(inner, br)) = self.sym_atom()
        {
            let args = match *inner {
                M::Seq(v) => v,
                other => vec![other],
            };
            let _ = save;
            return M::Call(Box::new(f), args, br);
        }
        f
    }

    /// Argument of sqrt / roots: `(x)` `{x}` or a single atom with postfix.
    fn root_arg(&mut self) -> Option<M> {
        if self.at_sym("{") {
            return self.brace_group();
        }
        let a = self.unary()?;
        Some(strip_parens(a))
    }

    // ------------------------------------------------------------------ big operators

    fn scoped<T>(&mut self, extra: &[&str], f: impl FnOnce(&mut Self) -> T) -> T {
        let old = self.stops.clone();
        self.stops.extend(extra.iter().map(|s| s.to_string()));
        let r = f(self);
        self.stops = old;
        r
    }

    /// `sum from i=1 to n [of] f`, `sum i=1 to n f`, `sum_{i=1}^n f`, `sum over i in S of f`.
    fn big_op(&mut self) -> Option<M> {
        let t = self.peek()?;
        let word_form =
            matches!(t.lower().as_str(), "sum" | "summation" | "product" | "union" | "intersection" | "intersect");
        let op: String = match t.lower().as_str() {
            "sum" | "summation" => "\\sum".into(),
            "prod" | "product" => "\\prod".into(),
            "union" => "\\bigcup".into(),
            "intersection" | "intersect" => "\\bigcap".into(),
            _ => match lexicon::get().lookup(&t.text) {
                Some((Kind::Big, tex)) => tex.to_string(),
                _ => return None,
            },
        };
        let save = self.i;
        self.i += 1;
        let (mut lower, mut upper) = (None, None);
        if self.at_sym("_") && !self.t[self.i].sp_before {
            self.i += 1;
            lower = if self.at_sym("{") { self.brace_group() } else { self.atom() };
            if self.at_sym("^") {
                self.i += 1;
                upper = if self.at_sym("{") { self.brace_group() } else { self.atom() };
            }
        } else {
            let bounded = self.at_word("from") || self.at_word("over");
            if bounded {
                self.i += 1;
            }
            // "the cross product u cross v …": English word forms need explicit bounds
            if (bounded || !word_form) && !self.at_word("of") && !self.stopped() {
                let s2 = self.i;
                lower = self.scoped(&["to", "up to", "until", "of", "through"], |p| p.rel_chain());
                if lower.is_none() {
                    self.i = s2;
                }
                if lower.is_some()
                    && (self.at_word("to") || self.at_word("up to") || self.at_word("until") || self.at_word("through"))
                {
                    self.i += 1;
                    upper = self.scoped(&["of"], |p| p.additive());
                }
            }
        }
        let mut body = None;
        if self.at_word("of") {
            self.i += 1;
            body = self.additive();
        } else if !self.stopped() && self.rel_op().is_none() && !self.at_end() {
            let s2 = self.i;
            body = self.multiplicative();
            if body.is_none() {
                self.i = s2;
            }
        }
        if lower.is_none() && upper.is_none() && body.is_none() {
            self.i = save;
            return None;
        }
        // English "sum"/"product" are big operators only with bounds or "of" ("the product u cross v"
        // is prose): `sum from i=1 to n of …`, `product of …`
        if word_form && lower.is_none() && upper.is_none() {
            self.i = save;
            return None;
        }
        Some(M::Big { op, lower: lower.map(Box::new), upper: upper.map(Box::new), body: body.map(Box::new) })
    }

    /// Integrals in their common English shapes (spec Ex. 2–3).
    fn integral(&mut self) -> Option<M> {
        let kw = self.peek()?.lower();
        let save = self.i;
        self.i += 1;
        let stops = ["from", "to", "of", "dx", "with respect to", "wrt", "over"];
        let (mut lower, mut upper) = (None, None);
        let body;
        let bounds = |p: &mut Self, lower: &mut Option<M>, upper: &mut Option<M>| {
            if p.at_word("from") {
                p.i += 1;
            }
            *lower = p.scoped(&["to", "of"], |p| p.additive());
            if lower.is_some() && p.at_word("to") {
                p.i += 1;
                *upper = p.scoped(&["of", "dx"], |p| p.additive());
            }
        };
        if kw == "integrate" {
            body = self.scoped(&stops, |p| p.additive());
            if self.at_word("from") || self.at_word("over") {
                bounds(self, &mut lower, &mut upper);
            }
        } else if self.at_sym("_") && !self.t[self.i].sp_before {
            self.i += 1;
            lower = if self.at_sym("{") { self.brace_group() } else { self.atom() };
            if self.at_sym("^") {
                self.i += 1;
                upper = if self.at_sym("{") { self.brace_group() } else { self.atom() };
            }
            body = self.scoped(&stops, |p| p.additive());
        } else if self.at_word("of") {
            self.i += 1;
            body = self.scoped(&stops, |p| p.additive());
            if self.at_word("from") {
                bounds(self, &mut lower, &mut upper);
            }
        } else {
            bounds(self, &mut lower, &mut upper);
            if self.at_word("of") {
                self.i += 1;
            }
            body = self.scoped(&stops, |p| p.additive());
        }
        let Some(body) = body else {
            self.i = save;
            return None;
        };
        // variable of integration
        let mut var = None;
        if let Some(t) = self.peek()
            && let Some(v) = is_d_var(&t.text)
        {
            var = Some(v.to_string());
            self.i += 1;
        } else if self.at_word("d") && self.peek_at(1).is_some_and(|n| n.kind == K::Word && n.text.len() == 1) {
            var = Some(self.t[self.i + 1].text.clone());
            self.i += 2;
        } else if self.at_word("with respect to") || self.at_word("wrt") {
            self.i += 1;
            if let Some(n) = self.peek()
                && n.kind == K::Word
            {
                var = Some(var_tex(&n.text));
                self.i += 1;
            }
        }
        let var = var.unwrap_or_else(|| guess_var(&body));
        Some(M::Int { lower: lower.map(Box::new), upper: upper.map(Box::new), body: Box::new(body), var })
    }

    /// `limit as x to a of f`, `lim x to a f`, `lim_{x to a} f`, `limit of f as x to a`.
    fn limit(&mut self) -> Option<M> {
        let save = self.i;
        self.i += 1;
        let sub;
        let body;
        if self.at_sym("_") {
            self.i += 1;
            sub = self.brace_group()?;
            body = self.additive()?;
        } else if self.at_word("of") {
            self.i += 1;
            body = self.scoped(&["as"], |p| p.additive())?;
            if !self.at_word("as") {
                self.i = save;
                return None;
            }
            self.i += 1;
            sub = self.rel_chain()?;
        } else {
            if self.at_word("as") {
                self.i += 1;
            }
            sub = self.scoped(&["of"], |p| p.rel_chain())?;
            if self.at_word("of") {
                self.i += 1;
            }
            body = self.additive()?;
        }
        Some(M::Lim { sub: Box::new(sub), body: Box::new(body) })
    }

    /// `derivative of f [with respect to|wrt x]`, `derivative wrt x of f`.
    fn derivative(&mut self) -> Option<M> {
        let save = self.i;
        self.i += 1;
        let mut var = None;
        if self.at_word("with respect to") || self.at_word("wrt") {
            self.i += 1;
            var = Some(var_tex(&self.peek()?.text));
            self.i += 1;
        }
        if !self.at_word("of") {
            self.i = save;
            return None;
        }
        self.i += 1;
        let body = self.scoped(&["with respect to", "wrt"], |p| p.additive())?;
        if var.is_none() && (self.at_word("with respect to") || self.at_word("wrt")) {
            self.i += 1;
            var = Some(var_tex(&self.peek()?.text));
            self.i += 1;
        }
        let var = var.unwrap_or_else(|| guess_var(&body));
        Some(M::Deriv { var, body: Box::new(body) })
    }
}

thread_local! {
    /// Capital letters declared as variables in the current document (`let A, B, C be sets`):
    /// they are never read as number sets (ℂ, ℝ, …).
    static DECLARED: std::cell::RefCell<std::collections::HashSet<String>> = Default::default();
}

pub fn declare(name: &str) {
    if name.chars().count() == 1 && name.chars().all(|c| c.is_ascii_uppercase()) {
        DECLARED.with(|d| d.borrow_mut().insert(name.to_string()));
    }
}

pub fn reset_declared() {
    DECLARED.with(|d| d.borrow_mut().clear());
    DECLARED_VARS.with(|d| d.borrow_mut().clear());
}

thread_local! {
    /// One-letter lowercase variables declared in the current document (`let x, y ∈ ℤ`).
    static DECLARED_VARS: std::cell::RefCell<std::collections::HashSet<char>> = Default::default();
}

/// Record a declared one-letter lowercase variable.
pub fn declare_var(c: char) {
    if c.is_ascii_lowercase() {
        DECLARED_VARS.with(|d| d.borrow_mut().insert(c));
    }
}

fn is_declared_var(c: char) -> bool {
    DECLARED_VARS.with(|d| d.borrow().contains(&c))
}

fn is_declared(name: &str) -> bool {
    DECLARED.with(|d| d.borrow().contains(name))
}

/// Lexicon constants that are also ordinary English nouns.
pub fn is_english_noun_symbol(w: &str) -> bool {
    matches!(
        w,
        "square"
            | "triangle"
            | "angle"
            | "top"
            | "partial"
            | "dagger"
            | "lozenge"
            | "checkmark"
            | "indicator"
            | "surd"
            | "dots"
            | "maltese"
            | "eth"
            | "ell"
            | "mho"
            | "measuredangle"
    )
}

/// "th", "st", "nd", "rd" glued after a number or expression: an ordinal ("n-th", "(k+1)st").
pub fn is_ordinal_suffix(w: &str) -> bool {
    matches!(w.to_ascii_lowercase().as_str(), "th" | "st" | "nd" | "rd")
}

/// `dx` → `x`, `dtheta` → `\theta`.
fn is_d_var(w: &str) -> Option<&str> {
    let rest = w.strip_prefix('d')?;
    if rest.len() == 1 && rest.as_bytes()[0].is_ascii_alphabetic() {
        return Some(rest);
    }
    match lexicon::get().lookup(rest) {
        Some((Kind::Greek, _)) => Some(rest),
        _ => None,
    }
}

fn var_tex(w: &str) -> String {
    match lexicon::get().lookup(w) {
        Some((Kind::Greek, tex)) => tex.to_string(),
        _ => w.to_string(),
    }
}

/// Variable of integration/differentiation when none is written: the argument of `f(x)`, else the
/// first single-letter variable, else `x`.
fn guess_var(m: &M) -> String {
    fn first_var(m: &M) -> Option<String> {
        match m {
            M::Call(_, args, _) => args.iter().find_map(first_var),
            M::Var(v) if v.len() == 1 => Some(v.clone()),
            M::Bin(_, a, b) | M::Implicit(a, b) | M::Frac(a, b) | M::Pow(a, b) => first_var(a).or_else(|| first_var(b)),
            M::Neg(a) | M::Func(_, a) | M::Group(a, _) | M::Sqrt(_, a) | M::Post(_, a) => first_var(a),
            _ => None,
        }
    }
    let call_arg = match m {
        M::Call(_, args, _) => args.first().and_then(first_var),
        _ => None,
    };
    call_arg.or_else(|| first_var(m)).unwrap_or_else(|| "x".into())
}

pub fn strip_parens(m: M) -> M {
    match m {
        M::Group(inner, '(') => *inner,
        other => other,
    }
}

/// Logical negation, folding into the relation when the operand is a single relation (what the
/// user wrote: "not x < 5" stays `x \\not< 5`).
pub fn negate(m: M) -> M {
    match m {
        M::Chain(first, mut steps) if steps.len() == 1 => match negate_op(&steps[0].op) {
            Some(op) => {
                steps[0].op = op.to_string();
                M::Chain(first, steps)
            }
            None => M::Not(Box::new(M::Chain(first, steps))),
        },
        M::Not(inner) => *inner,
        other => M::Not(Box::new(other)),
    }
}

/// The negation of a claim ("suppose not"), pushed inward the way a mathematician states it:
/// `x < 2` → `x ≥ 2` (numbers are totally ordered), even ↔ odd, De Morgan, ∀ ↔ ∃.
pub fn negate_claim(m: M) -> M {
    match m {
        M::Chain(first, mut steps) if steps.len() == 1 => {
            let complement = match steps[0].op.as_str() {
                "<" => Some("\\geq"),
                ">" => Some("\\leq"),
                "\\leq" => Some(">"),
                "\\geq" => Some("<"),
                op => negate_op(op),
            };
            match complement {
                Some(op) => {
                    steps[0].op = op.to_string();
                    M::Chain(first, steps)
                }
                None => M::Not(Box::new(M::Chain(first, steps))),
            }
        }
        M::Pred(p, a) => {
            let opposite = match p.as_str() {
                "even" => Some("odd"),
                "odd" => Some("even"),
                "positive" => Some("nonpositive"),
                "nonpositive" | "non-positive" => Some("positive"),
                "negative" => Some("nonnegative"),
                "nonnegative" | "non-negative" => Some("negative"),
                "zero" => Some("nonzero"),
                "nonzero" | "non-zero" => Some("zero"),
                "rational" => Some("irrational"),
                "irrational" => Some("rational"),
                _ => None,
            };
            match opposite {
                Some(q) => M::Pred(q.to_string(), a),
                None => M::Not(Box::new(M::Pred(p, a))),
            }
        }
        M::Bin(op, a, b) if op == "\\land" || op == "\\lor" => {
            let dual = if op == "\\land" { "\\lor" } else { "\\land" };
            M::Bin(dual.into(), Box::new(negate_claim(*a)), Box::new(negate_claim(*b)))
        }
        M::Quant { q, vars, body: Some(body) } if q == "\\forall" || q == "\\exists" => {
            let dual = if q == "\\forall" { "\\exists" } else { "\\forall" };
            M::Quant { q: dual.into(), vars, body: Some(Box::new(negate_claim(*body))) }
        }
        M::Quant { q, vars, body } if q == "\\nexists" => M::Quant { q: "\\exists".into(), vars, body },
        M::Not(inner) => *inner,
        other => M::Not(Box::new(other)),
    }
}

// ---------------------------------------------------------------------------------------- LaTeX

pub fn tex(m: &M) -> String {
    let mut s = String::new();
    emit(m, &mut s);
    s
}

fn needs_parens_for_pow(m: &M) -> bool {
    matches!(
        m,
        M::Bin(..) | M::Neg(_) | M::Pm(..) | M::Frac(..) | M::Implicit(..) | M::Chain(..) | M::Mod(..) | M::Func(..)
    ) || matches!(m, M::Num(n) if n.starts_with('-'))
}

/// A base that already carries a superscript: `(A^{c})^{c}`, `(x^{2})^{3}` (no double `^`).
fn emit_base(m: &M, s: &mut String) {
    if matches!(m, M::Pow(..)) || matches!(m, M::Post(op, _) if op.starts_with('^')) {
        s.push('(');
        emit(m, s);
        s.push(')');
    } else {
        emit_wrapped(m, s);
    }
}

fn emit_wrapped(m: &M, s: &mut String) {
    if needs_parens_for_pow(m) {
        s.push('(');
        emit(m, s);
        s.push(')');
    } else {
        emit(m, s);
    }
}

fn emit(m: &M, s: &mut String) {
    match m {
        M::Num(n) | M::Var(n) | M::Sym(n) | M::Raw(n) => s.push_str(n),
        M::Text(t) => {
            s.push_str("\\text{");
            s.push_str(&escape_text(t));
            s.push('}');
        }
        M::Neg(a) => {
            s.push('-');
            if matches!(**a, M::Bin(ref op, ..) if op == "+" || op == "-") {
                s.push('(');
                emit(a, s);
                s.push(')');
            } else {
                emit(a, s);
            }
        }
        M::Pm(op, a) => {
            s.push_str(op);
            s.push(' ');
            emit(a, s);
        }
        M::Bin(op, a, b) => {
            emit_dots(a, "\\cdots", s);
            s.push(' ');
            s.push_str(op);
            s.push(' ');
            emit_dots(b, "\\cdots", s);
        }
        M::Implicit(a, b) if matches!(**b, M::Text(_)) || matches!(**a, M::Text(_)) => {
            // prose inside explicit math: `x \text{ is prime}`
            match (&**a, &**b) {
                (_, M::Text(t)) => {
                    emit(a, s);
                    s.push_str(" \\text{ ");
                    s.push_str(&escape_text(t));
                    s.push('}');
                }
                (M::Text(t), _) => {
                    s.push_str("\\text{");
                    s.push_str(&escape_text(t));
                    s.push_str(" } ");
                    emit(b, s);
                }
                _ => unreachable!(),
            }
        }
        M::Implicit(a, b) => {
            let mut left = String::new();
            emit_dots(a, "\\cdots", &mut left);
            let mut right = String::new();
            if matches!(**b, M::Bin(..) | M::Neg(_)) {
                right.push('(');
                emit(b, &mut right);
                right.push(')');
            } else {
                emit_dots(b, "\\cdots", &mut right);
            }
            // spaced products of subscripted factors read better with a thin gap: p_{1} p_{2}
            if !right.is_empty()
                && !left.ends_with(' ')
                && (left.ends_with('}') || left.ends_with("\\cdots"))
                && (right.starts_with(|c: char| c.is_ascii_alphabetic()) || right.starts_with("\\cdots"))
            {
                left.push(' ');
            }
            s.push_str(&left);
            if ends_with_command(&left) && right.starts_with(|c: char| c.is_ascii_alphabetic())
                || left.ends_with(|c: char| c.is_ascii_alphabetic()) && right.starts_with('\\')
            {
                s.push(' ');
            }
            s.push_str(&right);
        }
        M::Frac(a, b) => {
            s.push_str("\\frac{");
            emit(a, s);
            s.push_str("}{");
            emit(b, s);
            s.push('}');
        }
        M::Binom(a, b) => {
            s.push_str("\\binom{");
            emit(a, s);
            s.push_str("}{");
            emit(b, s);
            s.push('}');
        }
        M::Mod(a, n) => {
            emit(a, s);
            s.push_str(" \\bmod ");
            emit_wrapped(n, s);
        }
        M::Pow(a, e) => {
            emit_base(a, s);
            s.push_str("^{");
            emit(e, s);
            s.push('}');
        }
        M::Sub(a, e) => {
            emit_wrapped(a, s);
            s.push_str("_{");
            emit(e, s);
            s.push('}');
        }
        M::Post(op, a) => {
            if op.starts_with('^') {
                emit_base(a, s)
            } else {
                emit_wrapped(a, s)
            }
            s.push_str(op);
        }
        M::Call(f, args, br) => {
            emit(f, s);
            s.push(*br);
            for (k, a) in args.iter().enumerate() {
                if k > 0 {
                    s.push_str(", ");
                }
                emit_dots(a, "\\ldots", s);
            }
            s.push(if *br == '[' { ']' } else { ')' });
        }
        M::Func(f, a) => {
            s.push_str(f);
            s.push(' ');
            emit_wrapped(a, s);
        }
        // "(9 choose 2)": a binomial is already bracketed
        M::Group(a, '(') if matches!(**a, M::Binom(..)) => emit(a, s),
        M::Group(a, br) => {
            s.push(*br);
            emit(a, s);
            s.push(if *br == '[' { ']' } else { ')' });
        }
        M::Abs(a) => {
            s.push_str("\\lvert ");
            emit(a, s);
            s.push_str(" \\rvert");
        }
        M::Norm(a) => {
            s.push_str("\\lVert ");
            emit(a, s);
            s.push_str(" \\rVert");
        }
        M::Floor(a, floor) => {
            s.push_str(if *floor { "\\lfloor " } else { "\\lceil " });
            emit(a, s);
            s.push_str(if *floor { " \\rfloor" } else { " \\rceil" });
        }
        M::Set(items) => {
            s.push_str("\\{");
            for (k, a) in items.iter().enumerate() {
                if k > 0 {
                    s.push_str(", ");
                }
                emit_dots(a, "\\ldots", s);
            }
            s.push_str("\\}");
        }
        M::SetB(a, b) => {
            s.push_str("\\{");
            emit(a, s);
            s.push_str(" \\mid ");
            emit(b, s);
            s.push_str("\\}");
        }
        M::Pred(p, a) => {
            s.push_str(&format!("\\operatorname{{{p}}}("));
            emit(a, s);
            s.push(')');
        }
        M::Accent(acc, a) => {
            s.push_str(acc);
            s.push('{');
            emit(a, s);
            s.push('}');
        }
        M::Sqrt(n, a) => {
            s.push_str("\\sqrt");
            if let Some(n) = n {
                s.push('[');
                emit(n, s);
                s.push(']');
            }
            s.push('{');
            emit(a, s);
            s.push('}');
        }
        M::Big { op, lower, upper, body } => {
            s.push_str(op);
            if let Some(l) = lower {
                s.push_str("_{");
                emit_tight(l, s);
                s.push('}');
            }
            if let Some(u) = upper {
                s.push_str("^{");
                emit(u, s);
                s.push('}');
            }
            if let Some(b) = body {
                s.push(' ');
                emit(b, s);
            }
        }
        M::Int { lower, upper, body, var } => {
            s.push_str("\\int");
            if let Some(l) = lower {
                s.push_str("_{");
                emit(l, s);
                s.push('}');
            }
            if let Some(u) = upper {
                s.push_str("^{");
                emit(u, s);
                s.push('}');
            }
            s.push(' ');
            emit(body, s);
            s.push_str(" \\,d");
            s.push_str(var);
        }
        M::Lim { sub, body } => {
            s.push_str("\\lim_{");
            emit(sub, s);
            s.push_str("} ");
            emit(body, s);
        }
        M::Deriv { var, body } => {
            s.push_str("\\frac{d}{d");
            s.push_str(var);
            s.push_str("} ");
            emit_wrapped(body, s);
        }
        M::Chain(first, steps) => {
            emit(first, s);
            for st in steps {
                s.push(' ');
                s.push_str(&st.op);
                s.push(' ');
                emit_rhs(&st.op, &st.rhs, s);
            }
        }
        M::Not(a) => {
            s.push_str("\\neg ");
            if matches!(**a, M::Chain(..) | M::Bin(..)) {
                s.push('(');
                emit(a, s);
                s.push(')');
            } else {
                emit(a, s);
            }
        }
        M::Quant { q, vars, body } => {
            s.push_str(q);
            s.push(' ');
            emit(vars, s);
            if let Some(b) = body {
                s.push_str(", \\; ");
                emit(b, s);
            }
        }
        M::Seq(items) => {
            for (k, a) in items.iter().enumerate() {
                if k > 0 {
                    s.push_str(", ");
                }
                emit_dots(a, "\\ldots", s);
            }
        }
    }
}

/// `≡ x + y mod n` parses the modulus into the last term; move it to the top (→ `\\pmod{n}`).
fn lift_mod(m: M) -> M {
    match m {
        M::Mod(..) => m,
        M::Big { op, lower, upper, body: Some(b) } => match *b {
            M::Mod(x, n) => M::Mod(Box::new(M::Big { op, lower, upper, body: Some(x) }), n),
            other => M::Big { op, lower, upper, body: Some(Box::new(other)) },
        },
        M::Bin(op, a, b) if op == "+" || op == "-" => match *b {
            M::Mod(x, n) => M::Mod(Box::new(M::Bin(op, a, x)), n),
            other => M::Bin(op, a, Box::new(other)),
        },
        other => other,
    }
}

/// An ellipsis atom (`...`): `\cdots` between operators, `\ldots` in lists.
pub const DOTS: &str = "\\dots";

pub fn is_dots(m: &M) -> bool {
    matches!(m, M::Sym(s) if s == DOTS)
}

fn emit_dots(m: &M, as_: &str, s: &mut String) {
    if is_dots(m) { s.push_str(as_) } else { emit(m, s) }
}

/// Operands of `∘`: functions and compositions (`g o f`, `(g o f)`, `f^{-1} o g`).
fn composable(m: &M) -> bool {
    match m {
        M::Var(_) | M::Group(..) | M::Call(..) | M::Raw(_) | M::Sym(_) => true,
        M::Bin(op, ..) => op == "\\circ",
        M::Pow(b, _) | M::Sub(b, _) => composable(b),
        _ => false,
    }
}

/// Right side of a relation: `≡ a mod n` renders as `\equiv a \pmod{n}`.
pub fn emit_rhs(op: &str, rhs: &M, s: &mut String) {
    if (op == "\\equiv" || op == "\\not\\equiv")
        && let M::Mod(a, n) = rhs
    {
        emit(a, s);
        s.push_str(" \\pmod{");
        emit(n, s);
        s.push('}');
        return;
    }
    emit(rhs, s);
}

/// Bounds under big operators: `i=1` without spaces.
fn emit_tight(m: &M, s: &mut String) {
    match m {
        M::Chain(a, steps) if steps.len() == 1 && steps[0].op == "=" => {
            emit(a, s);
            s.push('=');
            emit(&steps[0].rhs, s);
        }
        other => emit(other, s),
    }
}

fn ends_with_command(s: &str) -> bool {
    let tail: String = s.chars().rev().take_while(|c| c.is_ascii_alphabetic()).collect();
    !tail.is_empty() && s[..s.len() - tail.len()].ends_with('\\')
}

pub fn escape_text(t: &str) -> String {
    let mut o = String::with_capacity(t.len());
    for c in t.chars() {
        match c {
            '%' | '&' | '#' | '_' | '{' | '}' | '$' => {
                o.push('\\');
                o.push(c);
            }
            '~' => o.push_str("\\textasciitilde{}"),
            '^' => o.push_str("\\^{}"),
            '\\' => o.push_str("\\textbackslash{}"),
            _ => o.push(c),
        }
    }
    o
}

/// Rough rendered width in characters (for inline vs display vs align decisions).
pub fn width(m: &M) -> usize {
    let t = tex(m);
    let mut n = 0;
    let mut chars = t.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                let mut cmd = String::new();
                while let Some(&d) = chars.peek() {
                    if d.is_ascii_alphabetic() {
                        cmd.push(d);
                        chars.next();
                    } else {
                        break;
                    }
                }
                n += match cmd.as_str() {
                    "frac" | "text" | "mathbb" | "mathcal" | "left" | "right" | "," | ";" => 0,
                    _ => 1,
                };
            }
            '{' | '}' | '^' | '_' => {}
            ' ' => {}
            _ => n += 1,
        }
    }
    n
}

/// Does the expression look like math worth typesetting (STEX-DESIGN §9 plausibility)?
pub fn is_substantial(m: &M) -> bool {
    !matches!(m, M::Var(_) | M::Num(_) | M::Text(_)) || matches!(m, M::Num(n) if n.contains('.'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lex::lex;

    fn p(src: &str, explicit: bool) -> String {
        let t = lex(src).unwrap();
        let mut ps = Parser::new(&t, 0, explicit);
        let m = ps.expr().unwrap_or_else(|| panic!("no parse: {src}"));
        assert_eq!(ps.i, t.len(), "unconsumed in {src:?}: {:?}", &t[ps.i..]);
        tex(&m)
    }

    #[test]
    fn arithmetic() {
        assert_eq!(p("x = 5", false), "x = 5");
        assert_eq!(p("x^20+30 choose 4", false), "x^{20} + \\binom{30}{4}");
        assert_eq!(p("f(x) = 2x^34 + 7", false), "f(x) = 2x^{34} + 7");
        assert_eq!(p("a+b/c", false), "a + \\frac{b}{c}");
        assert_eq!(p("(a+b)/c", false), "\\frac{a + b}{c}");
        assert_eq!(p("a over b", false), "\\frac{a}{b}");
        assert_eq!(p("x * y dot z cross w", false), "x \\cdot y \\cdot z \\times w");
        assert_eq!(p("x +- 1 >= -y", false), "x \\pm 1 \\geq -y");
        assert_eq!(p("(x+3)(x-3) = 0", false), "(x + 3)(x - 3) = 0");
        assert_eq!(p("n! = 120", false), "n! = 120");
    }

    #[test]
    fn modular() {
        assert_eq!(p("x == 5 mod 6", false), "x \\equiv 5 \\pmod{6}");
        assert_eq!(p("a mod n = r", false), "a \\bmod n = r");
        assert_eq!(p("5% % 6%", false), "5\\% \\bmod 6\\%");
        assert_eq!(p("x !== 1 mod 2", false), "x \\not\\equiv 1 \\pmod{2}");
    }

    #[test]
    fn roots_and_sets() {
        assert_eq!(p("3rt(5)", false), "\\sqrt[3]{5}");
        assert_eq!(p("7rt 6", false), "\\sqrt[7]{6}");
        assert_eq!(p("sqrt(x+1)", false), "\\sqrt{x + 1}");
        assert_eq!(p("nrt(x)", false), "\\sqrt[n]{x}");
        assert_eq!(p("x in Z", false), "x \\in \\mathbb{Z}");
        assert_eq!(p("x in integers", false), "x \\in \\mathbb{Z}");
        assert_eq!(p("f: N to N", false), "f : \\mathbb{N} \\to \\mathbb{N}");
        assert_eq!(p("A = {1, 2, 3}", false), "A = \\{1, 2, 3\\}");
        assert_eq!(p("S = {x in Z | x > 0}", false), "S = \\{x \\in \\mathbb{Z} \\mid x > 0\\}");
        assert_eq!(p("A union B intersect C", false), "A \\cup B \\cap C");
        assert_eq!(p("A = empty set", false), "A = \\varnothing");
        assert_eq!(p("|x| <= 1", false), "\\lvert x \\rvert \\leq 1");
        assert_eq!(p("*A subset &B", false), "\\mathbb{A} \\subseteq \\mathcal{B}");
        assert_eq!(p("N = 5", false), "N = 5", "a capital outside set position is a variable");
    }

    #[test]
    fn backslash_is_set_difference() {
        // user decision (session 3): `\` always means set difference; raw LaTeX uses `latex{…}`
        assert_eq!(p("A\\B", false), "A \\setminus B");
        assert_eq!(p("A \\ B", false), "A \\setminus B");
        assert_eq!(p("A \\ (A\\B) = A intersect B", false), "A \\setminus (A \\setminus B) = A \\cap B");
        assert_eq!(p("(A\\B)\\C", false), "(A \\setminus B) \\setminus C");
        assert_eq!(p("A\\B union C", false), "A \\setminus B \\cup C");
        assert_eq!(p("x latex{\\leq} y", true), "x \\leq y");
    }

    #[test]
    fn between_and_ordinals() {
        assert_eq!(p("x is between 0 and 1", false), "0 \\leq x \\leq 1");
        assert_eq!(p("x is strictly between a and b + 1", false), "a < x < b + 1");
        let t = lex("n-th").unwrap();
        let mut ps = Parser::new(&t, 0, false);
        assert_eq!(tex(&ps.expr().unwrap()), "n");
        assert_eq!(ps.i, 1, "the ordinal suffix is not a subtraction");
    }

    #[test]
    fn library_words() {
        assert_eq!(p("x plus y times z", false), "x + y \\times z");
        assert_eq!(p("a divided by b", false), "\\frac{a}{b}");
        assert_eq!(p("G is isomorphic to H", false), "G \\cong H");
        assert_eq!(p("G is not isomorphic to H", false), "G \\ncong H");
        assert_eq!(p("x≤5", false), "x \\leq 5");
        assert_eq!(p("x ∈ ℝ", false), "x \\in \\mathbb{R}");
        assert_eq!(p("rank(A) = n", false), "\\operatorname{rank}(A) = n");
        assert_eq!(p("x hat + bar y", false), "\\hat{x} + \\bar{y}");
        assert_eq!(p("A otimes B", false), "A \\otimes B");
        assert_eq!(p("f_n converges uniformly to f", false), "f_{n} \\rightrightarrows f");
        assert_eq!(p("x squared = n factorial", false), "x^{2} = n!");
        assert_eq!(p("H is a normal subgroup of G", false), "H \\vartriangleleft G");
        assert_eq!(p("bbR cross calF", false), "\\mathbb{R} \\times \\mathcal{F}");
    }

    #[test]
    fn negation() {
        assert_eq!(p("x not = 5", false), "x \\neq 5");
        assert_eq!(p("x not equals 5", false), "x \\neq 5");
        assert_eq!(p("x neq 5", false), "x \\neq 5");
        assert_eq!(p("!x = 5", false), "x \\neq 5");
        assert_eq!(p("x not in S", false), "x \\notin S");
        assert_eq!(p("x != 5", false), "x \\neq 5");
    }

    #[test]
    fn calculus() {
        assert_eq!(p("sum from i=1 to 4 = 30", false), "\\sum_{i=1}^{4} = 30");
        assert_eq!(p("sum from i=1 to n of i^2", false), "\\sum_{i=1}^{n} i^{2}");
        assert_eq!(p("integrate f(x) from 1 to pi", false), "\\int_{1}^{\\pi} f(x) \\,dx");
        assert_eq!(p("integral 1 to pi of f(x) dx", false), "\\int_{1}^{\\pi} f(x) \\,dx");
        assert_eq!(p("integral from 0 to 1 of t^2 dt", false), "\\int_{0}^{1} t^{2} \\,dt");
        assert_eq!(p("integral of g(t) from a to b", false), "\\int_{a}^{b} g(t) \\,dt");
        assert_eq!(p("limit as x to 0 of sin(x)/x", false), "\\lim_{x \\to 0} \\frac{\\sin(x)}{x}");
        assert_eq!(p("lim x to infinity 1/x", false), "\\lim_{x \\to \\infty} \\frac{1}{x}");
        assert_eq!(p("derivative of x^2 wrt x", false), "\\frac{d}{dx} x^{2}");
        assert_eq!(p("d/dx x^2", false), "\\frac{d}{dx} x^{2}");
        assert_eq!(p("alpha + beta = pi", false), "\\alpha + \\beta = \\pi");
        assert_eq!(p("bigoplus from i=1 to n of V_i", false), "\\bigoplus_{i=1}^{n} V_{i}");
    }

    #[test]
    fn explicit_mode() {
        assert_eq!(p("exists x such that x = 5y", true), "\\exists x, \\; x = 5y");
        assert_eq!(p("x = 1 and y = 2 implies z", true), "x = 1 \\land y = 2 \\implies z");
        assert_eq!(p("not x = 5 or y", true), "x \\neq 5 \\lor y");
        assert_eq!(p("for all x in R, x^2 >= 0", true), "\\forall x \\in \\mathbb{R}, \\; x^{2} \\geq 0");
        assert_eq!(p("x is prime", true), "x \\text{ is prime}");
        assert_eq!(p("P iff Q", true), "P \\iff Q");
        assert_eq!(p("latex{\\exists} x", true), "\\exists x");
    }

    #[test]
    fn prose_mode_stops() {
        let t = lex("x = 5 and y = 6").unwrap();
        let mut ps = Parser::new(&t, 0, false);
        assert_eq!(tex(&ps.expr().unwrap()), "x = 5");
        assert_eq!(ps.i, 3);
        let t = lex("x in particular").unwrap();
        let mut ps = Parser::new(&t, 0, false);
        assert_eq!(tex(&ps.expr().unwrap()), "x");
        assert_eq!(ps.i, 1, "`in` needs a math operand");
    }
}
