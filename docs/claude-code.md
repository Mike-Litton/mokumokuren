# Wiring `mmk` into Claude Code

If you're using [Claude Code](https://claude.com/claude-code), you can
make `mmk` self-acting in three ways, in increasing order of strictness.
Pick whichever matches how rigid you want the integration to be.

All three assume `mmk` is on `$PATH` (`cargo install --path mmk-cli`,
or use a published binary).

## Option 1 — `CLAUDE.md` (advisory, easiest)

Auto-loads per project. Modern Claude follows well-written instructions
without further orchestration. Drop the snippet below into your repo's
`CLAUDE.md` (or merge into an existing one — keep total file length
under ~200 lines for adherence):

```markdown
## Using mmk for editing decisions

This project uses [mmk](https://github.com/mlitton/mmk) — a
deterministic Git-history sensor that catches LLM slop the linter
and tests can't see (hotspot blindness, hallucinated coupling,
thrashing). Use it before non-trivial edits and after a session of
commits.

**Before editing a file `<PATH>`:**

1. Run `mmk analyze --couples-of <PATH> --format json`. The
   `couples_of.entries` array lists files that historically
   co-change with `<PATH>`.
2. If your planned edit logically should also touch a high-jaccard
   partner (≥ 0.5) but you're not, either touch it or explain why
   this edit breaks the historical pattern.
3. Run `mmk analyze --top 5 --format json` once per session. If
   `<PATH>` is in the top 5, slow down — those files concentrate
   bugs disproportionately. Tighten review on edits there.

**After a session of commits, before declaring "done":**

1. Run `mmk session --base main --format json` (substitute your
   base ref).
2. If `session.base_resolved_via` is `head_minus_one` or starts
   with `merge_base_`, the comparison was synthetic — treat the
   session block as informational only.
3. Read `session.entered_top_n`. Files there are your session's
   center of mass; worth a focused re-read.
4. Read `session.churn_of_churn`. Ratios near 1.0 mean you wrote
   then unwrote code in the same file. Confirm the final state is
   the intended one.
5. Read `session.commit_entropy`. Below ~0.3 means one commit
   dominates; that commit deserves a separate review pass.

mmk does not validate semantics. Always run tests separately.
```

**Reliability:** advisory. Claude reads it and tries to follow it; the
docs are explicit that there's "no guarantee of strict compliance,
especially for vague or conflicting instructions." Phrasing as concrete
rules ("if X then Y") rather than principles ("be careful around hot
files") meaningfully improves adherence.

## Option 2 — Skill (auto-invokes on description match)

Place this at `.claude/skills/mmk-review/SKILL.md` in your repo. Claude
will auto-invoke it when its internal trigger logic matches the
description (or you can call it explicitly as `/mmk-review`):

```markdown
---
name: mmk-review
description: Run mmk to surface Git-history signals before editing a file or after a session of commits. Use when about to edit, when about to commit, or when checking what shifted in the current session.
---

# mmk-review

When invoked:

1. If we're about to edit a file `<PATH>`:
   - Run `mmk analyze --couples-of <PATH> --format json` and
     surface the top 5 partners.
   - Run `mmk analyze --top 5 --format json`. If `<PATH>` is in
     the top-5 ranking, flag it.
2. If we're at the end of a session (about to commit, or wrapping
   up a feature):
   - Run `mmk session --base main --format json` (or whatever the
     project's base ref is).
   - Surface `entered_top_n`, `churn_of_churn`, `commit_entropy`,
     and `base_resolved_via`.
3. If `base_resolved_via` is anything other than `explicit` or
   `since_commit`, mark the session block as informational only.

Treat the output as guidance, not gates. Tests and review still apply.
```

**Reliability:** higher than CLAUDE.md because the skill body only
loads when invoked — keeps it focused. Auto-invocation depends on
Claude's internal trigger logic; explicit `/mmk-review` is always
deterministic.

## Option 3 — Hooks (deterministic, strict)

Hooks execute as shell commands when the agent uses specific tools, and
their output is captured into the model's context. This is the
strongest guarantee.

Add to `.claude/settings.json`:

```json
{
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "Edit",
        "hooks": [
          {
            "type": "command",
            "command": "mmk analyze --couples-of \"$CLAUDE_TOOL_INPUT_FILE_PATH\" --format json --top 5 2>/dev/null || true"
          }
        ]
      }
    ]
  }
}
```

The exact environment-variable name for the tool input differs by
Claude Code version; check `claude --help` or the official docs for
the current name. The `|| true` ensures the hook can't fail the tool
call if `mmk` isn't installed or the path is invalid.

**Reliability:** deterministic — the command runs every time `Edit`
fires, and the output enters the agent's context whether the agent
"feels like it" or not. Best fit for CI-grade enforcement; heaviest
to maintain.

## Which one should you use?

| Goal | Use |
|---|---|
| "Make mmk available; trust the agent to use it" | CLAUDE.md |
| "Make mmk discoverable as a named workflow" | Skill |
| "Make mmk fire on every edit, no negotiation" | Hooks |

Most projects start with CLAUDE.md (zero config beyond the snippet) and
graduate to a hook if they find the agent skipping mmk in practice.

For non–Claude-Code harnesses, the same patterns translate: a
project-level instruction file, a named tool/skill, or a wrapper that
intercepts edits. The mechanisms differ; the shape doesn't.
