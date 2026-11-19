# CONTEXT — cmx session entry point

> **TEMPORARY FILE.** Part of the `claude/` tracking system. Remove only when the user says so.
> Claude may edit it but must never delete it.

## 0. Resume protocol (read this first, every session)

1. Read this file top to bottom (it is kept short on purpose).
2. Open the `*-DESIGN.md` for whatever area §4 says is active. Its *Checklist* is the source of truth for what is done.
3. Skim the last entry of `LOGS.md` for loose ends.
4. Check `RESEARCH.md` before re-researching anything (facts are tagged `[verified]` or `[unverified]`).
5. **At the end of a session:** update §3 and §4 here, tick checklists in the design doc, and append one entry to `LOGS.md`.

| File | Role | How it's written |
|------|------|-------------|
| `CONTEXT.md` | Current state + next steps. Small, always current. | Overwrite sections in place |
| `LOGS.md` | Chronological session journal | Append only, newest at bottom |
| `RESEARCH.md` | External facts: protocols, APIs, tool versions, sources | Add or correct by topic |
| `CORE-DESIGN.md` | Workspace layout, crates, dependency policy, CLI | Living spec |
| `EDITOR-DESIGN.md` | `cmx edit` / `cmx workspace` IDE | Living spec + checklist |
| `STEX-DESIGN.md` | `.stex` language and the `cmx stex` commands | Living spec + checklist |
| `MAIL-DESIGN.md` | `cmx link/draft/send` (email/DM) | Brainstorm, not fleshed out |
| `SCAFFOLD-DESIGN.md` | `cmx new repo/homework/resume`, `cmx resume` | Brainstorm |
| `CODE-DESIGN.md` | `cmx code` (NL → multi-language code) | Brainstorm |
| `TEMPLATES-DESIGN.md` | LLM prompt-template bank | Brainstorm |
| `WEBSITE-DESIGN.md` | Personal GitHub website tooling | Brainstorm |

