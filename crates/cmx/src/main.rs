//! `cmx`: one CLI for the whole terminal workflow. See claude/CORE-DESIGN.md §4 for the command map.

mod stex;
mod tinytex;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

const VERSION: &str = env!("CARGO_PKG_VERSION");

const HELP: &str = "\
cmx — one CLI for everything

USAGE:
    cmx <command> [args]

EDITOR / IDE:
    edit [FILE...]           Open the editor (an untitled buffer when no file is given)
    workspace [DIR]          Open the IDE on DIR (default: .) with explorer + terminal

STEX (steno-TeX):
    stex new                 Untitled .stex buffer in the editor (name it when you save)
    stex new NAME            Create a blank NAME.stex
    stex check FILE.stex     Translate to FILE.lean (in the current directory) and check it
                             with Lean + Mathlib (optional install; cmx explains how)
    stex compile FILE.stex [--natural|--fixed|--varied|--random] [--seed N] [--open]
                           [--pdflatex|--xelatex|--lualatex]
                             Write FILE.tex next to FILE.stex and build FILE.pdf with the
                             bundled TinyTeX (CMX_TEX=system uses an installed TeX instead).
                             Variance (default --natural) controls how varied the prose is;
                             every level is deterministic. --open shows the PDF in your browser.

OTHER:
    help                     Show this help
    version                  Show the version

Inside the editor press F1 for every keyboard shortcut.
";

fn die(msg: &str) -> ExitCode {
    eprintln!("cmx: {msg}");
    ExitCode::from(2)
}

fn run_editor(opts: cmx_editor::Options) -> ExitCode {
    match cmx_editor::run(opts) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("cmx: {e}");
            ExitCode::FAILURE
        }
    }
}

fn cwd() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn cmd_edit(args: &[String]) -> ExitCode {
    if let Some(bad) = args.iter().find(|a| a.starts_with('-') && a.as_str() != "-") {
        return die(&format!("edit: unknown option {bad}"));
    }
    let root = cwd();
    let files = args.iter().map(|a| root.join(a)).collect();
    run_editor(cmx_editor::Options { files, root, workspace: false, stex: false })
}

fn cmd_workspace(args: &[String]) -> ExitCode {
    if args.len() > 1 {
        return die("workspace: expected at most one directory");
    }
    let dir = match args.first() {
        Some(d) => cwd().join(d),
        None => cwd(),
    };
    let root = match std::fs::canonicalize(&dir) {
        Ok(p) if p.is_dir() => p,
        Ok(p) => return die(&format!("workspace: {} is not a directory", p.display())),
        Err(e) => return die(&format!("workspace: {}: {e}", dir.display())),
    };
    run_editor(cmx_editor::Options { files: Vec::new(), root, workspace: true, stex: false })
}

fn stex_name(name: &str) -> PathBuf {
    let p = Path::new(name);
    if p.extension().is_some_and(|e| e == "stex") { p.to_path_buf() } else { PathBuf::from(format!("{name}.stex")) }
}

fn cmd_stex(args: &[String]) -> ExitCode {
    let Some(sub) = args.first() else { return die("stex: expected a subcommand (new, check, compile)") };
    let rest = &args[1..];
    match sub.as_str() {
        "new" => match rest {
            [] => run_editor(cmx_editor::Options { files: Vec::new(), root: cwd(), workspace: false, stex: true }),
            [name] => {
                let path = cwd().join(stex_name(name));
                match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
                    Ok(_) => {
                        println!("Created {}", path.display());
                        ExitCode::SUCCESS
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                        die(&format!("stex new: {} already exists", path.display()))
                    }
                    Err(e) => die(&format!("stex new: {}: {e}", path.display())),
                }
            }
            _ => die("stex new: expected at most one file name"),
        },
        "check" => {
            let [file] = rest else { return die("stex check: expected exactly one .stex file") };
            if !Path::new(file).exists() {
                return die(&format!("stex check: {file}: no such file"));
            }
            stex::check(file)
        }
        "compile" => {
            let mut file = None;
            let mut variance = None;
            let mut engine = None;
            let mut open = false;
            let mut seed = None;
            let mut it = rest.iter();
            while let Some(a) = it.next() {
                match a.as_str() {
                    "--fixed" | "--natural" | "--varied" | "--random" | "—varied" | "—fixed" | "—natural"
                    | "—random" => {
                        let v = cmx_stex::Variance::parse(a.trim_start_matches(['-', '—'])).expect("listed");
                        if variance.replace(v).is_some() {
                            return die("stex compile: only one variance flag is allowed");
                        }
                    }
                    "--pdflatex" | "--xelatex" | "--lualatex" => {
                        if engine.replace(&a[2..]).is_some() {
                            return die("stex compile: only one engine flag is allowed");
                        }
                    }
                    "--open" => open = true,
                    "--seed" => match it.next().and_then(|n| n.parse::<u64>().ok()) {
                        Some(n) => seed = Some(n),
                        None => return die("stex compile: --seed needs a number"),
                    },
                    f if f.starts_with('-') => return die(&format!("stex compile: unknown option {f}")),
                    f => {
                        if file.replace(f).is_some() {
                            return die("stex compile: expected exactly one .stex file");
                        }
                    }
                }
            }
            let Some(file) = file else { return die("stex compile: expected a .stex file") };
            if !Path::new(file).exists() {
                return die(&format!("stex compile: {file}: no such file"));
            }
            stex::compile(stex::CompileArgs {
                file,
                variance: variance.unwrap_or_default(),
                engine: engine.unwrap_or("pdflatex"),
                open,
                seed,
            })
        }
        other => die(&format!("stex: unknown subcommand {other}")),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = args.first() else {
        print!("{HELP}");
        return ExitCode::SUCCESS;
    };
    let rest = &args[1..];
    match cmd.as_str() {
        "edit" | "e" => cmd_edit(rest),
        "workspace" | "ws" => cmd_workspace(rest),
        "stex" => cmd_stex(rest),
        "help" | "-h" | "--help" => {
            print!("{HELP}");
            ExitCode::SUCCESS
        }
        "version" | "-V" | "--version" => {
            println!("cmx {VERSION}");
            ExitCode::SUCCESS
        }
        other => die(&format!("unknown command '{other}' (try `cmx help`)")),
    }
}
