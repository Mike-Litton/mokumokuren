# Configuring mmk

A repo-local `mokumokuren.toml` at the Git root is auto-discovered.
Without one, every tracked text file is included in the ranking —
which on most real repos surfaces noise (translations, vendored
dependencies, lockfiles) at the top.

## Bundled profiles

`mmk init` writes a starter file. Pass `--profile <NAME>` to use an
opinionated, ecosystem-tuned starting point instead of the generic
template:

| Profile  | When to use                                                      |
| -------- | ---------------------------------------------------------------- |
| (none)   | Generic template with commented-out examples per ecosystem.      |
| `js-ts`  | Node / npm / yarn / pnpm projects, esp. monorepos.               |
| `rust`   | `cargo` workspaces.                                              |
| `python` | Poetry / pip / uv projects.                                      |
| `go`     | Go modules.                                                      |

```shell
mmk init --profile js-ts
```

Profiles are deliberately conservative — they ship the ignore globs
and `[coupling]` defaults derived from the v0.3 four-repo eval, but
nothing more. If a profile doesn't fit, edit the resulting
`mokumokuren.toml` directly; it's a checked-in file you own.

## Top-level `ignore`

A list of glob patterns. Files matching any pattern are excluded
from the hotspot ranking and from coupling history. Patterns are
unioned with `--ignore` flags on the command line.

```toml
ignore = [
    "po/**",
    "Cargo.lock",
    "vendor/**",
]
```

Some patterns that often pay off, by ecosystem:

- **Rust:** `Cargo.lock`, `target/**`
- **JavaScript / Node:** `node_modules/**`, `package-lock.json`,
  `yarn.lock`, `pnpm-lock.yaml`, `dist/**`, `.next/**`, `**/*.d.ts`
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
team, hand-authored for another). The point of `mokumokuren.toml`
is that the call belongs to the repo's maintainers, not the tool.

## `[coupling]`

Controls COUPLING findings emitted by `mmk review` and
`mmk pre-edit`. See [`coupling.md`](coupling.md) for the design
rationale (Wilson 95 % lower bound on conditional probability).

```toml
[coupling]
confidence_threshold = 0.30
min_sample_size      = 3
ignore_partners = [
    "**/package.json",
    "**/*-lock.json",
]
```

| Field                  | Default | Notes                                                                  |
| ---------------------- | ------- | ---------------------------------------------------------------------- |
| `confidence_threshold` | `0.30`  | Minimum Wilson 95 % lower bound on `P(partner | subject)` for COUPLING to fire. v0.6 calibration bumped this from `0.20` (which let `wilson_lower(1, 1) ≈ 0.206` clear and surface single-observation co-edits). |
| `min_sample_size`      | `3`     | Minimum `commits_touching(subject)` before COUPLING infers anything. v0.6 calibration bumped this from `1`; below 3, a 2-of-2 pair has Wilson ≈ 0.34 and would fire on a single coincidental co-edit. Pre-edit emits an OK fall-through finding when the floor isn't met. |
| `ignore_partners`      | `[]`    | Glob list — paths that never fire as the *missed partner*.             |
| `threshold`            | (alias) | Deprecated. Silently mapped to `confidence_threshold`; `--verbose` surfaces a one-line note. |

CLI overrides: `--coupling-threshold <FLOAT>` on review / pre-edit /
session-summary (also routed to `confidence_threshold`).

## `[bulk]`

Controls the per-commit / per-diff size guardrails. BUDGET ships
two complementary thresholds calibrated independently:

- **Per-diff cap** (`max_lines` / `max_files`, default 1000 / 15):
  *agentic-context safety*. Diffs over the cap silence
  HOTSPOT/COUPLING because their signal collapses at scale, and
  the analyzer drops over-cap commits from the historical
  baseline. Warn at 75 %, Over at 100 %.
