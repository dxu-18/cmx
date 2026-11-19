# SCAFFOLD-DESIGN: `cmx new repo|homework|resume`, `cmx resume`, `cmx link --github`

> **TEMPORARY FILE.** Part of the `claude/` tracking system. Remove only when the user says so.
> Status: **BRAINSTORM, not fleshed out.**

## Spec recap (prompt.txt 886–896)
- `cmx link EMAIL --github`: link GitHub (probably via `gh auth` or a PAT stored in the keychain).
- `cmx new homework`: CMU hw template (https://github.com/jez/cmu-latex-hw-template). Could integrate with stex (`hw.stex` → `hw.tex`).
- `cmx new resume`: Jake's resume (https://github.com/jakegut/resume).
- `cmx new repo`: TBD by the user.
- `cmx resume`: edit the resume/CV hosted on the user's website (ties into WEBSITE-DESIGN).

## Early thoughts
- Vendor both templates under `vendor/templates/{cmu-hw,jake-resume}/` with LICENSE + ORIGIN (CORE §3.2). Check the licenses first.
- `cmx new X [NAME]` copies the template, substitutes `{{name}}`/`{{date}}`/`{{course}}` placeholders, and runs `git init` for repos.
- Resume as data: keep `resume.toml` → render into Jake's template. `cmx resume` opens the data in `cmx edit` and recompiles on save.
