//! `cmx stex check` against real Lean 4 + Mathlib, over the corpus in crates/cmx-stex/tests/lean.
//!
//! Each `.stex` file starts with `// expect: ok` or `// expect: fail L1,L2,…` (the stex lines Lean
//! must reject). Needs Lean + Mathlib installed (`cmx stex check` prints the steps), so it's opt-in:
//!
//!     cargo test -p cmx --release --test stex_lean -- --ignored --nocapture
//!
//! Runs 4 files at a time (each Lean run loads Mathlib, ~10–25 s).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

#[derive(Debug, PartialEq)]
enum Expect {
    Ok,
    Fail(BTreeSet<u32>),
}

fn expectation(src: &str) -> Expect {
    let first = src.lines().next().unwrap_or_default();
    let spec = first.trim_start_matches("//").trim().strip_prefix("expect:").expect("first line: // expect: …").trim();
    match spec.strip_prefix("fail") {
        Some(lines) => Expect::Fail(lines.split(',').filter_map(|l| l.trim().parse().ok()).collect()),
        None => Expect::Ok,
    }
}

/// Run `cmx stex check` on one file; returns (verified?, failing stex lines, full output).
fn check(file: &Path, work: &Path) -> (bool, BTreeSet<u32>, String) {
    let name = file.file_name().unwrap().to_string_lossy().to_string();
    let dir = work.join(name.trim_end_matches(".stex"));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::copy(file, dir.join(&name)).unwrap();
    let out =
        Command::new(env!("CARGO_BIN_EXE_cmx")).args(["stex", "check", &name]).current_dir(&dir).output().unwrap();
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(!text.contains("are not installed"), "Lean/Mathlib missing:\n{text}");
    let prefix = format!("{name}:");
    let mut lines = BTreeSet::new();
    for l in text.lines() {
        let l = l.trim();
        if let Some(rest) = l.strip_prefix(&prefix)
            && rest.contains("could not verify")
            && let Some(n) = rest.split(':').next().and_then(|n| n.parse().ok())
        {
            lines.insert(n);
        }
    }
    (out.status.success() && text.contains("✓ Lean verified"), lines, text)
}

#[test]
#[ignore = "needs Lean 4 + Mathlib (cmx stex check prints install steps)"]
fn lean_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("../cmx-stex/tests/lean");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&corpus)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "stex"))
        .collect();
    files.sort();
    if let Ok(only) = std::env::var("STEX_LEAN_ONLY") {
        files.retain(|f| f.to_string_lossy().contains(&only));
    }
    let work = std::env::temp_dir().join(format!("cmx-stex-lean-{}", std::process::id()));
    let queue = Arc::new(Mutex::new(files.clone()));
    let results = Arc::new(Mutex::new(Vec::new()));
    let mut workers = Vec::new();
    for _ in 0..4 {
        let (queue, results, work) = (queue.clone(), results.clone(), work.clone());
        workers.push(std::thread::spawn(move || {
            loop {
                let Some(f) = queue.lock().unwrap().pop() else { break };
                let r = check(&f, &work);
                results.lock().unwrap().push((f, r));
            }
        }));
    }
    for w in workers {
        w.join().unwrap();
    }
    let mut results = std::mem::take(&mut *results.lock().unwrap());
    results.sort_by(|a, b| a.0.cmp(&b.0));
    let mut bad = Vec::new();
    for (f, (verified, lines, text)) in &results {
        let name = f.file_name().unwrap().to_string_lossy();
        let expect = expectation(&std::fs::read_to_string(f).unwrap());
        let got = if *verified && lines.is_empty() { Expect::Ok } else { Expect::Fail(lines.clone()) };
        let ok = got == expect;
        println!("{} {name}: expected {expect:?}, got {got:?}", if ok { "✓" } else { "✗" });
        if !ok {
            bad.push(format!("{name}: expected {expect:?}, got {got:?}\n{text}"));
        }
    }
    let _ = std::fs::remove_dir_all(&work);
    println!("{} / {} files as expected", results.len() - bad.len(), results.len());
    assert!(bad.is_empty(), "{} mismatches:\n\n{}", bad.len(), bad.join("\n\n"));
}
