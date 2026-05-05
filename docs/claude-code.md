# Wiring `mmk` into Claude Code

If you're using [Claude Code](https://claude.com/claude-code), you can
make `mmk` self-acting in three ways. Pick the strictness that fits.

All three assume `mmk` is on `$PATH` (`cargo install --path mmk-cli`,
or use a published binary).

## Working-tree vs history

The agent's actual edit loop happens in the **working tree**, not in
committed history. mmk exposes one subcommand per phase of that
loop:

| Loop phase                   | Subcommand                  | Sees                                         |
| ---------------------------- | --------------------------- | -------------------------------------------- |
| Before editing `<PATH>`      | `mmk pre-edit <PATH>`       | History only (rank, expected partners).      |
| After every edit             | `mmk review`                | Uncommitted working-tree diff vs history.    |
| Before staging               | `mmk review --staged`       | Staged index vs HEAD.                        |
| End of feature / PR review   | `mmk session-summary`       | Committed session vs base ref + drift / budget overlay. |

`mmk analyze` (the ranked top-N table) and `mmk drift` (climb signal
across K sessions) operate at coarser grain — useful in CI gates and
triage, not in the per-edit hook.

## Option 0 — Plugin (recommended)

If you're on a recent Claude Code, the cleanest path is the
in-repo marketplace:

```
/plugin marketplace add Mike-Litton/mokumokuren
/plugin install mokumokuren@mokumokuren-plugins
```

That wires the `PostToolUse:Edit|Write` review hook, the
`PostToolUse:Bash(git commit:*)` session-summary hook, the
`mmk-on-edit` and `mmk-findings` skills, and the `mmk-assessor`
subagent in one step. Plugin defaults to advisory
(`additionalContext`); for `--gate warn` strict mode, layer a
project-level hook on top — see Option 3.

The plugin only wires the integration; the `mmk` binary still
ships separately (installer / `cargo install`). Plugin source
lives at [`plugins/mokumokuren/`](../plugins/mokumokuren/).

## Option 1 — `CLAUDE.md` (advisory, easiest)

Auto-loads per project. Copy
[`agent-claude-md-template.md`](agent-claude-md-template.md) into
your repo's `CLAUDE.md` (or merge into an existing one). The
template covers invocation pattern, vertical-slice / commit-per-slice
discipline, override discipline, and per-sensor priors — calibrated
for adherence, not length.

**Reliability:** advisory. Claude reads it and tries to follow it;
adherence improves when rules are concrete ("if X then Y") rather
than principles ("be careful around hot files").

## Option 2 — Skill (auto-invokes on description match)

Place a skill at `.claude/skills/mmk-on-edit/SKILL.md` in your repo.
Claude auto-invokes it when its trigger logic matches the
description, or you can call it explicitly as `/mmk-on-edit`.

The canonical body is the one the plugin (Option 0) installs:
[`plugins/mokumokuren/skills/mmk-on-edit/SKILL.md`](../plugins/mokumokuren/skills/mmk-on-edit/SKILL.md).
Copy it into your repo if you want a single skill without the
plugin's hooks, or want to customize the workflow.

A second skill, `mmk-findings`
([source](../plugins/mokumokuren/skills/mmk-findings/SKILL.md)),
covers the interpretation side — sensor priors, override
discipline, reading silence — and auto-invokes when findings are
present.

**Reliability:** higher than CLAUDE.md because the skill body only
loads when invoked. Auto-invocation depends on Claude's trigger
logic; explicit `/mmk-on-edit` is always deterministic.

## Option 3 — Hooks (deterministic, strict)

Hooks execute as shell commands when the agent uses specific tools,
and their output is captured into the model's context. The strongest
guarantee.

mmk reads Claude Code's documented hook envelope on stdin —
`{session_id, transcript_path, cwd, hook_event_name, tool_input:
{file_path, ...}}` — and emits its findings back through the
documented hook output channels (`hookSpecificOutput.
additionalContext` for non-blocking inject; `decision: "block"` +
`reason` when the user opts into hard-yield via `--gate warn`). No
environment-variable plumbing required.

Add to `.claude/settings.json`:

```json
{
  "hooks": {
    "PostToolUse": [
      {
        "matcher": "Edit|Write",
        "hooks": [
          {
            "type": "command",
            "command": "mmk review"
          }
        ]
      }
    ],
    "PreToolUse": [
      {
        "matcher": "Edit|Write",
        "hooks": [
          {
            "type": "command",
            "command": "mmk pre-edit"
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
through `hookSpecificOutput.additionalContext`. The
`PreToolUse:Edit` hook is optional — useful if you want the agent
to see the historical-partner list *before* committing to the edit.
mmk auto-detects "I was invoked from a hook" by the presence of the
JSON envelope on stdin, so the same binary works for manual
invocation (no envelope → text or `--format json` output) and hook
invocation (envelope → hook-shape JSON).

When mmk runs but its dedup gate has already emitted these same
findings against the same HEAD within the TTL, hook output carries
a top-level `systemMessage` ("mmk: prior findings unchanged since
HEAD <sha7>") so the agent can distinguish "consulted, quiet" from
"wasn't run." That replaces the silent no-op older `2>/dev/null ||
true` recipes used to mask.

### Wiring `session-summary` to `git commit`

`mmk session-summary` is the post-commit / pre-PR view (window vs
session, DRIFT + BUDGET overlay) — it reads committed state, so
running it on uncommitted work returns a nudge toward `mmk review`.
The `PostToolUse:Bash(git commit:*)` matcher fires it automatically
the moment a commit lands, closing the gap where session-summary
otherwise has to be remembered as an explicit step.

```jsonc
{
  "hooks": {
    "PostToolUse": [
      {
        "matcher": "Bash(git commit:*)",
        "hooks": [
          {
            "type": "command",
            "command": "mmk session-summary --base $(git symbolic-ref --quiet refs/remotes/origin/HEAD | sed 's@.*/@@' || echo main) --drift-sessions 5"
          }
        ]
      }
    ]
  }
}
```

The `git symbolic-ref` substitution resolves the upstream's default
branch when present (`origin/HEAD` → `main` / `master` /
whatever); the `|| echo main` fallback handles repos where
`origin/HEAD` isn't set, so the hook works on freshly-cloned trees
and offline repos without manual `--base` configuration.

### CI gating with `--gate`

For CI, the same subcommands accept `--gate {none, warn, error}`:

```shell
mmk review --range main..HEAD --gate warn
```

`--gate warn` exits 2 if any warn-severity finding fires, 0 if the
diff is clean, 1 if mmk itself errors. The exit-2 / exit-1 split
lets a CI pipeline distinguish "policy failed" from "tool crashed."
`--gate error` is reserved for future severity tiers and behaves
like `none` today.

Inside a hook, `--gate warn` switches the output shape: instead of
`additionalContext`, mmk emits `decision: "block"` + `reason`,
which Claude Code surfaces as a hard yield to the agent. This is
the strict-deployment knob — the default hook recipe stays
advisory.

Same flag works on `mmk pre-edit` and `mmk session-summary`.

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
