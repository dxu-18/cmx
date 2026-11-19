//! Regression suite over `tests/corpus/*.stex` (195 hand-written proofs using every construct and
//! alias, with ordinary prose; STEX-DESIGN §12).
//!
//! - Each proof compiled with `--fixed` must match its reviewed snapshot in `tests/corpus/expected/`.
//!   After an intended change, re-bless with `STEX_BLESS=1 cargo test -p cmx-stex --test corpus` and
//!   review the diff (`git diff crates/cmx-stex/tests/corpus/expected`).
//! - Lints that catch the failure classes found in the audit hold for every proof.
//! - All variance modes and the Lean translation run on every proof without errors.

use cmx_stex::{Options, Variance, compile, to_lean};
use std::path::{Path, PathBuf};

fn corpus() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus");
    let mut v: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "stex"))
        .collect();
    v.sort();
    assert!(v.len() >= 150, "corpus has {} files", v.len());
    v
}

fn fixed(src: &str, name: &str) -> String {
    compile(src, name, &Options { variance: Variance::Fixed, seed: None }).expect("compiles").body
}

#[test]
fn snapshots() {
    let bless = std::env::var("STEX_BLESS").is_ok_and(|v| v == "1");
    let exp_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus/expected");
    std::fs::create_dir_all(&exp_dir).unwrap();
    let mut bad = Vec::new();
    for f in corpus() {
        let name = f.file_stem().unwrap().to_string_lossy().to_string();
        let body = fixed(&std::fs::read_to_string(&f).unwrap(), &format!("{name}.stex"));
        let exp = exp_dir.join(format!("{name}.tex"));
        if bless {
            std::fs::write(&exp, &body).unwrap();
            continue;
        }
        match std::fs::read_to_string(&exp) {
            Ok(want) if want == body => {}
            Ok(want) => bad.push(format!("{name}:\n--- expected\n{want}\n--- got\n{body}")),
            Err(_) => bad.push(format!("{name}: no snapshot (run with STEX_BLESS=1)")),
        }
    }
    assert!(bad.is_empty(), "{} snapshot mismatches:\n\n{}", bad.len(), bad.join("\n\n"));
}

/// Prose parts of a body (math replaced by a placeholder).
fn prose(body: &str) -> String {
    let mut out = String::new();
    let mut in_math = false;
    for c in body.chars() {
        if c == '$' && !out.ends_with('\\') {
            in_math = !in_math;
            if !in_math {
                out.push_str("MATH");
            }
            continue;
        }
        if !in_math {
            out.push(c);
        }
    }
    out
}

#[test]
fn lints() {
    let mut bad = Vec::new();
    for f in corpus() {
        let name = f.file_stem().unwrap().to_string_lossy().to_string();
        let body = fixed(&std::fs::read_to_string(&f).unwrap(), &format!("{name}.stex"));
        let p = prose(&body);
        let mut fail = |what: &str, ctx: &str| bad.push(format!("{name}: {what}: {ctx}"));
        // math split into pieces: "$x$ $=$ $y$"
        for op in ["$=$", "$\\neq$", "$<$", "$>$", "$\\leq$", "$\\geq$", "$\\in$", "$+$", "$\\times$"] {
            if body.contains(op) {
                fail("lone operator", op);
            }
        }
        if body.contains("$ $") {
            fail("adjacent math pieces", &body[body.find("$ $").unwrap().saturating_sub(30)..]);
        }
        // stex keywords that must never reach the output as words
        for w in p.split(|c: char| c.is_whitespace() || c == ',' || c == '.') {
            if matches!(w, "afsoc" | "aftsoc" | "wts" | "s.t." | "b/c" | "w/" | "qed" | "st." | "contra" | "forall") {
                fail("keyword leaked into prose", w);
            }
        }
        // the whole-formula \text{} fallback, raw dots, articles read as variables
        if body.contains("\\text{") && body.contains("\\_") {
            fail("formula rendered as text", "");
        }
        if p.contains("...") {
            fail("raw ellipsis", "");
        }
        for article_then_noun in ["there exists $a$ such that", "$a$ such that"] {
            if body.contains(article_then_noun) {
                fail("article read as a variable", article_then_noun);
            }
        }
        for frag in ["\nIs the ", "\nIs larger", "such that such that", "such that with", ",,"] {
            if body.contains(frag) {
                fail("sentence structure", frag);
            }
        }
    }
    assert!(bad.is_empty(), "{} lint failures:\n{}", bad.len(), bad.join("\n"));
}

#[test]
fn every_mode_and_lean_translation_run() {
    for f in corpus() {
        let src = std::fs::read_to_string(&f).unwrap();
        let name = f.file_name().unwrap().to_string_lossy().to_string();
        for (variance, seed) in [
            (Variance::Natural, None),
            (Variance::Varied, Some(7)),
            (Variance::Random, Some(7)),
            (Variance::Fixed, Some(1)),
        ] {
            compile(&src, &name, &Options { variance, seed }).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        }
        to_lean(&src, &name).unwrap_or_else(|e| panic!("{name}: lean: {e:?}"));
    }
}
