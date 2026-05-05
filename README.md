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
metric generator** that catches the failure modes the other three
structurally cannot see — hotspot blindness, co-change blindness,
unexpected cascade, and thrashing. It reads Git history (the one
durable record of what came before) and emits structured findings
a harness or CI gate can act on. It is not a substitute for typing,
linting, or tests; it complements them by supplying the historical
context the other three pillars assume the coder already has.

## What mmk does in the agent edit loop

The agent's actual work happens in the **working tree**, not in
committed history. mmk exposes one subcommand per phase of that
loop, each emitting line-by-line, sensor-labeled findings:

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

The full menu of sensors `mmk review` and `mmk pre-edit` emit:

| Sensor       | Question                                                                                          | Severity         |
| ------------ | ------------------------------------------------------------------------------------------------- | ---------------- |
| `HOTSPOT`    | "Is this file near the top of the rank?"                                                          | Warn             |
| `COUPLING`   | "Is a historical co-change partner missing from this diff?"                                       | Warn / Info      |
| `COHESION`   | "Does this diff decompose into multiple disjoint clusters?"                                       | Warn (v0.7)      |
| `STRUCTURE`  | "Does this file diverge from its directory's import / export shape?"                              | Warn / Info / Ok |
| `COMPLEXITY` | "Is this function structurally over the cap (nesting / LOC)?"                                    | Warn             |
| `HEALTH`     | "Is the test pair / registration peer / service consumer absent? Did a broad catch get added?"   | Warn / Info      |
| `EVASION`    | "Did this diff add a broad TS/JS catch handler not in HEAD?" (v0.7, under `HEALTH.broad_exception`) | Warn             |
| `BUDGET`     | "Is the diff over the size cap, or ramping toward it?"                                            | Warn / Info      |
| `DRIFT`      | "Is this file climbing the rank across recent sessions?"                                          | Warn             |

JSON output (`--format json`) is the same data with a stable
schema for harness consumers. See
[`docs/claude-code.md`](docs/claude-code.md) for the wiring
(CLAUDE.md / skill / hooks) and
[`docs/schema.md`](docs/schema.md) for the JSON envelope.

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
| Wire mmk into Claude Code with one command | `/plugin marketplace add Mike-Litton/mokumokuren && /plugin install mokumokuren@mokumokuren-plugins` ([README](plugins/mokumokuren/README.md)) |
| Wire mmk into Claude Code in 5 minutes     | [`docs/getting-started.md`](docs/getting-started.md)   |
| Drop-in agent guidance (CLAUDE.md content) | [`docs/agent-claude-md-template.md`](docs/agent-claude-md-template.md) |
| Configure ignores and `[coupling]`         | [`docs/configuration.md`](docs/configuration.md)       |
| Tune coupling for your repo                | [`docs/coupling.md`](docs/coupling.md) + `mmk eval`    |
| Wire into Claude Code (plugin / CLAUDE.md / skill / hooks) | [`docs/claude-code.md`](docs/claude-code.md) |
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
| macOS / Linux | `curl -LsSf https://github.com/Mike-Litton/mokumokuren/releases/latest/download/mokumokuren-installer.sh \| sh` |
| Windows | `iwr https://github.com/Mike-Litton/mokumokuren/releases/latest/download/mokumokuren-installer.ps1 \| iex` |
| From source | clone the repo, then `cargo install --path mmk-cli --locked` |
| Claude Code plugin (after binary install) | `/plugin marketplace add Mike-Litton/mokumokuren` then `/plugin install mokumokuren@mokumokuren-plugins` ([details](plugins/mokumokuren/README.md)) |

Both `mokumokuren` and `mmk` land on `$PATH`.

The first `mmk analyze` on a repository computes per-commit deltas
via gix (~5 s on a 14k-commit JS monorepo); subsequent calls reuse
a per-commit cache and finish in ~300 ms. See
[`docs/performance.md`](docs/performance.md) for cache layout and
`MMK_CACHE_DIR` overrides.

Known operational limits (shallow clones, non-UTF-8 paths, empirical
coupling) are documented in [`docs/metrics.md`](docs/metrics.md).

## Development

Build, test, lint, release: see [`docs/development.md`](docs/development.md).

## License

Dual-licensed under either:

- [MIT License](LICENSE-MIT)
- [Apache License, Version 2.0](LICENSE-APACHE)

at your option.
