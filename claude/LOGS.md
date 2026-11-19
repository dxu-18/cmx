# LOGS: session journal

> **TEMPORARY FILE.** Part of the `claude/` tracking system. Remove only when the user says so.
> Append-only, newest at the bottom. One entry per session (add sub-entries for big milestones).
> Entry format: `## YYYY-MM-DD: session N: <headline>`, then **Did / Decisions / Problems / Next**.

---

## 2026-11-13: session 1: tracking system + editor/IDE

**Did**
- Read `prompt.txt` (full spec). Created the `claude/` tracking system: CONTEXT, LOGS, RESEARCH, and design docs for every feature area. The later areas are marked as brainstorms.
- Added a root `CLAUDE.md` that points future sessions at `claude/CONTEXT.md`.
- Created the Cargo workspace and implemented the editor/IDE (details in `EDITOR-DESIGN.md`).

**Decisions**
- Zero crates. libc is reached through hand-written `extern "C"` declarations in `cmx-sys`, with per-OS constants, and termios is treated as an opaque buffer together with `cfmakeraw`. Rationale: CORE-DESIGN D-2.
- The workspace is split into crates by layer: sys → tui → vt → editor → cmx. Rationale: CORE-DESIGN D-1.
- Keyboard: query the kitty keyboard protocol and fall back to xterm modifyOtherKeys, then to legacy bytes. With legacy bytes, Ctrl+letter can stand in for Ctrl+Shift+letter. Rationale: EDITOR-DESIGN D-3.

**Problems**
- `lean`, `lake` and `pdflatex` are not installed on the dev machine. This doesn't block the editor, but stex check/compile can't be tested end-to-end here.

**Next**: see CONTEXT §4.

### 2026-11-13: session 1, milestone: editor/IDE done (macOS)

**Did**
- Built 5 crates (~14k lines, zero external crates):
  - `cmx-sys`: FFI, raw mode, pty, signals.
  - `cmx-tui`: unicode width, diff renderer, input parser with kitty/MOK/SGR mouse/paste.
  - `cmx-vt`: xterm emulator.
  - `cmx-editor`: text, undo, highlight, view ops, vim, tree, panes, layout, palette, prompts, app loop.
  - `cmx`: CLI, including working `stex new`.
- Tests: 53 unit + 16 e2e (real binary in a pty, emulated with our own cmx-vt) + an ignored screenshot/latency test. 0 clippy warnings. Release binary 0.8 MB.
- Measured ~1.3 ms keypress→screen median and ~60 ms to first frame.

**Decisions** (details in EDITOR-DESIGN D-8, D-12…D-15)
- `$CMX_CLAUDE` override. Dead panes hand keys to the keymap. Ctrl+Shift+Q quits from inside panes.
- No scroll margin. The search-highlight flag is separate from the query. Pane sync-output hold is capped at 100 ms.
- `:wq` on an untitled buffer: prompt for a name, then quit.

**Problems**
- `cargo clippy --fix` produced unformatted let-chains, so I added `rustfmt.toml` and ran `cargo fmt --all`.
- The e2e "claude missing" test first found a real `claude` outside PATH, which led to the `$CMX_CLAUDE` override. (Bonus: it proved Claude Code renders in cmx-vt.)
- Linux is unverified. Only the aarch64-apple-darwin target is installed, and adding one needs a download (asked in CONTEXT Q).

**Next**: CONTEXT §4.

## 2026-11-13: session 2: user answers, Cmd keys on macOS, stex implemented

**Did**
- Recorded the user's answers to Q-E1…E3 and Q-S1…S7 (CONTEXT §5, EDITOR D-16…D-20, STEX §8).
- Keyboard:
  - Modifier styles: Cmd primary on macOS; Ctrl does readline in the editor and goes to the shell in panes.
  - Help, palette and status labels follow the style.
  - macOS key bridge in `cmx-sys/src/keybridge.rs`: CGEventTap plus an AX focus test, frameworks loaded with dlopen.
  - `cmx setup keys`, using a mini XML plist reader/writer in `crates/cmx/src/plist.rs`.
  - `cmx keys` tester. VS Code hint.
