# CORE-DESIGN: workspace, crates, dependency policy, CLI

> **TEMPORARY FILE.** Part of the `claude/` tracking system. Remove only when the user says so.
> Status: **active**. Applies to every feature area.

## 1. Repository layout

```
Cargo.toml              workspace (no [dependencies] from crates.io, ever)
crates/
  cmx/                  binary `cmx`: argument parsing + dispatch to feature crates
  cmx-sys/              hand-written libc FFI: termios, winsize, poll, pty, signals, fd io
  cmx-tui/              terminal UI toolkit: unicode width, styles, cell buffer, diff renderer,
                        input parser (keys/mouse/paste/kitty), clipboard (OSC 52 + system tools)
  cmx-vt/               VT100/xterm terminal emulator (used for the embedded shell & claude panes)
  cmx-editor/           the IDE (`cmx edit`, `cmx workspace`)
  cmx-stex/             stex compiler: lexer, math parser, segmenter, realizer, layout, Lean emitter
    data/               symbols.tsv (word→symbol), phrases.txt (phrase bank), theorems.tsv (aliases/Lean)
vendor/tinytex/         bundled TinyTeX-1 (+5 packages) for macOS, Linux x86_64, Linux arm64 (.tar.xz, ~163 MB
                        total; ORIGIN.md has sources, checksums, changes, licenses). Unpacked on first use.
tools/stex-corpus/      dev tool (not shipped): proof-phrase frequency analysis → phrases.txt weights, and
                        symbol-base.txt → symbols.tsv. frequency-table.tsv is the measured corpus table.
crates/cmx/tests/e2e.rs end-to-end tests: real binary in a pty, cmx-vt as the host terminal
crates/cmx/tests/stex_cli.rs  `cmx stex compile/check` end to end (temp dir, PATH without TeX/Lean)
rustfmt.toml            max_width = 120
claude/                 temporary tracking docs (this folder)
```

Dependency graph (acyclic, top depends on bottom):

```
cmx ──► cmx-editor ──► cmx-vt ──► cmx-tui
   │          ├──────► cmx-sys
   │          └──────► cmx-stex   (stex highlighting uses the compiler's lexer)
   └──► cmx-stex (no dependencies at all)
tools/stex-corpus (no dependencies; dev only)
```

## 2. Decisions

- **D-1 Layered crates.** Each layer is small, testable without a TTY, and reusable. stex reuses tui for coloured diagnostics. A future `cmx code` reuses nothing UI-related. Crates also compile in parallel.
- **D-2 No `libc` crate.** `cmx-sys/src/ffi.rs` declares the ~30 libc functions we use, with per-OS constants under `#[cfg(target_os)]`. Structs whose layout differs per OS (termios) are kept opaque (see RESEARCH R3). The other option, vendoring the `libc` crate (~100k lines), was rejected as needless bulk.
- **D-3 Release profile:** `lto = "fat"`, `codegen-units = 1`, `panic = "abort"`, `opt-level = 3`, `strip = true`. A panic hook restores the terminal before abort. The dev profile uses `opt-level = 1` so debug builds of the TUI stay responsive.
- **D-4 Edition 2024**, MSRV is whatever stable is current (1.98 at the time of writing). No nightly features.
- **D-5 Single-threaded event loops** built on `poll(2)` with a self-pipe for signals. No async runtime. Fewer moving parts and lowest latency.
- **D-6 Quality gates:** `cargo test --workspace` and `cargo clippy --workspace --all-targets` (0 warnings) before calling work done. Test-only dependencies are also path-only (the e2e test uses cmx-sys/cmx-vt as dev-dependencies).

## 3. Dependency policy (user requirement: "no external dependencies")

1. Prefer writing it ourselves when it's < ~2k lines (base64, fuzzy match, unicode width tables, VT parser…).
2. If third-party code or data is needed, copy it into `vendor/<name>/` with its LICENSE, record the origin URL and commit hash in `vendor/<name>/ORIGIN`, and reference it as a path dependency. Never use a registry dependency.
3. Calling **system tools** at runtime (pbcopy, xclip, lean, pdflatex, claude) is allowed. They are optional integrations, not build dependencies. Each must degrade gracefully when the tool is missing.
   - **OS libraries** are allowed the same way: libc everywhere; on macOS the CoreGraphics framework for the modifier-state query (EDITOR D-27); on Linux libX11, loaded at runtime with `dlopen`, so it is never a build or link requirement and is skipped when missing.
4. **Exception (user, session 2):** Lean + Mathlib for `cmx stex check` is a user-installed, optional dependency. When it's missing, cmx prints install instructions. Nothing from Mathlib is vendored except our own alias and statement table.
5. **Bundled TinyTeX** (user decision, session 3): `cmx stex compile` must not need MacTeX (5 GB). TinyTeX archives are vendored in `vendor/tinytex/` per policy item 2. `CMX_TEX=system` (or a bin dir) opts back into an installed TeX.

## 4. CLI surface (current + planned)

| Command | Status |
|---|---|
| `cmx edit [FILE…]` | ✅ editor (untitled buffer when no file) |
| `cmx workspace [DIR]` | ✅ IDE: explorer + terminal open, "Open a file to begin" |
| `cmx stex new [NAME]` | ✅ untitled `.stex` in editor / creates blank `NAME.stex` |
| `cmx stex check FILE` | ✅ writes FILE.lean in cwd; runs `lake env lean` in the Mathlib project (`$CMX_MATHLIB` or `~/.cmx/stexcheck`) |
| `cmx stex compile FILE [--natural\|--fixed\|--varied\|--random] [--seed N] [--open] [--pdflatex\|--xelatex\|--lualatex]` | ✅ |
| `cmx link/logout/draft/send` | 💭 MAIL-DESIGN |
| `cmx new repo\|homework\|resume`, `cmx resume` | 💭 SCAFFOLD-DESIGN |
| `cmx code` | 💭 CODE-DESIGN |
| `cmx help`, `cmx --version` | ✅ |

Argument parsing is hand-rolled (`crates/cmx/src/main.rs`). Each subcommand validates its own flags and prints usage on error (exit 2).
