# Getting started

Pick the path that matches what you're trying to do.

## I want to use mmk with Claude Code on a JS/TS repo (5 minutes)

1. Install mmk:
   ```shell
   cargo install --path mmk-cli --locked
   ```
   (or grab a release binary — see [Install](../README.md#install).)

2. From your repo root:
   ```shell
   cd your-repo
   mmk init --profile js-ts
   ```
   This writes a `mokumokuren.toml` calibrated for JS/TS monorepos
   (workspace-package.json suppression, lockfile ignores, sane
   `[coupling]` defaults, plus the `[health.ts]` structural-pattern
   adapter enabled). Commit it.

3. Wire mmk into Claude Code's `PostToolUse` hook. Add to
   `.claude/settings.json`:
   ```json
   {
     "hooks": {
       "PostToolUse": [
         {
           "matcher": "Edit|Write",
           "hooks": [
             { "type": "command", "command": "mmk review" }
           ]
         }
       ]
     }
   }
   ```
   For drop-in CLAUDE.md content describing how the agent should
   read mmk's output, copy
   [`agent-claude-md-template.md`](agent-claude-md-template.md) into
   your repo's `CLAUDE.md`. To wire `mmk session-summary` to fire
   automatically on every commit, see
   [`claude-code.md` § Wiring `session-summary` to `git commit`](claude-code.md#wiring-session-summary-to-git-commit).

4. Edit a service file with Claude. The hook fires after every edit;
   the agent sees layer-labeled findings before its next turn:
   ```
   COUPLING:
     ⚠ src/foo.ts edited; src/foo.test.ts co-edited 16 of 24 prior commits, not in diff [id=coupling:src/foo.ts:src/foo.test.ts]
   ```

5. After a real session, measure your repo's noise floor:
   ```shell
   mmk eval --sample 50 --learn
   ```
   `wilson_lower_buckets` shows the COUPLING confidence
   distribution; if most fires sit below your
   `confidence_threshold`, raise it. `learn_suggestions` flags
   broad-partner paths to add to `[coupling] ignore_partners`. See
   [`coupling.md`](coupling.md) for the tuning approach.

Why text mode (no `--format json`) in the hook command? `mmk review`
emits the same findings as a few hundred bytes of text instead of
~1.5kB of JSON envelope per fire. Across a 50-edit session that's
the difference between ~8kB and ~75kB of injected context — measurably
better for any context-limited model. Switch to `--format json` only
if your harness genuinely consumes the structured shape.

## I want to use mmk with another LLM harness

The shape — `mmk pre-edit <PATH>` for context, `mmk review` for
verdict, `mmk session-summary` at end of feature — is the same. The
mechanism (named tool, slash command, instruction file, wrapper)
depends on the harness. See [`claude-code.md`](claude-code.md) for
patterns; translate as needed.

## I want to gate on mmk in CI

Use `--gate warn` to make `mmk review` exit non-zero when any
warn-severity finding fires:

```shell
mmk review --range main..HEAD --gate warn
```

Exit code semantics:

| `--gate` | Exit 0 | Exit 1 (mmk error) | Exit 2 (gate triggered) |
| -------- | ------ | ------------------ | ----------------------- |
| `none` (default) | always (unless mmk errors) | mmk error | n/a |
| `warn`   | no warn-severity findings | mmk error | any warn finding fired  |
| `error`  | reserved for future severity tiers | mmk error | n/a today |

Same flag works on `mmk pre-edit` and `mmk session-summary`.

## I want a pre-commit gate against tangled diffs

`mmk review --staged --gate warn` reads the staged index and exits 2
when COHESION (or any other warn-severity sensor) fires:

```shell
$ git add src/auth/login.ts src/auth/session.ts
$ git add src/billing/invoice.ts src/billing/plan.ts
$ mmk review --staged --gate warn
COHESION:
  ⚠ staged diff decomposes into 2 disjoint co-change clusters (2 + 2 files)
$ echo $?
2
```

Wire it into `.git/hooks/pre-commit` (or your `pre-commit-config.yaml`)
to block tangled commits before they land. The structured
`cohesion.tangles[].clusters[]` JSON block (`--format json`) carries
the full per-cluster path decomposition so a wrapper can propose
the split.

As of v0.7 COHESION is Warn-severity (was Info pre-v0.7). Empirical
grounding: MSR 2026 *"LGTM!"* (Canelas et al.) shows auto-merged PRs
across the AIDev corpus are smaller and more focused than
non-auto-merged ones — the structural property COHESION detects.

## I want to look at hotspots

```shell
mmk analyze --top 10
```

Read the table. Top of the list is where to look first. See the
[README](../README.md) for the metric, [`metrics.md`](metrics.md) for
when each one starts lying, and [`coupling.md`](coupling.md) for the
co-change layer.

## Subcommands

| Subcommand               | Use it for                                                                  |
| ------------------------ | --------------------------------------------------------------------------- |
| `mmk analyze`            | Ranked top-N hotspots over a window. Triage and CI gating.                  |
| `mmk pre-edit <PATH>`    | Rank, expected partners, optional drift for a path the agent is about to edit. |
| `mmk review`             | Diff vs history. The per-edit hot path; also `--staged` / `--range` / `--commit`. |
| `mmk explain --finding <id>` | Per-commit evidence behind a specific COUPLING finding. On-demand drill-down. |
| `mmk session-summary`    | End-of-feature view: window vs session, DRIFT + BUDGET overlay.             |
| `mmk drift --sessions K` | Climb signal across K session boundaries. Slow path; PR review.             |
| `mmk eval --sample N`    | Sample N recent commits, aggregate noise-floor report. Tune your config.    |
| `mmk init`               | Write a starter `mokumokuren.toml`. `--profile js-ts` etc. for ecosystems.  |
| `mmk cache`              | Inspect / clear the per-commit delta cache.                                 |

The `mmk analyze` table:

```shell
mmk analyze --top 10
```

```
rank  path                              loc  weighted_churn   commits       hotspot
----  ---------------------------  --------  --------------  --------  ------------
   1  mmk-git/tests/analyze.rs          462          369.00         1         36.30
   2  mmk-git/src/diff.rs               269          275.00         1         31.47
   3  mmk-git/src/lib.rs                206          131.00         1         26.04
   ...
```

- `weighted_churn` — added + deleted lines across the analysis window,
  exponentially decayed by commit age.
- `hotspot` — `log(1 + weighted_churn) × log(1 + loc)`. Larger files
  with sustained churn rank highest.

The top of the list is where to look first.

## I want to understand what mmk is doing

- [`metrics.md`](metrics.md) — what each number measures and doesn't.
- [`coupling.md`](coupling.md) — the topology layer.
- [`configuration.md`](configuration.md) — `mokumokuren.toml` reference.
- [`schema.md`](schema.md) — JSON schema for harness consumers.
- [`performance.md`](performance.md) — caching, cold-vs-warm cost.
