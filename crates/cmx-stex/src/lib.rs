//! cmx-stex: the stex language (STEX-DESIGN.md). Deterministic near-natural-language proofs → LaTeX,
//! plus a Lean 4 translation for `cmx stex check`.
//!
//! Pipeline: [`lex`] → [`proof::split`] (sentences) → [`proof::Realizer`] (constructs, clauses,
//! [`math`] expressions, phrase bank) → [`doc`] (layout pass, packages, preamble). The same parse
//! feeds [`lean`].

pub mod doc;
pub mod highlight;
pub mod lean;
pub mod lex;
pub mod lexicon;
pub mod math;
pub mod phrases;
pub mod proof;
pub mod theorems;

pub use phrases::Variance;

#[derive(Clone, Debug, Default)]
pub struct Options {
    pub variance: Variance,
    /// PRNG seed for the phrase choice; default = FNV-1a hash of the source text.
    pub seed: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Diag {
    pub line: u32,
    pub msg: String,
}

impl std::fmt::Display for Diag {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "line {}: {}", self.line, self.msg)
    }
}

pub struct Compiled {
    /// The complete .tex document.
    pub tex: String,
    /// Just the body (between `\begin{document}` and `\end{document}`).
    pub body: String,
    pub plan: doc::Plan,
    /// Theorems invoked with `by …` (prose names).
    pub theorems: Vec<&'static str>,
    pub facts: Vec<proof::Fact>,
}

struct Parsed {
    blocks: Vec<proof::Block>,
    facts: Vec<proof::Fact>,
    theorems: Vec<usize>,
}

fn parse(src: &str, opts: &Options) -> Result<Parsed, Diag> {
    math::reset_declared();
    let toks = lex::lex(src).map_err(|e| Diag { line: e.line, msg: e.msg })?;
    // "A, B, C", "B and C": capital letters listed with an ordinary variable are all variables,
    // so C here is never ℂ (and R never ℝ)
    let cap =
        |t: &lex::Tok| t.kind == lex::K::Word && t.text.len() == 1 && t.text.chars().all(|c| c.is_ascii_uppercase());
    let mut i = 0;
    while i < toks.len() {
        if !cap(&toks[i]) {
            i += 1;
            continue;
        }
        let mut run = vec![toks[i].text.clone()];
        let mut j = i + 1;
        while j + 1 < toks.len() && (toks[j].is_sym(",") || toks[j].is_kw("and")) && cap(&toks[j + 1]) {
            run.push(toks[j + 1].text.clone());
            j += 2;
        }
        // only in declarations: "let A, B, C be sets", "for all sets A and B" (not "on N, S contains")
        let before = i.checked_sub(1).map(|k| toks[k].lower()).unwrap_or_default();
        let after = toks.get(j).map(|t| t.lower()).unwrap_or_default();
        let declaring = matches!(
            before.as_str(),
            "let" | "fix" | "sets" | "set" | "assume" | "suppose" | "for all" | "for every" | "for each" | "for any"
        ) || matches!(after.as_str(), "be" | "are" | "denote");
        if declaring && run.len() >= 2 && run.iter().any(|v| lexicon::number_set(v).is_none()) {
            for v in &run {
                math::declare(v);
            }
        }
        i = j.max(i + 1);
    }
    // "let x, y ∈ ℤ", "fix a and b", "for all m, n": one-letter variables (for products like `xy`)
    let low =
        |t: &lex::Tok| t.kind == lex::K::Word && t.text.len() == 1 && t.text.chars().all(|c| c.is_ascii_lowercase());
    for (k, t) in toks.iter().enumerate() {
        if !matches!(
            t.lower().as_str(),
            "let" | "fix" | "for all" | "for every" | "for each" | "for any" | "suppose" | "assume" | "given"
        ) {
            continue;
        }
        let mut j = k + 1;
        while let Some(v) = toks.get(j).filter(|v| low(v)) {
            math::declare_var(v.text.chars().next().expect("one letter"));
            match toks.get(j + 1) {
                Some(sep) if sep.is_sym(",") || sep.is_kw("and") => j += 2,
                _ => break,
            }
        }
    }
    let stmts = proof::split(&toks);
    // --fixed/--natural: deterministic (reseeded per sentence from its text); --varied/--random:
    // a fresh pseudo-random stream each run unless --seed pins it (user decision, session 3).
    let seed = opts.seed.unwrap_or_else(|| match opts.variance {
        Variance::Fixed | Variance::Natural => 0,
        Variance::Varied | Variance::Random => phrases::entropy_seed(),
    });
    let mut picker = phrases::Picker::new(opts.variance, seed);
    let mut r = proof::Realizer::new(&mut picker);
    r.run(&stmts);
    let (blocks, facts, theorems) = r.finish();
    Ok(Parsed { blocks, facts, theorems })
}

/// Compile stex source to a complete LaTeX document. `source_name` appears in the header comment.
pub fn compile(src: &str, source_name: &str, opts: &Options) -> Result<Compiled, Diag> {
    let p = parse(src, opts)?;
    let body = doc::render(&p.blocks);
    let plan = doc::layout(&p.blocks);
    let tex = doc::assemble(&body, &plan, source_name);
    Ok(Compiled {
        tex,
        body,
        plan,
        theorems: p.theorems.iter().map(|&id| theorems::get(id).name).collect(),
        facts: p.facts,
    })
}

/// Translate stex source to a Lean 4 file (for `cmx stex check`).
pub fn to_lean(src: &str, source_name: &str) -> Result<lean::LeanOut, Diag> {
    let p = parse(src, &Options { variance: Variance::Fixed, seed: Some(0) })?;
    Ok(lean::emit(&p.facts, &p.theorems, source_name))
}

#[cfg(test)]
mod tests;
