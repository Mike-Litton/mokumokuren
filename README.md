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

The metric is recency-weighted churn × file size (lines of code).
Ignore patterns are configured per-repo via `mokumokuren.toml` — the
tool ships with no ecosystem-specific defaults because there is no
ecosystem-neutral right answer.

## Two ways to use it

`mmk` is a **deterministic, sub-second sensor** in two contexts:

- **LLM agent inner loop.** The agent runs `mmk analyze --format json`
  before deciding what to edit, or `mmk session --base main` to ask
  "what shifted while I was working?" The output is structured JSON
  with a stable schema (`schema_version`) the agent's harness pins
  against.
- **CI/CD pipeline + human review.** A pipeline step runs `mmk session`
  on each PR and a human reviewer reads the report. The synthetic-base
  warning (`fallback`/`synthetic` in `repo.warnings`) is greppable so
  CI can flag "this run wasn't comparing against a real base" for the
  reviewer.

The same binary, the same numbers, two consumers.

## What you see

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
  exponentially decayed by commit age (more recent edits weigh more).
- `commits` — number of commits that touched the file in the window.
- `hotspot` — `log(1 + weighted_churn) × log(1 + loc)`. Larger files
  with sustained churn rank highest; one-line tweaks to a 10k-LOC file
  beat 100-line rewrites to a 50-LOC stub.

The top of the list is where to look first.

## Independent angles

The tool emits more than one number on purpose. Each answers a
question the others can't. Within a layer, fields correlate; across
layers, they decouple — so one of them moving while the others stay
flat is meaningful.

| Layer                  | Field(s)                                            | Answers                                                                 |
| ---------------------- | --------------------------------------------------- | ----------------------------------------------------------------------- |
| Magnitude              | `weighted_churn`, `commits_touching`, `relative_churn`, `hotspot_score` | "How much, how often, how densely is this file moving?"                 |
| Time                   | `last_modified`                                     | "When was this last touched?" (a file can be hot but cold, or quiet but warm) |
| Topology               | `top_couples`, `blast_radius`                       | "If I touch this, what historically co-changes?"                        |
| Distribution / drift   | `commit_entropy`, `churn_of_churn`, `entered_top_n`, `rank_climbs` | "What's the *shape* of the work — spread out, thrashing, shifting?"     |

Volume tells you nothing about thrash; thrash tells you nothing about
coupling; coupling tells you nothing about distribution. Cross-check
across layers when something looks important.

See [`docs/metrics.md`](docs/metrics.md) for what each metric does
and doesn't measure, when each one starts lying, and how to combine
them for the agent inner loop, CI/CD review, and one-shot triage.

## How to read it

**Top of the list is where defects cluster.** Hotspots concentrate
bugs disproportionately. If a file's been near the top for a while,
it's a refactoring candidate.

**PR touches a top-N hotspot? Tighten review.** A change to a known
hotspot deserves more eyes than a change to a quiet file.

**Same file on top across multiple agent sessions? Agent is thrashing.**
If you're using an LLM agent and the same file dominates the ranking
across runs, the agent is rewriting one corner instead of making
forward progress. Either review the changes carefully or reset the
agent's context.

## Quickstart

```shell
mmk analyze --top 20
mmk analyze --since 90days
mmk analyze --ignore 'docs/**'
```

`mmk init` scaffolds a starter `mokumokuren.toml` in the current
directory; commit it.

## Coupling: what historically co-changes

`--couples-of <PATH>` answers "if I touch this file, what historically
changes alongside it?" The metric is Jaccard similarity of co-changing
file sets, capped at 1.0:

```shell
mmk analyze --couples-of a.rs
```

Each ranked entry in the JSON output also carries a `top_couples`
array — the same data, attached per file. In text mode the
co-change blocks are off by default (one-line-per-file table stays
grep-friendly); pass `--couples` to render them inline:

```shell
mmk analyze --couples
```

## Blast radius: 1-hop co-change neighborhood

`--blast-radius <PATH>` emits an explicit graph of partners with
Jaccard ≥ a threshold. Useful when the agent is about to edit a file
and wants to know what *else* it should re-read:

```shell
mmk analyze --blast-radius a.rs
mmk analyze --blast-radius a.rs --blast-radius-threshold 0.05
```

The threshold is the minimum Jaccard a partner must reach to be
included. Defaults to `0.10`. Override per call with
`--blast-radius-threshold`, or pin per-repo in `mokumokuren.toml`:

