---
name: mmk-on-edit
description: Runs mmk against the working tree to surface Git-history findings about an edit. Use when you've just edited a tracked file and need to know if a coupling partner went untouched. Use when you're about to edit a file and want the historical-partner list before committing. Use when wrapping up a feature and need the post-commit session-summary view.
---

# mmk-on-edit

## Overview

mmk reads Git history and emits findings about the working-tree
diff: hotspots about to be touched, expected coupling partners
that were not touched, structural divergence from sibling files,
budget overruns past the slice boundary, and drift across recent
sessions. This skill names which subcommand to run at which point
in the edit loop, and what to surface from its output.

The interpretation layer (override discipline, per-sensor priors,
reading silence) lives in the companion skill `mmk-findings`. When
this skill surfaces findings, hand them off there before deciding.

## When to Use

- Right after an `Edit` / `Write` to a tracked file → `mmk review`.
- Before editing a file you haven't loaded yet → `mmk pre-edit <PATH>`.
- About to create a new file in a directory → `mmk pre-edit <PATH>`.
- Wrapping up a feature, after the last commit → `mmk session-summary --base main --drift-sessions 5`.

**When NOT to use:**

- The repo has no Git history (`mmk init` hasn't run, or this is
  a fresh `git init`). The sensors key on history; there's nothing
  to gate against.
- Pure documentation / Markdown / config edits in a directory the
  sensors don't index. mmk no-ops cleanly; no need to invoke it.
- Inside the post-edit hook context — the hook already runs
  `mmk review` for you. Don't run it a second time manually.

## The Workflow

1. **After an Edit** — run `mmk review`. Surface every finding.
   Prioritize COUPLING misses: they are the most actionable
   (partner you didn't touch).

2. **Before editing `<PATH>`** — run `mmk pre-edit <PATH>`.
   Surface HOTSPOT, COUPLING, and STRUCTURE findings. The
   partners in COUPLING are files to re-read before editing
   `<PATH>`.

3. **Before creating a new file `<PATH>`** — run `mmk pre-edit
   <PATH>` even though `<PATH>` doesn't yet exist. STRUCTURE
   may surface the directory's import / export convention the
   new file should match.

4. **End of feature, after the final commit** — run `mmk
   session-summary --base main --drift-sessions 5`. Surface
   findings, `entered_top_n`, `churn_of_churn`, and
   `base_resolved_via`. (Add `--format json` for structured
   fields.) If `session.base_resolved_via` is anything other
   than `explicit` or `since_commit`, mark the session block
   as informational only.

5. **If `mmk review` surfaces a BUDGET Info at the review-
   effectiveness floor** (`review effectiveness degrades past
   ~200 lines`) — treat it as the slice boundary. Commit the
   current slice before adding more. The floor sits below the
   per-diff cap on purpose; it surfaces the slice cue earlier
   than the cap does.

6. **Hand findings to the `mmk-findings` skill** for
   interpretation. Don't act on a layer name without consulting
   its prior.

## Common Rationalizations

| Rationalization | Reality |
|---|---|
| "I already know this file is hot — no need to run `mmk`." | Hotspot rank is one signal among many. COUPLING / STRUCTURE / BUDGET key on the diff in front of you, not on long-term memory. Run it. |
| "The diff is small, mmk won't have anything to say." | A clean run is positive confirmation. `[no actionable signal] no findings (...)` means consulted-and-quiet, not skip-mmk. |
| "I'll batch a `mmk review` at the end of the slice." | The post-edit hook fires per Edit on purpose. Running once at the end loses the COUPLING signal that would have caught the missing partner three edits ago. |
| "I'll skip session-summary; the per-edit reviews already covered it." | session-summary is the only command that overlays DRIFT, window-truncation, and `entered_top_n` against the committed session. The per-edit hook never sees those. |
| "session-summary returned a nudge to `mmk review` — close enough." | The nudge means you ran session-summary on uncommitted state. Commit first, then re-run; that's the only path to the actual session view. |
| "BUDGET Info is just informational; I'll keep going." | Info at +200 LOC *is* the slice boundary by design. The cap (1000 LOC default) is the second register, fired later. The Info is a stop-and-commit cue, not advisory ambient noise. |

## Red Flags

- An `Edit` lands and `mmk review` is not invoked (hook missing
  or `mmk` not on PATH — and no install nudge surfaced).
- A COUPLING finding fires and the named partner is neither
  touched in this slice nor explicitly overridden in the response.
- A BUDGET Info fires at the +200 LOC floor and the next action
  is another Edit instead of a commit.
- `session-summary` runs on uncommitted state and the nudge to
  `mmk review` is ignored.
- mmk output is treated as a hard gate that blocks progress.
  It's signal that informs; tests and review still apply.

## Verification

- [ ] After every `Edit` / `Write` to a tracked file, `mmk review`
      has run (hook fired, or invoked manually if hook absent).
- [ ] Every COUPLING / HOTSPOT / STRUCTURE finding has either an
      action taken or a one-line override rationale (handed off
      to `mmk-findings` for the decision shape).
- [ ] No BUDGET Info at the +200 LOC floor was crossed without
      an intervening commit.
- [ ] At end of feature, `mmk session-summary` was run *after*
      the last commit, not before.
- [ ] If `session.base_resolved_via` is not `explicit` or
      `since_commit`, the session block was marked informational.
- [ ] On `[no actionable signal] no findings (...)`, the run was
      treated as positive confirmation, not as a tool failure.
