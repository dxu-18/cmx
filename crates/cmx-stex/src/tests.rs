//! Golden tests: the examples in the spec (prompt.txt) compiled with `--fixed`, plus determinism,
//! variance, layout, package detection and Lean translation checks.

use crate::{Options, Variance, compile, to_lean};

fn fixed(src: &str) -> String {
    compile(src, "t.stex", &Options { variance: Variance::Fixed, seed: Some(1) }).unwrap().body
}

#[track_caller]
fn golden(src: &str, want: &str) {
    assert_eq!(fixed(src), want, "\n--- source:\n{src}\n");
}

#[test]
fn spec_features_1_to_5() {
    golden("let x = 5", "Let $x = 5$.");
    golden("let x in integers", "Let $x \\in \\mathbb{Z}$.");
    golden("let x = 5, y > 4, z in Z", "Let $x = 5$, let $y > 4$, and let $z \\in \\mathbb{Z}$.");
    golden(
        "exists x in Z such that x^20+30 choose 4",
        "There exists $x \\in \\mathbb{Z}$ such that $x^{20} + \\binom{30}{4}$.",
    );
    golden(
        "define f: N to N via f(x) = 2x^34 + 7",
        "Define $f : \\mathbb{N} \\to \\mathbb{N}$ via $f(x) = 2x^{34} + 7$.",
    );
}

#[test]
fn spec_feature_10_sentence_boundaries() {
    golden("let x = 5, let y = 6, let z = 7", "Let $x = 5$, let $y = 6$, and let $z = 7$.");
    golden("let x = 5. let y = 6. let z = 7", "Let $x = 5$.\nLet $y = 6$.\nLet $z = 7$.");
    golden("let x = 5\nlet y = 6", "Let $x = 5$.\nLet $y = 6$.");
    golden("let x = 5,\ny = 6", "Let $x = 5$ and let $y = 6$.");
}

#[test]
fn spec_negation_forms() {
    for src in ["let not x = 5", "let x not = 5", "let x not equals 5", "let x neq 5", "let x != 5", "let !x = 5"] {
        golden(src, "Let $x \\neq 5$.");
    }
    golden("5! = 120", "$5! = 120$.");
}

#[test]
fn spec_by_theorem() {
    golden(
        "by CRT k == a mod m and k == b mod n",
        "By the Chinese Remainder Theorem, $k \\equiv a \\pmod{m}$ and $k \\equiv b \\pmod{n}$.",
    );
    golden(
        "k == a mod m and k == b mod n by CRT",
        "$k \\equiv a \\pmod{m}$ and $k \\equiv b \\pmod{n}$ by the Chinese Remainder Theorem.",
    );
    let c = compile("x == 1 mod 2 by the chinese remainder thm", "t.stex", &Options::default()).unwrap();
    assert_eq!(c.theorems, ["the Chinese Remainder Theorem"]);
}

#[test]
fn spec_align_with_tags() {
    golden(
        "$\nx^2 = 9 implies x^2 - 9 = 0 by subtraction, implies (x+3)(x-3) = 0 by factoring, implies x = 3 or x = -3 by simplifying\n$",
        "\\begin{align*}\n  x^{2} = 9 &\\implies x^{2} - 9 = 0 \\tag*{subtraction} \\\\\n  &\\implies (x + 3)(x - 3) = 0 \\tag*{factoring} \\\\\n  &\\implies x = 3 \\lor x = -3 \\tag*{simplifying}\n\\end{align*}",
    );
    // short standalone $…$ → display; inside a sentence → inline
    golden("$exists x such that x = 5y$", "\\[\n  \\exists x, \\; x = 5y\n\\]");
    golden("so $exists x such that x = 5y$ holds", "So $\\exists x, \\; x = 5y$ holds.");
    golden("$a = b = c$.", "\\[\n  a = b = c.\n\\]");
    assert!(crate::lex::lex("$$x$$").is_err(), "$$ is not stex syntax");
}

#[test]
fn spec_constructs() {
    golden("let x in integers w/ x > 5", "Let $x \\in \\mathbb{Z}$ with $x > 5$.");
    golden("proof\nlet x = 5", "\\textit{Proof.} Let $x = 5$.");
    golden("i.e. x = 5", "That is, $x = 5$.");
    golden("ie x = 5", "That is, $x = 5$.");
    golden("wts x = 5\nqed", "We show that $x = 5$.\nHence $x = 5$, as required. \\hfill$\\square$");
    golden("qed x = 5", "Hence $x = 5$, as required. \\hfill$\\square$");
    golden(
        "afsoc x > 0\ncontradiction",
        "Assume for the sake of contradiction that $x > 0$.\nThis is a contradiction.",
    );
    golden("assume x is even", "Suppose that $x$ is even.");
    golden("then x = 2", "Then $x = 2$.");
    golden("thus x = 2", "Thus $x = 2$.");
    golden("x > 0 because y > 0", "$x > 0$ because $y > 0$.");
    golden("since y > 0, x > 0", "Since $y > 0$, $x > 0$.");
    golden("by subtraction, y = 2", "By subtraction, $y = 2$.");
    golden("S := {1, 2, 3}", "Let $S := \\{1, 2, 3\\}$.");
    golden("exists x, P(x)", "There exists $x$ such that $P(x)$.");
    golden("for every x, P(x)", "For every $x$, $P(x)$.");
    golden("for each x P(x)", "For every $x$, $P(x)$.");
    golden("if x = 1 then x^3 = 1", "If $x = 1$, then $x^{3} = 1$.");
    golden("x = 1 or x = 2", "$x = 1$ or $x = 2$.");
    golden("x = 1 xor y = 2", "Either $x = 1$ or $y = 2$, but not both.");
    golden("x = 1 nor y = 2", "Neither $x = 1$ nor $y = 2$.");
    golden("P iff Q", "$P$ if and only if $Q$.");
    golden(
        "for all eps > 0 exists delta > 0 such that |x - a| < delta",
        "For every $\\varepsilon > 0$, there exists $\\delta > 0$ such that $\\lvert x - a \\rvert < \\delta$.",
    );
}