- stex: new crate `cmx-stex`:
  - lexer, lexicon (`data/symbols.tsv`), math parser, segmenter, construct realizer
  - phrase bank (`data/phrases.txt`) with seeded variance
  - layout pass + package detection
  - theorem library (`data/theorems.tsv`)
  - Lean emitter
- CLI `stex compile` (engine, `--open` in the default browser, `--seed`) and `stex check` (Mathlib hint). Editor palette commands.
- Tests: +1 editor e2e (Cmd style), +4 stex CLI e2e, +31 stex golden/unit, +plist/keymap/bridge unit tests. 111 total, clippy clean.
- Perf: 6000-line .stex compiles in ~50 ms; Lean emission for 2500 obligations takes ~130 ms. That was ~340 ms before a fix for quadratic line counting.

**Decisions**
- Terminal.app's profile key map has no Command modifier (RESEARCH R2.2), so Cmd needs an event tap with Accessibility permission. The tap swallows only cmx's chords, and only while cmx's tab is focused (AX focused-element compare). Ctrl+Shift distinction comes from a generated Terminal profile, with no permission needed.
- Standalone `$…$` is emitted as `\[…\]` (LaTeX's spelling of `$$…$$`) or `align*` + `\tag*` for tagged or long chains. Inline `$…$` stays inline. `$$` is a lex error.
- Fixed mode keeps the user's own connective ("thus", "since", "if…then"). The other modes draw from the bank.
- `F.tex`/`F.pdf` are written next to the source and `F.lean` in the cwd (spec). Exit code 4 means "tool missing, file still written".

**Problems**
- pdflatex and Lean are not installed, so the generated output has never been run through them. The Mathlib names are from memory.
- The key bridge can't be exercised without a human at a GUI. Only symbol resolution, translation and encoding are unit-tested.

**Next**: CONTEXT §4.

## 2026-11-13: session 3: Ctrl-only keys, bundled TinyTeX, corpus-based phrase bank, Lean cases

**Did**
- Keyboard (user: "Cmd doesn't work… use Ctrl"): removed the key bridge (`cmx-sys/src/keybridge.rs`), `cmx setup keys` (+ plist helper) and `cmx keys`. Ctrl is the command modifier everywhere; Cmd chords are ignored. New e2e test: Ctrl+Z vs Ctrl+Shift+Z on a kitty terminal, Cmd+S not saving. VS Code hint now mentions the kitty-protocol setting.
- TinyTeX: downloaded TinyTeX-1 v2026.11 for darwin / linux-x86_64 / linux-arm64, added needspace, jknapltx (mathrsfs), rsfs, enumitem, pgf (+fp) with tlmgr on macOS. Copied the additions into the Linux trees (regenerated ls-R, updmap.cfg, font maps, tlpdb records) and repacked into `vendor/tinytex/` (163 MB, ORIGIN.md). `crates/cmx/src/tinytex.rs` unpacks on first use (atomic rename). `CMX_TEX` overrides. The preamble now loads T1 + lmodern.
- Corpus study: 19,265 proofs / 167k sentences / 2.46M words (6 open textbooks, 183 arXiv papers, the Stacks Project). Results in RESEARCH R6. The new `tools/stex-corpus` (Rust, no deps) measures phrases and generates `phrases.txt` (269 templates / 51 roles) from `phrase-map.txt` and `symbols.tsv` (2005 entries) from `symbol-base.txt`.
- stex realizer:
  - labels start paragraphs; `latex{\begin…}` environments become displays
  - embedded quantifiers use `.inner` roles
  - new `choose/pick/take/select` statements (witnesses)
  - "…, so" + `$…$` flows into the display
  - `qed` ends case lists
  - per-sentence seeding for fixed/natural, entropy seeds for varied/random
- Lean: `rcases` case analysis, `obtain` for witnesses and derived ∃, `refine ⟨x, ?_⟩` for ∃ goals, parenthesized implication chains, and `stex_auto` now also splits conjunctions and tries `subst_vars`.
- Math parser: new kinds (add, div, setop, funcp, accent; any `big` operator), Unicode symbols lexed as their own tokens, phrase merging indexed by first word.
- Verified with real TeX for the first time: a document with every construct compiles with no warnings in pdflatex, xelatex and lualatex. The user's test.stex renders correctly.

**Decisions**
- TinyTeX archives live in the repo (user: "keep a copy of TinyTeX as part of the library"); unpacked trees are git-ignored. TinyTeX-1 + 6 packages, not the 210 MB full TinyTeX.
- Corpus weights are averaged per source group; the `--fixed` template is kept at ≥ 50% of its role's top weight (user: simpler wordings stay high).
- English words that double as operator names (`rank`, `degree`, `trace`) need parentheses to become math.

**Problems**
- The first TinyTeX compile failed: TS1 glyph, no bitmap font sources. Fixed with T1 + lmodern.
- rustfmt reflows long `matches!` lists, so text-based patches must target the formatted code.
- The Linux TinyTeX binaries can't be run here. Their trees were checked at the file level only.

**Next**: CONTEXT §4.

### 2026-11-13: session 3, fix: setequality.stex failed in pdflatex

- **Bugs:**
  1. `A\B` (no spaces) was emitted as the raw command `\B`, so pdflatex reported "Undefined control sequence".
  2. "if and only if" inside `$…$` was swallowed as `\text{…}` by implicit multiplication, which misaligned the align steps and their `by` tags.
  3. "then" before a display was dropped.
  4. "by double containment qed" rendered literally.
- **Fixes:**
  - `\` in operator position is set difference unless it starts a typed operator command (`\cup`, `\leq`, `\cdot`, `\pm`). It counts as implicit multiplication only before operand commands (`2\pi`, `\mathbb`).
  - "if and only if" and "only if" are keywords.
  - Tag text keeps source gluing ("non-contradiction").
  - A lone connective before a display is kept.
  - `by T qed` / `since Q qed` closes the proof with the reason.
- **New tests:** `regression_setequality` (golden) and `every_construct_compiles_with_tinytex` (pdflatex + xelatex on a document using every construct). 115 tests pass.
- Follow-up (user): `\` always means set difference; typed LaTeX commands (`\cup`, `\leq`, `2\pi`) are no longer recognized, so use `latex{…}` (STEX §8).

### 2026-11-13: session 4: Lean verification, stex proof constructs, editor features, Linux

User asked for items 1–4 of the plan: (1) install Lean and test extensively, (2) close the stex gaps (casing, n-th, proof by contradiction, induction, counting procedures), (3) the remaining editor features (soft wrap, regex search, multiple cursors, noticing on-disk changes incl. the explorer, a resize pointer on splitters), (4) Linux testing. Item 5 (mail/scaffold/code/templates/website) is saved for later.

**Done**
- **Lean (STEX §11, RESEARCH R4/R7).**
  - Installed Lean v4.36.0-rc2 + Mathlib in `~/.cmx/stexcheck`.
  - Every `theorems.tsv` name checks; AM–GM and Cauchy–Schwarz were wrong, now fixed.
  - Built a 52-file corpus (44 correct proofs, 8 wrong ones with expected failing lines) plus the opt-in test `crates/cmx/tests/stex_lean.rs`. The first run matched 24/42; it now matches 52/52.
  - Fixes on the way:
    - Recursive `stex_auto` with a `fail` message, and `check` prints the goal.
    - Unhygienic `stex_close` so it can use `ih`, plus a `stex_strong n` helper.
    - Sum/product step lemmas.
    - Closers for ModEq, abs, mul_eq_zero, divisibility, parity and disjunctions.
    - `ab` → `a * b`.
    - ∃-binder types: ℕ witnesses and `[MOD n]`.
    - Sums with ℕ subtraction are stated in ℤ.
    - The step variable is no longer taken from a sum index.
    - Prose counts ("there are C(5,2) = 10 ways") are checked.
- **stex (STEX §6, §10).**
  - Induction, nested cases/subcases and `step N` procedures.
  - "suppose not" negates the goal the way a mathematician writes it, e.g. "$x \geq 2$" and "$n$ is odd".
  - Also: "between" ranges, ordinals, typed declarations, predicates, and spaced juxtaposition (`a^2 + ab + b^2` stays math).
  - 6 new golden tests (47 stex unit tests).
- **Editor (EDITOR D-11, D-21…D-25).**
  - Our own regex engine (Pike VM) and `Query` search with Alt+R/C/W toggles and `$1` replacements. Vim `/ n N * :s` use regex.
  - Soft wrap: Alt+Z, on by default for prose files.
  - Multiple cursors: Ctrl+D, Ctrl+F2, Ctrl+Alt+↑/↓, Alt+click, Esc. One undo step via nested transactions and an edit log.
  - Files changed on disk: clean buffers reload (undoable), edited ones get ⚠ and an overwrite confirmation, deleted ones show "(deleted)". The explorer refreshes when a listed folder's mtime changes.
  - Splitter hover: highlight plus an OSC 22 resize pointer, using mouse mode `?1003`.
  - The Find box prefill is replaced by the first keystroke.
  - 6 new unit tests and 5 new pty e2e tests (22 e2e).
- **Linux (RESEARCH R9).**
  - `cargo test --workspace` is fully green in the Ubuntu arm64 VM, including all e2e tests and real PDF builds with the bundled linux-arm64 TinyTeX.
  - xetex needs the system `libfontconfig.so.1`. cmx now shows the loader error and an install hint, and the CLI tests skip xelatex on such hosts.
  - The x86_64 archive was checked structurally only.

**Decisions**
- Alt+Z now toggles word wrap. The legacy redo fallbacks are Ctrl+Y, Alt+Shift+Z and F4.
- Multiple cursors are a default-mode feature. Vim mode clears them.
- Change detection polls `stat` once a second plus on focus-in, with no kqueue/inotify (simple, portable, microseconds).
- Vim searches are always regex. The Find box is literal unless Alt+R is on.

**Problems**
- The VM's e2e tests flaked once while the Lean corpus ran on the host (CPU contention). They passed on rerun.
- Lean `first` reports only the last alternative's error, which made the early failures confusing (R7).

**Next**: CONTEXT §4.

### 2026-11-13: session 5 (part 1): stex corpus audit, findings before fixes

User feedback:
- `~/dev/sandbox/prime.stex` renders wrong.
- stex keywords vs prose are unclear in the editor (aliases like `s.t.` uncolored, "such" colored alone, "there is"/"exists" in different colors).
- `cross` doesn't become `\times` inside `$…$`.
- The user asked for 100–200 proofs to find which keyword interactions fail, with the findings documented *before* fixing.
- Answers: the user runs **Terminal.app**, and wants resize feedback + Ctrl+Shift detection in every terminal (macOS and Linux). The **TinyTeX archives stay in the repo as normal files** (no Git LFS).
- Aside from the user: Ctrl+arrows should go to line/document start and end, and the Ctrl+Delete keys should work.

Done (part 1):
- Wrote 177 corpus proofs (`crates/cmx-stex/tests/corpus/`).
- Compiled them in all four variance modes: 177/177 PDFs, so no TeX errors.
- Read every `--fixed` output and wrote the root-cause taxonomy in STEX-DESIGN §12 (A1–A10, B1–B7, C1–C5, D1–D4, E1–E10, F) with affected files and a fix order. The user's prime.stex combines A1 (ellipsis), A10 (`$…$` fallback), C1 (article `a` read as a variable) and E1 (a line starting with "is").

### 2026-11-13: session 5 (part 2): fixes, regression suite, highlighting, Terminal.app

- **stex fixes, in the planned order** (STEX §12.1): the user-reported items first (A1 ellipsis, A10 lenient `$…$`, C1 articles, E1 line continuation), then by frequency.
  - Audit re-runs: 84 files flagged → 25 → only false positives.
  - Every output was reread after each round; regressions caught on the way:
    - number sets inside set-builders, from a too-greedy capital-letter pre-scan;
    - `born` read as math;
    - the if/then comma inside braces.
  - Root bug behind A9: `rel_op` ignored the stop list, so the `|` in `{x | …}` was "divides".
- **Regression suite:** `crates/cmx-stex/tests/corpus.rs` (177 snapshots, lints, all modes, Lean translation), plus 5 named audit golden tests. The Lean corpus is still 52/52.
- **Editor:**
  - stex colors come from the compiler's lexer (D-26).
  - The Shift probe (D-27): macOS CoreGraphics, Linux X11 via dlopen, verified under Xvfb + xdotool.
  - The resize marker (D-28).
  - Ctrl+arrows and Ctrl+Backspace/Delete (D-29).
  - New e2e tests: `stex_highlighting_follows_the_compiler`, `ctrl_arrows_and_ctrl_delete`.
  - The harness sets `CMX_MODPROBE=0`.
- **Linux:** full suite green in the VM.
- **Decision (user):** the TinyTeX archives stay in the repo as plain files (no LFS).

### 2026-11-13: session 5 (part 3): Terminal.app regression

- **User report:** Ctrl+Shift+Z doesn't redo, and Ctrl+L/R/Y (with or without Shift) no longer open the panels in Terminal.app.
- **Cause:** macOS gates the modifier-state query behind **Input Monitoring** for the terminal app. Without it the query reports no Shift. The probe trusted that answer, so every Ctrl+letter became a certain plain Ctrl chord, which also switched off the legacy panel fallback.
- **Fix:**
  - The probe is trusted only when `CGPreflightListenEventAccess()` is true; otherwise the legacy promotion is back.
  - In Terminal.app cmx asks macOS for the permission prompt once and explains in the status bar.
  - A palette command, *Keyboard: Detect Ctrl+Shift in This Terminal*, re-asks and opens the right Settings pane.
  - New e2e test `legacy_panels_work_with_probe_enabled`.
  - RESEARCH R10 corrected: my earlier "no permission needed" note was wrong.

### 2026-11-13: session 5 (part 4): Terminal.app Ctrl+Shift, for real

- **User:** Ctrl+Shift still didn't work after granting Input Monitoring.
- **Diagnosis**, run inside the user's Terminal.app via `.command` files and Quartz-posted key events:
  - Terminal already had Input Monitoring, and the modifier query works.
  - The new `CMX_KEYLOG` showed that Ctrl+Shift+Z never reached cmx.
  - A raw byte logger confirmed that Terminal.app sends **nothing** for any Ctrl+Shift+letter.
- **Fix:**
  - `cmx_sys::keytap`: a listen-only event tap in Terminal.app, gated on focus reporting and the Terminal ancestor (D-30). Verified end to end: undo, then Ctrl+Shift+Z redo, saved file `ab`; Ctrl+Shift+L → ToggleTree.
  - In Terminal.app the Shift probe is no longer used, so plain Ctrl+L/R/Y keep their panel fallback whenever the tap isn't running.
  - Also fixed two newer-clippy lints that only Linux's toolchain reports.
- **Follow-up (user):** no macOS beep on the Ctrl+Shift chords. Terminal.app beeps at keys it can't use, so the tap is now active (Accessibility, which Terminal has). It consumes only the chords cmx handles (Ctrl+Shift+letter → cmx's Terminal → while focused). Without Accessibility it falls back to listening, with a hint. Verified `silent=true` and chords working in the user's Terminal.app.

### 2026-11-14: session 5 (part 5): case numbering, "exhaustive", agreement

- **User request** (four items; the 4th, the email/Instagram/Discord system, comes after the research below):
  1. Automatic case numbering beside manual labels: "case k is odd" must not read "Case k: Suppose is odd"; a letter label only in forms like "case k n = 1".
  2. An `exhaustive` keyword family, alone or with `b/c`, for justifying that the cases cover everything. When used at the end, add a closing transition ("In all exhaustive cases, …"); otherwise no closing prose.
  3. Wrong agreement: "Hence there are $l \in \mathbb{Z}$" (odd8).
- **Done** (STEX-DESIGN §14):
  - Label detection plus auto-numbering, with sub-cases numbered "2.1".
  - Blank lines inside a case stay in the case when more cases or a closer follow (odd8's list had split at every blank line).
  - The exhaustive opener/closer family, with justifications derived from the case conditions: predicate/opposite, P/not P, complementary relations, trichotomy, residues mod m, `n = mq + r`.
  - Binder agreement in every mode.
- **Bugs fixed on the way** (found by writing the new corpus files):
  - sibling `subcase`s nested ever deeper;
  - "we show |x−y|" swallowed "show" into the math;
  - "xy" after `let x, y` stayed prose;
  - "$0$ or 1 upon";
  - "there are no integer x".
  - Also, `qed` after "In both cases, P" no longer restates the goal: nt15 and st03 snapshots changed, reviewed.
- **Corpus:** 191 proofs (`ca01`–`ca14`), plus 5 new unit tests. The user's odd8 now renders as intended in all four modes.
- macOS and Linux green, 0 clippy warnings.

### 2026-11-14: session 5 (part 5b): research for the email/DM sender (no code)

- **User's rule:** research and brainstorm for ≥30 minutes before implementing anything for the send system. Started 03:53. Findings are in RESEARCH R12; the design proposal is MAIL-DESIGN.md (rewritten).
- **Spikes** (scratchpad only, nothing installed):
  - **curl:** curl against a fake local SMTP server checked AUTH PLAIN, XOAUTH2 (byte-exact with Microsoft's format), dot-stuffing, Bcc, the EHLO-from-URL-path quirk, `-K -` credentials on stdin, and `-X NOOP` login checks.
  - **Keychain:** a throwaway keychain checked `security -i`.
  - **Linux VM:** a transient systemd timer (default 1-minute accuracy).
  - **Discord:** an ISPDB lookup, then a WebSocket handshake to Discord's gateway through `/usr/bin/openssl` (HELLO received; LibreSSL does no hostname check).
- **Design-changing findings:**
  - Outlook is OAuth-only, and new accounts may even block OAuth SMTP, so Outlook goes through Graph `sendMail` with MIME.
  - Device code is blocked in many work tenants, so sign-in uses PKCE loopback.
  - Gmail OAuth is heavy (no device flow, verification), so app passwords are the default.
  - School Workspace accounts may block both; the escape hatches are Apple Mail and GNOME Online Accounts.
  - Discord user automation is banned and bots need a server you administer, so hand-off is the default.
  - Instagram has no compliant personal DM API: hand-off only.
- **Open questions Q1–Q7** (MAIL-DESIGN §11) go to the user before building.
- **Test note:** running the macOS and Linux suites at the same time on this host made 3 Linux pty e2e tests time out in `wait_for` (5 s screen waits). Run alone, they all pass on Linux and macOS. Avoid concurrent runs, or raise the e2e timeouts if this recurs.

### 2026-11-14: session 5 (part 6): case-analysis prose only from the user

- **User:** "cases exhaustive" alone compiled to "Since k is either odd or even, the following cases are exhaustive". A reason must appear only when the user gives one (`b/c` before or after the keyword). A bare keyword says just "The following cases are exhaustive." / "These cases are exhaustive." Without the keyword, nothing at all. New: "case on k" → "We proceed by case analysis on k" (only when written).
- **Done** (STEX-DESIGN §14, revised):
  - Removed the derived justifications and the auto closing line (the common conclusion of the cases).
  - Reasons keep the user's word and order: "Because R, the following cases are exhaustive".
  - Added the `case on` family (12 phrasings, also inside a case before sub-cases, combinable with `, exhaustive b/c R`).
  - Highlighting covers the new phrases.
- **Corpus:** 195 proofs (+ca15–ca18). The 9 changed ca snapshots were reviewed: only the derived text disappeared. The user's odd8 now reads "The following cases are exhaustive." … "In exhaustive cases, $8 \mid (x^2 - 1)$."
- MAIL design: the user says "looks fine for now", with tweaks to come.
- **Linux e2e flake:** right after a rebuild, the full Linux run twice had 2–3 pty e2e tests time out in `wait_for`/`wait_exit` (5 s). Re-running alone (3×) and in full passed. This is pre-existing timing sensitivity and unrelated to stex. If it keeps recurring, raise the e2e wait timeouts.

