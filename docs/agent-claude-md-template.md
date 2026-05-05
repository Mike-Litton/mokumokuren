# mmk agent guidance — CLAUDE.md template

Drop this section into your repo's `CLAUDE.md` (or whatever
agent-instruction surface your harness honours). It encodes the
invocation pattern, override discipline, and per-sensor priors that
the v0.8 N=20 calibration cohort validated.

---

## 1. Invocation pattern

mmk wires into Claude Code via two hooks plus one explicit
post-commit step.

- **`mmk pre-edit`** — auto-run by the `PreToolUse:Edit|Write` hook
  on every Edit / Write. Treat its output as additional context for
  the file you're about to touch. Do not invoke it manually except
  when you suspect hidden coupling on a file you are *not* yet about
  to edit.
- **`mmk review`** — auto-run by the `PostToolUse` hook. Findings
  arrive as `additionalContext` in the next turn. Read them; act on
  them.
- **`mmk session-summary`** — **post-commit, pre-PR.** It reads
  committed state. Running it on uncommitted state returns a nudge
  that points you at `mmk review`. Do not run it as part of "end of
  feature" before you've committed.

The pre-experiment three-step prescription that asked you to run all
three on every change was ceremonial. The pattern above is the
right one.

## 2. Vertical-slice / commit-per-slice discipline

Work in vertical slices. Each slice ships a small, self-coherent
change with its tests. Commit each slice as you complete it; do not
accumulate slices into one large diff.

**When BUDGET fires: stop, commit the current slice, continue.**
BUDGET fires in two registers, calibrated independently:

- **Info at +200 LOC** (review-effectiveness floor). The diff has
  crossed the threshold past which review effectiveness degrades.
  This fire *is* the slice boundary — finish the slice and commit
  before adding more.
- **Warn at 75% / Over at 100% of the per-diff cap (1000 LOC default).**
  The diff is too big to review well. Split it.

The pattern compounds: small commits make BUDGET fires meaningful,
which makes the cut-and-commit response cheap, which keeps commits
small.

## 3. Override discipline

Findings are signal, not law. Read them; decide; explain when you
don't act.

- For COUPLING with a real partner: ~50 % of the time you should add
  the partner's test or peer; ~50 % you should explain why this
  change doesn't need it. Don't ignore.
- For COMPLEXITY: a Warn means the agent's edit made the metric
  materially worse (Δ ≥ 50 % or Δ ≥ 20 absolute over the HEAD
  baseline); an Info means the edit is small even if the function
  was already over cap. Refactor when Warn fires unless you have a
  concrete reason this slice doesn't include the refactor.
- For STRUCTURE on a role file (Info): the divergence is expected;
  confirm the role peers (factories / contributions / registrations)
  share the *role* convention rather than the directory shape.
- "I read this and chose to override because <X>" is a complete
  response; "I didn't address this" is not.
- When a finding is borderline or surprising, run
  `mmk explain --finding <id>` to see the underlying commits before
  deciding.

## 4. Sensor priors

One paragraph per sensor; tuned from the v0.8 experiment data.

- **HOTSPOT**: top of rank → tighten review on this file. The fact
  the file is hot is the signal; the rank itself rarely needs action.
- **COUPLING**: a real partner missing → either add it or justify.
  A partner you've already touched in this diff is not a finding.
- **COMPLEXITY**: delta-weighted (v0.8). Warn = you made it worse;
  Info = your edit is small but the function was already over.
- **BUDGET**: two registers. Info at +200 LOC (review-
  effectiveness floor — the slice is done; commit). Warn / Over
  near and past the per-diff cap (1000 LOC default — split). See
  section 2.
- **STRUCTURE**: divergence is real signal except on role files.
  Role-file findings (Info) name the role status; act on them only
  when the divergence is from peer role files.
- **HEALTH** (`test_pair`, `service`, `broad_exception`): act on
  each. A `test_pair` finding means you wrote impl without its test;
  add the test or explain.
- **COHESION**: a tangle → propose a commit split.
- **DRIFT**: same file climbing across sessions → you are rewriting,
  not progressing. Step back and check whether the abstraction is
  wrong.

## 5. Reading silence

`[no actionable signal] no findings (...)` is the canonical empty
output. It fires on a clean tree, on an edited tree where nothing
crossed a gate, and on dedup-suppressed re-runs. Read it as positive
confirmation: mmk ran and found nothing to act on — not as a tool
failure.

Most cold-file or small-diff edits land here, and that is correct.
The gates key on historical correlation, per-function delta vs HEAD,
and architectural divergence; an isolated edit to a low-history file
legitimately doesn't trip them. Quiet on a thin slice is the expected
baseline — running mmk on a hotter slice is what surfaces signal.

---

A current copy of this template lives at
`docs/agent-claude-md-template.md` in the mmk repo. Pull updates
when the experimental cohort retunes the priors.