- **Review-effectiveness floor** (`review_quality_lines`, default
  200): an absolute LOC threshold past which defect-detection
  during code review degrades sharply. Empirical lineage: Jureczko
  2020 (*Code review effectiveness: an empirical study on selected
  factors influence*, IET Software,
  [doi:10.1049/iet-sen.2020.0134](https://digital-library.theiet.org/doi/full/10.1049/iet-sen.2020.0134))
  replicates the SmartBear/Cisco case-study finding under controls
  for developer ability and team dynamics; Demeyer et al. 2024
  (*Developer perceptions of modern code review processes in
  practice*, JSS) reaffirms patch size as the dominant lever on
  review duration / comment density. v0.10 surfaces the threshold
  as an Info-severity floor below the cap so the agent sees the
  slice-boundary cue at the empirical threshold, not at 50 % of
  an engineering-heuristic ceiling.

The two thresholds encode different evidence and stay independent:
raising the cap doesn't drag the floor up proportionally — that
would silently weaken the floor. The file cap is an engineering
heuristic; the published change-review literature is almost
entirely LOC-based and there is no peer-reviewed file-count
threshold to derive a default from.

```toml
[bulk]
max_files            = 15
max_lines            = 1000
review_quality_lines = 200
greenfield_threshold = 0.5
ignore_for_budget = [
    "**/routeTree.gen.ts",
    "**/*.pb.go",
]
```

| Field                  | Default | Notes                                                                       |
| ---------------------- | ------- | --------------------------------------------------------------------------- |
| `max_files`            | `15`    | Per-commit / per-diff file cap. Affects both the historical-baseline filter (commits with > this many files don't contribute to coupling priors) and the working-tree bulk-self-filter (diffs over the cap silence HOTSPOT/COUPLING). v0.6 made this overridable from `mokumokuren.toml`. **Repos with naturally wider feature-commit grain** (workspace projects, infra repos, scaffold-heavy histories) need to bump this — at the default 15, every wide-grain commit gets dropped from the analyzer's commit set, leaving cross-cutting files reading as "no analyzable history" even with rich edit history. |
| `max_lines`            | `1000`  | Per-commit / per-diff line cap (agentic-context safety half of BUDGET). Same dual purpose as `max_files`. v0.6 made this overridable. Independent of the review-effectiveness floor below — the cap and the floor encode different evidence. |
| `review_quality_lines` | `200`   | (v0.10) Review-effectiveness floor in absolute LOC. A working-tree-vs-HEAD diff that crosses this line emits a BUDGET Info finding even when far under `max_lines`. Independent of `max_lines`: tying them together would mean a user who raises the cap silently weakens the floor. Empirical lineage in the docstring on `mmk_config::DEFAULT_REVIEW_QUALITY_LINES` (Jureczko 2020 IET Software replication; Demeyer et al. 2024 JSS reaffirmation). Set to `0` to disable; the per-diff cap stays. |
| `greenfield_threshold` | `0.5`   | When the working-tree diff's new-file fraction exceeds this, `mmk review` emits one explicit greenfield-acknowledgement Info finding so the agent reads HOTSPOT/COUPLING/DRIFT silence as expected, not as "mmk decided to be quiet." |
| `ignore_for_budget`    | `[]`    | (v0.6) Globs whose paths are excluded from diff-time BUDGET accounting (bulk-self-filter, over-cap trigger, under-cap ramp). Generated-file regenerations no longer trip BUDGET on every edit and silence HOTSPOT/COUPLING. The full diff still appears in `review.diff.files[]`; the optional `review.diff.budget` JSON sub-block surfaces gross / net counts plus the active glob list so silent dropping never recurs. |

To diagnose whether your repo's natural commit grain is hitting the
cap, run `git log --shortstat` and look at how many commits regularly
clear `max_files`. If most real feature commits are ≥30 files (common
on workspace projects), bump `max_files` to 50; the line cap rarely
needs tuning for the same reason.

## `[sensor.cohesion]`

Controls the COHESION sensor (v0.6) — tangled-diff detection on the
historical co-change graph. Fires Info when a working-tree diff
decomposes into ≥2 disjoint connected components, the structural
fingerprint Herzig & Zeller (2013) identified as elevating
revert / review cost. The edge metric is the max-symmetrized
Wilson 95 % lower bound on the directional conditional co-change
probability — same statistical primitive as COUPLING, generalized
to a graph-connectivity question. See `connected_components_by_wilson`
in `mmk-core/src/coupling.rs` for the derivation.

```toml
[sensor.cohesion]
enabled               = true
confidence_threshold  = 0.20
min_sample_size       = 3
min_files_per_cluster = 2
```

| Field                   | Default | Notes                                                                       |
| ----------------------- | ------- | --------------------------------------------------------------------------- |
| `enabled`               | `true`  | Whether the sensor runs. Default-on; turn off for repos where a tangled diff is the *intent* (e.g. monolithic codegen rebuilds). |
| `confidence_threshold`  | `0.20`  | Wilson 95 % lower-bound floor for the symmetrized edge metric. Lower than COUPLING's `0.30` because cohesion gates a graph-connectivity question, not a "you missed an edit" question — a missing edge that *should* connect a cluster fragments the graph and produces a false tangled-diff finding. |
| `min_sample_size`       | `3`     | Minimum `max(commits_touching(A), commits_touching(B))` for the pair to admit an edge. Mirrors COUPLING's floor; below 3, single-commit pairs reach Wilson's small-sample lower bound (~0.21) and admit edges with no real evidence. |
| `min_files_per_cluster` | `2`     | Minimum cluster size for a component to count toward the fire decision. Singleton greenfield files (no commit history) are dropped before the count to avoid flagging "added one new file alongside two coupled ones" as a tangled diff. |

`mmk eval --learn` reports per-repo `cohesion_tangled_diffs_seen`
and `cohesion_components_p95` distribution data, and emits a
suggested `[sensor.cohesion]` block when > 10 % of sampled commits
would fire COHESION on default thresholds.

## `[sensor.structure].role_patterns` (v0.8)

Stem-suffix patterns (`*<suffix>`) for architectural-role files.
Subjects whose stem matches demote STRUCTURE `ReviewDivergent`
from `Warn` to `Info` — role files legitimately diverge from
sibling shape conventions. In-code defaults cover
contribution / factory / registration / module / routes / config;
extend per-repo:

```toml
[sensor.structure]
role_patterns = [
    "*.contribution", "*Factory", "*.action", "*.actions",
    "*Registry", "*.module", "*Module", "*.routes", "*.config",
    "*Resolver",  # repo-specific addition
]
```

To find your repo's role patterns: run `mmk eval --replay`, look
at per-file STRUCTURE fire counts, and add the common stem
suffix on the obviously-role-bearing files. The full
`StructureCfg` field set lives on
`mmk_config::StructureCfg` (`cargo doc -p mmk-config`).

## `[sensor.complexity]` delta knobs (v0.8)

`delta_warn_pct` (default `0.50`) and `delta_warn_abs` (default
`20`) gate severity on pre-existing functions: a finding emits
`Warn` only when `Δ ≥ delta_warn_pct × head_actual` OR
`Δ ≥ delta_warn_abs`; otherwise `Info`. New files and new
functions stay `Warn`. The full `ComplexityCfg` field set lives
on `mmk_config::ComplexityCfg`.

## `[blast_radius]`

Controls the 1-hop co-change neighborhood emitted by
`--blast-radius <PATH>`. **Distinct knob** from `[coupling]` —
blast-radius is the exploratory query where a low threshold is
right.

```toml
[blast_radius]
threshold = 0.10
```

CLI override: `--blast-radius-threshold <FLOAT>`.

## `[health.ts]`

Enables the structural-pattern adapter (`mmk-health`) for
TypeScript / JavaScript files. Surfaces architectural neighbors
empirical co-change history cannot see — e.g. a
contribution-registration file's peer contribution files in the same
`contrib/` subtree, the `*.test.ts` partner of an implementation
file, or a newly-added broad TS/JS catch handler that wasn't in HEAD.

```toml
[health.ts]
enabled  = true
patterns = ["registration", "service", "test_pair", "broad_exception"]
```

| Field      | Default     | Notes                                                                                                                  |
| ---------- | ----------- | ---------------------------------------------------------------------------------------------------------------------- |
| `enabled`  | `false`     | Off by default outside the `js-ts` profile so non-TS users aren't surprised.                                           |
| `patterns` | all four    | Subset of `"registration"`, `"service"`, `"test_pair"`, `"broad_exception"`. Unknown tokens are dropped silently.       |

Pattern semantics (review/pre-edit):

| Pattern           | Trigger                                                                                                                                | Severity                            |
| ----------------- | -------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------- |
| `registration`    | `*.contribution.ts` or imports/extends from `vs/platform/actions/...`.                                                                 | Info (architectural precedent).     |
| `service`         | Declares `interface IFoo` + `registerSingleton(IFoo, ...)` / `createDecorator`.                                                        | Info (consumer list).               |
| `test_pair`       | Implementation file with a sibling `*.test.{ts,tsx,js,jsx}` / `*.spec.{ts,tsx,js,jsx}`. Pairs across the TS family (`.ts` ↔ `.tsx`) and the JS family (`.js` ↔ `.jsx`); cross-family rejected. | Warn in review when the test partner isn't in the diff; Info in pre-edit. |
| `broad_exception` | Working tree adds a non-top-level broad TS/JS catch handler not present at HEAD (empty body, no parameter, or `any`/`unknown`/`Error` type). v0.7. | Warn in review; suppressed in pre-edit (no working-vs-HEAD diff yet). |

## CLI flags vs file config

| Source              | When it wins                                          |
| ------------------- | ----------------------------------------------------- |
| `--coupling-threshold` etc. | Always wins for the call.                     |
| `mokumokuren.toml`  | Loaded if discovered at the repo root (or via `--config <PATH>`). |
| Built-in defaults   | Apply when neither of the above sets the field.       |

The effective `Config` is echoed in JSON output under `config`, so a
consumer can see exactly what produced the result.
