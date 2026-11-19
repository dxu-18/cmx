# STEX-DESIGN: `.stex` → LaTeX (+ Lean check)

> **TEMPORARY FILE.** Part of the `claude/` tracking system. Remove only when the user says so.
> Status: **active (session 4)**. User answers to Q-S1…Q-S7 are folded in (§8). §6 is the checklist; §10–§11 describe the session-4 proof constructs and the Lean verification.
> The full feature spec is in `prompt.txt` lines 10–836. This doc turns it into an architecture plus decisions.

## 1. Commands

| Command | Behaviour |
|---|---|
| `cmx stex new` | Opens the editor on an untitled buffer in stex mode. Save-as asks for a name and appends `.stex` when there is no extension. ✅ |
| `cmx stex new NAME` | Creates a blank `NAME.stex` (appends the extension; refuses to overwrite). ✅ |
| `cmx stex check F.stex` | Compiles to `F.lean` in the **current directory** and runs `lean` on it. Reports goals that fail, mapped back to stex lines. |
| `cmx stex compile F.stex [variance] [--open] [engine] [--seed N]` | Writes `F.tex` **next to `F.stex`** and runs the engine (default pdflatex) → `F.pdf`. Later runs overwrite the same names. `--open` opens the PDF in the **default web browser** (macOS: LaunchServices https handler via `open -b`; Linux: `$BROWSER`, then `xdg-settings` + `gtk-launch`, then `xdg-open`). Variance: `--natural` (**default**, user decision), `--fixed`, `--varied`, `--random`. A missing engine still writes `F.tex` and prints install hints (exit 4). |

Default variance is **`--natural`** (user, session 2).

**Engines (session 3, user):** `compile` runs the **bundled TinyTeX** (`vendor/tinytex/`, TinyTeX-1 + needspace, mathrsfs/rsfs, enumitem, pgf/TikZ), unpacked on first use next to the archive (~3 s, ~250 MB), so no MacTeX/TeX Live install is needed. `--pdflatex`, `--xelatex` and `--lualatex` all work with it. `CMX_TEX=system` uses an installed TeX on PATH; `CMX_TEX=/some/bin` uses that directory. The preamble always loads `\usepackage[T1]{fontenc}` + `lmodern` (Type 1 fonts; TinyTeX ships no METAFONT sources for the EC/TS1 bitmap fonts, see RESEARCH R5).

Output locations: `F.tex`/`F.pdf` next to `F.stex`; `F.lean` in the current directory (spec). Exit codes: 0 ok, 1 stex/engine/Lean error, 2 usage, 4 tool missing (output file still written).

## 2. Hard requirements

- **Determinism (user, session 3):** `--fixed` and `--natural` are deterministic: same input + flags → byte-identical `.tex`. The PRNG is reseeded **per sentence** from that sentence's own text, so editing one sentence doesn't reshuffle the phrasing of the others. `--varied` and `--random` use a fresh pseudo-random stream each run (time + pid seeded), so every compile reads differently; `--seed N` makes them reproducible.
- No ML or LLM. Classification uses **hand-tuned likelihood scores** (spec feature 1: "largely-deterministic simple probabilities").

## 3. Pipeline

```
source ─► Lexer ─► Segmenter ─► Parser ─► IR (proof AST) ─┬─► Realizer(variance) ─► LaTeX emitter ─► Layout pass ─► .tex
                    (prose vs math)                        └─► Lean emitter ─► .lean ─► lean ─► diagnostics
```