#[test]
fn spec_cases() {
    golden(
        "case 1 x = 5:\nthen y = 6\ncase 2 x != 5\nthen y = 7",
        "\\begin{itemize}\n  \\item Case 1: Suppose $x = 5$.\n    Then $y = 6$.\n  \\item Case 2: Suppose $x \\neq 5$.\n    Then $y = 7$.\n\\end{itemize}",
    );
}

#[test]
fn spec_symbols_and_styles() {
    golden(
        "let A = *A union &B union ~C union #D union @E",
        "Let $A = \\mathbb{A} \\cup \\mathcal{B} \\cup \\mathscr{C} \\cup \\mathtt{D} \\cup \\mathfrak{E}$.",
    );
    golden("we use _fermat and __qed here", "We use Fermat and QED here.");
    golden("bold { all of this } and italic word ok", "\\textbf{All of this} and \\textit{word} ok.");
    golden("ul{under} bf {b} it{i}", "\\underline{Under} \\textbf{b} \\textit{i}.");
    golden("we write latex {\\exists}", "We write $\\exists$.");
    golden("we use -exists literally", "We use exists literally.");
    golden("let A = empty set", "Let $A = \\varnothing$.");
    golden("let x = 5% % 6%", "Let $x = 5\\% \\bmod 6\\%$.");
    golden("let y = 3rt(5) + 7rt 6 + sqrt(x)", "Let $y = \\sqrt[3]{5} + \\sqrt[7]{6} + \\sqrt{x}$.");
    golden("let y = a+b/c + (a+b)/c", "Let $y = a + \\frac{b}{c} + \\frac{a + b}{c}$.");
    golden("let y = a * b dot c cross d", "Let $y = a \\cdot b \\cdot c \\times d$.");
    golden("let y = alpha + beta >= -pi", "Let $y = \\alpha + \\beta \\geq -\\pi$.");
    golden("x == 5 mod 6", "$x \\equiv 5 \\pmod{6}$.");
    golden("// a comment\nlet x = 1 // trailing", "Let $x = 1$.");
}

#[test]
fn spec_examples_1_to_3() {
    golden(
        "let x in integers s.t. sum from i=1 to 4 = 30.",
        "Let $x \\in \\mathbb{Z}$ such that $\\sum_{i=1}^{4} = 30$.",
    );
    golden("integrate f(x) from 1 to pi", "$\\int_{1}^{\\pi} f(x) \\,dx$.");
    golden("integral 1 to pi of f(x) dx", "$\\int_{1}^{\\pi} f(x) \\,dx$.");
    golden("integral from 1 to pi of f(x) dx", "$\\int_{1}^{\\pi} f(x) \\,dx$.");
    for st in ["s.t.", "st", "st.", "s.t", "such that"] {
        golden(&format!("exists x {st} x > 0"), "There exists $x$ such that $x > 0$.");
    }
}

#[test]
fn articles_and_numbers_stay_prose() {
    golden("let p be a prime", "Let $p$ be a prime.");
    golden("there is a unique solution in 2 steps", "There is a unique solution in 2 steps.");
    golden("the limit is 0", "The limit is $0$.");
    golden("in particular x = 1", "In particular $x = 1$.");
}

#[test]
fn variance_determinism_rules() {
    // user decision (session 3): --fixed/--natural are deterministic; --varied/--random vary per run
    // unless --seed pins them
    let src = "let x in Z. wts x^2 >= 0. then x^2 >= 0. then x^2 + 1 > 0. then x^2 + 2 > 0. \
               then x^2 + 3 > 0. then x^2 + 4 > 0. qed";
    let run = |v, seed| compile(src, "t.stex", &Options { variance: v, seed }).unwrap().body;
    for v in [Variance::Fixed, Variance::Natural] {
        assert_eq!(run(v, None), run(v, None), "{v:?} must be deterministic");
    }
    for v in [Variance::Varied, Variance::Random] {
        assert_eq!(run(v, Some(5)), run(v, Some(5)), "{v:?} with --seed must be reproducible");
        let outs: std::collections::HashSet<String> = (0..6).map(|_| run(v, None)).collect();
        assert!(outs.len() > 1, "{v:?} without --seed should vary between runs");
    }
    let fixed = fixed(src);
    assert_eq!(fixed.matches("Then").count(), 5, "fixed repeats the default connective");
    let varied = run(Variance::Varied, Some(3));
    let firsts: std::collections::HashSet<&str> = varied.lines().filter_map(|l| l.split(' ').next()).collect();
    assert!(firsts.len() >= 4, "varied should use several connectives:\n{varied}");
}

#[test]
fn natural_is_stable_per_sentence() {
    // editing one sentence must not reshuffle the phrasing of the others
    let a = "let x in Z. then x = 1. then x + 1 = 2. then x + 2 = 3. then y = 7.";
    let b = "let x in Z. then x = 1. then x + 1 = 2. then x + 2 = 3. then y = 8.";
    let na = compile(a, "t.stex", &Options::default()).unwrap().body;
    let nb = compile(b, "t.stex", &Options::default()).unwrap().body;
    let la: Vec<&str> = na.lines().collect();
    let lb: Vec<&str> = nb.lines().collect();
    assert_eq!(la[..4], lb[..4], "\n{na}\n---\n{nb}");
}

#[test]
fn witnesses_displays_and_inner_quantifiers() {
    // the user's test.stex (session 3)
    let src = "wts exists x such that x divides 3.\n\nchoose x = 3. then exists k in integers such that k = 1, so\n$\n    x = 3k implies x = 3(1) implies 3 divides x\n$\n\nqed";
    let body = fixed(src);
    assert!(body.starts_with("We show that there exists $x$ such that $x \\mid 3$."), "{body}");
    assert!(body.contains("Choose $x = 3$."), "{body}");
    assert!(body.contains("such that $k = 1$, so\n\\begin{align*}"), "{body}");
    // embedded existentials never use the sentence-initial "we have … for some …" form
    for seed in 0..40 {
        let b = compile(
            "wts exists x such that x > 0",
            "t.stex",
            &Options { variance: Variance::Random, seed: Some(seed) },
        )
        .unwrap()
        .body;
        assert!(!b.contains("that we have"), "{b}");
    }
    let l = to_lean(src, "t.stex").unwrap().text;
    assert!(l.contains("∃ (x : ℤ), x ∣ 3 := by"), "{l}");
    assert!(l.contains("obtain ⟨x, w1⟩ : ∃ x : ℤ, x = 3 := ⟨3, rfl⟩") && l.contains("refine ⟨x, ?_⟩"), "{l}");
    assert!(l.contains("obtain ⟨k, s2⟩ : ∃ (k : ℤ), k = 1"), "{l}");
    assert!(l.contains("((x = 3 * k) → (x = 3 * (1))) ∧ ((x = 3 * (1)) → (3 ∣ x))"), "{l}");
    assert!(!l.contains("(k : ℤ) :"), "k is obtained, not a theorem binder:\n{l}");
}

