# RESEARCH: external facts and findings

> **TEMPORARY FILE.** Part of the `claude/` tracking system. Remove only when the user says so.
> Organized by topic. Every fact carries a tag: `[verified]` means tested on a machine or checked against a
> primary source this project used; `[unverified]` means from memory or secondary sources, so check it
> before relying on it. Add the source or the test that verified it.

## R1. Dev environment (session 1)

- macOS 27.2 (Darwin 27.2.0), arm64. rustc/cargo 1.98.0. python3 3.15. `claude` at `~/.local/bin/claude`. `[verified]`
- **Not installed:** `lean`, `lake`, `pdflatex`/`xelatex`/`lualatex`. `[verified]` (`which`)
- The shell spawned by the VS Code extension reports `TERM=xterm-256color TERM_PROGRAM=Apple_Terminal COLORTERM=truecolor`. The env is inherited, so `TERM_PROGRAM` is not a reliable signal for "which terminal am I in". `[verified]`

## R2. Terminal protocols used by the editor

| Feature | Enable / disable | Notes |
|---|---|---|
| Alt screen | `CSI ?1049 h/l` | saves cursor too `[verified: xterm ctlseqs]` |
| Mouse: press/release, drag | `CSI ?1000 h`, `CSI ?1002 h` | 1003 = any motion (not used) |
| SGR mouse encoding | `CSI ?1006 h` | report `CSI < b;x;y M/m`. Bits: +4 shift, +8 alt, +16 ctrl, +32 motion, +64 wheel `[verified]` |
| Bracketed paste | `CSI ?2004 h` | `CSI 200~` … `CSI 201~` |
| Focus events | `CSI ?1004 h` | `CSI I` / `CSI O` |
| Synchronized output | `CSI ?2026 h/l` | wraps each frame. Unknown to a terminal = ignored |
| Cursor shape (DECSCUSR) | `CSI n SP q` | 1/2 block, 3/4 underline, 5/6 bar (odd = blink), 0 = default |
| Kitty keyboard protocol | query `CSI ? u` → reply `CSI ? flags u`; push `CSI > 1 u`; pop `CSI < u` | flag 1 = disambiguate. Keys arrive as `CSI code[:alts];mods[:event][;text] u`. mods = 1 + bits(shift 1, alt 2, ctrl 4, super 8, hyper 16, meta 32, caps 64, num 128). With flag 1, Esc = `CSI 27u`, Ctrl+Shift+L = `CSI 108;6u` `[unverified: from the kitty spec, from memory]` |
| xterm modifyOtherKeys | `CSI > 4;2 m` / reset `CSI > 4 m` | keys arrive as `CSI 27;mods;code ~` (formatOtherKeys=0) or `CSI code;mods u` (=1) |
| DA1 | `CSI c` → `CSI ? … c` | sent after the kitty query: the DA1 reply arriving without a kitty reply means "no kitty" |
| OSC 52 clipboard | `OSC 52 ; c ; <base64> BEL` | kitty, WezTerm, Ghostty, Alacritty, foot, iTerm2 (opt-in), tmux (set-clipboard). **Not** Terminal.app `[unverified]` |

**Terminal support matrix for enhanced keys** `[unverified: compiled from memory, verify per terminal]`:
- kitty protocol: kitty, WezTerm, Ghostty, foot, Alacritty ≥ 0.13, iTerm2 ≥ 3.5, Rio. xterm.js / VS Code: possibly, behind a setting.
- modifyOtherKeys: xterm, iTerm2, (tmux `extended-keys`).
- Neither: macOS Terminal.app. So Ctrl+Shift+letter arrives as Ctrl+letter, and the editor falls back (EDITOR-DESIGN D-3).
- macOS **Cmd** combos are mostly consumed by the terminal app itself. Under the kitty protocol, terminals that forward Cmd send it as the `super` modifier, and the editor treats super like ctrl.
- VS Code's integrated terminal swallows many Ctrl+Shift chords unless `terminal.integrated.sendKeybindingsToShell` is true, or the commands are listed in `commandsToSkipShell`.

### R2.1 Observed in session 1 (verified on macOS 27.2 arm64)

- A pty child spawned with `setsid` + `open(slave)` + `ioctl(TIOCSCTTY, 0)` gets a working controlling tty on macOS (job control works, `sh -l` runs). `[verified]`
- Claude Code v2.1.x (Ink-based TUI) renders its onboarding screen correctly inside cmx-vt: box drawing, colors, the theme picker, diffs. It needs only the xterm subset we implement. It does *not* require kitty keyboard support inside the pane. `[verified]`
- Latency through pty → cmx → cmx-vt (release build) is about 1.3 ms keypress-to-screen median (session 1), 1.6 ms in session 2 while other jobs were running. `[verified: ide_screenshot test]`
- stex compile speed: 6000-line source → 430 KB .tex in ~50 ms; `stex check` emission with 2500 obligations in ~130 ms (release). `[verified: session 2]`
- `claude` can live in `/usr/local/bin` or `/opt/homebrew/bin` even when PATH lacks it. That's why `$CMX_CLAUDE` exists as an override. `[verified]`

### R2.2 Terminal.app keyboard (session 2; the features built on it were removed in session 3)

