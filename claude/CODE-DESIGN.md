# CODE-DESIGN: `cmx code` (deterministic partial-NL → code)

> **TEMPORARY FILE.** Part of the `claude/` tracking system. Remove only when the user says so.
> Status: **BRAINSTORM, not fleshed out** (spec: "more on this later").

## Spec recap (prompt.txt 898–906)
- One shared syntax that emits Python, C++, Java, C, Rust and assembly.
- A large built-in bank of data structures and algorithms (generic + specific, e.g. binary search).

## Early thoughts
- Reuse the stex architecture: lexer → construct table → IR → per-language emitters. Deterministic.
- IR = a small typed imperative language (let, if, while, for-range, fn, struct, call, return) + "library call" nodes that expand to per-language snippets from the data-structure bank.
- Assembly target: pick one (x86-64 SysV or AArch64) and emit through a tiny register allocator. Probably a later phase.
