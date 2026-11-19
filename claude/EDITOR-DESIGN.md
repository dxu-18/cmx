# EDITOR-DESIGN: `cmx edit` / `cmx workspace`

> **TEMPORARY FILE.** Part of the `claude/` tracking system. Remove only when the user says so.
> Status: **implemented; tested on macOS and Linux arm64 (session 4)**. Checklist in §9 is the source of truth.

## 1. Requirements (from prompt.txt)

- `cmx edit`: blank untitled buffer, like `vim` with no args. Saving asks for a name.
- `cmx workspace`: current directory. File tree on the left, terminal open at the bottom, no file open, centre says "Open a file to begin".
- VS Code keybindings by default: mouse drag selection, Ctrl+click extends the selection to the cursor, Ctrl+Z/Ctrl+Shift+Z, Ctrl+C/V/X/A, arrows, Ctrl+arrows, plus the Mac (Cmd) equivalents.
- **Ctrl+K** toggles vim keybindings on and off.
- **Ctrl+Shift+L**: file explorer on the left. Clicking opens files. When the tree has focus, Up/Down move through files (prev/next) and leave the text alone.
- **Ctrl+Shift+R**: Claude pane on the right, auto-runs `claude` if installed.
- **Ctrl+Shift+Y**: shell at the bottom, in the current directory.
- Tabs in the centre.
- Draggable split borders (VS Code-like) to resize panes. "Split terminal" is supported too.

## 2. Architecture

```
cmx-editor/src/
  lib.rs        run(opts): terminal setup/teardown, panic hook, main loop entry
  app.rs        App state + poll loop + event dispatch + command execution
  render.rs     draws every component into the cell buffer (layout → widgets)
  layout.rs     computes rects for tree / editor / tabs / panel / claude / status / splitters
  keymap.rs     Cmd enum, default bindings, legacy-promotion rule, palette labels
  text.rs       Text (Vec<String> lines), Pos, grapheme/word navigation, display columns
  history.rs    undo/redo transactions with coalescing + save-point versioning
  document.rs   Document = Text + path + line ending + history + highlight cache
  view.rs       per-tab View (cursor/anchor/scroll) + Ed (doc+view editing operations)
  highlight.rs  table-driven syntax highlighter (~25 languages) + markdown/latex/stex modes
  vim.rs        vim emulation (normal/insert/visual/visual-line, operators, text objects, dot-repeat)
  tree.rs       lazy file explorer
  termpane.rs   PTY + cmx-vt emulator + selection/scrollback per terminal pane
  prompt.rs     one-line input widget (save-as, find, replace, goto, :ex, confirm)
  palette.rs    quick-open (fuzzy files) + command palette overlay
  theme.rs      Dark+ inspired palette
```

**Frame pipeline:** events → mutate state → `render::draw(app, &mut Buffer)` redraws everything into an off-screen cell buffer → `cmx_tui::Renderer` diffs it against the previous frame and emits the minimal escape sequences, wrapped in synchronized-output (`CSI ?2026`). A full re-layout is cheap (< 0.1 ms at 200×60). Diffing keeps bytes-on-wire tiny.

**Event loop:** `poll()` on stdin, the signal self-pipe (SIGWINCH/SIGCHLD) and every PTY master. Keyboard and mouse input renders immediately. PTY-only output is throttled to ≤ 1 frame / 8 ms. A lone ESC is disambiguated with a 25 ms timeout (not needed under the kitty protocol, where Esc = `CSI 27u`).

## 3. Decisions

