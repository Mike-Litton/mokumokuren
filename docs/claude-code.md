# Wiring `mmk` into Claude Code

If you're using [Claude Code](https://claude.com/claude-code), you can
make `mmk` self-acting in three ways. Pick the strictness that fits.

All three assume `mmk` is on `$PATH` (`cargo install --path mmk-cli`,
or use a published binary).

## Working-tree vs history

The agent's actual edit loop happens in the **working tree**, not in
committed history. v0.3 exposes one subcommand per phase of that
loop:

| Loop phase                   | Subcommand                  | Sees                                         |
| ---------------------------- | --------------------------- | -------------------------------------------- |
| Before editing `<PATH>`      | `mmk pre-edit <PATH>`       | History only (rank, expected partners).      |
| After every edit             | `mmk review`                | Uncommitted working-tree diff vs history.    |
| Before staging               | `mmk review --staged`       | Staged index vs HEAD.                        |
| End of feature / PR review   | `mmk session-summary`       | Committed session vs base ref + drift / budget overlay. |

`mmk analyze` (the ranked top-N table) and `mmk drift` (climb signal
across K sessions) are also v0.3 subcommands but operate at coarser
grain — useful in CI gates and triage, not in the per-edit hook.

## Option 1 — `CLAUDE.md` (advisory, easiest)

Auto-loads per project. Drop the snippet below into your repo's
`CLAUDE.md` (or merge into an existing one — keep total file length
under ~200 lines for adherence):

```markdown
## Using mmk for editing decisions

This project uses [mmk](https://github.com/mlitton/mmk) — a
deterministic Git-history sensor that catches LLM slop the linter
and tests can't see (hotspot blindness, hallucinated coupling,
thrashing). The agent edit loop wires to it like this:

**Before editing a file `<PATH>`:**

Run `mmk pre-edit <PATH> --format json`. It returns `findings[]`
with HOTSPOT (rank if top-N) and COUPLING (historical co-change
partners above the threshold). If a partner is not part of your
plan, either touch it or say why this edit breaks the pattern.

**After every edit (or batch of edits) before declaring "done":**

Run `mmk review --format json`. It compares the working tree
against HEAD and emits `findings[]`:

- `HOTSPOT` — file you edited is in the top-N hotspot list.
  Tighten the review on this change.
- `COUPLING` — file you edited has a historical partner you did
  not touch. Decide deliberately whether the partner needs the
  matching change.
- `BUDGET` — diff exceeds `bulk.max_files` / `bulk.max_lines` in
  `mokumokuren.toml`. Likely a sweep; consider splitting.

**At end of feature, before opening a PR:**

Run `mmk session-summary --base main --drift-sessions 5 --format json`.
Read `findings[]` for DRIFT (paths that climbed across recent
sessions — a thrashing signal) and BUDGET (session-aggregate cost).
Read `session.entered_top_n` and `session.churn_of_churn` for the
"what shifted while I was working?" view.

If `session.base_resolved_via` is `head_minus_one` or any
`merge_base_*`, the comparison was synthetic — treat the session
block as informational only.

mmk does not validate semantics. Always run tests separately.
```

**Reliability:** advisory. Claude reads it and tries to follow it;
adherence improves when rules are concrete ("if X then Y") rather
than principles ("be careful around hot files").

## Option 2 — Skill (auto-invokes on description match)

Place this at `.claude/skills/mmk-on-edit/SKILL.md` in your repo.
Claude auto-invokes it when its trigger logic matches the
description, or you can call it explicitly as `/mmk-on-edit`:

```markdown
---
name: mmk-on-edit
description: Run mmk to surface Git-history findings about an edit. Use after editing a file to check for missed coupling partners and hotspot risk, before staging, or at end of feature for session summary.
---

# mmk-on-edit

When invoked:

1. **Right after an Edit tool use** — run `mmk review --format json`.
   Surface every finding in `findings[]`. Prioritize COUPLING
   misses: they're the most actionable (partner you didn't touch).

2. **About to edit a file `<PATH>`** — run
   `mmk pre-edit <PATH> --format json` and surface the HOTSPOT and
   COUPLING findings. The partners in COUPLING are files the agent
   should re-read before editing `<PATH>`.

3. **End of feature / wrapping up** — run
   `mmk session-summary --base main --drift-sessions 5 --format json`.
   Surface findings, `session.entered_top_n`,
   `session.churn_of_churn`, and `session.base_resolved_via`.

4. If `session.base_resolved_via` is anything other than `explicit`
   or `since_commit`, mark the session block as informational only.

Treat the output as guidance, not gates. Tests and review still apply.
```

**Reliability:** higher than CLAUDE.md because the skill body only
loads when invoked. Auto-invocation depends on Claude's trigger
logic; explicit `/mmk-on-edit` is always deterministic.

## Option 3 — Hooks (deterministic, strict)

Hooks execute as shell commands when the agent uses specific tools,
and their output is captured into the model's context. The strongest
guarantee.

Add to `.claude/settings.json`:

```json
{
  "hooks": {
    "PostToolUse": [
      {
        "matcher": "Edit",
        "hooks": [
          {
            "type": "command",
            "command": "mmk review --format json 2>/dev/null || true"
          }
        ]
      }
    ],
    "PreToolUse": [
      {
        "matcher": "Edit",
        "hooks": [
          {
            "type": "command",
            "command": "mmk pre-edit \"$CLAUDE_TOOL_INPUT_FILE_PATH\" --format json 2>/dev/null || true"
          }
        ]
      }
    ]
  }
}
```

The `PostToolUse:Edit` hook is the primary integration: every Edit
fires `mmk review`, which sees the working-tree diff including the
edit just made, and feeds findings back into the model's context
before its next turn. The `PreToolUse:Edit` hook is optional —
useful if you want the agent to see the historical-partner list
*before* committing to the edit.

The exact environment-variable name for the tool input may differ by
Claude Code version; check `claude --help` for the current name. The
`|| true` ensures the hook can't fail the tool call if `mmk` isn't
installed.

**Reliability:** deterministic — the command runs every time `Edit`
fires, output enters the agent's context whether the agent "feels
like it" or not. Best fit for CI-grade enforcement; heaviest to
maintain.

## Which one should you use?

| Goal                                                      | Use      |
| --------------------------------------------------------- | -------- |
| "Make mmk available; trust the agent to use it"           | CLAUDE.md |
| "Make mmk discoverable as a named workflow"               | Skill    |
| "Make mmk fire on every edit, no negotiation"             | Hooks    |

Most projects start with CLAUDE.md (zero config beyond the snippet)
and graduate to a hook if they find the agent skipping mmk in
practice.

For non–Claude-Code harnesses, the same patterns translate: a
project-level instruction file, a named tool/skill, or a wrapper
that intercepts edits. The mechanisms differ; the shape — `mmk
pre-edit` for context, `mmk review` for verdict, `mmk
session-summary` for the end-of-feature view — doesn't.