#[test]
fn lean_case_analysis() {
    let src = "let n in N. wts n^2 >= n\ncase 1 n = 0: then n^2 = 0\ncase 2 n >= 1: then n^2 >= n\nin either case n^2 >= n\nqed";
    let l = to_lean(src, "t.stex").unwrap().text;
    assert!(l.contains("  have s1 : n ^ 2 ≥ n := by  -- stex:4"), "{l}");
    assert!(l.contains("    rcases (show (n = 0) ∨ (n ≥ 1) by stex_auto) with c1 | c2  -- stex:2"), "{l}");
    assert!(l.contains("    · skip  -- case 1  -- stex:2\n      have s2 : n ^ 2 = 0 := by stex_auto"), "{l}");
    assert!(l.contains("    · skip  -- case 2  -- stex:3"), "{l}");
    // cases that end the proof close the goal directly
    let src2 = "let x in Z. wts x^2 >= 0\ncase 1 x >= 0: then x^2 >= 0\ncase 2 x < 0: then x^2 > 0\nqed";
    let l2 = to_lean(src2, "t.stex").unwrap().text;
    assert!(l2.contains("  rcases (show (x ≥ 0) ∨ (x < 0) by stex_auto) with c1 | c2"), "{l2}");
    assert!(l2.contains("· skip  -- case 2  -- stex:3\n    have s2 : x ^ 2 > 0 := by stex_auto  -- stex:3\n    stex_auto  -- stex:3\n"), "{l2}");
    assert!(!l2.contains("\n  stex_auto  -- stex:4"), "goal closed inside the bullets:\n{l2}");
    let prose = fixed(src2);
    assert!(prose.contains("\\end{itemize}\n\nHence"), "qed ends the case list:\n{prose}");
}

#[test]
fn library_words_in_prose() {
    golden("so G is isomorphic to H", "So $G \\cong H$.");
    golden("then f_n converges uniformly to f", "Then $f_{n} \\rightrightarrows f$.");
    golden("let x ∈ ℝ with x ≥ 0", "Let $x \\in \\mathbb{R}$ with $x \\geq 0$.");
    golden("take a polynomial of degree n", "Take a polynomial of degree $n$.");
    golden("for p prime, p > 1", "For $p$ prime, $p > 1$.");
}

#[test]
fn labels_start_paragraphs() {
    let body = fixed("theorem: x = 1\nproof\nlet x = 1\nqed");
    assert_eq!(
        body,
        "\\textit{Theorem.} $x = 1$.\n\n\\textit{Proof.} Let $x = 1$.\nHence $x = 1$, as required. \\hfill$\\square$"
    );
}

#[test]
fn natural_exists_forall_split_evenly() {
    // spec: in --natural, "There exists x such that P" and "We have P for some x" have equal weight
    let (mut there, mut have) = (0, 0);
    for seed in 0..400 {
        let b =
            compile("exists x such that x > 0", "t.stex", &Options { variance: Variance::Natural, seed: Some(seed) })
                .unwrap()
                .body;
        if b.starts_with("There exists") {
            there += 1;
        } else if b.starts_with("We have") {
            have += 1;
        }
    }
    let ratio = there as f64 / have as f64;
    assert!((0.7..1.4).contains(&ratio), "there={there} have={have}");
}

#[test]
fn contradiction_mentions_assumption_in_some_variants() {
    let mut seen = false;
    for seed in 0..60 {
        let b =
            compile("afsoc x > 0\ncontradiction", "t.stex", &Options { variance: Variance::Random, seed: Some(seed) })
                .unwrap()
                .body;
        if b.contains("our assumption that $x > 0$") || b.contains("the assumption that $x > 0$") {
            seen = true;
        }
    }
    assert!(seen, "the {{A}} variants must be reachable");
}

#[test]
fn document_layout_and_packages() {
    let short = compile("let x = 5", "t.stex", &Options::default()).unwrap();
    assert!(short.tex.contains("\\usepackage[margin=1in]{geometry}"));
    assert!(short.tex.contains("\\usepackage[T1]{fontenc}\n\\usepackage{lmodern}"), "Type 1 fonts for TinyTeX");
    assert!(short.tex.contains("\\usepackage{amsmath}"));
    assert!(!short.tex.contains("amssymb"), "no amssymb without its symbols");
    assert!(!short.tex.contains("\\raggedbottom"));
    let scr = compile("let A = ~A", "t.stex", &Options::default()).unwrap();
    assert!(scr.tex.contains("\\usepackage{mathrsfs}") && !scr.tex.contains("\\usepackage{amssymb}"));
    let bb = compile("let x in Z", "t.stex", &Options::default()).unwrap();
    assert!(bb.tex.contains("\\usepackage{amssymb}"));
    let tikz = compile("latex{\\begin{tikzpicture}\\end{tikzpicture}}", "t.stex", &Options::default()).unwrap();
    assert!(tikz.tex.contains("\\usepackage{tikz}"));
    // ~1.05 pages → tighter margins keep it on one page
    let mut src = String::new();
    while compile(&src, "t.stex", &Options::default()).unwrap().plan.pages < 1.05 {
        src.push_str("let x = 5 and this sentence has quite a lot of words in it to fill the page up.\n\n");
    }
    let near = compile(&src, "t.stex", &Options::default()).unwrap();
    assert_eq!(near.plan.margin, "0.75in", "{:?}", near.plan);
    // many short labelled sections over several pages → needspace + ragged bottom
    let mut long = String::new();
    for k in 0..30 {
        long.push_str(&format!("lemma {k}. x_{k} > 0\nproof. let x = {k}. then x + 1 > {k}. qed\n\n"));
    }
    let many = compile(&long, "t.stex", &Options::default()).unwrap();
    assert!(many.plan.pages > 1.2 && many.plan.needspace && many.plan.ragged_bottom, "{:?}", many.plan);
    assert!(many.tex.contains("\\usepackage{needspace}") && many.tex.contains("\\needspace{6\\baselineskip}"));
}

