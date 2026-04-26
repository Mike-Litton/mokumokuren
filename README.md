# Mokumokuren (`mmk`)

![Mokumokuren](docs/img/mokumokuren.png)

Ranks the files in your repository by how much maintenance effort they
consume, using only Git history. The same ranking, computed across an
LLM agent's commits, is a thrashing detector — when one corner of the
codebase dominates the list across sessions, the agent is rewriting it
instead of making progress.

The premise: files that get touched a lot are files that are hard to
get right. Bugs cluster where code keeps changing, and the cost of the
next change is highest where the last ten changes already landed.
Mokumokuren makes that "where" visible.

## Where mmk fits

LLM coding agents are typically guarded by three checks:

1. **Plan / requirements** — usually from a human, defines what should be true.
2. **Static typing and linters** — fast, deterministic, but stateless: they only see the file in front of them.
3. **Automatic tests** — protect against regression, but pass on each iteration of an agent's thrash.

These three structurally cannot see *historical* patterns. A linter
doesn't know which file has been rewritten 200 times this quarter. A
test doesn't know that touching `packfile.c` historically also touches
`object-file.c` — it only fails when the missing edit breaks something.

`mmk` is the fourth check: a **deterministic, fast, computational
metric generator that guards against a specific class of LLM slop**
the other three miss — hotspot blindness, hallucinated coupling,
unexpected cascade, and thrashing. It reads Git history (the one
durable record of what came before) and emits structured findings
a harness or CI gate can act on. It is not a substitute for typing,
linting, or tests; it complements them by supplying the historical
context the other three pillars assume the coder already has.

## What mmk does in the agent edit loop

The agent's actual work happens in the **working tree**, not in
committed history. v0.3 exposes one subcommand per phase of that
loop, each emitting line-by-line, layer-labeled findings:

| Loop phase                 | Subcommand                  | Sees                                                |
| -------------------------- | --------------------------- | --------------------------------------------------- |
| Before editing `<PATH>`    | `mmk pre-edit <PATH>`       | History only — rank, expected partners, optional drift. |
| After every edit           | `mmk review`                | Uncommitted working-tree diff vs history.           |
| Before staging             | `mmk review --staged`       | Staged index vs HEAD.                               |
| Reviewing a committed range| `mmk review --range A..B`   | Committed diff vs history (PR-style review).        |
| End of feature / PR        | `mmk session-summary`       | Committed session vs base + DRIFT/BUDGET overlay.   |
| Across recent sessions     | `mmk drift --sessions K`    | Climb signal across K boundaries (slow path).       |

The `mmk review` hot path is the agent's real edit loop:
`PostToolUse:Edit` → `mmk review` → findings about what just
happened, before the next turn. For a quick view of what it
prints on a typical uncommitted-diff scenario:

```shell
mmk review
```

```
HOTSPOT:
  ⚠ core/a.rs ranks #1 (top-20 hotspot)
COUPLING:
  ⚠ core/a.rs edited; expected partner core/b.rs not touched (jaccard 0.75)
```

JSON output (`--format json`) is the same data with a stable
schema for harness consumers. See
[`docs/claude-code.md`](docs/claude-code.md) for the wiring
(CLAUDE.md / skill / hooks).

## Subcommands at a glance

| Subcommand               | Use it for                                                                  |
| ------------------------ | --------------------------------------------------------------------------- |
| `mmk analyze`            | Ranked top-N hotspots over a window. Triage and CI gating.                  |
| `mmk pre-edit <PATH>`    | Rank, expected partners, optional drift for a path the agent is about to edit. |
| `mmk review`             | Diff vs history. The per-edit hot path; also `--staged` / `--range` / `--commit`. |
| `mmk session-summary`    | End-of-feature view: window vs session, DRIFT + BUDGET overlay.             |
| `mmk drift --sessions K` | Climb signal across K session boundaries. Slow path; PR review.             |
| `mmk eval --sample N`    | Sample N recent commits, aggregate noise-floor report. Tune your config.    |
| `mmk init`               | Write a starter `mokumokuren.toml`. `--profile js-ts` etc. for ecosystems.  |
| `mmk cache`              | Inspect / clear the per-commit delta cache.                                 |