- `Terminal.app/Contents/Resources/modifiers.loctable` lists the only modifier prefixes its profile keyboard map offers: `^` ⌃, `~` ⌥, `$` ⇧ and their combinations. **There is no Command (`@`).** `keys.loctable` lists only special keys (F1–F20, arrows, ⌦, ⌫, keypad). Map entries look like `"^$F702" = "\033[1;6D"`. The key is modifier prefixes plus 4-hex-digit unicode; the value is the raw string, and plist XML stores the ESC byte raw. `[verified: read the files on macOS 27.2]`
- Consequence: Terminal.app cannot be configured to send Cmd chords to a program, so cmx needs an event tap (EDITOR-DESIGN D-17). Letter keys with ⌃⇧ may work through the same table (D-18). `[unverified]`
- Event taps that modify events (`kCGEventTapOptionDefault`) need **Accessibility** permission. For a CLI the permission belongs to the *responsible* GUI app (the terminal). Listen-only taps need Input Monitoring instead. "Secure Keyboard Entry" in Terminal blocks taps. `[unverified: from Apple docs, from memory]`
- `defaults export com.apple.Terminal -` returns the prefs as an XML plist. `Default Window Settings` names the default profile; profiles live under `Window Settings`. Opening a `.terminal` file imports it as a profile. `[verified: export; import unverified]`

## R3. POSIX / FFI facts (cmx-sys)

- `struct termios` differs: Linux glibc uses `tcflag_t=u32` and NCCS=32 (60 bytes). macOS uses `tcflag_t=unsigned long` and NCCS=20 (72 bytes). **Solution:** keep an opaque 256-byte aligned buffer and use `tcgetattr`, `cfmakeraw` and `tcsetattr`, so we never touch the fields. `[verified: compiles+runs on macOS arm64]`
- ioctl numbers: Linux `TIOCGWINSZ=0x5413 TIOCSWINSZ=0x5414 TIOCSCTTY=0x540E`; macOS `0x40087468 / 0x80087467 / 0x20007461`. `[unverified on Linux]`
- `ioctl`, `fcntl` and `open` are **variadic**. They must be declared with `...` in Rust because Apple arm64 passes variadic arguments on the stack. `[verified]`
- Open flags: Linux `O_NOCTTY=0o400 O_NONBLOCK=0o4000 O_CLOEXEC=0o2000000`; macOS `0x20000 / 0x4 / 0x1000000`.
- Signals: SIGWINCH=28 on both. SIGCHLD = 17 on Linux, 20 on macOS.
- `nfds_t` is `unsigned int` on macOS and `unsigned long` on Linux.
- PTY: `posix_openpt` → `grantpt` → `unlockpt` → `ptsname`. The child runs `setsid`, opens the slave, then `ioctl(TIOCSCTTY)` and `dup2` 0/1/2. **Reset SIGPIPE to SIG_DFL** before exec: the Rust runtime sets it to ignore, and ignored dispositions survive exec.
- Rust std ignores SIGPIPE in the parent, so writes to a closed pty return EPIPE instead of killing us. Good.

## R4. Lean / Mathlib (for stex check)