#[test]
fn lean_translation() {
    let src = "let x, y in Z. assume x = 5\nwts x + y > y\nthen x > 0\nqed\n\n\
               define f: N to N via f(n) = 2n + 1\nlemma: for all n in N, f(n) > n\nproof. let n in N. qed\n\n\
               k == a mod m by CRT";
    let l = to_lean(src, "t.stex").unwrap();
    let t = &l.text;
    assert!(t.starts_with("import Mathlib"));
    assert!(
        t.contains("theorem stex_1\n    (x : ℤ)\n    (y : ℤ)\n    (h1 : x = 5) :\n    x + y > y := by  -- stex:2"),
        "{t}"
    );
    assert!(t.contains("  have s1 : x > 0 := by stex_auto  -- stex:3"), "{t}");
    assert!(t.contains("@[simp] def f (n : ℕ) : ℕ := (2 * n) + 1  -- stex:6"), "{t}");
    assert!(t.contains("∀ (n : ℕ), f n > n := by  -- stex:7\n  intro n"), "{t}");
    assert!(t.contains("k ≡ a [ZMOD m]") && t.contains("Nat.chineseRemainder"), "{t}");
    // every marker maps back
    let have_line = t.lines().position(|l| l.contains("have s1 : x > 0")).unwrap() as u32 + 1;
    assert_eq!(crate::lean::stex_line(&l.line_map, have_line), Some(3));
}

#[test]
fn regression_setequality() {
    // the user's setequality.stex (session 3): `A\B` without spaces, "if and only if" chains with
    // `by` tags, "then" before a display, "by … qed"
    let src = "wts A \\ (A \\ B) = A intersect B\n\nlet A, B be sets, let x in A \\ (A\\B). then\n$\n\
x in A \\ (A\\B) iff x in A and x not in (A\\B) by set difference\n\
if and only if x in A and not (x in (A\\B)) by negation\n\
iff x in A and (x not in A or x in B) by De Morgan's\n\
iff false or (x in A and x in B) by law of non-contradiction\n\
iff x in A intersect B by set intersection\n$\n\nby double containment qed";
    let body = fixed(src);
    assert!(!body.contains("\\B") || body.contains("\\setminus B"), "raw \\B leaked:\n{body}");
    assert!(body.contains("let $x \\in A \\setminus (A \\setminus B)$.\nThen\n\\begin{align*}"), "{body}");
    for (step, tag) in [
        ("&\\iff x \\in A \\land x \\notin (A \\setminus B)", "set difference"),
        ("&\\iff x \\in A \\land \\neg (x \\in (A \\setminus B))", "negation"),
        ("&\\iff x \\in A \\land (x \\notin A \\lor x \\in B)", "De Morgan's"),
        ("&\\iff \\text{false} \\lor (x \\in A \\land x \\in B)", "law of non-contradiction"),
        ("&\\iff x \\in A \\cap B", "set intersection"),
    ] {
        assert!(body.contains(&format!("{step} \\tag*{{{tag}}}")), "missing step {step} / {tag}:\n{body}");
    }
    assert!(
        body.ends_with("By double containment, $A \\setminus (A \\setminus B) = A \\cap B$. \\hfill$\\square$"),
        "{body}"
    );
}

#[track_caller]
fn has(src: &str, parts: &[&str]) {
    let out = fixed(src);
    for p in parts {
        assert!(out.contains(p), "missing {p:?} in:\n{out}\n--- source:\n{src}");
    }
}

#[test]
fn induction_prose() {
    has(
        "theorem: for all n in N, sum from i=1 to n of (2i - 1) = n^2\nproof\nwe proceed by induction on n.\n\
         base case n = 0: sum from i=1 to 0 of (2i - 1) = 0\n\
         inductive step: assume sum from i=1 to k of (2i - 1) = k^2.\n\
         then sum from i=1 to k+1 of (2i - 1) = k^2 + 2(k+1) - 1 = (k + 1)^2\nqed",
        &[
            "We proceed by induction on $n$.",
            "\\textit{Base case} ($n = 0$). $\\sum_{i=1}^{0} (2i - 1) = 0$.",
            "\\textit{Inductive step.} Suppose that $\\sum_{i=1}^{k} (2i - 1) = k^{2}$.",
            "Then $\\sum_{i=1}^{k + 1} (2i - 1) = k^{2} + 2(k + 1) - 1 = (k + 1)^{2}$.",
            "as required. \\hfill$\\square$",
        ],
    );
    has("we proceed by strong induction on n.\nqed", &["We proceed by strong induction on $n$."]);
}

#[test]
fn nested_cases_and_subcases() {
    has(
        "let n in Z. wts n^2 + n is even\ncase 1 n is even: then n^2 + n = n(n+1) is even\ncase 2 n is odd:\n\
         case 2a n = 1: then n^2 + n = 2\ncase 2b n != 1: then n + 1 is even\nin either case n^2 + n is even\nqed",
        &[
            "\\begin{itemize}\n  \\item Case 1: Suppose $n$ is even.\n    Then $n^{2} + n = n(n + 1)$ is even.\n  \\item Case 2: Suppose $n$ is odd.\n    \\begin{itemize}\n      \\item Case 2a: Suppose $n = 1$.",
            "      \\item Case 2b: Suppose $n \\neq 1$.\n        Then $n + 1$ is even.\n    \\end{itemize}\n\\end{itemize}",
            "In either case, $n^{2} + n$ is even.",
        ],
    );
    has("subcase 1 n = 1: then n is odd", &["\\item Subcase 1: Suppose $n = 1$.\n    Then $n$ is odd."]);
}

