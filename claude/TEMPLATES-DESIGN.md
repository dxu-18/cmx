# TEMPLATES-DESIGN: template bank for Claude/LLM tasks

> **TEMPORARY FILE.** Part of the `claude/` tracking system. Remove only when the user says so.
> Status: **BRAINSTORM, not fleshed out** (spec: "more on this later").

## Early thoughts
- `cmx prompt list|show|new|use NAME [--var k=v]...` renders a parameterized prompt to stdout, the clipboard, or straight into the editor's Claude pane.
- Storage: plain-text files with front-matter variables under `~/.config/cmx/templates/` + built-ins compiled into the binary (`include_str!`).
- Editor integration: a command-palette entry "Send template to Claude pane".