1. **Lexer**: tokens: words, numbers, symbols (`= == != <= >= -> => <-> && || ! ^^ !&& !|| % / * ^ _ ( ) [ ] { } | , . ; :`), sigils (`*A &A ~A #A @A _word __word -keyword`), explicit math spans `$…$` (**single dollars only**: `$$` is a lex error, user decision), `latex{…}`, style blocks (`bold{…} bf{…} it{…} ul{…}`). **Comments: `//` to end of line** (user-confirmed Q-S7). Each token keeps line/column and whether it is glued to its neighbours (no whitespace). Gluing drives `2x`, `f(x)`, `5%`, `!x`, `n!`.
   - **Explicit `$…$` output rule** (user, session 2; spec "$$" section): a `$…$` inside a sentence stays **inline `$…$`**. A `$…$` that is a whole statement (its own line or block) becomes **display math**, emitted as `\[ … \]` (LaTeX's spelling of `$$…$$`), or as **`\begin{align*}`** when it is a chain (≥ 2 top-level relation/implication steps) that has `by` tags or won't fit on one line (est. > 60 chars or written over several lines). `align*` + `\tag*{…}` avoids stray equation numbers; the spec example's `align` renders identically because every line carries `\tag*`.
2. **Segmenter**: decides prose vs math per token run. Each token gets a math-likelihood score from features:
   - +strong: operators/relations (`= < > in subset`), digits, single letters (`x`, `n`), Greek names, known math words (`sum integral sqrt choose mod`), `^ _`, function-call shape `f(x)`.
   - −strong: dictionary English words that aren't math keywords, stex structural keywords (`let assume wts …`), punctuation that closes a sentence.
   - context: neighbours (a single letter between two math tokens is math, while "a" before a noun is prose).
   - A run of tokens whose summed score is > 0 becomes one math span. Ties use fixed rules from a table, so the result is still deterministic.
   - Single-letter English words (`a`, `I`) are handled by rule: `a` is prose when followed by a prose noun, and math when adjacent to an operator.
3. **Parser**: recognizes **constructs** from a pattern table. Each construct has many surface syntaxes, compiled into a trie of token patterns:
   - declarations: `let X [= | in | be] …` (comma lists turn into "let …, let …, and let …"), `fix`, `assume`, `afsoc`
   - goals: `wts`, `claim`, `qed [custom]`
   - quantifiers: `exists`/`there exists`/`there is`/… + `such that` variants (`s.t.`, `st`, `st.`, `s.t`, `such that`, `with`, `where`, `,`); `for all`/`for every`/`for each`/`for any`/`∀`
   - logic: `then implies iff and or not xor nand nor`, symbolic `&& || ! ^^ !&& !||`
   - math structures: `sum/prod from i=1 to n [of] f`, `integral[ from] a to b [of] f dx` / `integrate f from a to b`, `limit as x to a of f`, `derivative of f [with respect to|wrt] x`, `n choose k`, `sqrt`, `Nrt`, fractions (`/`, `over`, PEMDAS + parens), `mod`, `%`
   - justification: `by THEOREM`, `by assumption/definition/hypothesis`, `b/c|because|since …`. Inside `$…$`, `by WORD` becomes `\tag*{WORD}` in an align
   - structure: `proof claim lemma proposition theorem corollary observation remark fact sublemma aside`, `case N cond:`, `i.e.`
   - formatting: `bold/italic/underline` word or `{…}`, `_word`, `__word`, `-keyword` escape, `latex{…}`
   - sets (spec addition, session 1): a bare `{…}` is **a set** → `\{…\}`. Braces right after a style keyword (`bold`, `bf`, `italic`, `it`, `underline`, `ul`, `latex`) are style or raw blocks instead. The lexer decides by looking at the previous token.
   - definitions (spec addition): `define … via …` and `… := …` define a function, set or relation. The compiler **records the definition in a symbol table** (name, kind, arity, domain/codomain when given). Later uses resolve against it (e.g. `f(x)` is known to be math; Lean gets a `def`).
   - plain operators (spec addition): `+ - +- -+ ( ) [ ] > < >= <=` behave as normal arithmetic, grouping and comparison (`+-` → `\pm`, `-+` → `\mp`, `>=` → `\geq`, `<=` → `\leq`).
4. **IR**: a proof is a list of `Block`s (Env(kind, title, body) | Paragraph(Sentence*) | Cases | Align). A `Sentence` is a sequence of `Clause`s, each tagged with a **rhetorical role** (Declare, Assume, Goal, Conclude, Justify, Contradiction, Quantify, Restate, …) plus its math payload as a small math AST (`MExpr`).
5. **Realizer**: picks surface prose per role from the **phrase bank** (§5) using the variance policy, then capitalizes, punctuates and adds connectives ("and" before the last item of a list, Oxford comma configurable).
6. **LaTeX emitter**: MExpr → LaTeX (`\in \mathbb{Z} \binom \frac \sqrt[n]{} \pmod{} \bmod \cdot \times \sum_{}^{} \int_{}^{} \,dx`). It records which packages each construct needs (§7).
7. **Layout pass** (spec feature 8): page-fit heuristics. Estimates line and page usage from character counts and display math. Chooses between `\vspace` tweaks, `\allowdisplaybreaks`, the `changepage`/`fullpage` geometry, and keeping short sections together with `\needspace`. Separate cases for many short sections vs one long section spanning pages.
8. **Lean emitter**: maps IR to Lean 4 + Mathlib. `let x in Z` → `(x : ℤ)`; `assume P` → hypothesis; `wts P` → goal; `by CRT` → `Nat.chineseRemainder`-based `have`; arithmetic steps → `by omega` / `by ring` / `by simp` / `by decide`, chosen per relation shape. Prose `because …` is ignored (spec).

## 4. Theorem alias library (spec feature 11)

- Data file `crates/cmx-stex/data/theorems.tsv`: `canonical_name \t lean_ident \t aliases(|-separated) \t latex_name`.
  e.g. `chinese remainder theorem \t Nat.chineseRemainder \t crt|chinese remainder theorem|chinese remainder thm|sunzi \t the Chinese Remainder Theorem`.
- Lookup structure: normalize (lowercase, strip "the", punctuation, "thm"→"theorem"), then build a **compile-time perfect hash** (or a sorted array with binary search) of alias → id. Matching is greedy longest-match over the token stream after `by`.
- "All ~200k Lean theorems": Q-S1 below. The alias table only needs the **named** theorems people cite in prose (a few thousand). The full Mathlib name index can be a separate generated file (name list only, ~10 MB), produced by a script against a local Mathlib.

## 5. Phrase bank + variance

- Bank = `role → [(phrase_template, weight)]`, generated from the lists in `prompt.txt` (Setting up, Stating the goal, Implication, …). Templates use slots: `{P}`, `{Q}`, `{x}`.
- Every role has **many** phrasings (user, Q-S4). Example `contradiction`: "This is a contradiction.", "This contradicts our assumption that P." (P = the active `afsoc` hypothesis), "which is absurd", …. The variance flags pick among them.
- `--fixed`: always the top phrase (≈95% predictable; spec).
- `--natural`: sample by **corpus frequency weights** measured on ~19k real proofs (RESEARCH R6). `data/phrases.txt` is **generated** by `tools/stex-corpus weights` from `tools/stex-corpus/phrase-map.txt` (template → corpus evidence) and `frequency-table.tsv`. Weight = corpus rate × simplicity (−20% per extra word), the `--fixed` template kept at ≥ 50% of its role's top weight (user: simpler wordings stay high), scaled to 100, floor 1. Spec ties are kept: for `exists`, "There exists $x$ such that $P(x)$" and "We have $P(x)$ for some $x$" get equal weight; same for `for every`. 269 templates in 51 roles (session 3, up from ~130).
- Embedded quantifiers (after "We show that …", inside `then …`) use `exists.inner` / `forall.inner`, which exclude sentence-initial forms like "we have P for some x" (fixes "We will prove that we have …").
- `--varied`: flattened weights (weight^0.3) + no immediate repeats of the same phrase within 3 sentences.
- `--random`: uniform over the bank (seeded).
- PRNG: SplitMix64/xoshiro, hand-written. Seed = FNV-1a(path ‖ content) unless `--seed`.

## 6. Implementation order (checklist)

- [x] `cmx-stex` crate skeleton + CLI wiring for check/compile flags (default variance natural, `--seed`)
- [x] Lexer + golden tests from every example in prompt.txt (`src/tests.rs`, 31 unit tests; `crates/cmx/tests/stex_cli.rs`, 4 CLI tests)
- [x] Math expression parser (PEMDAS, fractions, choose, roots, mod/pmod, %, sets/set-builder, abs/norm, sums/products, integrals, limits, derivatives, logic in `$…$`, negated relations)
- [x] Segmenter with plausibility rules (articles a/A/I, lone integers, glued products, `p, q in Z` lists) (features 1, 6)
- [x] Construct table + parser (let/fix, assume, afsoc, contradiction, wts, qed [P], then/thus/…, define…via, `:=`, by THM / P by THM, because/since, i.e., note, wlog, case N, labels, cases-close)
- [x] Realizer + phrase bank (`data/phrases.txt`, ~60 roles) + variance flags (fixed/natural/varied/random, seeded)
- [x] Emitter + package detection (feature 9) + document skeleton
- [x] Sentence boundary/capitalization/punctuation (feature 10)
- [x] Engine invocation (`-interaction=nonstopmode -halt-on-error -file-line-error`, aux/log cleaned on success), `--open` (default browser), idempotent output names
- [x] Layout pass (feature 8): margins 0.75in for ~1–1.12 pages, multi-page → `\raggedbottom` + widow/club penalties, many short sections → `needspace`, long single section with aligns → `\allowdisplaybreaks`
- [x] Word→symbol library (feature 12): `data/symbols.tsv`, **2005 entries**, generated by `tools/stex-corpus symbols` from `symbol-base.txt`: every amsmath/amssymb symbol command by name, English relation phrases with "is …"/"is not …"/"isn't …"/"not …" forms (negated latex), Unicode math characters (`≤ ∈ ∀ ℝ 𝔸 𝒜 𝔄`), Greek incl. "capital gamma", `bbR`/`calF`/`scrA`/`frakg`, ~140 operator names, convergence arrows, accents (`x hat`). Kinds: greek const set func funcp rel arrow bin add setop div post accent big. `funcp` = English-word operator names (`rank`, `degree`, `trace`, …) that are math only as `rank(A)`, so "a polynomial of degree n" stays prose. Words with common non-math meanings were excluded from rel/bin/arrow (e.g. "p prime" is not `p'`).
- [x] Theorem alias table (`data/theorems.tsv`, ~55 theorems with aliases, Lean names, statements) + Lean emitter + `stex check` (writes `F.lean` in cwd, maps errors to stex lines, prints the install hint when Lean/Mathlib are missing)
- [x] Editor integration: palette "stex: Compile" / "stex: Check" run the command in the terminal panel (saving first)
- [x] LaTeX verified end to end with the bundled TinyTeX (pdflatex, xelatex, lualatex): a document using every construct compiles with no warnings; CLI tests build real PDFs
- [x] **Lean verified for real** (session 4): Lean 4 + Mathlib installed (RESEARCH R7). A 52-file corpus (`crates/cmx-stex/tests/lean/`, 44 correct proofs + 8 deliberately wrong ones with the expected failing lines) runs through `cmx stex check` with `crates/cmx/tests/stex_lean.rs` (ignored by default) and matches 52/52 (§11). Every Mathlib name in `theorems.tsv` resolves (`#check`), after fixing AM–GM and Cauchy–Schwarz
- [x] `--natural` weights from a real corpus (RESEARCH R6)
- [x] Lean case analysis: `case N cond` blocks → `rcases (show c1 ∨ c2 by stex_auto) with c1 | c2` with one bullet per case; the cases prove the following "in either case P" (as `have … := by rcases …`) or, if none, the goal. `qed` ends an open case list
- [x] Lean witnesses and existentials: `choose/pick/take x = v` → `obtain ⟨x, w⟩ : ∃ x, x = v := ⟨v, rfl⟩` (+ `refine ⟨x, ?_⟩` when the goal is `∃ x, …`); a derived `∃ k, P` → `obtain ⟨k, s⟩`; such variables are no longer theorem binders. Implication chains are parenthesized (`(a → b) ∧ (b → c)`)
- [x] A sentence ending in a connective ("…, so", "we have:") flows into a following `$…$` display (no stray period)
- [ ] Mathlib name index (all ~200k names) as a generated data file, built against a local Mathlib (`cmx stex index`?), for alias lookup beyond the curated table
- [x] Session-3 gaps closed: "x is (strictly) between a and b" / "lies between" → `a ≤ x ≤ b`; ordinals (`nth`, `n-th`, `(k+1)st`, `2nd`); `afsoc` → `by_contra hcN` + the user's assumption derived by name; "suppose not / assume the contrary" negate the goal the way it is written (`x < 2` → "$x \geq 2$", even ↔ odd, De Morgan, ∀ ↔ ∃; what the user writes with "not" stays literal); nested cases (`case 2a`, `subcase`) render as nested lists and nest in Lean; casing ("THEN X = 5" → "Then $X = 5$")
- [x] Induction (§10): `we proceed by [strong] induction on n`, `base case n = b: …`, `inductive step: assume P(k) …`, with prose "\textit{Base case} ($n = 0$)." / "\textit{Inductive step.}" and Lean `induction n with | zero | succ k ih`, `Nat.le_induction` for a base b > 0 (`∀ n ≥ b`), `Nat.strong_induction_on`
- [x] Counting procedures (§10): `step N: …` lists ("Step 1: Choose the chair in 10 ways."), "in total / altogether / overall / all in all, there are … ways"; counts in prose (`there are C(5,2) = 10 ways`) are Lean obligations; 13 counting theorems in `theorems.tsv` (multiplication/addition principle, inclusion–exclusion, complement, bijection, Pascal, Vandermonde, hockey stick, subsets, binomial formula, stars and bars, double counting, permutations)
- [x] Typed declarations ("let p be a prime", "n be an integer", "x, y be real numbers") and predicates (even, odd, prime, positive, …, rational, irrational, perfect square) as Lean facts
- [x] Juxtaposed variables: `a^2 + ab + ba` stays one math run; in Lean `ab` with `a`, `b` in scope is `a * b`
- [x] "P, so Q, thus R": each part is its own Lean obligation
- [ ] Lean automation limits (honest failures, see §11): facts needing a lemma the tactics don't find (e.g. `2^n = 2·2^(n-1)` outside a strong induction), ℕ subtraction outside sums, real analysis (limits, integrals stay comments)

## 7. Package detection (feature 9)

Each emitter construct carries a bitset of package needs. The final preamble is the union: amsmath (align, `\binom`, `\text`), amssymb (`\mathbb`, `\varnothing`), amsthm (theorem envs/proof), mathrsfs (`\mathscr`), enumitem (cases lists), fullpage/changepage (layout pass), graphicx/tikz (only when `latex{}` blocks use them, detected by scanning for `\includegraphics`/`\begin{tikzpicture}`).

## 8. Resolved questions (user answers, session 2)

- **Q-S1 Mathlib**: an **allowed exception** to "no dependencies", but **optional**. The user installs Lean (elan) + a Lake project with Mathlib. If it's missing, `cmx stex check` still writes `F.lean` and prints a hint with install steps. cmx ships its own **theorem library** (`data/theorems.tsv`): aliases → Lean identifier + plain-text statement. It is used for alias detection, prose names, and showing what was invoked.
- **Q-S2** Default variance = **natural**.
- **Q-S3** `==` → `\equiv` (spec typo confirmed).
- **Q-S4** Every prose role has many options. Variance flags switch among them (§5).
- **Q-S5** Spec typo fixed by the user in prompt.txt.
- **Q-S6** `><` is not an operator; `>` and `<` are separate.
- **Q-S7** `//` line comments, as in C.
- **`$$`** is **not** stex syntax. Only single `$…$` exists (output rule in §3.1).
- **`\` always means set difference** (`A\B`, `A \ B` → `A \setminus B`), in prose and in `$…$` (user, session 3). stex has no typed LaTeX commands; raw LaTeX goes only through `latex{…}`.

## 9. Implementation design (session 2)

- **Crate `cmx-stex`** (no deps): `lex` (tokens + glue flags + multi-word phrase merge), `lexicon` (word→symbol data, number sets, functions, Greek), `math` (Pratt parser → `M` AST → LaTeX; logic words are operators only in explicit `$…$`), `parse` (sentences → `Stmt`s), `realize` (phrase bank + variance → LaTeX prose), `doc` (preamble, package detection, layout pass), `theorems`, `lean` (emitter + line map), `rng` (SplitMix64, FNV-1a seed).
- **Segmentation = greedy math parse + plausibility.** At each token, try the math parser. Accept the result as math if it contains an operator or relation, a digit, a function call, a math word, or a single-letter variable that isn't an English article in prose position (`a`/`A`/`I` followed by a prose word stays prose). Otherwise emit the word as prose. Math parsing stops at prose words, so `in particular` and `want to show` never become math (`in`/`to` need math operands on both sides).
- **Context for capitals:** `N Z Q R C` become blackboard letters only in **set position** (after `in`/`notin`/`subset`/`:` in a signature/`to`/`->`/`times` between sets). Elsewhere they're variables.
- **Precedence (low→high):** iff < implies < or/xor/nor < and/nand < not < relations (chainable; `mod` after `≡` → `\pmod`) < set ops (∪ ∩ ∖ △) < `+ - ± ∓` < `* / · × mod % over choose` & implicit multiplication < unary − < `^ _` < postfix `! '` & calls.
- **Statements:** `let/fix` (comma lists; bare variables merge into the next item: `let x,y in Z` → "Let $x, y \in \mathbb{Z}$"), `assume/suppose`, `afsoc/aftsoc`, `contradiction`, `wts`, `qed [P]`, `then/thus/hence/so/therefore`, `define … via …` / `:=`, `by THM …` / `… by THM`, `because/since/b/c`, `i.e./ie/i.e/ie.`, `note`, `wlog`, `case N cond:`, labels `proof claim lemma …`, style blocks, `latex{}`. Clause connectives `and or xor nand nor implies iff then`, quantifiers `exists … such that …` / `for every …`.
- **Sentence boundaries:** `.` (not in numbers or abbreviations like `s.t.` `i.e.` `e.g.` `a.e.` `w.r.t.`), `?`, prose `!`, and newline, unless the line ends with a continuation (`,`, an operator, `and`/`or`/`such that`, an open bracket). A blank line starts a new paragraph.
- **Lean** (`stex check`): binders from `let … in S`, hypotheses from `let x = …`/`assume`, goal from `wts`, every derived statement → `have sN : P := by stex_auto`, `by THM` → `first | (simp_all [Lemma]; done) | stex_auto`. A `-- stex:LINE` map turns Lean errors into stex line numbers; `check` prints one failure per source line with the Lean goal (`goal: ⊢ …`). Prose justifications are ignored (spec). Details: §11.

## 10. Proof constructs (session 4)

- **Induction.** `we proceed by induction on n` / `by strong induction on n` / `we induct on n` opens an induction (`Fact::Induction`). `base case n = b: …` and `inductive step: …` are paragraphs with italic heads ("\textit{Base case} ($n = b$).", "\textit{Inductive step.}"). In the step, `assume P(k) [for some k ≥ b]` names the step variable (the first free single letter that isn't a sum index); `wts` inside a base/step adds prose but no new Lean goal. `qed` closes the induction. A strong induction usually goes with cases (`case 1 n = 0: …`).
- **Cases.** `case N cond:` items form a list; `case 2a …`, `case 2.1 …` and `subcase …` nest one level deeper (`Fact::Case { depth }` + `EndCases`). Rendered as nested `itemize`. "in either case / in all cases P" is what the cases prove.
- **Procedures.** `step N: …` / `stage N: …` items form a list (`ListKind::Steps`); `in total`, `altogether`, `overall`, `in all`, `all in all` close it.
- **Contradiction.** `afsoc P`; `suppose not` / `assume not` / `suppose otherwise` / `assume the contrary` = negation of the current goal (`math::negate_claim`: order complement, parity, De Morgan, quantifier duals; predicates read "$n$ is odd"). Without a goal: "Assume for the sake of contradiction that the claim is false."
- **Between.** "x is between a and b" → `a \leq x \leq b`; "strictly between", "lies (strictly) between" → `<`.
- **Ordinals.** `nth`, `n-th`, `(k+1)st`, `2nd` render as `$n$th`, `$n$-th`, `$(k + 1)$st`, `2nd`.

## 11. Lean translation and verification (session 4)

- **Corpus:** `crates/cmx-stex/tests/lean/NN_name.stex`, first line `// expect: ok` or `// expect: fail L1,L2` (source lines, counting the header line). Run: `cargo test -p cmx --release --test stex_lean -- --ignored --nocapture` (4 files at a time, ~90 s; `STEX_LEAN_ONLY=substr` filters). Covers algebra identities, inequalities, divisibility, congruences, parity, witnesses, functions, sums, induction (ordinary, from 1, strong, over sums and products), cases (incl. nested), contradiction (named and "suppose not"), counting, typed primes, "between". Negative files check that wrong arithmetic, a wrong inequality, a wrong induction step, a wrong case, a wrong witness, a missing contradiction, a wrong parity claim, a wrong count and a wrong base case are rejected **at the right line**. 52/52 as expected.
- **Translation rules added in session 4:** induction variable and step variable are ℕ; an ∃/∀ binder's set gives its type (`∃ k ∈ ℕ` → witness `obtain ⟨k, _⟩ : ∃ k : ℕ, …`, congruences on ℕ use `[MOD n]`); `ab` → `a * b` when a, b are in scope; `n(n+1)` is a product unless `n` is a function; a chain about a sum/product with ℕ subtraction (`∑ (2i − 1)`) is stated in ℤ (`(… : ℤ) = …`, as meant); predicates map to `Even/Odd/Nat.Prime/…`.
- **Automation (prelude of every `.lean`):** `stex_auto` = split ∧-hypotheses, then `stex_close` (omega, simp_all, parity simp set, norm_num, linarith, nlinarith, ring1, ring_nf, decide, positivity, `Int.ModEq.pow`, ModEq → omega, divisibility lemmas, abs lemmas, `mul_eq_zero`, order trichotomy, push_neg + linarith for disjunctions, the induction hypothesis `ih`, `stex_sum`, excluded middle, tauto), else split an ∧/↔ goal recursively, else `intro` and retry, else `fail` with a readable message. `stex_sum` peels the last term off `∑/∏ i ∈ Icc {0,1} (k+1)` (helper lemmas `stex_sum_Icc1_succ` …) and finishes with linarith / rewriting by the hypotheses. Inside a strong induction every step also tries `stex_strong n` (use `ih` at n−1, n−2, n/2, or write n = m+1 to make `2^n = 2·2^(n−1)` linear). `stex_contra` closes `False`. `stex_close` is unhygienic (`set_option hygiene false`) so it can refer to `ih`.

## 12. Corpus audit (session 5): failure taxonomy

**Method.** 177 hand-written proofs in `crates/cmx-stex/tests/corpus/*.stex`. Each was written the way a student would type it: every alias of every construct (§1–§10), plus ordinary prose that reuses keyword-like words ("in this case", "there is a subtle point", "a number", "the product", "by the way"). All 177 were compiled with `--fixed` through the bundled pdflatex, and every output was read next to its source. `--natural/--varied/--random` were compiled too.
- **TeX level:** 177/177 build a PDF in every variance mode, so no failure crashes LaTeX. Every problem is semantic: wrong or garbled output.
- **Automatic lints** (fragmented math `$x$ $=$`, lone operators, `$a$` + noun, verb-fragment sentences, leaked keywords, math words in prose, `\text` in math, raw `...`) flagged 84 files. Reading all outputs found more than the lints did.
- **The user's `prime.stex`** hits A1, A10, C1 and E1 below.

Root causes, most damaging first. Files are corpus names; ✗ marks a user-reported item.

**A. Math the grammar can't express.** These fragment the formula into `$…$ $…$` pieces, or, inside `$…$`, wrap the whole formula in `\text{}`.
- **A1 ✗ Ellipsis `...`** in sums, products, lists, sets, `let` lists and explicit math: `p_1 cross ... cross p_k`, `{p_1, ..., P}`, `a_1 + ... + a_n`, `let a_1, ..., a_n be`, `A_1, A_2, ...`. Files: nt01 nt02 nt25 al10 se11 an04 pr01 pr06 pr09.
- **A10 ✗ All-or-nothing fallback in `$…$`.** One unparseable token turns the whole formula into `\text{N = ( p \_ 1 cross … )}`, which is why `cross` "doesn't become \times" inside `$…$`. The fallback must be local: the bad token becomes `\text{}` and the rest stays math.
- **A2 Juxtaposition across a space** (implicit product): `2 sqrt(ab)`, `a_i 10^i`, `e^a e^b`, `(1+x)^k (1+x)`, `|A| |B|`, `n! sum …`, `z z bar`, `x e^x`, `p_1 p_2 … p_k`. Files: al01 al13 al14 al17 nt21 nt01 se08 co11 an06 pr06 al10.
- **A3 Composition `o`** (`g o f`, `(g o f)(x)`) is not an operator; inside `$…$` it renders `gof`. Files: fn01 fn03 fn14.
- **A4 Names:** multi-letter subscripted identifiers (`id_X`), and `comp(A)` as complement. Files: fn03 se15.
- **A5 Big set operators** `union/intersection from i=1 to n of A_i`. File: se11.
- **A6 `||`** is lexed as logical "or" even as norm bars (`||v||`) or nested absolute values (`||x| - |y||`). Files: al05 fm12.
- **A7 `|…|` with set operators after a `by THEOREM,` lead-in** (`|A union B| = …`) breaks into pieces. Files: se20 co10.
- **A8 Redundant parentheses** in `\binom{n}{(n - k)}` (the user's grouping parentheses are kept). Files: co03 co12 co13 co15.
- **A9 Set-builder with a prose predicate** (`{x in Z | x is even}`) and `:=` after it. File: se04.

**B. Keywords and math words colliding with each other.**
- **B1 `by` inside math** (`ax + by = d`) is read as a justification. File: nt09.
- **B2 `b/c` inside math** (`a + b/c`) is read as "because". File: al07.
- **B3 English nouns that are also symbol names become symbols in prose:**
  - "perfect square" → `\square`
  - "cross product u cross v is orthogonal to" → `\prod … \perp`
  - "every perfect square is congruent to 0" → `\equiv` with a prose subject
  - "a triangle", "a dot product"

  The lexicon must only fire in math position. Files: nt24 al16 lq08.
- **B4 Hyphenated words split into math:** AM-GM, sigma-algebra, 4-digit, three-element, ten-element, L-shaped. Files: al01 se13 co04 co07 co08 in09.
- **B5 Prose "not"** ("cannot both be and not be divisible") becomes "it is not the case that". File: ct07.
- **B6 Prose prepositions become operators:**
  - "functions from A to B" and "from (0,0) to (m,n)" → `\to`
  - "solutions … in nonnegative integers" → `\in`
  - "f maps R to R" leaves the first R plain

  Files: fn13 co12 co05 pr05.
- **B7 Prose `xor`** ("P xor Q holds exactly when …") expands into "either P or …, but not both" in the middle of a clause. File: al20.

**C. Quantifiers ("there is", "for all").**
- **C1 ✗ Articles read as variables.**
  - "there is a largest prime" / "a least such n" / "a real number" → "there exists $a$ such that …". Files: nt02 in08 ct03 ct10 pr03.
  - "a" before a lexicon word → `$a$` ("a triangle", "a maximum", "a square", "a dot product"). Files: lq08 ct05 st07 al16.
  - "a" before a number → `$a$` ("a 2^n by 2^n board"). File: in09.
- **C2 Commas and colons inside the bound part read as "such that":**
  - "integers m, n s.t" → "$m$ such that $n$ such that"
  - "g: Y to X such that"
  - "c in (a, b) such that"

  Files: nt06 nt10 nt17 lq01 fn03 fn12.
- **C3 "there is/are" followed by prose or a count is not a quantifier:**
  - "there are only finitely many primes, say …"
  - "there are 10 choose 3 = 120 … subsets"
  - "there are 26^n words"
  - "there is a subtle point here, and …"

  Files: nt01 co04 co12 co14 pr02.
- **C4 "there is δ > 0 with …"** → "there exists $\delta > 0$ such that with". File: fm07.
- **C5 Grammar:**
  - "there exists integers" (plural subject needs "there exist").
  - "for all sets A and B" → "For every sets".
  - "for all but finitely many" → "For every but finitely many".

  Files: nt05 nt06 nt09 lq01 se19 lq17.

**D. Declarations and lists.**
- **D1** "fix a, b in N with a, b > 0" / "with a, b in R": a comma after `with` starts a new `let`. Files: nt20 al17.
- **D2 Inconsistent list joining:**
  - "let x, y in R s.t." → "Let $x$ and $y \in …$"
  - "let F_0 = 0, F_1 = 1 and F_n = …" → "Let … and let … and …"

  Files: al06 al08 fn13 in04.
- **D3** The spec form "let x in Z, sum from i = 1 to 4 = 30" (the comma means "such that") becomes two lets. File: lq13.
- **D4 Declared variables must beat number-set letters:**
  - "let A, B, C be sets … B subseteq C" and "g: B to C" → ℂ.
  - Undeclared capitals in set position after verbs (`maps R to R`, `increasing on N`, `= Z` for sets) stay plain.

  Files: se05 fn01 pr05 se17 se04.

**E. Sentence structure.**
- **E1 ✗ A line starting with a verb continues the previous sentence** (`A = {…}` ⏎ `is the set of all primes`). Today it becomes "Is the set of all prime numbers." Files: nt02 pr06.
- **E2 "suppose otherwise, and let S be …"**: the second clause is taken as the assumption. File: ct05.
- **E3** `proof` on a line before `case 1 …` ends up inside the list ("\item \textit{Proof.} Case 1"). File: nt19.
- **E4 A lemma's claim** used by `qed` is its first sentence ("let a, b, c in Z") instead of the statement. File: nt05.
- **E5 Clause joining:** ",," and a misplaced "and" ("$p$,, $p \mid n$ since $p \mid a$, and $a \mid n$"). Files: nt14 al16.
- **E6 Assumption wording:**
  - "wlog we may write …" → "assume that we may write".
  - "assume the claim for …" → "Suppose that the claim for …".

  Files: fn14 in09.
- **E7 "Hence P, as required"** repeats a sentence that already concluded P (style; prefer "This completes the proof."). File: nt04.
- **E8 "the derivative of f with respect to x is …"** → "the $\frac{d}{dx} f$ is". Files: fn09 an10.
- **E9 "= 0 is divisible by 7"** becomes `\text{ is divisible by }` inside the formula. File: in06.
- **E10 "by CRT"** in the middle of a sentence is not expanded to the theorem's name. File: nt07.

**F ✗ Editor highlighting does not match the compiler.**
- The stex highlighter is a flat keyword list in `cmx-editor/src/highlight.rs`, separate from the stex lexer.
- It misses aliases (`s.t.`, `st.`, `b/c`, `w/`, `i.e.`, `afsoc`, `aftsoc`, `forall`, `there is`, `if and only if`).
- It colors fragments of phrases ("such", "there", "is") on their own.
- Different aliases of one keyword get different colors.
- Fix: the highlighter calls the stex lexer itself, so a phrase is colored only once complete ("such" stays plain until "that" follows). Every alias of a construct gets the same role color, and "is" alone stays prose.

**Fix order.**
1. The user-reported items: A1, A10, C1, E1, F.
2. Then by number of files affected: A2, C2/C3, B3/B4, D1–D4, A3–A9.
3. Then the rest of B/C/E.

Every fix gets a corpus file or a golden test. The corpus becomes a regression suite: expected `.tex` snapshots per file plus the lints, run by `cargo test`.

### 12.1 Fix status (session 5) and the regression suite

All the ✗ user-reported items and nearly every root cause above are fixed. Each has a named golden test in `src/tests.rs` (`audit_a_math_grammar`, `audit_b_…`, `audit_c_…`, `audit_d_…`, `audit_e_sentences`). The whole corpus is frozen as reviewed snapshots in `crates/cmx-stex/tests/corpus/expected/*.tex`. `crates/cmx-stex/tests/corpus.rs` checks three things on every `cargo test`:
1. the snapshots;
2. lints over all 177 outputs: no lone `$=$`, no adjacent `$…$ $…$`, no leaked keywords, no raw `...`, no article read as a variable, no ", ,", "such that such that", or verb-fragment sentences;
3. every variance mode plus the Lean translation run without errors.

After an intended change: `STEX_BLESS=1 cargo test -p cmx-stex --test corpus`, then review `git diff …/expected`.

How each root cause was fixed:
- **A1** `...` is an atom: `\cdots` between operators and in products, `\ldots` in lists, sets and arguments. `let` lists, quantifier lists and prose lists ("say p_1, ..., p_k") accept it.
- **A10** An unparseable `$…$` is rendered piecewise (`explicit_lenient`): only the bad tokens become `\text{}`.
- **A2** Spaced juxtaposition multiplies in prose when the next atom is unmistakably math: subscripted/powered variables, numbers with `^`/`!`, function calls, `sqrt`, Greek, `(`, `|…|`, `...`, accents. Never after a set word ("primes p_i"). Never in a quantifier binder ("for each x P(x)").
- **A3** ` o `/`circ` between functions is `\circ`.
- **A4** `id_X`/`rk_A` names; `comp(A)` = `A^c`; nested superscripts are parenthesized: `(A^{c})^{c}`.
- **A5** `union/intersection from … to … of` → `\bigcup`/`\bigcap`.
- **A6** The lexer splits a `||` glued to an operand into two bars; the parser reads `||v||` as a norm, else nested absolute values. A spaced `||` stays logical or.
- **A7** `|…|` parses a full set expression inside.
- **A8** `choose` strips grouping parentheses.
- **A9** `{x in S | x is even}` keeps the words as `\text{ is even}`. The root bug: `rel_op` ignored the stop list, so the set-builder `|` was read as "divides".
- **B1** `ax + by = d`: a two-letter keyword between arithmetic operators is a product. The justification splitter skips it too.
- **B2** `b/c` between operators is a fraction.
- **B3** Symbol names that are English nouns (square, triangle, angle, top, partial, …) are symbols only inside `$…$`. Word forms `sum`/`product`/`union` need bounds (`from`, `_`, `over`).
- **B4** Hyphenated words (`AM-GM`, `4-digit`, `L-shaped`, `$\sigma$-algebra`) are one prose word.
- **B5** Prose "not" stays "not".
- **B6** "from X to Y", "maps R to R" stay prepositions (with `\mathbb` sets). "= 10 in nonnegative integers" stays prose.
- **B7** "P xor Q holds …" → `$P \oplus Q$` holds.
- **C1** Article rule (`is_article_at`): "a"/"an" before a word, a number, or an operator word followed by a noun ("a dot product").
- **C2** `binder_sep`: brackets nest; a comma before another variable is a list comma; a colon before a signature belongs to the binder; ∀ keeps "with …" restrictions; ∃ uses "with … such that".
- **C3** "there is/are" + number/"only"/"(" → a count in prose. An article phrase without math → prose.
- **C4** `with` is a separator.
- **C5** "there exist" / "for all" with plural binders; "for all but finitely many" and "for all sufficiently large" kept; "there is no integer" (no extra article); "there exists some" → "there exists".
- **D1** A comma inside a `let` condition stays in it.
- **D2** "and" before another declared variable splits the `let` list ("Let $F_0 = 0$, let $F_1 = 1$, and let …"). Bare variables merge, including `a_1`, `...`, and "x, y in R s.t. … and …".
- **D3** The spec's comma-as-such-that: an item that declares nothing becomes "such that …".
- **D4** Capital letters listed in a declaration (`let A, B, C be sets`, `for all sets A and B`) are variables, never ℂ/ℝ. `C` after a set variable in a signature (`g: B to C`) is a variable. `R^n` after prose "in/on/over" is ℝ.
- **E1** A line starting with a verb or connective continues the sentence.
- **E2** "suppose otherwise, and let …" → the negated goal, then the rest as its own sentence.
- **E3** A pending Proof label stays above a list.
- **E4** "lemma: let …. if … then …": the setup sentence first, then the claim.
- **E5** Prose parts keep the user's own "and"s.
- **E6** "wlog we …"; "assume the claim for …" → "Assume …".
- **E7** `qed` after a sentence that already concluded the goal (or with no goal) ends that sentence with □.
- **E8** "the derivative of f with respect to x" stays prose.
- **E9** "is divisible by" is prose in LaTeX and `b ∣ a` for Lean.
- **E10** "by THEOREM" mid-sentence reads by the theorem's name.

Still open, minor:
- "every common divisor of a and b divides d" → "$b \mid d$" (the relation takes only the nearest operand).
- "integrate … by parts: … [x e^x] from 0 to 1" (evaluation brackets).
- "a 2 by 2 board" → "$2$ by 2" (inconsistent lone numbers).
- A theorem cited mid-sentence is not added to the Lean check.

## 13. Editor highlighting from the compiler (session 5)

`cmx_stex::highlight::line_roles(line, in_display)` lexes one line with the real lexer and returns `(start, end, Role)` byte ranges. The lexer handles phrase merging, abbreviations, `$…$`, style blocks and `latex{}`; tokens carry byte spans (`Tok.span`) for this.

Roles:

| Role | Members |
|---|---|
| Structure | let, assume, wts, afsoc, qed, case, step, note, wlog, suppose not, base case, the exhaustive and "case on" families (§14), … |
| Label | proof, lemma, … |
| Quantifier | exists, there is, for all, such that, s.t., st, with, w/, where |
| Logic | and, or, not, implies, iff, then, thus, hence, so, … |
| Reason | by, because, since, b/c, i.e., via |
| MathWord | lexicon words |
| Math | `$…$`, `latex{}`, sigils |
| Others | Number, Operator, Style, Comment |

The editor maps roles to theme colors. A lone `$` line opens a display block (state carried to the next line). A mid-edit lex error (an unclosed `{`) still colors what precedes it.

Consequences, tested in `highlight::tests` and e2e `stex_highlighting_follows_the_compiler`:
- A phrase is colored only once complete ("such" stays plain until "that").
- All aliases share a color.
- "is" alone stays prose, while "is in" is a math word.

## 14. Case analyses: numbering, paragraphs, exhaustiveness (session 5, part 5)

User request (2026-11-14), from `~/dev/sandbox/odd8.stex`. The old output had "Case k: Suppose is odd.", the list split at every blank line, and "Hence there are $l \in \mathbb{Z}$".

**Labels (`case_label`).** An explicit label is one of:
- a number: `1`, `2a`, `1.2`;
- a parenthesized tag: `(i)`;
- a short name before `:`/`.`/end: `case k:`;
- a one-letter or roman name followed by a *new* expression: `case k n = 1` → "Case k: Suppose $n = 1$".

The first word is part of the condition when what follows continues it: a verb, an operator, a predicate word, `in`… So `case k is odd`, `case n = 0`, `case n even` and `case 0 < x` have no label.

**Auto-numbering.** Unlabeled cases are numbered by position: "Case 1", "Case 2". Sub-cases take the enclosing case's label: "Subcase 2.1".
- `subcase` after a `subcase` is a sibling. (Before this fix every sub-case went one level deeper.)
- An unlabeled `case` is always top level.
- "case n even" reads "Suppose $n$ is even" (the implied verb is inserted).

**Blank lines inside a case (`para_fate`).** While a list is open, a statement after a blank line:
- **Stays in the current case** as a new paragraph (`Out::Break`, rendered as a blank line inside the `\item`) when, before any `qed`, label or `wts`, one of these follows:
  - another case or step;
  - a closer (`in either case` …);
  - an `exhaustive` that closes the cases.
- **Ends the list** (as before) otherwise. That covers trailing prose after the last case with no closer, and a new analysis: an explicit first label (`case 1` again), an `exhaustive` opener followed by a new case, or a `case on X`.
- **Paragraph after cases inside a step:** if the next structural statement is a step, the case list ends and the paragraph continues the step (`LeaveCases`).

**Closers.**
- `in either/both/all/every/any case(s)` and `either way` close every open case list but keep an enclosing step list.
- `qed` right after such a closer (or after an `exhaustive` transition) whose text states the goal just appends □. This is the E7 rule extended: nt15 and st03 changed this way.

**Case-analysis prose comes only from the user** (revised 2026-11-14 at the user's request). The first version derived exhaustiveness reasons from the case conditions ("since $k$ is either odd or even") and added a closing line from what the cases concluded. The user rejected both: a reason must be one the user wrote. That code (`exhaustive_reason`, `known_exhaustive`, the per-case condition and conclusion tracking) is removed.

| User writes | Output (--fixed) |
|---|---|
| nothing (just `case …` lines) | just the list: no announcement, no exhaustiveness sentence |
| `case on X` (aliases: `cases on`, `casing on`, `case analysis on`, `case split on`, `split into cases on`, `we case on`, `by cases on`, `[we] proceed by cases/case analysis on`) | "We proceed by case analysis on X." before the list. `case on X, exhaustive b/c R` adds the exhaustiveness sentence after it. Inside a case, before `subcase`s, it stays in that case. |
| `cases exhaustive` / `exhaustive cases` / `the cases are exhaustive` / … before the cases | "The following cases are exhaustive." |
| the same after the cases | "These cases are exhaustive." |
| `exhaustive [cases] b/c R` (`because`, `since`, `as`; or `by T`) | "The following cases are exhaustive, because R." / "These cases are exhaustive, because R." The user's word is kept in --fixed. |
| `since R, exhaustive cases` / `b/c R, the cases are exhaustive` (reason first) | "Since R, the following cases are exhaustive." / "Because R, these cases are exhaustive." |
| `exhaustive [b/c R], so P` after the cases | the sentence above, then "In all exhaustive cases, P." |
| `in all/every/each/both/either exhaustive case(s), P` / `since/as/because the cases are exhaustive, P` | the user's words: "In exhaustive cases, P." Nothing else is added. |

- **Before or after** is decided by position: an `exhaustive` statement directly followed by a case (or by sub-cases of the current case) is "before"; otherwise it closes the innermost case analysis.
- **Variance:** in --natural/--varied the phrasing varies, but every template states only the user's claim and reason.
- **Phrase roles** (`phrase-map.txt`, explicit rates; the slot `{R}` is the body of a "since" clause):
  - `cases.on`, `cases.on.bare`;
  - `cases.exhaustive.open`, `.open.by`, `.open.bare`;
  - `cases.exhaustive`, `.by`, `.bare`;
  - `cases.exhaustive.close` (after the user's justification sentence);
  - `cases.exhaustive.close.since` (the user's closer phrase, varied modes).

**Agreement (item 3).** `agree()` fixes the binder's verb after the phrase bank picks a template, in every variance mode:
- singular: "there is / there exists $l$";
- plural: "there are / there exist integers $m, n$".

It applies to `exists`, `exists.inner`, `exists.bare`, `exists.none` and `.bare`. New lexer phrases `there are no` / `there exist(s) no` map to ∄. Tested over 40 seeds × 3 modes.

**Other fixes found while writing the new corpus files:**
- "we show |x − y| = …": a bar glued to the next token opens an absolute value. Only a spaced ` | ` is "divides" next to a word operand. Before, the result was "we $show \lvert…$".
- "xy is odd" after `let x, y ∈ ℤ` → $xy$. A 2–3-letter word spelled only with declared one-letter variables, not an English word or keyword, is their product. Undeclared "xy" stays prose; "an" with a, n declared stays prose.
- "0 or 1 upon division by 4": a number before a preposition (upon, when, after, as, than, modulo…) is math, not a counted noun.

**Corpus:** `ca01`–`ca18` (195 proofs). New unit tests:
- `case_labels_and_auto_numbering`
- `blank_lines_inside_cases`
- `exhaustive_and_case_on_only_say_what_the_user_wrote` (also checks that no mode invents a reason)
- `existential_agreement`
- `words_near_bars_and_numbers`