#[test]
fn counting_steps() {
    has(
        "we count the committees of 3 from 10 people with a chair.\nstep 1: choose the chair in 10 ways.\n\
         step 2: choose the other two members in 9 choose 2 ways.\n\
         in total, there are 10 * (9 choose 2) = 360 ways by the multiplication principle.",
        &[
            "We count the committees of 3 from 10 people with a chair.",
            "\\item Step 1: Choose the chair in 10 ways.\n  \\item Step 2: Choose the other two members in $\\binom{9}{2}$ ways.\n\\end{itemize}",
            "In total, there are $10 \\cdot \\binom{9}{2} = 360$ ways by the multiplication principle.",
        ],
    );
    golden("in total there are 20 ways", "In total there are 20 ways.");
    golden("all in all, x = 3", "All in all, $x = 3$.");
}

#[test]
fn between_ordinals_and_casing() {
    golden("let x in R. assume x is between 1 and 2", "Let $x \\in \\mathbb{R}$.\nSuppose that $1 \\leq x \\leq 2$.");
    golden("then y lies strictly between 0 and 1", "Then $0 < y < 1$.");
    golden(
        "the nth term is a_n. the (k+1)st term is a_(k+1). the n-th prime is p_n. the 2nd and 3rd terms",
        "The $n$th term is $a_{n}$.\nThe $(k + 1)$st term is $a_{k + 1}$.\nThe $n$-th prime is $p_{n}$.\nThe 2nd and 3rd terms.",
    );
    golden("THEN X = 5. Let Y be odd", "Then $X = 5$.\nLet $Y$ be odd.");
}

#[test]
fn contradiction_negates_the_goal() {
    has(
        "let x in R. assume x^2 < 4\nwts x < 2\nsuppose not. then x >= 2, so x^2 >= 4\ncontradiction\nqed",
        &["Assume for the sake of contradiction that $x \\geq 2$.", "This is a contradiction.", "Hence $x < 2$"],
    );
    has("let n in Z. wts n is even\nsuppose not.", &["that $n$ is odd."]);
    golden(
        "assume the contrary. then x = 2",
        "Assume for the sake of contradiction that the claim is false.\nThen $x = 2$.",
    );
    // what the user writes stays literal
    golden("let not x < 5", "Let $x \\not< 5$.");
}

#[test]
fn typed_declarations_products_and_connectives() {
    golden(
        "let p be a prime. let n be an integer. let x, y be real numbers",
        "Let $p$ be a prime.\nLet $n$ be an integer.\nLet $x, y$ be real numbers.",
    );
    golden(
        "then x^2 >= 0, so x^2 + 1 > 0, thus x^2 + 1 != 0",
        "Then $x^{2} \\geq 0$, so $x^{2} + 1 > 0$, thus $x^{2} + 1 \\neq 0$.",
    );
    golden("choose a committee of 3 people. choose k = 4", "Choose a committee of 3 people.\nChoose $k = 4$.");
    golden(
        "then (a + b)^2 = a^2 + ab + ba + b^2 = a^2 + 2ab + b^2",
        "Then $(a + b)^{2} = a^{2} + ab + ba + b^{2} = a^{2} + 2ab + b^{2}$.",
    );
}

#[test]
fn lean_new_constructs() {
    let lean = |src: &str| to_lean(src, "t.stex").unwrap().text;
    // ab with a, b declared is a product; ∑ (2i - 1) over ℕ is stated in ℤ
    let l = lean("let a, b in R. wts (a + b)^2 = a^2 + 2ab + b^2\nqed");
    assert!(l.contains("(a + b) ^ 2 = ((a ^ 2) + (2 * (a * b))) + (b ^ 2)"), "{l}");
    let l = lean(
        "theorem: for all n in N, sum from i=1 to n of (2i - 1) = n^2\nproof\nwe proceed by induction on n.\n\
         base case n = 0: sum from i=1 to 0 of (2i - 1) = 0\ninductive step: assume sum from i=1 to k of (2i - 1) = k^2.\n\
         then sum from i=1 to k+1 of (2i - 1) = k^2 + 2(k+1) - 1 = (k + 1)^2\nqed",
    );
    assert!(l.contains("∀ (n : ℕ), (∑ i ∈ Finset.Icc 1 n, ((2 * i) - 1) : ℤ) = n ^ 2"), "{l}");
    assert!(l.contains("| succ k ih =>"), "{l}");
    assert!(!l.contains("(k : ℤ)"), "{l}");
    // strong induction uses the hypothesis through stex_strong
    let l = lean(
        "theorem: for all n in N, 2^n >= n + 1\nproof\nwe proceed by strong induction on n.\n\
         case 1 n = 0: then 2^0 = 1\ncase 2 n >= 1: then 2^(n-1) >= n by the inductive hypothesis\nqed",
    );
    assert!(l.contains("induction n using Nat.strong_induction_on with"), "{l}");
    assert!(l.contains("first | stex_strong n | stex_auto"), "{l}");
    // the ∃ binder's type carries to the witness, and ℕ congruences use MOD
    let l = lean("wts exists k in N such that k == 2 mod 3\nchoose k = 8.\nqed");
    assert!(l.contains("∃ k : ℕ, k = 8") && l.contains("[MOD 3]"), "{l}");
    // suppose not: the negated goal, by name
    let l = lean("let x in R. assume x^2 < 4\nwts x < 2\nsuppose not.\nqed");
    assert!(l.contains("have s1 : x ≥ 2 := by first | exact hc1"), "{l}");
    // a count in prose is checked
    let l = lean("there are 5 choose 2 = 11 ways to pick two of five items.");
    assert!(l.contains("Nat.choose 5 2 = 11"), "{l}");
}