The original requirements live in `/prompt.txt` (the user's spec). When the spec and a design doc disagree, the spec wins unless the design doc records a decision that the user approved.

## 1. Project in one paragraph

`cmx` is one Rust CLI that bundles the user's whole terminal workflow. It targets macOS and Linux, aims for ultra-low latency, and has **zero external crates**: everything is std, our own FFI to libc, or code vendored into the repo. Planned feature areas: (1) a terminal IDE (`cmx edit`, `cmx workspace`); (2) `.stex`, a deterministic near-natural-language-to-LaTeX language, with Lean checking (`cmx stex new/check/compile`); (3) email/DM drafting and sending; (4) repo, homework and resume scaffolding; (5) `cmx code`; (6) an LLM template bank; (7) personal-website tooling.

## 2. Hard constraints (from the user)

- Pure Rust. **No external dependencies.** Anything we need is written by us or vendored as files in the repo. Policy: `CORE-DESIGN.md §3`.
- macOS **and** Linux (x86_64 and aarch64).
- The stex compiler is **deterministic**: no trained models or LLMs.
- The `claude/` docs are temporary but must **not** be deleted by Claude.
- Session priority: **editor/IDE first**, then stex.

## 3. Current state (update every session)

_Last updated: 2026-11-14 (session 5, part 5)_

- Crates: `cmx`, `cmx-sys`, `cmx-tui`, `cmx-vt`, `cmx-editor` (now also uses `cmx-stex` for highlighting), `cmx-stex`, plus the dev tool `tools/stex-corpus` (not shipped). Zero external crates. `vendor/tinytex/` holds the bundled TeX: **plain files in the repo, no Git LFS** (user, session 5).
- Tests: about 160 passing on **macOS and Linux arm64**:
  - 24 editor pty e2e (+1 ignored screenshot test) and 6 stex CLI tests (real PDFs);
  - 59 stex unit/golden and 3 corpus regression tests over **195 proofs** (snapshots + lints + all modes + Lean translation);
  - 39 editor unit tests, and the rest unit.
  - Opt-in: the Lean corpus (52/52) and the Shift-probe test.
  - 0 clippy warnings.
- **stex (session 5):**
  - 177-proof corpus audit (STEX §12): about 100 failures traced to 31 root causes and fixed (§12.1).
  - Highlighting now comes from the compiler's own lexer (§13).
  - The user's `prime.stex` renders correctly.
  - Parts 5–6 (STEX §14): automatic case numbering and real label detection; blank lines stay inside a case; existential agreement.
    - The `exhaustive` family and `case on X`. Case-analysis prose comes **only from the user's words**: no derived reasons (user decision, part 6).
    - The user's `odd8.stex` renders correctly.
- **Editor (session 5):**
  - Ctrl+Shift is detected on Terminal.app by reading Shift from the OS (D-27, `⌨ legacy+⇧`).
  - A ↔/↕ resize marker under the mouse works in any terminal (D-28).
  - Ctrl+arrows go to line/document ends, and Ctrl+Backspace/Delete delete to the line start/end (D-29).
  - Live stex keyword colors (D-26).
- Not yet verified:
  - Shift detection in real Terminal.app: the probe runs (no keys → false), but a held Shift has only been tested on Linux X11 (Xvfb).
  - Whether Terminal.app reports mouse motion (`?1003`) for hover; dragging shows the marker regardless.
  - linux-x86_64 TinyTeX at runtime.

## 4. Next steps (ordered)

1. User checks in Terminal.app:
   - Terminal has Input Monitoring (verified). cmx now reads Ctrl+Shift chords through an event tap (D-30): the status bar shows `⌨ legacy+⇧`, Ctrl+Shift+Z redoes, and Ctrl+Shift+L/R/Y toggle the panels. Verified with posted key events; the user confirms with the real keyboard.
   - Without the permission, Ctrl+L/R/Y open the panels as before.
   - Hovering a border shows ↔.
   - Ctrl+arrows work. They need Mission Control's ⌃-arrow shortcuts turned off (RESEARCH R11).
2. User writes real stex and reports misrenders. Every new failure becomes a corpus file plus a snapshot (`STEX_BLESS=1 cargo test -p cmx-stex --test corpus`). The §12.1 open items are next.
3. **Now (user, 2026-11-14):** the automatic email/Instagram/Discord system.
   - The research (≥30 min) and the design proposal are done: MAIL-DESIGN.md (+ RESEARCH R12).
   - **Waiting on the user's answers to Q1–Q7** (MAIL-DESIGN §11), then build in the phases of §10. Phase 1 (app-password SMTP, MIME, outbox, time parser) doesn't depend on the answers and can start first if the user agrees.
4. The other later feature areas (SCAFFOLD / CODE / TEMPLATES / WEBSITE).

## 5. Open questions for the user

- MAIL Q1–Q7 (MAIL-DESIGN §11): which accounts; a Microsoft app registration for Outlook; Discord hand-off vs bot; Instagram hand-off; what "phone number" means; the `cmx outbox` command; holding late sends.
- (Answered in session 5: the terminal is Terminal.app, with resize and Ctrl+Shift support wanted in all terminals; the TinyTeX archives stay in the repo as normal files.)

## 6. Build / test cheatsheet

```sh
cargo build --release                       # → target/release/cmx
cargo test --workspace                      # unit + e2e (e2e spawns the binary in a pty)
cargo test -p cmx --release --test e2e ide_screenshot -- --ignored --nocapture
                                            # text "screenshot" of the full IDE + latency numbers
cargo clippy --workspace --all-targets      # keep at 0 warnings
cargo fmt --all                             # rustfmt.toml: max_width 120
./target/release/cmx workspace              # try it
CMX_CLAUDE=/path/to/claude cmx edit         # override which binary the Claude pane runs
CMX_TRUECOLOR=1|0                           # force truecolor on/off
cmx stex compile f.stex [--fixed|--natural|--varied|--random] [--seed N] [--open] [--xelatex|--lualatex]
cmx stex check f.stex                       # f.lean in cwd; CMX_MATHLIB=/path/to/lake/project
cargo test -p cmx --release --test stex_lean -- --ignored --nocapture
                                            # Lean corpus (needs ~/.cmx/stexcheck; STEX_LEAN_ONLY=substr filters)
STEX_BLESS=1 cargo test -p cmx-stex --test corpus   # re-snapshot the 195-proof corpus after an intended change
CMX_MODPROBE=0                              # turn off the OS Shift probe (D-27)
limactl shell cmx-linux -- bash -lc 'cd /Users/dx2008/dev/projects/cmx && CARGO_TARGET_DIR=$HOME/cmx-target cargo test --workspace'
                                            # Linux run (our Lima VM; never touch the user's VM `cma`)
CMX_TEX=system|/path/to/bin                 # use an installed TeX instead of the bundled TinyTeX
cargo run --release -p stex-corpus -- G=DIR... --symbols crates/cmx-stex/data/symbols.tsv --out t.tsv
                                            # re-measure the corpus (see RESEARCH R6)
cargo run --release -p stex-corpus -- weights --report tools/stex-corpus/frequency-table.tsv \
    --map tools/stex-corpus/phrase-map.txt > crates/cmx-stex/data/phrases.txt     # regenerate the phrase bank
cargo run --release -p stex-corpus -- symbols --base tools/stex-corpus/symbol-base.txt \
    > crates/cmx-stex/data/symbols.tsv                                              # regenerate the symbol library
```