- **D-1 Text storage = `Vec<String>` lines.** O(1) line access, cheap rendering, and edits are memmove of one line. That's fine up to ~10⁶ lines. A rope can come later if profiling demands it. Columns are **byte offsets**. Display columns are computed with tab expansion + Unicode width.
- **D-2 Undo = transactions of (Insert|Remove, pos, text)** with before/after selections. Consecutive typing or deleting at the cursor coalesces for 1.5 s, broken by whitespace or newline. Each committed txn gets a fresh id, and dirty = top-of-undo id ≠ saved id.
- **D-3 Keyboard tiers.** At startup we send `CSI ? u` + `CSI c`:
  1. Kitty reply → push flags=1 (disambiguate). Every chord is distinct.
  2. No kitty → enable xterm modifyOtherKeys=2. If the terminal honours it, keys arrive as `CSI 27;…~` and are distinct too.
  3. Legacy (Terminal.app): a control byte can't carry Shift. The parser flags such events `ambiguous`. The keymap **promotes** ambiguous Ctrl+L/R/Y/P/S to their Ctrl+Shift binding (tree / claude / terminal / palette / save-as) unless the focused terminal pane needs the raw key. Ctrl+Z stays undo, so **redo has extra bindings: Ctrl+Y, Alt+Shift+Z, F4, `:redo`** (Alt+Z itself became the word-wrap toggle in session 4, D-21). The status bar shows `⌨ legacy` and a one-time hint.
  - Cmd (super) chords are ignored (D-16). The promotion fallback applies on legacy terminals only.