- Mathlib has on the order of 200k+ theorems/lemmas (the user's "200,000"). `[unverified]` A Mathlib project with its build cache is **7.2 GB** on disk `[verified session 4]`, so vendoring it is impractical (STEX Q-S1).
- Install (session 4, `[verified]`): `curl https://elan.lean-lang.org/elan-init.sh -sSf | sh -s -- -y`, then `lake new stexcheck math` in `~/.cmx/` (the `math` template adds Mathlib), then `lake exe cache get` in the project; check a file with `lake env lean FILE.lean` from the project dir. Installed: **Lean v4.36.0-rc2** + current Mathlib at `~/.cmx/stexcheck`, `lake` at `~/.elan/bin/lake`. A file importing Mathlib takes ~10–25 s to check (loading Mathlib dominates).
- `data/theorems.tsv`: every Lean name was checked with `#check` against this Mathlib (2026-11-13). Two were wrong and are fixed: AM–GM → `Real.geom_mean_le_arith_mean2_weighted`, Cauchy–Schwarz → `norm_inner_le_norm`. `[verified]`
- Generated syntax `[ZMOD n]` / `[MOD n]`, `∑ i ∈ Finset.Icc a b, f`, `Nat.choose`, `Real.sqrt` is accepted. `[verified]`

## R5. LaTeX

- Engines: pdflatex (default), xelatex, lualatex. The user's machine has MacTeX at `/Library/TeX/texbin` (not on cmx's dev PATH); the user doesn't want it (5 GB), so cmx bundles TinyTeX. `[verified]`
- **TinyTeX** v2026.11 (rstudio/tinytex-releases): TinyTeX-1 archives are 67 MB (darwin, universal binaries), 54 MB (linux-x86_64, linux-arm64). Linux archives unpack to `.TinyTeX/`, darwin to `TinyTeX/`. TinyTeX-1 has amsmath, amsfonts/amssymb, amsthm, geometry, graphics, hyperref, xcolor, lmodern, but **not** needspace, mathrsfs (package `jknapltx`), rsfs, enumitem, pgf; added with `tlmgr` (needs `tlmgr update --self` first; repository `https://tlnet.yihui.org`). The trees are byte-identical across platforms outside `bin/` except generated files (ls-R, maps, fmtutil.cnf, updmap.cfg). The tree is relocatable (kpathsea finds texmf via the binary's location). `[verified]`
- TinyTeX has no METAFONT sources for the EC/TS1 fonts, so an OT1 document that needs a TS1 glyph fails with "Font tcrm1095 at 600 not found". `\usepackage[T1]{fontenc}` + `lmodern` (Type 1) fixes it; pdflatex, xelatex and lualatex all compile the full stex feature set that way. `[verified]`
- VS Code ≥ 1.109's terminal (xterm.js) reportedly supports the kitty keyboard protocol behind `terminal.integrated.enableKittyKeyboardProtocol` (exact setting name varies across reports). `[unverified: blog.fsck.com/releases/2026/02/26/terminal-keyboard-protocol, user bug reports]`
- Install hints printed when missing: macOS `brew install --cask mactex-no-gui` (or `basictex`), Debian/Ubuntu `sudo apt install texlive-latex-extra`, Fedora `sudo dnf install texlive-scheme-medium`. `[unverified package names]`
- `--open` uses the default *browser* (spec): macOS reads the `https` handler from `~/Library/Preferences/com.apple.LaunchServices/com.apple.launchservices.secure.plist` (`LSHandlers`) and runs `open -b <bundle-id> F.pdf`; falls back to Safari / `open`. `[unverified: plist location from memory]`
- Templates the user named: Jake's resume `https://github.com/jakegut/resume`; CMU homework `https://github.com/jez/cmu-latex-hw-template`. Not yet fetched. Licenses need checking before vendoring.

## R6. Proof-prose phrase frequencies (for stex `--natural`) — measured session 3

Corpus (downloaded 2026-11-13 to a scratch dir; only aggregate counts are kept in the repo):

| Group | Sources | Proofs | Sentences | Words |
|---|---|---|---|---|
| textbook | Lebl *Basic Analysis* (jirilebl/ra) and *Several Complex Variables* (jirilebl/scv), Judson *AATA* (twjudson/aata, PreTeXt), Beezer *FCLA* (rbeezer/fcla, PreTeXt), Open Logic Project, HoTT book | 2,376 | 20,159 | 278,771 |
| research | 183 most recent arXiv papers in math.NT, CO, CA, PR, GR, FA, LO, AC (`export.arxiv.org` e-prints, 3 s between requests) | 3,701 | 40,880 | 636,140 |
| stacks | The Stacks Project (stacks/stacks-project) | 13,188 | 106,282 | 1,545,774 |

Method (`tools/stex-corpus`): extract `\begin{proof}…\end{proof}` / `<proof>`, replace math with `math` and references with `ref`, split sentences, lowercase. Count ~300 probe phrasings (sentence starts or anywhere) and the top sentence openings. Rates are per 10k proof sentences; the combined rate is the unweighted **mean over the three groups** so Stacks doesn't dominate. Full table: `tools/stex-corpus/frequency-table.tsv`. `[verified: measured]`

Findings that shaped the bank:
- Conclusions: Then 448 ≫ Hence 351 ≈ Thus 349 > So 200 > Therefore 87 > Consequently 33 ≈ It follows that 31; "We deduce that" is rare (0.7).
- Justification: "since" 883 ≫ "because" 193 > ", as" 73; sentence-initial "Since" 503 > "As" 134 > "Because" 26. "By X, …" 595 opens more sentences than any connective.
- Contradiction: "a contradiction" 44 and "contradicting" 16 dominate; "This is a contradiction" 4. Opening one: "Suppose to the contrary" 4.3 (textbooks), "If not" 3.7 (Stacks); "Assume for the sake of contradiction" is almost never written out (0.1) but stays the `--fixed` choice (it is what `afsoc` abbreviates).
- Goals: "We claim that" 22, "We have to show" 21.5, "We prove" 18, "We will prove" 14, "We show that" 7.
- Closings: "as desired" 47 > "this proves the …" 23 > "we are done" 18 > "as required" 14.
- Quantifiers: "for all" 228 > "for every" 155 > "for any" 114 ≈ "for each" 103; "there is" 183 > "for some" 148 > "there exists" 134 > "there are" 83.
- Restating: "in other words" 40, "namely" 33, "that is" 15 (sentence-initial); mid-sentence "that is" 78.
- Notes: "Note that" 71, "Observe that" 36, "Recall that" 27, "Notice that" 13. WLOG: "Without loss of generality" 8.8 ≈ "We may assume" 8.2.
- Library words actually used in proof prose (sentences per 10k): "inverse" 80, "equal to" 75, "subset" 58, "is in" 54, "union" 43, "is equal to" 39, "intersection" 33, "is at most" 28, "belongs to" 25, "maps to" 24, "divides" 13.

## R7. Lean behaviour that shaped `stex check` (session 4, all `[verified]` on v4.36.0-rc2)

- `first | t1 | … | tn` reports **only the last alternative's error**. A failing `stex_auto` used to say "Insufficient number of fields for ⟨...⟩" (from the `refine ⟨?_, ?_⟩` branch), so the last branch is now `fail "…"`, whose message includes the goal.
- Macros are **hygienic**: an `ih` written inside a `macro` body does not see the user's `ih`. `set_option hygiene false in` before the macro fixes it. A doc comment must come *after* `set_option … in`, not before (parse error otherwise).
- A recursive tactic macro needs `syntax "stex_auto" : tactic` + `macro_rules`: a `macro` can't mention its own syntax.
- `ring` succeeds with "Try this: ring_nf" without closing the goal; use `ring1` in automation.
- `push_neg` is deprecated in favour of `push Not` (warning only; kept for older Mathlibs).
- `linarith` treats nonlinear subterms (`k*(k+1)`, `2^k`) as atoms, so a hypothesis about `k*(k+1)` is usable when the goal has the same syntactic term. `omega` also atomizes `2^(n-1)` but can't relate `2^n` to it: rewrite `n = m + 1` first.
- `simp_all [parity_simps]` leaves `Even m ↔ Even n` for odd+odd; `Odd.add_odd` or `simp_all [parity_simps, ← Int.not_even_iff_odd]` closes it. `Int.ModEq.pow _ h` lifts `x ≡ 2` to `x^2 ≡ 2^2`. `(8:ℕ) ≡ 2 [MOD 3]` and `(15:ℤ) ≡ 1 [ZMOD 7]` close by `decide`.
- `Finset.sum_Icc_succ_top` needs `a ≤ b + 1`; `rw [lemma (by omega)]` can't solve that side goal before unification, hence the side-condition-free helper lemmas `stex_sum_Icc1_succ` etc. in the prelude.
- `(e : ℤ)` ascription on the left of `=` makes Lean elaborate a ℕ-indexed sum body in ℤ (`2 * ↑i - 1`), so `∑ (2i − 1) = n^2` means what the user meant.

## R8. Pointer shape and mouse motion (session 4)

- **OSC 22** sets the mouse pointer shape: `ESC ] 22 ; <name> ST`. kitty, Ghostty, foot and WezTerm accept CSS cursor names (`ew-resize`, `ns-resize`, `text`, `default`); xterm uses X cursor-font names. Terminals without support ignore it. `[unverified: from memory, not tested in those terminals]`
- Mouse mode `?1003` (any-event tracking) reports motion with no button as SGR button code 35 (`CSI < 35 ; x ; y M`). `[verified with the cmx-vt host in e2e]`

## R9. Bundled TinyTeX on Linux (session 4)

- linux-arm64 archive, Ubuntu (Lima VM): unpacks to `~/.cmx/tinytex/linux-arm64-v2026.11-cmx1` (the repo mount is read-only); **pdflatex and lualatex build PDFs out of the box**. `[verified]`
- Linux **xetex links against the system `libfontconfig.so.1`** (missing on minimal servers/VMs). With `libfontconfig1` installed, xelatex works. `cmx stex compile --xelatex` now prints the loader error and the install hint. `[verified]`
- linux-x86_64 archive: same file list as arm64 (binaries, formats, extra packages); its binaries were not executed (no x86_64 loader in the arm64 VM; Rosetta is off). `[unverified at runtime]`

## R10. Reading Shift from the OS (session 5)

- **macOS:** `CGEventSourceFlagsState(0 combined / 1 HID)` from CoreGraphics returns the current modifier flags; Shift = `0x20000` (`kCGEventFlagMaskShift`).
  - **Correction (session 5):** reading it from a background process is gated by the Input Monitoring permission (TCC ListenEvent) of the responsible app, i.e. the terminal. Without it the flags show no keyboard modifiers: they read `0x100` with Shift held. `[verified: the user's Terminal.app showed Ctrl+Shift+Z as Ctrl+Z; CGPreflightListenEventAccess() = false and IOHIDCheckAccess(listen) = denied here]`
  - `CGPreflightListenEventAccess()` checks the permission; `CGRequestListenEventAccess()` shows the system prompt (first time only).
  - The Settings pane URL is `x-apple.systempreferences:com.apple.preference.security?Privacy_ListenEvent`.
  - A granted permission applies after the terminal is reopened. `[unverified: from Apple's TCC behaviour]`
  - Linking the system framework (`#[link(name = "CoreGraphics", kind = "framework")]`) is a system library, not a crate.
- **Linux/X11:** `XQueryKeymap(display, char[32])` returns the server's key vector. Shift keycodes come from `XKeysymToKeycode(XK_Shift_L 0xffe1 / XK_Shift_R 0xffe2)`. libX11 is loaded at runtime with `dlopen("libX11.so.6")`, so nothing is needed at build time. `[verified: Ubuntu arm64, Xvfb :99; Some(true) while held, Some(false) after]`
  - Keys pressed through XTEST (`xdotool keydown`) are released when that client disconnects, so a test must keep the xdotool process alive (`xdotool keydown shift sleep 6 keyup shift &`). `[verified]`
- **Native Wayland** gives other programs no global key state. XWayland's keymap only reflects X11 clients' focus. So the probe returns false/None there. `[unverified: from Wayland's security model]`
- **SSH:** `SSH_CONNECTION`/`SSH_TTY` set means the keyboard is on another machine, so the probe is off. `limactl shell` is SSH, so it sets them too. `[verified]`

## R11. Terminal.app limits that shape cmx (session 5)

- **Terminal.app sends nothing for Ctrl+Shift+letter.** Raw bytes, with key events posted through Quartz:
  - ⌃Z `1a`, ⌃L `0c`, ⌃Y `19`, ⌃R `12`, ⌃P `10`;
  - ⌃⇧Z, ⌃⇧L, ⌃⇧Y, ⌃⇧R and ⌃⇧P send nothing.

  `[verified 2026-11-13 on the user's Mac]`
- Terminal.app supports focus reporting (`CSI ?1004h` → `ESC [ O` / `ESC [ I` on app switches). `[verified]`
- With Input Monitoring granted, a listen-only `CGEventTapCreate(kCGAnnotatedSessionEventTap, …, kCGEventTapOptionListenOnly, keyDown)` started from a process under Terminal sees those chords, with the target pid field (40) set to Terminal's pid. `[verified]`
- Programs under Terminal inherit its permissions: Input Monitoring was granted (`CGPreflightListenEventAccess() = true`), and Accessibility too. `[verified]`
- An active tap (`kCGEventTapOptionDefault`, Accessibility needed) whose callback returns NULL drops the event, so the target app never sees it. That is how cmx avoids Terminal's beep for its Ctrl+Shift chords. The tap is created from a process under Terminal on the user's Mac. `[verified: created; effect on the beep not audible from here]`

- No OSC 22 (pointer shape) and no kitty keyboard protocol or modifyOtherKeys. So cmx draws its own resize marker (EDITOR D-28) and reads Shift from the OS (D-27). `[unverified: no Terminal.app documentation lists these; observed behaviour reported by the user]`
- macOS reserves ⌃←/→/↑/↓ for Mission Control by default. They reach the terminal only after those shortcuts are turned off in System Settings → Keyboard → Keyboard Shortcuts → Mission Control. `[unverified: from memory]`
- Terminal.app sends `^H` (0x08) for ⌃⌫. cmx's parser reads 0x08 as Ctrl+Backspace (delete to line start). `[unverified for Terminal.app specifically; standard xterm behaviour]`

## R12. Email and DM sending (session 5, part 5): research before designing MAIL

Researched 2026-11-14 03:53–04:23 (30 min). Sources are listed per point; `[verified]` means checked here by a spike.

**Provider authentication (2026)**
- **Gmail (personal):**
  - SMTP `smtp.gmail.com:465` (SSL) or `:587` (STARTTLS) accepts an **app password**, which needs 2-Step Verification; the plain password and "less secure apps" are gone.
  - App passwords aren't offered when 2SV is off, with security-keys-only 2SV, or with Advanced Protection.
  - ~500 recipients/day; 25 MB attachments.
  - The server files SMTP-sent mail in Sent automatically.
  - Gmail rewrites `From` to the authenticated address unless a "Send mail as" alias exists.
  - Sources: mailmeteor.com, cli.nylas.com, support.google.com/accounts/answer/7539956, wpmailsmtp.com, community.latenode.com, support.atlassian.com.
- **Google's own rule:** app passwords can't be created by **work, school, or other organization accounts**, security-keys-only 2SV, or Advanced Protection. The page is https://myaccount.google.com/apppasswords; the password is a 16-character code (support.google.com/accounts/answer/185833).
- **Google Workspace (school accounts, e.g. andrew.cmu.edu, whose MX is Google `[verified: dig MX cmu.edu]`):**
  - Less-secure-app access ended May 1, 2025 (workspaceupdates.googleblog.com).
  - Admins can disable app passwords.
  - Unverified OAuth apps are blocked unless the admin trusts them (knowledge.workspace.google.com, "Control which apps access Google Workspace data").
- **Gmail OAuth:**
  - The device flow does **not** allow Gmail scopes: only openid/email/profile, drive.file/appdata and youtube (developers.google.com/identity/protocols/oauth2/limited-input-device).
  - A desktop client must use the loopback redirect (127.0.0.1); OOB is gone (developers.google.com/identity/protocols/oauth2/resources/loopback-migration).
  - `gmail.send` is a *sensitive* scope: app verification needed, no CASA audit. `mail.google.com` (needed for SMTP XOAUTH2) is *restricted*: CASA required (developer.nylas.com Google verification guide; nango.dev).
  - Sensitive-scope verification: Google says 3–5 business days. Needs a consent screen, a per-scope justification and a demo video, plus a verified domain and privacy policy. 2026 forum threads report weeks-long stalls at the privacy-policy stage (developers.google.com sensitive-scope-verification; discuss.google.dev).
  - Apps in "Testing" status get refresh tokens that expire after **7 days** (dev.to, unipile.com).
  - The Gmail API `users.messages.send` accepts the raw MIME message as a media upload (`/upload/gmail/v1/users/me/messages/send`, `message/rfc822`) up to ~35 MB of encoded message (cli.nylas.com; gmass.co; SO answers quoting the reference). The same `.eml` works with no base64url step.
- **Outlook.com / Hotmail:**
  - Basic auth, app passwords included, is off for SMTP since 2024-09-16: **OAuth2 only** (Microsoft Q&A threads; support.microsoft.com known issue).
  - Microsoft documents OAuth for IMAP/POP/SMTP for Outlook.com users, using auth-code or **device code** flow, scope `https://outlook.office.com/SMTP.Send` + `offline_access`, SASL `XOAUTH2 base64("user=U^Aauth=Bearer T^A^A")` (learn.microsoft.com/…/how-to-authenticate-an-imap-pop-smtp-application-by-using-oauth).
  - A self-made app registration that includes personal accounts works for a limited number of personal accounts (mutt_oauth2.py README).
  - Mixing Graph and Outlook scopes in one request fails (Microsoft Q&A).
  - Graph `sendMail` can defer delivery on the server via the extended property `SystemTime 0x3FEF`, but this is unconfirmed for consumer accounts (pnp.github.io).
  - **New accounts:** since about 2026-10, *new* personal Outlook.com accounts fail OAuth SMTP with `535 … SmtpClientAuthentication is disabled for the Mailbox`. There is no official fix (learn.microsoft.com/answers/questions/6022235).
  - **Graph sendMail** (learn.microsoft.com/graph/api/user-sendmail):
    - Delegated `Mail.Send` is supported for **personal Microsoft accounts**.
    - The body can be the whole message as **base64 MIME** with `Content-Type: text/plain`.
    - Returns `202` and saves to Sent Items. Malformed MIME gives `400 ErrorMimeContentInvalidBase64String`.
    - The request limit is ~4 MB, base64 included (Microsoft Q&A 1824204; Stack Overflow).
    - Attachments of 3–150 MB need a draft plus `createUploadSession`, with byte ranges ≤ 4 MB per PUT (learn.microsoft.com/graph/outlook-large-attachments).
  - **Device code** (learn.microsoft.com/entra/identity-platform/v2-oauth2-device-code):
    - `POST /{common|consumers|organizations}/oauth2/v2.0/devicecode` (client_id, scope), then poll `/token` with `grant_type=urn:ietf:params:oauth:grant-type:device_code`.
    - Errors: `authorization_pending`, `authorization_declined`, `expired_token` (15 min).
    - Personal accounts sign in again on the device page.
  - **Admin approval:** org tenants often require admin approval for mail scopes ("Need admin approval"), via user consent turned off or the Microsoft-managed `microsoft-user-default-recommended` consent policy (Aug 2025; topedia.com 2026-05; vendor help pages naming Mail.Send).
  - **Device code blocked:** a Microsoft-managed Conditional Access policy (announced Feb 2025) blocks the device code flow by default in tenants that haven't used it in 25 days. Expect school/work M365 accounts to reject it (learn.microsoft.com/entra/identity/conditional-access/managed-policies; securityboulevard.com 2025-04).
  - **Microsoft loopback redirect:** the port is ignored when matching, but the path must match literally. 127.0.0.1 is preferred over localhost; `[::1]` is unsupported. So register `http://127.0.0.1` and redirect to `http://127.0.0.1:<random>/` with the root path (learn.microsoft.com/entra/identity-platform/reply-url; claudeissues 53290).
  - **Google desktop OAuth:** PKCE (S256) is required; `client_secret` is optional and "desktop apps are public clients"; redirect to `http://127.0.0.1:port` or `[::1]:port` with a listener on a random port (developers.google.com/identity/protocols/oauth2/native-app).
  - **App-password formats:** Google's is shown as 4×4 letters, and the spaces can be dropped. Apple's is `xxxx-xxxx-xxxx-xxxx`: keep the dashes, and if login fails retry without them (Readdle Spark support; Apple Community).
  - **ISPDB aliases:** me.com/mac.com → smtp.mail.me.com; hotmail/live/msn → smtp-mail.outlook.com; ymail → smtp.mail.yahoo.com; googlemail → smtp.gmail.com `[verified]`.
- **iCloud:**
  - `smtp.mail.me.com:587` STARTTLS with an **app-specific password** (2FA needed; resetting the Apple ID password revokes them); username is the full address (support.apple.com/HT202304; mailmeteor).
  - 1000 messages/day, 20 MB.
  - Generate at account.apple.com → Sign-In and Security → App-Specific Passwords. Up to 25 active; changing the Apple Account password revokes them all (support.apple.com/102654).
  - SMTP-sent mail is **not** filed in Sent: the client must IMAP-APPEND to "Sent Messages" (forum.emclient.com; limilabs).
- **Yahoo:**
  - `smtp.mail.yahoo.com:465/587` with an **app password** (Account Security → "Generate app password"; help.yahoo.com/kb/SLN15241).
  - Its ISPDB entry lists OAuth2 too.
  - Sent-folder behaviour is unknown.
  - Basic-auth IMAP ended 2024-05-15 (help.yahoo.com/kb/SLN36636). One unofficial report (JustAnswer) says app passwords were "temporarily unavailable". Risk: Yahoo may need OAuth (a Yahoo developer app) later.
- **Provider by MX:**
  - iCloud+ custom domains use `mx01/mx02.mail.icloud.com` (support.apple.com/102374; inwx.de).
  - Microsoft 365 uses `<domain-with-dashes>.mail.protection.outlook.com`; Google Workspace uses `aspmx.l.google.com` (+ `altN.`), or `smtp.google.com` on newer setups.
  - `[verified: dig MX]` microsoft.com and mit.edu → `*.mail.protection.outlook.com`; cmu.edu → `ASPMX.L.GOOGLE.COM`; icloud.com → `mx01/mx02.mail.icloud.com`. So school email can be either Google or Microsoft, and MX tells which.
- **Any domain:** the Thunderbird ISPDB serves `https://autoconfig.thunderbird.net/v1.1/<domain>` XML with `outgoingServer` host, port, socketType and authentication. `[verified for yahoo.com, gmail.com, outlook.com, icloud.com]` For other domains, look up the MX first (`dig`/`host` exist on macOS and Ubuntu `[verified]`).

**Transport and storage without crates** (all `[verified]` with a local fake SMTP server or a temporary keychain)
- **curl:** macOS (8.7.1) and Ubuntu (8.20) curl both support smtp/smtps/imap/imaps.
  - `curl smtp(s)://host:port/<ehlo-domain> --mail-from A --mail-rcpt B… --upload-file msg.eml` does dot-stuffing, and extra `--mail-rcpt` gives Bcc.
  - The URL **path is the EHLO name**: without one curl sent `EHLO msg.eml`.
  - `--oauth2-bearer T --login-options AUTH=XOAUTH2` produces exactly Microsoft's XOAUTH2 string.
  - `-X NOOP` authenticates and sends nothing, which gives credential verification at link time.
  - `-K -` reads `user = "…"` from stdin, so **passwords never appear in argv/ps**.
  - With `--mail-rcpt-allowfails`, a rejected recipient (`550`) doesn't stop delivery to the others, and the exit is still `250`. The rejection shows **only in `-v` output** (`> RCPT TO:<…>` then `< 550 …`). That output also contains the base64 AUTH line (the password), so it must be parsed for RCPT replies and never logged `[verified]`.
- **IMAP:**
  - `curl imap://host/ -X 'LIST "" "*"'` prints the server's folders with their special-use flags (`\Sent`), so cmx finds the Sent folder by flag, not by name.
  - `curl imap://host/Sent%20Messages -T msg.eml` sends `APPEND "Sent Messages" (\Seen) {n}` and uploads the bytes.
  - `%{response_code}` is `000` for IMAP, so success is judged by curl's exit status `[verified: fake IMAP server]`.
- **macOS Keychain:** `security -i` with `add-generic-password -U -s … -a … -w …` on stdin keeps the secret off argv; `find-generic-password -w` reads it back.
- **Linux:** `secret-tool store` reads the secret from stdin (man page), but needs a running Secret Service. Our Ubuntu VM has none (no libsecret-tools/gnome-keyring/pass), so the fallback is a 0600 file.

**Scheduling**
- **launchd:** a missed `StartCalendarInterval` runs on wake (coalesced), but not after power-off unless `RunAtLoad`. launchd can't wake the Mac (developer.apple.com forums; launchd.plist(5)).
- **launchd.plist(5) on this Mac** `[verified: man page]`:
  - `StartInterval` firings are **missed** while the system sleeps (kqueue).
  - `QueueDirectories` keeps a job alive while a directory is non-empty.
  - `ThrottleInterval` defaults to 10 s between spawns.
  - systemd's `.path` units have `DirectoryNotEmpty=` with the same meaning (systemd.path(5), VM).
    - It fires immediately if the directory is already non-empty when the path unit starts, and it ignores dotfiles.
    - Activations are rate-limited by `TriggerLimitIntervalSec` (default 2 s) and `TriggerLimitBurst`, so the runner must not exit and re-trigger in a loop.
- **Background Items alerts:** macOS 13+ shows "Background Items Added" when a persistent background item is registered. Users report repeats when items are re-registered (Apple Community threads; eclecticlight.co 2023-02-15). So: install once, never churn.
- **systemd:** `systemd-run --user --on-active=…` works but is transient (gone after reboot), and the default `AccuracySec=1min` delayed a 3 s timer to ~6 s `[verified in VM]`.
  - A unit-file timer with `Persistent=true` catches up after boot; `loginctl enable-linger` keeps user timers running after logout (wiki.archlinux.org/title/Systemd/Timers). Our VM has `Linger=yes`.
- **TCC:** an Apple Event sent by a launchd job is attributed to the job's binary, the "responsible process". An ad-hoc-signed binary re-prompts after every rebuild, so run the automation once at setup while the user watches (github.com/rjwalters/loom #6366; scriptingosx.com).
- **Notifications:** `osascript display notification` shows as Script Editor and fails silently without its permission (macscripter; claudeissues). Background results must also be reported in-band, on the next `cmx` run.

**DMs and texts**
- **Discord:**
  - Automating a user account ("self-bot") is against the ToS and can get the account terminated (discord-api-docs #440 quoting Discord support).
  - A bot can DM a user only if they share a server and the user allows DMs from server members (error 50007), and DMs should follow a user action (docs.discord.com/developers/resources/user, Create DM).
  - Search Guild Members (username → id) needs no privileged intent (docs.discord.com "you might not need a privileged intent").
  - User-installed apps can only respond to interactions (support.discord.com).
  - The Social SDK can DM on a user's behalf (`sdk.social_layer`), but access is limited, approval-gated and aimed at games (docs.discord.com Social SDK).
- **Instagram:**
  - The official messaging API works only from a *professional* account, only after the recipient messaged first, inside a 24 h window (developers.facebook.com, Instagram API with Instagram Login → Send Messages).
  - Private-API libraries (instagrapi) risk the account and break the terms (instagrapi maintainers; bot.space).
  - `ig.me/m/<user>` opens a DM on mobile, but Meta's docs say ig.me links are **not supported on Instagram Web**; on a computer they land on a login page or the inbox. `?text=` prefill is undocumented (Meta documents only `?ref=`) (developers.facebook.com, "Using ig.me Links"; setsmart.io July 2026 test).
- **Compose deep links:**
  - Gmail: `https://mail.google.com/mail/?view=cm&to&cc&bcc&su&body` (URL-encoded, `%0A` newlines; community-documented: til.simonwillison.net, Stack Overflow).
  - Outlook web: `https://outlook.live.com/mail/0/deeplink/compose?to&subject&body`; cc/bcc unconfirmed (Microsoft Q&A 4619953; techcommunity).
- **Common professional email types:** introductory, cold, reminder, follow-up, job application, confirmation, thank-you, recap (woodpecker.co; Grammarly; UWaterloo writing centre: mostly "request and reply").
- **Phone:**
  - macOS Messages is scriptable: `send … to participant` of an iMessage/SMS/RCS account, verified in the `.sdef` on this macOS 27.2. There is no delivery confirmation, and SMS needs iPhone Text Message Forwarding (Keyboard Maestro forum; macscripter).
  - Linux + Android: `kdeconnect-cli --send-sms TEXT --destination N -d DEV` (kdeconnect docs/bugs).
  - Carrier email-to-SMS gateways are closing: AT&T off June 2025, Verizon by March 2027 (phonescoop; textbolt).
- **Discord gateway:** a new bot must IDENTIFY over the gateway once before REST message sends work (community reports quoting the API error).
  - macOS's curl 8.7.1 has no ws/wss; Linux curl 8.20 has both `[verified]`.
  - `/usr/bin/openssl s_client -quiet -connect gateway.discord.gg:443` (LibreSSL 3.3.6) plus a hand-written WebSocket upgrade got `HTTP/1.1 101` and the gateway HELLO (op 10, heartbeat 41250 ms) `[verified, no token used]`.
- **LibreSSL hostname check:** macOS's `/usr/bin/openssl` (LibreSSL 3.3.6) `s_client` verifies the chain with `-verify_return_error` but has **no hostname check**: there is no `-verify_hostname`, and a valid certificate for another name passed `[verified]`.
  - Any token sent through it needs cmx's own SAN check, done on the same session's `-showcerts` output via `openssl x509 -noout -text`.
  - Alternatives: curl `wss://` (Linux) or an OpenSSL ≥ 1.1 with `-verify_hostname` (Homebrew's 4.0.3 is present here but not guaranteed).
- **Making a Discord app:** Developer Portal → New Application (a bot user is on by default) → **Bot → Reset Token** (shown once) → **Installation**: Guild Install with the `bot` scope and Send Messages permission → Discord Provided Link → "Add to server" (docs.discord.com/developers/quick-start/getting-started).
- **Adding a bot:** only the server owner or a member with Manage Server (or Administrator) can add a bot to a server (discordjs.guide; technipages).
- **Deep links:** `discord.com/users/<id>` is the profile link everyone uses, but Discord doesn't document it. It opens the app if installed, otherwise the browser, and behaves differently per platform (u2l.ai 2026; replug.io). `discord://-/users/<id>` is unofficial. The hand-off must therefore say what to click ("Message") and keep the text on the clipboard.
- **Ambiguous times:** a bare "9" is ambiguous, so ask rather than guess AM/PM. "Tomorrow" with no time conventionally means 9:00 AM (todobar write-up; AWS Lex AMAZON.Time).
- **`flock` constants match** on macOS (SDK sys/fcntl.h) and Linux (glibc sys/file.h): `LOCK_EX 2`, `LOCK_NB 4`, `LOCK_UN 8`. One FFI declaration serves both `[verified]`.
- **Prior art for late sends:** classic Outlook's "Do not deliver before" needs Outlook open and online. For POP/IMAP accounts a closed Outlook sends late, at next start, with no hold policy (support.microsoft.com "Delay or schedule sending email messages"; Microsoft Community). Holding overdue messages (MAIL Q7) improves on that.
- **Queue prior art:** Stebalien/msmtp-queue.
  - `msmtpq` only enqueues; `msmtpq-flush` sends.
  - systemd **path unit** triggers a flush when a message is queued, plus a 10-min timer.
  - Its author replaced lock *files* with advisory locks because a killed flush left the queue stuck (github.com/Stebalien/msmtp-queue README).
  - This matches the MAIL design; cmx uses `flock`.
- **CLI auth UX reference:** `gh auth login` (cli.github.com/manual/gh_auth_login):
  - A browser flow with `--clipboard` to copy the one-time code.
  - The system credential store, falling back to a plain-text file when no store exists; `--insecure-storage` forces it.
  - `--with-token` reads the secret from stdin; `gh auth status` shows where the token is stored.
- **Formality markers in emails:**
  - Polite requests use modals and if-clauses; a few hedges (possibly, perhaps) signal politeness; indirectness reads as more polite.
  - Student emails often use direct requests softened by mitigators.
  - "I hope this email finds you well" is formal; "How's it going?" is close-relationship only; "Cheers" is informal.
  - Sources: edtechbooks.org "Formal Emails" (BYU-I writing guide); Ibérica 2008 corpus of 100 institutional emails; TESL Canada journal; UCLan chapter.
- **Sign-off perception:** "Kind regards" is the most liked sign-off and "Hi" the most liked greeting; "Cheers" and "Love" rate poorly (Perkbox survey of 1,928 UK workers; Preply survey of 1,005 US respondents).
- **Reusing existing sign-ins:**
  - Apple Mail signs in to Google Workspace with OAuth, so no app password is needed (knowledge.workspace.google.com, "Set up Gmail with Apple Mail").
  - GNOME Online Accounts' Google provider requests `https://mail.google.com/` "for GMail IMAP and SMTP" (goagoogleprovider.c) and hands tokens to session apps via `GetAccessToken` (GOA D-Bus docs). These are the escape hatches for school accounts.
  - In 2025 GOA is still *the* way Geary and Evolution reach Gmail, and only Google and Microsoft OAuth providers are supported (discourse.gnome.org). GNOME 49 removes the old Windows Live provider, not Google. No current-release confirmation of Google mail was found `[unverified for GNOME 47–49]`.
- **AppleScript checked by compiling** (`osacompile`: no events sent, no TCC) `[verified]`:
  - `tell application "Messages" … send theText to participant theHandle of (1st account whose service type = iMessage)`.
  - `tell application "Mail" … make new outgoing message with properties {subject, content, visible:false}` → `set sender` → `make new to recipient at end of to recipients with properties {address}` → `send m`.
- **Mail.app scripting on macOS 15+:**
  - Bodies set by AppleScript are wrapped in `<blockquote type="cite">` (Apple FB11734014, open since Ventura), so recipients see a quoted-looking message.
  - The apple-mail-mcp project added a direct SMTP path for this reason. It also reports that the scripted path can't set `In-Reply-To`, and attachments need a wait (github.com/sweetrb/apple-mail-mcp).
  - Tahoe: AppleScript error retrieval is much slower, risking timeouts (mjtsai.com 2025-09-17).
- **Apple Mail scripting:** `make new outgoing message … send` still exists (Mail.sdef, macOS 27.2). It sends plain text only (`html content` is deprecated). Older and Sequoia-era reports of silent failures make it a fallback, not the main path.
