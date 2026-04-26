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

3. Wire mmk into Claude Code's `PostToolUse:Edit` hook. Add to
   `.claude/settings.json`:
   ```json
   {
     "hooks": {
       "PostToolUse": [
         {
           "matcher": "Edit",
           "hooks": [
             { "type": "command", "command": "mmk review 2>/dev/null || true" }
           ]
         }
       ]
     }
   }
   ```

4. Edit a service file with Claude. The hook fires after every edit;
   the agent sees layer-labeled findings before its next turn:
   ```
   COUPLING:
     ⚠ src/foo.ts edited; expected partner src/foo.test.ts not touched (jaccard 0.67)
   ```

5. After a real session, measure your repo's noise floor:
   ```shell
   mmk eval --sample 50
   ```
   If a majority of findings sit in the 0.10–0.30 jaccard bucket,
   raise `[coupling] threshold` in `mokumokuren.toml`. If a specific
   partner path dominates the noisy-partner list, add it to
   `[coupling] ignore_partners`. See
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

## I want to look at hotspots

```shell
mmk analyze --top 10
```

Read the table. Top of the list is where to look first. See the
[README](../README.md) for the metric, [`metrics.md`](metrics.md) for
when each one starts lying, and [`coupling.md`](coupling.md) for the
co-change layer.

## I want to understand what mmk is doing

- [`metrics.md`](metrics.md) — what each number measures and doesn't.
- [`coupling.md`](coupling.md) — the topology layer.
- [`configuration.md`](configuration.md) — `mokumokuren.toml` reference.
- [`schema.md`](schema.md) — JSON schema for harness consumers.
- [`performance.md`](performance.md) — caching, cold-vs-warm cost.