The original `mmk analyze` table:

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

## How to read it

**Top of the list is where defects cluster.** Hotspots concentrate
bugs disproportionately. If a file's been near the top for a while,
it's a refactoring candidate.

**PR touches a top-N hotspot? Tighten review.** A change to a known
hotspot deserves more eyes than a change to a quiet file.
`mmk review --range main..HEAD --gate warn` makes this a CI-able
one-shot.

**Same file on top across multiple agent sessions? Agent is thrashing.**
`mmk drift --sessions 5` puts a number on it.

## Quickstart

| What you want                              | Where to go                                            |
| ------------------------------------------ | ------------------------------------------------------ |
| Wire mmk into Claude Code in 5 minutes     | [`docs/getting-started.md`](docs/getting-started.md)   |
| Configure ignores and `[coupling]`         | [`docs/configuration.md`](docs/configuration.md)       |
| Tune coupling for your repo                | [`docs/coupling.md`](docs/coupling.md) + `mmk eval`    |
| Wire into Claude Code (CLAUDE.md / skill / hooks) | [`docs/claude-code.md`](docs/claude-code.md)    |
| Read the JSON schema                       | [`docs/schema.md`](docs/schema.md)                     |
| Understand each metric                     | [`docs/metrics.md`](docs/metrics.md)                   |

A first analyze on a fresh repo:

```shell
mmk init --profile js-ts   # or rust / python / go / (no --profile for generic)
mmk analyze --top 20
```

## Independent angles

The tool emits more than one number on purpose. Each answers a
question the others can't. Within a layer, fields correlate; across
layers, they decouple.

| Layer                  | Field(s)                                            | Answers                                                                 |
| ---------------------- | --------------------------------------------------- | ----------------------------------------------------------------------- |
| Magnitude              | `weighted_churn`, `commits_touching`, `relative_churn`, `hotspot_score` | "How much, how often, how densely is this file moving?"                 |
| Time                   | `last_modified`                                     | "When was this last touched?"                                           |
| Topology               | `top_couples`, `blast_radius`                       | "If I touch this, what historically co-changes?"                        |
| Distribution / drift   | `commit_entropy`, `churn_of_churn`, `entered_top_n`, `rank_climbs` | "What's the *shape* of the work — spread out, thrashing, shifting?"     |

See [`docs/metrics.md`](docs/metrics.md) for what each metric does
and doesn't measure, and [`docs/coupling.md`](docs/coupling.md) for
the topology layer in depth.

## Install

| Audience | Command |
|---|---|
| macOS / Linux | `curl -LsSf https://github.com/mlitton/mmk/releases/latest/download/mokumokuren-installer.sh \| sh` |
| Windows | `iwr https://github.com/mlitton/mmk/releases/latest/download/mokumokuren-installer.ps1 \| iex` |
| From source | clone the repo, then `cargo install --path mmk-cli --locked` |

Both `mokumokuren` and `mmk` land on `$PATH`.

The first `mmk analyze` on a repository computes per-commit deltas
via gix (~5 s on a 14k-commit JS monorepo); subsequent calls reuse
a per-commit cache and finish in ~300 ms. See
[`docs/performance.md`](docs/performance.md) for cache layout and
`MMK_CACHE_DIR` overrides.

## Known limitations

- **Shallow clones are warned but not rejected.** History before the
  shallow boundary isn't analyzed; the JSON output flags it.
- **Non-UTF8 paths are lossy.** A repository with non-UTF-8 path bytes
  may silently undercount churn for those paths. Vanishingly rare in
  practice; the failure mode is a missing entry, not a wrong ranking.
- **Coupling is empirical, not architectural.** It's the historical
  co-change cone, not a counterfactual model. A file pair that *should*
  co-change but historically hasn't will not appear; one that has but
  shouldn't will.

## Development

Build, test, lint, release: see [`docs/development.md`](docs/development.md).

## License

Dual-licensed under either:

- [MIT License](LICENSE-MIT)
- [Apache License, Version 2.0](LICENSE-APACHE)

at your option.