- **D-4 Panel toggle = 3-state:** hidden → show+focus; shown+unfocused → focus; shown+focused → hide (focus returns to the editor). This gives VS Code's "toggle" and "focus" from one key.
- **D-5 Focus escape from terminal panes:** every key goes to the shell except the panel chords (only when unambiguous), F6 (cycle focus, VS Code's binding), and mouse clicks elsewhere. In a legacy terminal Ctrl+L/R/Y are left for the shell (clear, history search, yank).
- **D-6 Tree preview:** Up/Down in the focused tree opens the file under the cursor as a *preview tab* (italic, reused by the next preview), like VS Code. Enter/double-click pins it and focuses the editor. Editing a preview pins it.
- **D-7 Embedded terminal emulator is our own** (`cmx-vt`): DEC/xterm state machine, 256/truecolor SGR, alt screen, scroll regions, DECSET modes incl. mouse 1000/1002/1003/1006, bracketed paste, sync output 2026, OSC 0/2/10/11/52, DA/DSR/DECRQM/XTVERSION replies, 10k-line scrollback. `TERM=xterm-256color`, `COLORTERM=truecolor`, `TERM_PROGRAM=cmx`.
- **D-8 Claude pane:** `$CMX_CLAUDE` (override), then PATH, then `~/.local/bin`, `~/.claude/local`, `/opt/homebrew/bin`, `/usr/local/bin`. If missing, the pane shows install instructions. If it exits, Enter restarts it. Shift+Enter is sent as `ESC CR`, Claude Code's newline chord without kitty. A pane with no live process (missing CLI, exited) hands keys to the normal keymap, so Ctrl+Q etc. still work.
- **D-9 Clipboard:** an internal register, plus OSC 52 to the host terminal, plus a system tool if present (pbcopy/pbpaste; wl-copy/wl-paste; xclip; xsel). Paste prefers the system tool and falls back to the register. Host-terminal paste (Cmd+V) arrives as bracketed paste and is handled natively.
- **D-10 Colours:** truecolor when `COLORTERM` is truecolor/24bit and the terminal is not Apple Terminal (or `CMX_TRUECOLOR=1` forces it). Otherwise RGB is mapped to the nearest xterm-256 colour. Terminal-pane "default" colours map to theme colours so the UI is uniform on light host themes.
- **D-11 Regex search (session 4).** Our own engine, `regex.rs`: a Pike VM (Thompson NFA with thread priorities), so matching is leftmost-first like Perl, captures work, and time is linear in the line length (no catastrophic backtracking; `(a*)*b` on 5000 `a`s is instant). Syntax: literals, `.`, classes with ranges/negation/`\d \w \s` inside, `\d \D \w \W \s \S \b \B \t \n \xHH`, `^ $` (per line), groups, `(?:…)`, `(?<n>…)`/`(?P<n>…)` (numbered), `|`, `* + ? {n} {n,} {n,m}` with lazy `?`, leading `(?i)`. Matching is per line (a pattern can't span lines). `search.rs` wraps it in a `Query`: literal with smart case (default), regex, whole word. In the Find/Replace prompt **Alt+R** regex, **Alt+C** match case, **Alt+W** whole word; the label shows the count and options (`Find [3] [.* Aa]: `); an invalid pattern shows `(invalid regex)`. Replacements expand `$1 ${10} \1 $$`. Vim `/ ? n N :s` always use regex (`*`/`#` search `\bword\b`); `:s` also expands `&` and takes flags `g i I`.
- **D-12 No scroll margin** (VS Code's `cursorSurroundingLines: 0`). This keeps vim `zt`/`zb` exact.
- **D-13 Search highlight** is a separate flag from the query. Esc / closing Find / `:noh` hide the highlights but keep the query for F3 / `n`.
- **D-14 Synchronized output from panes** (`?2026`) holds rendering for at most 100 ms, after which the pane is drawn anyway. This stops a misbehaving app from freezing the UI or busy-waking the loop.
- **D-15 Testing strategy:** `crates/cmx/tests/e2e.rs` runs the real binary in a pty, with `cmx-vt` as the host terminal. The harness answers DA1 and, optionally, the kitty query, so both keyboard tiers are exercised. Assertions are on the emulated screen and on files written to disk.
- **D-16 Ctrl is the command modifier everywhere (user decision, session 3; reverses session 2).** Session 2 made Cmd the command key on macOS with a key bridge; the user found Cmd unworkable in practice and asked to drop it. Now: every IDE command uses Ctrl (Linux and macOS). **Cmd (super) chords are never cmx commands**: Cmd is left to macOS and the terminal app. Ctrl vs Ctrl+Shift stays distinct wherever the terminal can report it (D-3: kitty protocol or modifyOtherKeys), so Ctrl+Shift+Z redo, Ctrl+Shift+L/R/Y/P panels work as in VS Code. On terminals that can't (Terminal.app), the D-3 legacy fallbacks remain.
- **D-17, D-18, D-20 removed (session 3, user):** the macOS key bridge (CGEventTap), `cmx setup keys` (Accessibility prompt + Terminal.app profile) and `cmx keys` were deleted. The research behind them is kept in RESEARCH R2.2.
- **D-21 Soft wrap (session 4).** Per-view `wrap` flag, toggled with **Alt+Z** (palette: View: Toggle Word Wrap). Prose files (Markdown, LaTeX, stex, `.txt`) open with wrap on; code opens with it off. A line breaks after the last space that fits, or mid-word if a word is wider than the view (`view::wrap_starts`). Continuation rows have no line number. The viewport is `(scroll_y, scroll_sub)` = first line and its first visible wrap row. With wrap on, Up/Down/PageUp/PageDown move by screen rows (sticky column relative to the row), the wheel scrolls rows, and clicks map through the wrapped rows. Vim `j`/`k` keep moving by logical lines (vim's own `gj`/`gk` are not implemented).
- **D-22 Multiple cursors (session 4, default mode only).** `View.extra` holds secondary `(cursor, anchor)` pairs; the primary stays `cursor`/`anchor`. **Ctrl+D** selects the word, then adds the next occurrence (the new one becomes primary, as in VS Code); **Ctrl+F2** selects every occurrence; **Ctrl+Alt+↑/↓** add a cursor above/below; **Alt+click** adds a cursor (Alt+click on one removes it); **Esc** goes back to one cursor; a plain click too. Editing runs the single-cursor operation at each cursor from the last to the first (`Ed::each`), inside one outer transaction: `History` counts nested `begin`s so the whole multi-edit is **one undo step**. `Document.log` records each cursor's edits, and the cursors already processed are shifted through them (`view::map_pos`). Overlapping results merge. Typing, Backspace/Delete (and word variants), Enter, Tab, movement (with or without Shift), copy/cut/paste work at all cursors. Copy joins the selections with newlines; paste gives one line per cursor when the counts match, else the whole text at each. Other commands (line ops, undo, select all, page/doc moves) collapse to the primary cursor first. Vim mode clears extra cursors.
- **D-23 External changes (session 4).** Each document remembers `(mtime, length)` of its file from the last read/write. Every second (the poll loop wakes for it) and on terminal focus-in (`CSI I`), cmx compares: a **clean** buffer reloads as one undoable edit (Ctrl+Z brings back the old text; message "Reloaded X"); an **edited** buffer is not touched: the tab shows `⚠`, a warning says so, and Ctrl+S asks "Overwrite it? (y/n)" (palette: File: Reload from Disk discards the edits); a **deleted** file shows `(deleted)`; Ctrl+S writes it again. The explorer remembers the mtime of the root and every expanded folder (a folder's mtime changes when entries are created, deleted or renamed) and refreshes, keeping expansion and selection, when one changes. No inotify/kqueue: a handful of `stat` calls per second costs microseconds and works everywhere (network mounts included).
- **D-24 Splitter hover feedback (session 4).** Mouse mode `?1003` (any motion) is on, so cmx sees the pointer move. Over a splitter (explorer│editor, editor│Claude, between terminal splits, the panel header = height handle) the splitter is drawn bold in the focus blue (`┃`, or an underlined header) and cmx asks the terminal for a resize pointer with **OSC 22** (`ew-resize` / `ns-resize`, back to `text` when leaving; kitty, Ghostty, foot, WezTerm and xterm-compatible terminals that support it show it; others ignore the sequence, the highlight still works). The highlight stays while dragging. Motion events go to panes only when their app asked for any-motion tracking.
- **D-25 Find box prefill.** Opening Find/Replace with a selection prefills it and shows the count without moving; the first typed character replaces the prefill (VS Code).
- **D-26 stex highlighting = the compiler's lexer (session 5, user).** The flat keyword list is gone. The stex highlighter calls `cmx_stex::highlight::line_roles` (STEX-DESIGN §13), so what is colored is exactly what the compiler reads as a keyword:
  - A phrase is colored only when complete: "such" stays plain until "that", "there" until "is"/"exists".
  - Every alias of a construct gets the same color: such that, s.t., st, st., w/ and with; exists, there is and for all.
  - "is" alone is prose.

  Colors: structure keywords blue, labels blue bold, quantifiers/binders purple, logic teal, reasons orange, math words light blue, `$…$` yellow.
- **D-27 Ctrl+Shift on terminals that can't send it (session 5, user: "for all terminals").**
  - The problem: Terminal.app (and some Linux terminals) send Ctrl+Shift+Z as Ctrl+Z.
  - The fix: cmx runs on the machine with the keyboard, so when such an ambiguous Ctrl+letter arrives it asks the OS whether Shift is held at that moment (`cmx_sys::modkeys`).
    - macOS: `CGEventSourceFlagsState`. This needs the **Input Monitoring** permission for the terminal app. Without it macOS reports "no modifiers", so cmx trusts the query only when `CGPreflightListenEventAccess()` is true and otherwise keeps the legacy fallback (D-3).
      - In Terminal.app cmx asks macOS for the prompt once (`CGRequestListenEventAccess`) and says so in the status bar. The palette command **Keyboard: Detect Ctrl+Shift in This Terminal** asks again and opens System Settings › Privacy & Security › Input Monitoring.
      - After allowing it, Terminal must be quit and reopened.
      - Session-5 regression, fixed: without the permission the probe reported "Shift up" for every key, which also disabled the Ctrl+L/R/Y panel fallback. e2e `legacy_panels_work_with_probe_enabled` guards it.
    - Linux: `XQueryKeymap` through `dlopen("libX11.so.6")` when `$DISPLAY` is reachable. Verified under Xvfb with xdotool holding Shift.
  - The event then becomes a real Ctrl+Shift chord (or a certain Ctrl chord), so the legacy promotion (D-3) is only the fallback.
  - The status bar shows `⌨ legacy+⇧`.
  - Off over SSH (the keyboard is elsewhere), on native Wayland (no global key state), and with `CMX_MODPROBE=0`; the e2e harness sets that, because it simulates the terminal.
- **D-30 Terminal.app: Ctrl+Shift through a listen-only event tap (session 5, after the user's "Ctrl+Shift still doesn't work").** Byte-level tests showed that Terminal.app sends **nothing** for Ctrl+Shift+letter (Ctrl+letter arrives as the control byte). So the Shift probe (D-27) can't help there.
  - **How it works:** in Terminal.app, with Input Monitoring granted, cmx starts a listen-only Quartz event tap (`cmx_sys::keytap`) on a background run loop.
    - It sees Ctrl+Shift+letter key-downs aimed at the Terminal.app process cmx runs under (an ancestor process; not under tmux/ssh). The character comes from the event, so the keyboard layout is respected.
    - It writes each chord to a pipe; the main loop turns it into a real Ctrl+Shift chord, only while cmx's tab has focus (Terminal.app supports focus reporting `?1004`, verified).
    - Plain control bytes are then plain Ctrl (no promotion).
  - **Fallback:** without the permission, the D-3 promotion stays (Ctrl+L/R/Y/P open panels).
  - **Verified** in the user's Terminal.app with posted key events: Ctrl+Shift+Z → Redo, Ctrl+Shift+L → explorer.
  - **Diagnostics:** `CMX_KEYLOG=/path` logs every key with what the probe/tap decided (`tap=… silent=…`).
  - **No beep (user):** Terminal.app beeps at a key it can't use.
    - When Terminal also has **Accessibility**, the tap is an active one (`kCGEventTapOptionDefault`) and *consumes* the chords it reports: only Ctrl+Shift+letter, only aimed at cmx's Terminal process, only while cmx's tab has focus (the focus state is shared with the tap thread). Terminal never sees them, so it doesn't beep. Other keys, apps, tabs and unfocused moments pass through untouched.
    - Without Accessibility it listens only (chords work, Terminal beeps) and a one-time hint says how to silence it.
    - Verified in the user's Terminal.app: the tap starts in consuming mode (`silent=true`) and the chords still reach cmx. The beep itself can't be heard from here.
- **D-28 Resize cursor in every terminal (session 5).** Terminal.app (like most terminals) has no escape sequence for the mouse-pointer shape. So besides OSC 22 (D-24), cmx draws a resize marker in the cell under the mouse while it is over (or dragging) a splitter: `↔` on vertical borders, `↕` on the panel's height handle, in the focus color, on top of the highlighted border.
- **D-29 Ctrl+arrows and Ctrl+Delete (session 5, user).** Ctrl stands in for macOS Cmd, so these follow macOS:
  - Ctrl+←/→ go to the line start (smart home) and end.
  - Ctrl+↑/↓ go to the document start and end; Shift selects.
  - Ctrl+Backspace deletes to the line start, Ctrl+Delete to the line end.
  - Word motion and deletion moved to Alt/Option (+Shift).
  - The old Ctrl+↑/↓ scroll bindings are gone (the wheel scrolls).
  - Note: macOS Mission Control uses ⌃←/→/↑/↓ by default; turn those shortcuts off (System Settings → Keyboard → Keyboard Shortcuts → Mission Control) for Terminal.app to pass them to cmx.
- **D-19 VS Code hint (user decision).** With `TERM_PROGRAM=vscode` and no kitty reply, the startup message suggests enabling VS Code's kitty keyboard protocol setting (`terminal.integrated.enableKittyKeyboardProtocol`, VS Code ≥ 1.109, name `[unverified]`) and `terminal.integrated.sendKeybindingsToShell`. The same tip is in F1 help.

## 4. Layout

```
┌──────────┬───────────────────────────────┬────────────────┐
│ EXPLORER │ tab │ tab● │ tab ×            │ CLAUDE       × │
│ ▾ src    ├───────────────────────────────┤ (claude PTY)   │
│   main.rs│  1 │ fn main() {              │                │
│   lib.rs │  2 │     ...                  │                │
│          ├ TERMINAL  1: zsh  2: zsh  + ⫽ ×                │
│          │ $ cargo build      │ $        │                │
└──────────┴────────────────────┴──────────┴────────────────┘
 EDIT  main.rs ●                      Ln 2, Col 5  Rust  LF  UTF-8
```
- Splitters: tree│centre and centre│claude are 1-column `│` borders. Editor/terminal uses the panel header row as the drag handle. Terminal splits are separated by `│`. Everything is draggable with the mouse. Minimum sizes are enforced.
- Status bar on the last row. Prompts (save-as, find, `:`) replace it while active.

## 5. Keybindings (default mode)

Ctrl is the command modifier on every platform; Cmd is left to macOS (D-16).

| Action | Keys | Legacy fallback |
|---|---|---|
| Undo / Redo | Ctrl+Z / Ctrl+Shift+Z, Ctrl+Y | redo: Alt+Shift+Z, F4 |
| Quit | Ctrl+Q (Ctrl+Shift+Q also works inside terminal panes) | |
| Copy / Cut / Paste / Select all | Ctrl+C / X / V / A | (copy/cut with no selection = whole line) |
| Save / Save as | Ctrl+S / Ctrl+Shift+S | |
| New / Close tab / Quit | Ctrl+N / Ctrl+W / Ctrl+Q | |
| Quick open / Command palette | Ctrl+P / Ctrl+Shift+P | Ctrl+P then `>` |
| Find / Replace / Next / Prev | Ctrl+F / Ctrl+H / F3, Enter / Shift+F3, Shift+Enter | Ctrl+H is Backspace in some terminals, so use palette |
| Go to line | Ctrl+G | |
| Explorer / Claude / Terminal | Ctrl+Shift+L / R / Y (+ Ctrl+B, Ctrl+` aliases) | Ctrl+L / R / Y |
| Split terminal | Ctrl+Shift+5, header ⫽ button, palette | |
| Cycle focus | F6 | |
| Vim mode toggle | Ctrl+K | |
| Line / document ends | Ctrl+←/→ line start/end, Ctrl+↑/↓ document start/end (+Shift selects) | Home/End, Ctrl+Home/End |
| Delete to line start / end | Ctrl+Backspace / Ctrl+Delete | |
| Word move / delete | Alt+←/→ (Mac Option), Alt+Backspace/Delete | ESC b / ESC f |
| Line ops | Alt+↑/↓ move, Alt+Shift+↑/↓ copy, Ctrl+Shift+K delete, Ctrl+Enter / Ctrl+Shift+Enter insert below/above, Ctrl+L select line, Ctrl+/ comment, Ctrl+D select next occurrence, Tab/Shift+Tab indent | |
| Tabs | Ctrl+PgUp/PgDn, Alt+1..9 | |
| Multiple cursors | Ctrl+D next occurrence, Ctrl+F2 all occurrences, Ctrl+Alt+↑/↓, Alt+click, Esc = one cursor | |
| Find options (in the prompt) | Alt+R regex, Alt+C match case, Alt+W whole word | |
| Word wrap | Alt+Z | palette |
| Help | F1 | |

Mouse: click places the cursor. Drag selects. Double-click selects a word, triple-click a line. Ctrl+click (or Shift+click) extends from the cursor. Wheel scrolls. Clicking the gutter selects a line. The scrollbar can be clicked and dragged. Tabs: click switches, middle-click or `×` closes, wheel cycles. Tree: click opens, double-click pins, click on a folder toggles it.

## 6. Vim mode (Ctrl+K)

Normal / Insert / Visual / Visual-Line. Counts, operators `d c y > < ~`, `dd cc yy >> <<`. Motions `h j k l w W b B e E 0 ^ $ gg G f F t T ; , % { } H M L`, arrows, Home/End. Text objects `iw aw iW aW i" a" i' a' i` a` i( a( ib i{ a{ iB i[ a[ i< a<`. Commands `i a I A o O x X s S r~ D C Y p P J u Ctrl+R . v V n N * # zz zt zb Ctrl+D/U/F/B/E/Y`. Ex: `:w [f] :q :q! :wq :x :e f :N :s/a/b/[g] :%s/a/b/[g] :noh :tree :term :claude :vsplit-term :help :bn :bp :redo`. Yank syncs to the system clipboard. Deletes stay in the internal register.

## 7. Terminal pane behaviour

- Keys are encoded per xterm (DECCKM aware). Mouse is forwarded when the app requested it (SGR or X10), otherwise drag selects, Ctrl+C copies when a selection exists, and the wheel scrolls back (or sends arrows in the alt screen).
- Shift+PgUp/PgDn scroll back. Any keypress snaps to the bottom.
- When the shell exits, the pane closes. The claude pane instead offers a restart.
- Resize: no reflow (like xterm). Shrinking pushes rows to scrollback to keep the cursor visible, and growing pulls them back.

## 8. Open questions / future

- LSP, git gutter, persistent sessions; vim `gj`/`gk` and multi-cursor in vim mode; multi-line regex.
- Terminal reflow on resize. Kitty keyboard support *inside* panes (we answer as legacy).

## 9. Checklist

- [x] cmx-sys: raw mode, winsize, poll, signals self-pipe, pty spawn/resize/reap (unit-tested)
- [x] cmx-tui: width tables, cell buffer, diff renderer (truecolor / 256), input parser (legacy, SS3, CSI ~, modifyOtherKeys, kitty CSI u, SGR mouse, X10 mouse, paste, focus, DA/kitty replies), clipboard
- [x] cmx-vt: parser + grid + scrollback + alt screen + modes + replies + key/mouse/paste encoding (14 unit tests; Claude Code observed rendering correctly)
- [x] Text buffer, grapheme-aware movement, word movement, display columns
- [x] Undo/redo with coalescing, dirty tracking
- [x] Tabs (open/switch/close/preview/dirty marker), untitled buffers, save / save-as prompt, quit confirm, `:wq` on untitled
- [x] Selection: shift-arrows, mouse drag, double/triple click, Ctrl/Shift+click extend
- [x] Clipboard copy/cut/paste (whole line when nothing selected), bracketed paste
- [x] Find (incremental, match count) / interactive replace (y/n/a/q) / goto line[:col] / select next occurrence
- [x] Line ops: move/copy/delete/insert lines, indent/outdent, toggle comment, auto-indent, bracket auto-close/surround
- [x] Syntax highlighting (table-driven, ~28 languages) incl. markdown, LaTeX, stex, Lean
- [x] Explorer tree: lazy load, keyboard nav with preview, mouse, new file/folder, rename, delete, refresh, hidden toggle
- [x] Terminal panel: multiple split terminals, resize, scrollback, selection/copy, mouse forwarding, alt-screen wheel → arrows
- [x] Claude pane with restart, `$CMX_CLAUDE` override and missing-binary message
- [x] Draggable splitters (tree, claude, panel height, terminal splits); tree drag is e2e-tested
- [x] Quick open (fuzzy) + command palette (`>`) + `:line` + help overlay (F1)
- [x] Vim mode (see §6), incl. dot-repeat, text objects, visual modes, ex commands
- [x] Status bar, messages, keyboard-tier indicator (⌨ kitty / xterm / legacy)
- [x] End-to-end pty tests (`crates/cmx/tests/e2e.rs`, 16 tests + ignored screenshot/latency test)
- [x] Soft wrap (D-21; unit `soft_wrap_segments_and_movement`, e2e `soft_wrap_long_lines`)
- [x] Regex search + replace with groups (D-11; `regex.rs` 6 tests, `search.rs` 2, e2e `regex_find_and_replace_with_groups`)
- [x] Multiple cursors (D-22; unit `multi_cursor_*`, `next_and_all_occurrences_paste_and_copy`, `map_pos_through_edits`; e2e `multiple_cursors`)
- [x] External file change detection: reload / ⚠ conflict / deleted / explorer refresh (D-23; e2e `files_changed_on_disk`, unit `reload_is_undoable`)
- [x] Splitter hover highlight + OSC 22 resize pointer (D-24; e2e `splitter_hover_feedback`)
- [x] Resize marker drawn under the mouse for terminals without OSC 22 (D-28)
- [x] stex highlighting from the stex lexer (D-26; e2e `stex_highlighting_follows_the_compiler`)
- [x] Ctrl+Shift via the OS modifier state on legacy terminals (D-27; macOS needs the user's confirmation in Terminal.app; Linux X11 verified under Xvfb)
- [x] Ctrl+arrows / Ctrl+Backspace / Ctrl+Delete (D-29; e2e `ctrl_arrows_and_ctrl_delete`)
- [x] Ctrl-only command keys, Cmd ignored (D-16; e2e `kitty_ctrl_shift_z_redo_and_cmd_ignored`)
- [x] VS Code hint (D-19)
- [x] Palette: "stex: Compile" / "stex: Check" run in the terminal panel
- [x] Verify on Linux: Ubuntu arm64 VM (Lima `cmx-linux`), full `cargo test --workspace` green incl. 22 pty e2e tests (session 4)
- [ ] Manual check in real terminals: Terminal.app, iTerm2, Ghostty/kitty, VS Code