/// Session-5 corpus audit (STEX-DESIGN §12): one test per root cause.
#[test]
fn audit_a_math_grammar() {
    // A1 ellipsis, A10 lenient $…$ (the user's prime.stex)
    golden(
        "let N = p_1 cross p_2 cross ... cross p_k + 1.",
        "Let $N = p_{1} \\times p_{2} \\times \\cdots \\times p_{k} + 1$.",
    );
    has("$N = (p_1 cross p_2 cross ... cross P) + 1$", &["N = (p_{1} \\times p_{2} \\times \\cdots \\times P) + 1"]);
    golden("A = {p_1, p_2, ..., P}", "$A = \\{p_{1}, p_{2}, \\ldots, P\\}$.");
    golden("then a_1 + ... + a_n = 0.", "Then $a_{1} + \\cdots + a_{n} = 0$.");
    has("$x = 5 qux zip + cross$", &["\\text{"]); // a bad token is local text, not the whole formula
    // A2 juxtaposition across a space
    golden("so a + b >= 2 sqrt(ab).", "So $a + b \\geq 2\\sqrt{ab}$.");
    golden("for all a, b in R, e^(a + b) = e^a e^b.", "For all $a, b \\in \\mathbb{R}$, $e^{a + b} = e^{a} e^{b}$.");
    golden("then |A times B| = |A| |B|.", "Then $\\lvert A \\times B \\rvert = \\lvert A \\rvert \\lvert B \\rvert$.");
    golden("then N = p_1 p_2 ... P + 1.", "Then $N = p_{1} p_{2} \\cdots P + 1$.");
    golden("for each x P(x)", "For every $x$, $P(x)$.");
    // A3 composition, A4 id_X / comp(A), A5 big union, A6 bars, A7 |A ∪ B|, A8 binom parens, A9 set-builder words
    golden("then g = (g o f) o h.", "Then $g = (g \\circ f) \\circ h$.");
    golden("so g o f = id_X.", "So $g \\circ f = \\operatorname{id}_{X}$.");
    golden("also comp(comp(A)) = A.", "Also $(A^{c})^{c} = A$.");
    has("then x in union from i=1 to infinity of A_i.", &["\\bigcup_{i=1}^{\\infty} A_{i}"]);
    golden(
        "thus ||x| - |y|| <= |x - y|.",
        "Thus $\\lvert \\lvert x \\rvert - \\lvert y \\rvert \\rvert \\leq \\lvert x - y \\rvert$.",
    );
    golden("so ||v|| >= 0 and x > 0 || x < 1.", "So $\\lVert v \\rVert \\geq 0$ and $x > 0$ or $x < 1$.");
    golden("|A union B| = 3", "$\\lvert A \\cup B \\rvert = 3$.");
    golden("so n choose (n - k) = 1.", "So $\\binom{n}{n - k} = 1$.");
    golden("let S = {n in N | n is even}.", "Let $S = \\{n \\in \\mathbb{N} \\mid n \\text{ is even}\\}$.");
}

#[test]
fn audit_b_keyword_and_word_collisions() {
    golden("then ax + by = d.", "Then $ax + by = d$."); // B1
    golden("note that a + b/c > 0, b/c c > 0.", "Note that $a + \\frac{b}{c} > 0$, because $c > 0$."); // B2
    golden("every perfect square is odd.", "Every perfect square is odd."); // B3
    golden("the cross product of u and v.", "The cross product of $u$ and $v$.");
    golden("this is the AM-GM inequality for a 4-digit number.", "This is the AM-GM inequality for a 4-digit number."); // B4
    golden("a sigma-algebra", "A $\\sigma$-algebra.");
    golden("a number cannot both be and not be odd.", "A number cannot both be and not be odd."); // B5
    golden("the functions from A to B.", "The functions from $A$ to $B$."); // B6
    golden("f maps R to R.", "$f$ maps $\\mathbb{R}$ to $\\mathbb{R}$.");
    golden(
        "the solutions of x + y = 10 in nonnegative integers.",
        "The solutions of $x + y = 10$ in nonnegative integers.",
    );
    golden("then P xor Q holds.", "Then $P \\oplus Q$ holds."); // B7
}

#[test]
fn audit_c_quantifiers() {
    // C1 articles
    golden("there is a largest prime number.", "There is a largest prime number.");
    golden("then there is a largest even number m.", "Then there is a largest even number $m$.");
    golden("a triangle is a polygon.", "A triangle is a polygon.");
    golden("a 2 by 2 board.", "A $2$ by 2 board.");
    // C2 lists, signatures, intervals inside the binder
    golden("there are integers m, n s.t. a = dm.", "There exist integers $m$, $n$ such that $a = dm$.");
    has(
        "there is a function g: Y to X such that g = f.",
        &["There exists a function $g : Y \\to X$ such that $g = f$"],
    );
    golden("there is c in (a, b) such that c > 0.", "There exists $c \\in (a, b)$ such that $c > 0$.");
    golden(
        "there exist p, q in Z with q != 0 such that x = p/q.",
        "There exist $p, q \\in \\mathbb{Z}$ with $q \\neq 0$ such that $x = \\frac{p}{q}$.",
    );
    golden(
        "for all n, k in N with k <= n, n choose k > 0.",
        "For all $n, k \\in \\mathbb{N}$ with $k \\leq n$, $\\binom{n}{k} > 0$.",
    );
    // C3 counts and prose after "there is/are"
    golden("there are 10 choose 3 = 120 subsets.", "There are $\\binom{10}{3} = 120$ subsets.");
    golden("there are only finitely many primes.", "There are only finitely many primes.");
    golden("there is a subtle point here, and it matters.", "There is a subtle point here, and it matters.");
    // C4 with, C5 agreement and idioms
    golden("for every e > 0 there is d > 0 with d < e.", "For every $e > 0$, there exists $d > 0$ such that $d < e$.");
    golden("for all sets A and B, A = A.", "For all sets $A$ and $B$, $A = A$.");
    golden("for all but finitely many n in N, n > 3.", "For all but finitely many $n \\in \\mathbb{N}$, $n > 3$.");
    golden("there does not exist an integer n with n^2 = 2.", "There is no integer $n$ such that $n^{2} = 2$.");
}

#[test]
fn audit_d_declarations() {
    golden("fix a, b in N with a, b > 0.", "Let $a, b \\in \\mathbb{N}$ with $a, b > 0$."); // D1
    golden(
        "let x, y in R s.t. x + y = 5 and x - y = 1.",
        "Let $x, y \\in \\mathbb{R}$ such that $x + y = 5$ and $x - y = 1$.",
    ); // D2
    golden("let F_0 = 0, F_1 = 1 and F_2 = 1.", "Let $F_{0} = 0$, let $F_{1} = 1$, and let $F_{2} = 1$.");
    golden("let a_1, ..., a_n be reals.", "Let $a_{1}, \\ldots, a_{n}$ be reals.");
    golden("let x in Z, sum from i = 1 to 4 = 30.", "Let $x \\in \\mathbb{Z}$ such that $\\sum_{i=1}^{4} = 30$."); // D3 (spec)
    golden(
        "let A, B, C be sets with B subseteq C.", // D4: a declared C is not ℂ
        "Let $A, B, C$ be sets with $B \\subseteq C$.",
    );
    golden("let f: A to B and g: B to C be maps.", "Let $f : A \\to B$ and $g : B \\to C$ be maps.");
    golden("since kl in Z, a | c.", "Since $kl \\in \\mathbb{Z}$, $a \\mid c$.");
}

