# WEBSITE-DESIGN: personal website on GitHub Pages

> **TEMPORARY FILE.** Part of the `claude/` tracking system. Remove only when the user says so.
> Status: **BRAINSTORM, not fleshed out.**

## Spec recap (prompt.txt 912–918)
1. Instantiate the git repo for a GitHub Pages site (`<user>.github.io`).
2. Automatic hosting of the resume and similar files (links to SCAFFOLD-DESIGN resume pipeline: build PDF → commit → push).
3. A library containing "all the 14 github website templates" directly (vendored; need to identify which 14 the user means: open question Q-W1).

## Early thoughts
- `cmx site init [TEMPLATE]`, `cmx site publish`, `cmx site add-resume`.
- Uses the `git` + `gh` system tools (allowed per CORE §3.3).
