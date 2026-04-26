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

mmk is not the only possible source of historical signal — PR
reviews, incident logs, and benchmark drift are also history the
agent lacks. mmk's slice is the git-derived one.

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
schema for harness consumers. See [`docs/claude-code.md`](docs/claude-code.md)
for the wiring (CLAUDE.md / skill / hooks).

The original `mmk analyze` table — ranked top-N hotspots over a
window — is still here too, used in CI gates and one-shot triage:

## `mmk analyze`: the ranked hotspot table

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
hotspot deserves more eyes than a change to a quiet file. `mmk
review --range main..HEAD` makes this a one-shot.

**Same file on top across multiple agent sessions? Agent is thrashing.**
If you're using an LLM agent and the same file dominates the ranking
across runs, the agent is rewriting one corner instead of making
forward progress. `mmk drift --sessions 5` puts a number on it.

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

## `mmk pre-edit`: context before editing a file

Before committing to an edit on `<PATH>`, ask what the history says
about it. Returns `findings[]` with HOTSPOT (rank if top-N) and
COUPLING (historical co-change partners above the threshold):

```shell
mmk pre-edit a.rs
mmk pre-edit a.rs --format json
```

Pre-edit is *informational* — the agent hasn't acted yet, so
COUPLING fires as Info ("you should probably re-read these too"),
not Warn. Add `--drift-sessions 5` to overlay DRIFT findings for
the queried path (slower path, K × analyze cost).

## `mmk session-summary`: end-of-feature review

Renamed from `mmk session` in v0.3 (the old name remains as an
alias). Ranks twice — once over the full `--since` window, once
over commits since a resolved base ref — reports the delta, and
overlays DRIFT (with `--drift-sessions K`) and BUDGET findings:

```shell
mmk session-summary --base main
mmk session-summary --base main --top 10
mmk session-summary --base main --drift-sessions 5
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

The `findings[]` overlay surfaces:

- `BUDGET` — any session-window commit was bulk-filtered (>
  `bulk.max_files` or `bulk.max_lines`), or the surviving session
  aggregate exceeds `2 × max_lines × commits`.
- `DRIFT` (with `--drift-sessions K`) — files climbing in a
  majority of K-1 transitions.

## `mmk drift`: climb signal across recent sessions

Re-runs `analyze` at K historical session-boundary commits (PR-style
merges by default; linear-chunk fallback when there aren't enough
merges) and surfaces files climbing in a majority of K-1 transitions.
Pure function of git state — no persistence:

```shell
mmk drift --sessions 5
mmk drift --sessions 5 --format json
```

Slow path (K × analyze). Intended for end-of-session / PR review,
not the per-edit hook.

## JSON output and schema stability

`--format json` emits a stable schema. The top-level `schema_version`
field tracks the `mmk` minor release (`"0.3.0"` in this build);
consumers (LLM harnesses, CI parsers) pin against it. `crate_version`
is reported separately and is diagnostic only.

```shell
mmk analyze --format json
mmk review --format json
mmk pre-edit a.rs --format json
```

`mmk review`, `mmk pre-edit`, and `mmk session-summary` all carry a
top-level `findings[]` array with the same `{layer, severity,
message}` shape. Layers in v0.3 are `hotspot`, `coupling`, `drift`,
`budget`; `health` and `anchor` are reserved for v0.4.

The full contract is documented in [`docs/schema.md`](docs/schema.md):
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

## Caching

The first `mmk analyze` on a repository computes per-commit deltas via
gix; that takes ~5 s on a 14k-commit JS monorepo. Every subsequent
call reuses a per-commit cache and finishes in ~300 ms.

The cache lives at the OS user cache directory, separate per repository:

- **macOS**: `~/Library/Caches/mmk/<repo-id>/cache.bincode.v1`
- **Linux**: `~/.cache/mmk/<repo-id>/cache.bincode.v1`
- **Windows**: `%LOCALAPPDATA%\mmk\cache\<repo-id>\cache.bincode.v1`

Override with `MMK_CACHE_DIR`. Inspect / clear via:

```shell
mmk cache info     # location, entry count, size
mmk cache clear    # delete cache for the current repo
```

Nothing inside the repository tree changes — no `.gitignore` entries
needed. CI runs are cold by default; pipelines can opt-in by caching
the directory above (e.g. GitHub Actions `actions/cache` keyed on
the cache path).

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