#[test]
fn audit_e_sentences() {
    // E1 a line starting with a verb continues the sentence (the user's prime.stex)
    golden(
        "let P denote the largest prime such that\nA = {p_1, ..., P}\nis the set of all primes.",
        "Let $P$ denote the largest prime such that $A = \\{p_{1}, \\ldots, P\\}$ is the set of all primes.",
    );
    // E2 suppose otherwise, … ; E3 proof label above a list; E4 lemma claim after a setup sentence
    has(
        "wts every finite set has a maximum\nsuppose otherwise, and let S be a counterexample.",
        &["Assume for the sake of contradiction that the claim is false.\nLet $S$ be a counterexample."],
    );
    has(
        "claim: x = 1\nproof\ncase 1 x > 0: then x = 1\ncase 2 x <= 0: then x = 1\nqed",
        &["\\textit{Proof.}\n\n\\begin{itemize}"],
    );
    has(
        "lemma: let a, b in Z. if a divides b then a divides 2b.\nproof. then 2b = 2b.\nqed",
        &["Hence if $a \\mid b$, then $a \\mid 2b$, as required."],
    );
    // E5 joins, E6 wording, E7 no repeated conclusion, E8 derivative, E9 divisible by, E10 by THM
    golden(
        "so a has a prime divisor p, and p | n since p | a and a | n.",
        "So $a$ has a prime divisor $p$, and $p \\mid n$ since $p \\mid a$ and $a \\mid n$.",
    );
    golden("wlog we may write x = 1.", "Without loss of generality, we may write $x = 1$.");
    golden("assume the claim for small boards.", "Assume the claim for small boards.");
    has("wts x = 1\nthen x = 1.\nhence x = 1.\nqed", &["Hence $x = 1$. \\hfill$\\square$"]);
    golden("the derivative of f with respect to x is 2x.", "The derivative of $f$ with respect to $x$ is $2x$.");
    golden("then 8 - 1 = 7 is divisible by 7.", "Then $8 - 1 = 7$ is divisible by $7$.");
    golden(
        "since gcd(3, 5) = 1, by CRT there is k = 8.",
        "Since $\\gcd(3, 5) = 1$, by the Chinese Remainder Theorem there is $k = 8$.",
    );
    golden("so if p_1, ..., p_k were all, then N > 1.", "So if $p_{1}, \\ldots, p_{k}$ were all, then $N > 1$.");
}

#[test]
fn case_labels_and_auto_numbering() {
    // a condition that starts with a letter is not a label: numbered automatically
    has(
        "case k is odd. then k = 1.\ncase k is even. then k = 0.",
        &["\\item Case 1: Suppose $k$ is odd.", "\\item Case 2: Suppose $k$ is even."],
    );
    has("case n = 0. a.\ncase 0 < n. b.", &["Case 1: Suppose $n = 0$.", "Case 2: Suppose $0 < n$."]);
    has("case n even. a.\ncase n odd. b.", &["Case 1: Suppose $n$ is even.", "Case 2: Suppose $n$ is odd."]);
    // explicit labels: numbers, number+letter, a letter before a new expression, "(i)", "k:"
    has("case k n = 1: then x = 1.", &["Case k: Suppose $n = 1$.\n    Then $x = 1$."]);
    has(
        "case 1 n = 0: then x = 1.\ncase 2a n > 0: then x = 2.",
        &["Case 1: Suppose $n = 0$.", "Case 2a: Suppose $n > 0$."],
    );
    has("case (i) x > 0. a.", &["Case (i): Suppose $x > 0$."]);
    has("case k:\nk = 1.", &["\\item Case k:\n    $k = 1$."]);
    // sub-cases number under their case; siblings stay siblings
    has(
        "case x is odd\nsubcase y is odd\na = 1.\nsubcase y is even\na = 2.\ncase x is even\nb = 1.",
        &[
            "\\item Subcase 1.1: Suppose $y$ is odd.",
            "\\item Subcase 1.2: Suppose $y$ is even.",
            "\\item Case 2: Suppose $x$ is even.",
        ],
    );
}

#[test]
fn blank_lines_inside_cases() {
    // more cases follow: the blank line is a paragraph break inside the case, not the list's end
    has(
        "case x is odd\nthen y = 2.\n\nhence z = 3.\ncase x is even\nthen y = 4.\n\nhence z = 3.\n\nexhaustive",
        &[
            "    Then $y = 2$.\n\n    Hence $z = 3$.\n  \\item Case 2: Suppose $x$ is even.",
            "\\end{itemize}\n\nThese cases are exhaustive",
        ],
    );
    // nothing list-related follows: the blank line ends the list (as before)
    has("case 1 x > 0. a.\ncase 2 x <= 0. b.\n\nthus c.", &["\\end{itemize}\n\nThus $c$."]);
    // a new analysis ("case 1" again) after a paragraph starts a new list
    has(
        "case 1 x > 0. a.\ncase 2 x <= 0. b.\n\nnext we show y = 1.\n\ncase 1 y > 0. c.",
        &["\\end{itemize}\n\nNext we show $y = 1$.\n\n\\begin{itemize}\n  \\item Case 1: Suppose $y > 0$."],
    );
}

