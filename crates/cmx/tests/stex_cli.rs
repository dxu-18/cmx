//! `cmx stex compile` / `cmx stex check` end to end: real binary, temp directory, controlled PATH
//! and HOME (so a TeX or Lean install on the machine doesn't change the expectations). Compiling
//! uses the bundled TinyTeX (vendor/tinytex), unpacked on first use.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

struct Dir(PathBuf);

impl Dir {
    fn new(tag: &str) -> Dir {
        let p = std::env::temp_dir().join(format!("cmx-stex-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        Dir(p)
    }
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn cmx_env(dir: &Path, args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut c = Command::new(env!("CARGO_BIN_EXE_cmx"));
    c.args(args)
        .current_dir(dir)
        .env("PATH", "/usr/bin:/bin") // system tools only: no TeX, no lake
        .env("HOME", dir)
        .env_remove("CMX_MATHLIB")
        .env_remove("CMX_TEX");
    for (k, v) in env {
        c.env(k, v);
    }
    c.output().expect("run cmx")
}

fn cmx(dir: &Path, args: &[&str]) -> Output {
    cmx_env(dir, args, &[])
}

#[test]
fn compile_builds_pdf_with_bundled_tinytex() {
    let d = Dir::new("compile");
    std::fs::create_dir(d.path().join("notes")).unwrap();
    std::fs::write(
        d.path().join("notes/hw.stex"),
        "let x = 5, y > 4, z in Z\nwts x = 5\nlet A = ~A union *B\ncase 1 x = 5: then x > 4\ncase 2 x != 5: then x = 5\nqed\n",
    )
    .unwrap();
    let out = cmx(d.path(), &["stex", "compile", "notes/hw.stex", "--fixed"]);
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(0), "stderr: {err}\nstdout: {}", String::from_utf8_lossy(&out.stdout));
    let tex = std::fs::read_to_string(d.path().join("notes/hw.tex")).expect(".tex next to the .stex");
    assert!(tex.contains("Let $x = 5$, let $y > 4$, and let $z \\in \\mathbb{Z}$."), "{tex}");
    let pdf = std::fs::read(d.path().join("notes/hw.pdf")).expect("PDF built by the bundled TinyTeX");
    assert!(pdf.starts_with(b"%PDF"), "not a PDF");
    // recompiling overwrites the same files and leaves no aux/log behind (spec: no new names)
    std::fs::write(d.path().join("notes/hw.stex"), "let x = 6").unwrap();
    let mut again = cmx(d.path(), &["stex", "compile", "notes/hw.stex", "--fixed", "--xelatex"]);
    if missing_host_lib(&again) {
        again = cmx(d.path(), &["stex", "compile", "notes/hw.stex", "--fixed"]);
    }
    assert_eq!(again.status.code(), Some(0), "{}", String::from_utf8_lossy(&again.stderr));
    assert!(std::fs::read_to_string(d.path().join("notes/hw.tex")).unwrap().contains("Let $x = 6$."));
    let mut names: Vec<String> = std::fs::read_dir(d.path().join("notes"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    names.sort();
    assert_eq!(names, ["hw.pdf", "hw.stex", "hw.tex"]);
}

#[test]
fn compile_explains_missing_engine() {
    let d = Dir::new("noengine");
    std::fs::write(d.path().join("a.stex"), "let x = 1").unwrap();
    let out = cmx_env(d.path(), &["stex", "compile", "a.stex"], &[("CMX_TEX", "/nonexistent/bin")]);
    assert_eq!(out.status.code(), Some(4));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("/nonexistent/bin/pdflatex not found") && err.contains("a.tex is ready"), "{err}");
    assert!(d.path().join("a.tex").is_file());
}

#[test]
fn compile_is_deterministic_per_variance_and_seed() {
    let d = Dir::new("determinism");
    let src = "let x in Z. wts x^2 >= 0. then x^2 >= 0. then x^2 + 1 > 0. qed\nexists y such that y > x\n";
    std::fs::write(d.path().join("p.stex"), src).unwrap();
    let read = |args: &[&str]| {
        let mut a = vec!["stex", "compile", "p.stex"];
        a.extend_from_slice(args);
        let out = cmx(d.path(), &a);
        assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
        std::fs::read_to_string(d.path().join("p.tex")).unwrap()
    };
    let natural1 = read(&[]);
    let natural2 = read(&["--natural"]);
    assert_eq!(natural1, natural2, "--natural is the default and deterministic");
    let r1 = read(&["--random", "--seed", "7"]);
    let r2 = read(&["--random", "--seed", "7"]);
    assert_eq!(r1, r2);
}

#[test]
fn compile_reports_errors_with_line_numbers() {
    let d = Dir::new("errors");
    std::fs::write(d.path().join("bad.stex"), "let x = 5\n$$x$$\n").unwrap();
    let out = cmx(d.path(), &["stex", "compile", "bad.stex"]);
    assert_eq!(out.status.code(), Some(1));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("bad.stex:2:") && err.contains("$$"), "{err}");
    let bad_flag = cmx(d.path(), &["stex", "compile", "bad.stex", "--fast"]);
    assert_eq!(bad_flag.status.code(), Some(2));
    let two = cmx(d.path(), &["stex", "compile", "bad.stex", "--fixed", "--random"]);
    assert_eq!(two.status.code(), Some(2));
}

#[test]
fn check_writes_lean_in_cwd_and_explains_install() {
    let d = Dir::new("check");
    std::fs::create_dir(d.path().join("src")).unwrap();
    std::fs::write(d.path().join("src/p.stex"), "let x in Z. assume x = 5\nwts x + 1 = 6\nqed\n").unwrap();
    let out = cmx(d.path(), &["stex", "check", "src/p.stex"]);
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(4), "{err}");
    assert!(err.contains("Lean 4 + Mathlib are not installed") && err.contains("lake exe cache get"), "{err}");
    let lean = std::fs::read_to_string(d.path().join("p.lean")).expect("p.lean in the current directory");
    assert!(lean.contains("import Mathlib") && lean.contains("(h1 : x = 5)") && lean.contains("x + 1 = 6"), "{lean}");
}

/// Every construct (and the regressions users hit) must produce LaTeX that real TeX accepts.
#[test]
fn every_construct_compiles_with_tinytex() {
    let d = Dir::new("all");
    let src = r#"theorem: for all n in N, n^2 >= n
proof
let n in N. wts n^2 >= n
case 1 n = 0: then n^2 = 0 >= 0
case 2 n >= 1: then n^2 = n * n >= n since n >= 1
in either case n^2 >= n
qed

lemma 2. sqrt 2 is irrational
proof. afsoc sqrt(2) = p/q with p, q in Z and gcd(p, q) = 1
then 2q^2 = p^2, so 2 divides p by Euclid's lemma
contradiction since gcd(p,q) = 1
qed

let x = 5, y > 4, z in Z. exists x in Z such that x^20+30 choose 4.
define f: N to N via f(x) = 2x^34 + 7. let A = *A union &B union ~C union #D union @E.
by CRT k == a mod m and k == b mod n. let y = 3rt(5) + 7rt 6 + sqrt(x) and 5% % 6% = 5.
$
x^2 = 9 implies x^2 - 9 = 0 by subtraction, implies (x+3)(x-3) = 0 by factoring, implies x = 3 or x = -3 by simplifying
$
integrate f(x) from 1 to pi. limit as x to 0 of sin(x)/x = 1. derivative of x^2 wrt x = 2x.
for all eps > 0 exists delta > 0 such that |x - a| < delta implies |f(x) - f(a)| < eps
bold {important} italic word and ul{under}. latex{\begin{tikzpicture}\draw (0,0) -- (1,1);\end{tikzpicture}}
S := {x in Z | x > 0}. $a = b implies c = d by hyp$. not x in S. x !== 1 mod 2. A setminus B subseteq A cross B.
G is isomorphic to H. x ∈ ℝ and x ≤ 5. f_n converges uniformly to f. x hat + bbR + calF + frakg.
let x in A \ (A\B). choose x = 3. so
$
x in A iff x in A and x not in (A\B) by set difference
if and only if x in A intersect B by law of non-contradiction
$
by double containment qed
"#;
    std::fs::write(d.path().join("all.stex"), src).unwrap();
    for engine in ["--pdflatex", "--xelatex"] {
        let out = cmx(d.path(), &["stex", "compile", "all.stex", "--fixed", engine]);
        if missing_host_lib(&out) {
            continue;
        }
        assert_eq!(
            out.status.code(),
            Some(0),
            "{engine}: {}\n{}",
            String::from_utf8_lossy(&out.stderr),
            std::fs::read_to_string(d.path().join("all.tex")).unwrap_or_default()
        );
        assert!(std::fs::read(d.path().join("all.pdf")).unwrap().starts_with(b"%PDF"));
    }
}

/// xelatex links against the system's fontconfig (present on desktop Linux and macOS, not on
/// minimal servers); cmx explains this. Skip that engine on such hosts (with a note).
fn missing_host_lib(out: &std::process::Output) -> bool {
    let err = String::from_utf8_lossy(&out.stderr);
    let missing = err.contains("needs the system library");
    if missing {
        eprintln!("note: skipping xelatex on this host: {}", err.lines().find(|l| l.contains("needs")).unwrap_or(""));
    }
    missing
}
