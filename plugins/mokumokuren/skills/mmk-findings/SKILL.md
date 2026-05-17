---
name: mmk-findings
description: Interprets mmk findings via override discipline and per-sensor priors. Use when mmk has surfaced one or more findings (HOTSPOT, COUPLING, COMPLEXITY, BUDGET, STRUCTURE, HEALTH, COHESION, DRIFT) and you need to decide whether to act, override, or split a slice. Use when a borderline finding ends with `[id=…]` and needs an `mmk explain` drill-down before deciding. Use when reading `[no actionable signal]` and confirming silence is the right answer.
---

# mmk-findings

## Overview

Findings are signal, not law. mmk surfaces a layer-labeled finding
when its sensor crosses a calibrated gate; the agent decides
whether to act on the signal, override it, or split the slice.
This skill encodes the per-sensor priors and the override
discipline that turn raw findings into decisions.

The invocation layer (when to run `mmk review` / `pre-edit` /
`session-summary`) lives in the companion skill `mmk-on-edit`.
This skill picks up after that skill has surfaced findings.

## When to Use

- Immediately after `mmk review`, `mmk pre-edit`, or
  `mmk session-summary` returns one or more `findings[]` entries.
- When a `[low-confidence n=N]` suffix appears on a COUPLING
  finding and you're weighing whether the n is high enough to act.
- When BUDGET fires and you need to decide between commit-the-slice
  vs. split-the-diff.
- When a finding feels borderline or surprising and you want
  ground-truth commits before deciding — pair with `mmk explain
  --finding <id>`.

**When NOT to use:**

- The surface is `[no actionable signal] no findings (...)`. That
  is positive confirmation, not a finding to interpret. Read it
  and move on — invoking this skill in that case is busywork.
- The finding has already been overridden in writing earlier in
  the same slice. Re-applying the workflow on every Edit
  re-derives the same answer.

## The Workflow

For each finding in the surfaced output:

1. **Classify by `layer`.** The layer (`hotspot`, `coupling`,
   `complexity`, `budget`, `structure`, `health`, `cohesion`,
   `drift`) selects the prior in *Sensor Priors* below.

2. **Consult the prior.** Read what the layer means and what the
   default response is. Read the severity (`warn` vs `info`) — it
   shapes the action.

3. **If the line ends with `[id=…]` and the call is borderline**,
   run `mmk explain --finding <id>` to see the underlying commits
   before deciding. Don't act on a K-of-N summary you don't trust
   when the drill-down is one CLI call away.

4. **Decide: act or override.**
   - "I read this and chose to override because <X>" is a complete
     response.
   - "I didn't address this" is not.

5. **If a BUDGET Info fired at the review-effectiveness floor**
   (`review effectiveness degrades past ~200 lines`) — commit the
   current slice before continuing. Stop. Commit. Then resume.

6. **If a BUDGET Warn / Over fired near or past the per-diff cap**
   (1000 LOC default) — split the diff. It's too big to review
   well.

## Sensor Priors

One paragraph per sensor; tuned from the v0.8 N=20 calibration
cohort.

- **HOTSPOT**: top of rank → tighten review on this file. The fact
  the file is hot is the signal; the rank itself rarely needs
  action.
- **COUPLING**: a real partner missing → either add it or justify.
  A partner you've already touched in this diff is not a finding.
  ~50 % of the time you should add the partner's test or peer;
  ~50 % you should explain why this change doesn't need it. Don't
  ignore. When the suffix is `[low-confidence n=N]`, the gate
  cleared near the floor — weight the override toward "explain"
  rather than "add."
- **COMPLEXITY**: delta-weighted (v0.8). Warn = you made it worse
  (Δ ≥ 50 % or Δ ≥ 20 absolute over the HEAD baseline); Info =
  your edit is small but the function was already over. Refactor
  when Warn fires unless you have a concrete reason this slice
  doesn't include the refactor.
- **BUDGET**: Info ramp at ≥50 % of the per-diff cap (1000 LOC
  default), Warn at ≥75 %, Over above 100 %. The cap is an
  agent-context guardrail — split when Warn fires. See workflow
  steps 5–6.
- **STRUCTURE**: divergence is real signal except on role files.
  Role-file findings (Info) name the role status; act on them
  only when the divergence is from peer role files. Confirm the
  role peers (factories / contributions / registrations) share
  the *role* convention rather than the directory shape.