#[test]
fn exhaustive_and_case_on_only_say_what_the_user_wrote() {
    let cases = "case k is odd. a.\ncase k is even. b.";
    // no keyword: just the cases, no announcement, no exhaustiveness sentence, no reason
    let plain = fixed(cases);
    for never in ["exhaustive", "proceed", "either odd or even", "In all"] {
        assert!(!plain.contains(never), "{never:?} in:\n{plain}");
    }
    // a bare "exhaustive": the plain statement, never a derived reason
    for (src, want) in [
        (format!("cases exhaustive\n{cases}"), "The following cases are exhaustive.\n\n\\begin{itemize}"),
        (format!("the cases are exhaustive\n{cases}"), "The following cases are exhaustive.\n\n\\begin{itemize}"),
        (format!("{cases}\nexhaustive"), "\\end{itemize}\n\nThese cases are exhaustive."),
        (format!("{cases}\nthese cases are exhaustive."), "\\end{itemize}\n\nThese cases are exhaustive."),
    ] {
        let out = fixed(&src);
        assert!(out.contains(want), "{src}\n---\n{out}");
        assert!(!out.contains("odd or even") && !out.contains("since"), "derived reason in:\n{out}");
    }
    // the user's reason, before or after the keyword, in the user's word and order
    has(
        &format!("exhaustive cases b/c k is either odd or even\n{cases}"),
        &["The following cases are exhaustive, because $k$ is either odd or even.\n\n\\begin{itemize}"],
    );
    has(
        &format!("because k is either odd or even, exhaustive cases\n{cases}"),
        &["Because $k$ is either odd or even, the following cases are exhaustive."],
    );
    has(
        &format!("since k is either odd or even, the following cases are exhaustive\n{cases}"),
        &["Since $k$ is either odd or even, the following cases are exhaustive."],
    );
    has(
        &format!("{cases}\nexhaustive since k is odd or even"),
        &["These cases are exhaustive, since $k$ is odd or even."],
    );
    has(
        &format!("{cases}\nb/c k is odd or even, the cases are exhaustive"),
        &["Because $k$ is odd or even, these cases are exhaustive."],
    );
    has(&format!("{cases}\nexhaustive by trichotomy"), &["These cases are exhaustive by trichotomy."]);
    // what holds in every case: only when the user says it
    has(
        &format!("{cases}\nexhaustive b/c k is odd or even, so k = k"),
        &["These cases are exhaustive, because $k$ is odd or even.\nIn all exhaustive cases, $k = k$."],
    );
    let closed = fixed(&format!("{cases}\nin exhaustive cases, k = k"));
    assert!(closed.contains("\\end{itemize}\n\nIn exhaustive cases, $k = k$."), "{closed}");
    assert!(!closed.contains("These cases are exhaustive"), "{closed}");
    has(&format!("{cases}\nsince the cases are exhaustive, k = k"), &["Since the cases are exhaustive, $k = k$."]);
    // "case on X" announces the analysis; nothing else is added
    let on = fixed(&format!("case on k\n{cases}"));
    assert!(on.starts_with("We proceed by case analysis on $k$.\n\n\\begin{itemize}"), "{on}");
    assert!(!on.contains("exhaustive"), "{on}");
    for form in ["cases on k:", "casing on k", "we case on k", "case analysis on k", "we proceed by cases on k"] {
        has(&format!("{form}\n{cases}"), &["We proceed by case analysis on $k$.\n\n\\begin{itemize}"]);
    }
    has(
        &format!("case on n mod 2, exhaustive b/c n is even or odd\n{cases}"),
        &[
            "We proceed by case analysis on $n \\bmod 2$.\nThe following cases are exhaustive, because $n$ is even or odd.",
        ],
    );
    // inside a case, before its sub-cases
    has(
        "case x is odd\ncase on y\nsubcase y is odd\na.\nsubcase y is even\nb.\ncase x is even\nc.",
        &["\\item Case 1: Suppose $x$ is odd.\n    We proceed by case analysis on $y$.\n    \\begin{itemize}"],
    );
    // after sub-cases, "exhaustive" closes only the sub-case list
    has(
        "case 1 x > 0\nsubcase y > 0\na.\nsubcase y <= 0\nb.\nexhaustive\ncase 2 x <= 0\nc.",
        &["    \\end{itemize}\n    These cases are exhaustive.\n  \\item Case 2: Suppose $x \\leq 0$."],
    );
    // a closing sentence that states the goal ends the proof without restating it
    has(
        "wts z = 3\ncase x is odd. hence z = 3.\ncase x is even. hence z = 3.\nin all exhaustive cases, z = 3\nqed",
        &["In all exhaustive cases, $z = 3$. \\hfill$\\square$"],
    );
    // the varied modes never invent a reason either
    for seed in 0..20 {
        for v in [Variance::Natural, Variance::Varied, Variance::Random] {
            let out = compile(
                &format!("cases exhaustive\n{cases}\nexhaustive"),
                "t.stex",
                &Options { variance: v, seed: Some(seed) },
            )
            .unwrap()
            .body;
            assert!(!out.contains("since") && !out.contains("because") && !out.contains("odd or even"), "{out}");
        }
    }
}

#[test]
fn existential_agreement() {
    for (src, want) in [
        ("there are l in integers such that n = 2l + 1", "There exists $l \\in \\mathbb{Z}$ such that"),
        ("there is integers a, b such that a = b", "There exist integers $a$, $b$ such that"),
        ("there are no integer x such that x^2 = 2", "There is no integer $x$ such that"),
        ("there is no integers a, b such that a = b", "There are no integers $a$, $b$ such that"),
    ] {
        has(src, &[want]);
    }
    // every phrasing the bank can pick agrees with its binder
    for seed in 0..40 {
        for v in [Variance::Natural, Variance::Varied, Variance::Random] {
            let out = compile(
                "then exists l in integers st k = 2l + 1.",
                "t.stex",
                &Options { variance: v, seed: Some(seed) },
            )
            .unwrap()
            .body;
            assert!(!out.contains("here are $l") && !out.contains("here exist $l"), "{out}");
        }
    }
}

#[test]
fn words_near_bars_and_numbers() {
    golden("next we show |x - y| = 1.", "Next we show $\\lvert x - y \\rvert = 1$.");
    golden(
        "so n^2 leaves remainder 0 or 1 upon division by 4.",
        "So $n^{2}$ leaves remainder $0$ or $1$ upon division by $4$.",
    );
    has("let x, y in Z. then xy is odd.", &["Then $xy$ is odd."]);
    // undeclared letters, and English words spelled with declared letters, stay prose
    golden("xy is odd.", "Xy is odd.");
    has("let a, n in Z. then a = n, an integer.", &["an integer"]);
}
