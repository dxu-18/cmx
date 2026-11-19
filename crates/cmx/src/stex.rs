//! `cmx stex compile` / `cmx stex check` (STEX-DESIGN §1).

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

use cmx_stex::{Options, Variance};

fn fail(msg: &str) -> ExitCode {
    eprintln!("cmx: {msg}");
    ExitCode::FAILURE
}

fn read_source(file: &str) -> Result<(PathBuf, String), ExitCode> {
    let path = PathBuf::from(file);
    match std::fs::read_to_string(&path) {
        Ok(s) => Ok((path, s)),
        Err(e) => Err(fail(&format!("{file}: {e}"))),
    }
}

/// `dir/name.stex` → (`dir`, `name`).
fn stem(path: &Path) -> (PathBuf, String) {
    let dir = path
        .parent()
        .map(Path::to_path_buf)
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| PathBuf::from("."));
    let name = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "out".into());
    (dir, name)
}

/// A program on PATH, or in one of `extra` directories.
fn find_tool(name: &str, extra: &[&str]) -> Option<PathBuf> {
    if let Some(paths) = std::env::var_os("PATH") {
        for d in std::env::split_paths(&paths) {
            let p = d.join(name);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    let home = std::env::var("HOME").unwrap_or_default();
    extra.iter().map(|d| PathBuf::from(d.replace('~', &home)).join(name)).find(|p| p.is_file())
}

pub struct CompileArgs<'a> {
    pub file: &'a str,
    pub variance: Variance,
    pub engine: &'a str,
    pub open: bool,
    pub seed: Option<u64>,
}

pub fn compile(a: CompileArgs) -> ExitCode {
    let (path, src) = match read_source(a.file) {
        Ok(x) => x,
        Err(code) => return code,
    };
    let (dir, name) = stem(&path);
    let display_name = path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let out = match cmx_stex::compile(&src, &display_name, &Options { variance: a.variance, seed: a.seed }) {
        Ok(o) => o,
        Err(d) => {
            eprintln!("{}:{}: error: {}", a.file, d.line, d.msg);
            return ExitCode::FAILURE;
        }
    };
    let tex_path = dir.join(format!("{name}.tex"));
    if let Err(e) = std::fs::write(&tex_path, &out.tex) {
        return fail(&format!("writing {}: {e}", tex_path.display()));
    }
    println!("Wrote {}", tex_path.display());
    if !out.theorems.is_empty() {
        println!("Theorems cited: {}", out.theorems.join(", "));
    }
    let engine = match resolve_engine(a.engine) {
        Ok(e) => e,
        Err(msg) => {
            eprintln!("cmx: {msg}, so no PDF was made ({} is ready).", tex_path.display());
            return ExitCode::from(4);
        }
    };
    let tex_file = format!("{name}.tex");
    let res = Command::new(&engine)
        .args(["-interaction=nonstopmode", "-halt-on-error", "-file-line-error", &tex_file])
        .current_dir(&dir)
        // a system TeX's settings must not leak into the bundled tree
        .env_remove("TEXMFCNF")
        .env_remove("TEXMFDIST")
        .stdin(Stdio::null())
        .output();
    let res = match res {
        Ok(r) => r,
        Err(e) => return fail(&format!("running {}: {e}", engine.display())),
    };
    let pdf = dir.join(format!("{name}.pdf"));
    if !res.status.success() || !pdf.is_file() {
        let log = String::from_utf8_lossy(&res.stdout);
        let err = String::from_utf8_lossy(&res.stderr);
        eprintln!("cmx: {} failed on {}:", a.engine, tex_path.display());
        let tex_errors: Vec<&str> = log.lines().filter(|l| l.starts_with('!') || l.contains(".tex:")).take(8).collect();
        for l in &tex_errors {
            eprintln!("  {l}");
        }
        let log_file = dir.join(format!("{name}.log"));
        if tex_errors.is_empty() || !log_file.is_file() {
            // the engine did not get as far as TeX (a missing system library, a broken install)
            for l in err.lines().chain(log.lines()).filter(|l| !l.trim().is_empty()).take(6) {
                eprintln!("  {l}");
            }
        }
        if err.contains("error while loading shared libraries") {
            let lib = err.split("libraries: ").nth(1).and_then(|r| r.split(':').next()).unwrap_or("a library");
            eprintln!(
                "  {} needs the system library {lib}: install it (Debian/Ubuntu: sudo apt install libfontconfig1, \
                 Fedora: sudo dnf install fontconfig), or use pdflatex (the default)",
                a.engine
            );
        }
        if log_file.is_file() {
            eprintln!("  (full log: {})", log_file.display());
        }
        return ExitCode::FAILURE;
    }
    for ext in ["aux", "log", "out"] {
        let _ = std::fs::remove_file(dir.join(format!("{name}.{ext}")));
    }
    println!("Wrote {}", pdf.display());
    if a.open {
        open_in_browser(&pdf);
    }
    ExitCode::SUCCESS
}

/// Engine binary: `$CMX_TEX` (`system` = PATH, or a bin directory), else the bundled TinyTeX
/// (vendor/tinytex), else a system TeX on PATH.
fn resolve_engine(engine: &str) -> Result<PathBuf, String> {
    let system = || {
        find_tool(engine, &["/Library/TeX/texbin", "/usr/texbin"])
            .ok_or_else(|| format!("{engine} not found (no bundled TinyTeX for this platform and none on PATH)"))
    };
    match std::env::var("CMX_TEX").as_deref() {
        Ok("system") => return system(),
        Ok(dir) if !dir.is_empty() => {
            let p = PathBuf::from(dir).join(engine);
            return if p.is_file() { Ok(p) } else { Err(format!("{} not found", p.display())) };
        }
        _ => {}
    }
    match crate::tinytex::engine(engine) {
        Ok(Some(p)) if p.is_file() => Ok(p),
        Ok(Some(p)) => Err(format!("the bundled TinyTeX has no {} ({})", engine, p.display())),
        Ok(None) => system(),
        Err(e) => {
            eprintln!("cmx: could not unpack the bundled TinyTeX: {e}");
            system()
        }
    }
}

/// Open a file in the default *web browser* (spec: "--open prompts the default browser").
fn open_in_browser(file: &Path) {
    let abs = std::fs::canonicalize(file).unwrap_or_else(|_| file.to_path_buf());
    let url = format!("file://{}", abs.display());
    #[cfg(target_os = "macos")]
    {
        let id = default_browser_macos().unwrap_or_else(|| "com.apple.Safari".into());
        let ok = Command::new("open").args(["-b", &id]).arg(&abs).status().is_ok_and(|s| s.success());
        if !ok {
            let _ = Command::new("open").arg(&abs).status();
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        if let Ok(b) = std::env::var("BROWSER")
            && !b.is_empty()
            && Command::new(&b).arg(&url).spawn().is_ok()
        {
            return;
        }
        let desktop = Command::new("xdg-settings")
            .args(["get", "default-web-browser"])
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .filter(|s| !s.is_empty());
        if let Some(d) = desktop
            && Command::new("gtk-launch").arg(&d).arg(&url).status().is_ok_and(|s| s.success())
        {
            return;
        }
        let _ = Command::new("xdg-open").arg(&url).status();
    }
    let _ = url;
}

/// Bundle id of the default https handler, from the LaunchServices preferences.
#[cfg(target_os = "macos")]
fn default_browser_macos() -> Option<String> {
    let home = std::env::var("HOME").ok()?;
    let plist = format!("{home}/Library/Preferences/com.apple.LaunchServices/com.apple.launchservices.secure.plist");
    let out = Command::new("plutil").args(["-extract", "LSHandlers", "json", "-o", "-", &plist]).output().ok()?;
    let json = String::from_utf8_lossy(&out.stdout);
    for obj in json.split('}') {
        if obj.contains("\"LSHandlerURLScheme\":\"https\"") {
            let key = "\"LSHandlerRoleAll\":\"";
            let start = obj.find(key)? + key.len();
            let end = obj[start..].find('"')? + start;
            return Some(obj[start..end].to_string());
        }
    }
    None
}

// ------------------------------------------------------------------------------------------ check

fn mathlib_project() -> Option<PathBuf> {
    let candidates: Vec<PathBuf> = match std::env::var("CMX_MATHLIB") {
        Ok(p) if !p.is_empty() => vec![PathBuf::from(p)],
        _ => {
            let home = PathBuf::from(std::env::var("HOME").unwrap_or_default());
            vec![home.join(".cmx/stexcheck"), home.join(".cmx/mathlib")]
        }
    };
    candidates.into_iter().find(|p| {
        (p.join("lakefile.lean").is_file() || p.join("lakefile.toml").is_file())
            && p.join(".lake/packages/mathlib").is_dir()
    })
}

const INSTALL_HINT: &str = "\
Lean 4 + Mathlib are not installed. They are optional: only `cmx stex check` needs them.
To install them once (about 5 GB with Mathlib's prebuilt cache):
  1. curl https://elan.lean-lang.org/elan-init.sh -sSf | sh       # elan, the Lean toolchain manager
  2. mkdir -p ~/.cmx && cd ~/.cmx && lake new stexcheck math       # a Lake project that depends on Mathlib
  3. cd ~/.cmx/stexcheck && lake exe cache get                     # download prebuilt Mathlib
Then run `cmx stex check` again. To use an existing Mathlib project instead, set CMX_MATHLIB=/path/to/it.";

pub fn check(file: &str) -> ExitCode {
    let (path, src) = match read_source(file) {
        Ok(x) => x,
        Err(code) => return code,
    };
    let (_, name) = stem(&path);
    let display_name = path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let out = match cmx_stex::to_lean(&src, &display_name) {
        Ok(o) => o,
        Err(d) => {
            eprintln!("{file}:{}: error: {}", d.line, d.msg);
            return ExitCode::FAILURE;
        }
    };
    // spec: the .lean file goes in the current directory
    let lean_path = std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")).join(format!("{name}.lean"));
    if let Err(e) = std::fs::write(&lean_path, &out.text) {
        return fail(&format!("writing {}: {e}", lean_path.display()));
    }
    println!("Wrote {} ({} proof obligations)", lean_path.display(), out.obligations);
    for (line, what) in &out.skipped {
        println!("  {file}:{line}: not translatable to Lean ({what}); skipped");
    }
    let lake = find_tool("lake", &["~/.elan/bin"]);
    let project = mathlib_project();
    let (Some(lake), Some(project)) = (lake, project) else {
        eprintln!("{INSTALL_HINT}");
        return ExitCode::from(4);
    };
    println!("Checking with Lean (Mathlib project: {}) ...", project.display());
    let res =
        Command::new(&lake).args(["env", "lean"]).arg(&lean_path).current_dir(&project).stdin(Stdio::null()).output();
    let res = match res {
        Ok(r) => r,
        Err(e) => return fail(&format!("running lake: {e}")),
    };
    let text = format!("{}{}", String::from_utf8_lossy(&res.stdout), String::from_utf8_lossy(&res.stderr));
    let lean_name = lean_path.to_string_lossy().to_string();
    // "<path>:LINE:COL: error: message", then continuation lines (the goal state, after ⊢)
    let mut found: Vec<(u32, String, String)> = Vec::new();
    let mut cur: Option<(u32, String, String)> = None;
    for l in text.lines() {
        let head = l.strip_prefix(&lean_name).or_else(|| l.split_once(".lean:").map(|(_, r)| r));
        if let Some(rest) = head {
            found.extend(cur.take());
            let rest = rest.trim_start_matches(':');
            let mut parts = rest.splitn(3, ':');
            let (Some(ln), Some(_col), Some(msg)) = (parts.next(), parts.next(), parts.next()) else { continue };
            let Ok(ln) = ln.trim().parse::<u32>() else { continue };
            let msg = msg.trim();
            if msg.starts_with("error") {
                cur = Some((ln, msg.trim_start_matches("error:").trim().to_string(), String::new()));
            }
        } else if let Some((_, _, goal)) = &mut cur
            && (!goal.is_empty() || l.trim_start().starts_with('⊢'))
        {
            goal.push(' ');
            goal.push_str(l.trim());
        }
    }
    found.extend(cur.take());
    let mut errors = 0;
    let mut seen = std::collections::BTreeSet::new();
    for (ln, msg, goal) in found {
        let sl = cmx_stex::lean::stex_line(&out.line_map, ln).unwrap_or(0);
        if !seen.insert(sl) {
            continue; // one report per source line
        }
        errors += 1;
        if sl == 0 {
            // not from a proof step: the generated preamble itself (a Lean/Mathlib version mismatch?)
            println!("  {}:{ln}: error in cmx's generated Lean preamble: {msg}", lean_path.display());
            continue;
        }
        println!("  {file}:{sl}: Lean could not verify this step: {msg}");
        let goal = goal.split_whitespace().collect::<Vec<_>>().join(" ");
        if !goal.is_empty() {
            let short: String = goal.chars().take(200).collect();
            let more = if goal.chars().count() > 200 { " …" } else { "" };
            println!("      goal: {short}{more}");
        }
    }
    if errors == 0 && res.status.success() {
        println!("✓ Lean verified all {} obligations.", out.obligations);
        ExitCode::SUCCESS
    } else {
        if errors == 0 {
            eprintln!("{text}");
        }
        println!("✗ {errors} step(s) not verified (see {}).", lean_path.display());
        ExitCode::FAILURE
    }
}