- **HEALTH** (`test_pair`, `broad_exception`, `test_weakening`):
  act on each. `test_pair` means you wrote impl without its test —
  add the test or explain. `broad_exception` flags a newly-added
  broad catch handler (the EVASION sensor, arXiv:2509.13941) —
  narrow the catch or rethrow. `test_weakening` flags net erosion
  of an existing test (skips added, assertions lost, mocks added,
  `@ts-expect-error` sprinkled, test cases removed) — the failure
  mode documented in arXiv:2503.15223. If the test was wrong, say
  so explicitly; otherwise undo the weakening.
- **COHESION**: a tangle → propose a commit split.
- **DRIFT**: same file climbing across sessions → you are
  rewriting, not progressing. Step back and check whether the
  abstraction is wrong.

## Reading Silence

`[no actionable signal] no findings (...)` is the canonical empty
output. It fires on a clean tree, on an edited tree where nothing
crossed a gate, and on dedup-suppressed re-runs. Read it as
positive confirmation: mmk ran and found nothing to act on — not
as a tool failure.

Most cold-file or small-diff edits land here, and that is correct.
The gates key on historical correlation, per-function delta vs
HEAD, and architectural divergence; an isolated edit to a
low-history file legitimately doesn't trip them.

## Common Rationalizations

| Rationalization | Reality |
|---|---|
| "This finding doesn't apply because the partner is unrelated." | Run `mmk explain --finding <id>` first. The K-of-N summary hides whether the co-change is a sustained convention or a single merge storm — the drill-down is the only way to know. |
| "I'll address it in a follow-up commit." | "Follow-up" is the rationalization that ends with the partner never getting touched. If it needs a change, this slice owns it; if it doesn't, write the one-line override now. |
| "Low-confidence findings can be skipped silently." | Low-confidence is exactly when the override-rationale matters. Silent skips erode the discipline; one-line "explain why this change doesn't need it" preserves it. |
| "BUDGET Info is just informational." | Info at +200 LOC is the *slice boundary by design*, not advisory noise. Warn / Over near 1000 LOC is the second register. Each is a different kind of stop. |
| "STRUCTURE divergence is fine — every role file diverges." | Only role-file *Info* findings are expected divergence. Warn fires precisely when divergence is from peer *role* files, not from sibling shape. Read the severity. |
| "DRIFT just means the file is being worked on." | DRIFT fires on rank-climb across sessions, not on activity. A climbing rank means rewrite-without-progress. Step back; check the abstraction. |
| "The COMPLEXITY Info already existed at HEAD, so it's not mine." | Correct — but only Info severity is suppressible. If the same edit moved Info to Warn, the delta is yours. Re-read severity, don't aggregate. |

## Red Flags

- A finding is acknowledged in narration but neither acted on nor
  given a written override rationale.
- A `[low-confidence n=N]` finding is treated as "not real" and
  silently dropped instead of being overridden in writing.
- BUDGET Info crosses the +200 LOC floor and the next action is
  another Edit instead of a commit.
- DRIFT fires across ≥3 of the last 5 sessions and the response
  is to keep editing the same file.
- A COUPLING finding's `[id=…]` is ignored — `mmk explain` is the
  cheap drill-down and skipping it leaves the call uninformed.
- Severity (`warn` vs `info`) is collapsed in narration ("there
  was a complexity finding") instead of read explicitly.

## Verification

- [ ] Every finding has either an action taken or a one-line
      written override rationale. No silent skips.
- [ ] Borderline COUPLING findings were drilled into via `mmk
      explain --finding <id>` before deciding.
- [ ] BUDGET Info at the +200 LOC floor was followed by a commit
      before further edits.
- [ ] BUDGET Warn / Over at the per-diff cap led to a split
      proposal, not a "keep going."
- [ ] STRUCTURE Warn (peer-role divergence) was either reshaped
      or explained; STRUCTURE Info on role files was not treated
      as Warn.
- [ ] DRIFT fires were resolved with a step-back / abstraction
      review, not by continuing the rewrite.
- [ ] On `[no actionable signal] no findings (...)`, the run was
      treated as positive confirmation, not invoked again.