```toml
[blast_radius]
threshold = 0.10
```

The effective threshold is echoed in the JSON output (`blast_radius.threshold`)
so consumers can see what filter produced the listed nodes.

## Sessions: what shifted since I started

`mmk session` ranks twice — once over the full `--since` window, once
over commits since a resolved base ref — and reports the delta:

```shell
mmk session --base main
mmk session --base main --top 10
```

The base resolution cascade is `--base` / `--since-commit` →
`merge-base(HEAD, origin/main)` → `main` → `origin/master` →
`master` → `HEAD~1`. The last step is **synthetic** — when the
cascade falls all the way to `HEAD~1`, the JSON's
`session.base_resolved_via` is the literal `"head_minus_one"` and a
warning containing `fallback` lands in `repo.warnings`. A CI gate
keying on either of those signals can refuse synthetic results.

The `session` block reports `entered_top_n` (files newly in the top
ranking compared to the window), `rank_climbs`, `churn_of_churn`
(add/delete thrash ratio), and `commit_entropy` (Shannon entropy of
files-touched-per-commit, normalized to `[0, 1]` — uniform → 1.0,
concentrated bulk-edits → near 0).

`session_files[].loc` is line count at the **session base** (not at
HEAD), so `session.relative_churn = session_weighted_churn /
base_LOC` reflects "fraction of the file touched during the session,
relative to its size at session start" — a file truncated post-session
doesn't get an inflated ratio. See [`docs/schema.md`](docs/schema.md)
for the full epoch contract and field reference.

## JSON output and schema stability

`--format json` emits a stable schema. The top-level `schema_version`
field tracks the `mmk` minor release; consumers (LLM harnesses, CI
parsers) pin against it. `crate_version` is reported separately and
is diagnostic only.

```shell
mmk analyze --format json
```

The contract is documented in [`docs/schema.md`](docs/schema.md):
additive changes (new optional fields, new top-level blocks) do not
bump `schema_version`; renames, removals, type changes, and semantic
changes do.

## Configuring ignores

A repo-local `mokumokuren.toml` at the Git root is auto-discovered:

```toml
ignore = [
    "po/**",
    "Cargo.lock",
    "vendor/**",
]

[blast_radius]
threshold = 0.10
```

Without a config file, every tracked text file is included — which on
most real repos surfaces noise (translation files, vendored
dependencies, lockfiles) at the top of the ranking. Run `mmk init` to
scaffold a starter template with commented-out examples covering the
common cases (translations, vendored, lockfiles, generated, engine
assets); uncomment what applies.

Some patterns that often pay off, by ecosystem:

- **Rust:** `Cargo.lock`, `target/**`
- **JavaScript / Node:** `node_modules/**`, `package-lock.json`,
  `yarn.lock`, `pnpm-lock.yaml`, `dist/**`, `.next/**`
- **Python:** `__pycache__/**`, `*.pyc`, `poetry.lock`,
  `**/migrations/**` (Django, often)
- **Ruby / Rails:** `Gemfile.lock`, `vendor/bundle/**`, `tmp/**`,
  `log/**`
- **Go:** `go.sum`, `vendor/**`
- **iOS / Swift:** `Pods/**`, `*.pbxproj`
- **Game engines:** `*.tscn`, `*.tres` (Godot); `*.unity` (Unity);
  `*.uasset`, `*.umap` (Unreal)
- **Translations (any ecosystem):** `**/*.po`, `**/*.pot`,
  `**/locale/**`, `**/Localization/**`

Ignore patterns are not portable across repos — what's noise in one
project is signal in another (`migrations/` is mechanical for one
team, hand-authored for another). The point of `mokumokuren.toml` is
that the call belongs to the repo's maintainers, not the tool.

## Install

| Audience | Command |
|---|---|
| macOS / Linux | `curl -LsSf https://github.com/mlitton/mmk/releases/latest/download/mokumokuren-installer.sh \| sh` |
| Windows | `iwr https://github.com/mlitton/mmk/releases/latest/download/mokumokuren-installer.ps1 \| iex` |
| From source | clone the repo, then `cargo install --path mmk-cli --locked` |

Both `mokumokuren` and `mmk` land on `$PATH`.

## Known limitations

- **Single snapshot per run.** No persistence between runs beyond what
  `mmk session --since-commit <SHA>` enables; no caching.
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
