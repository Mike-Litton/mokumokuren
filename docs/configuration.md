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

Controls the per-commit / per-diff size guardrails — *agent-context
safety*. Diffs over the cap silence HOTSPOT/COUPLING because their
signal collapses at scale (the analyzer drops over-cap commits from
the historical baseline, so the working-tree diff isn't being scored
against a baseline that would have included it anyway), and the
under-cap ramp warns the agent at 50 % / 75 % before the binary
"you blew it" at 100 %.

The `max_lines = 1000` and `max_files = 15` defaults are **internal
calibration**, not derived from published research. (v0.13 dropped
the `review_quality_lines` knob and the Jureczko / SmartBear /
Cohen framing it carried — those numbers are about *human reviewer
attention*, which isn't what mmk targets after the v0.13 reframe to
agent self-verification.) Bump the caps for repos with naturally
wider feature-commit grain.

```toml
[bulk]
max_files            = 15
max_lines            = 1000
greenfield_threshold = 0.5
ignore_for_budget = [
    "**/routeTree.gen.ts",
    "**/*.pb.go",
]
```

| Field                  | Default | Notes                                                                       |
| ---------------------- | ------- | --------------------------------------------------------------------------- |
| `max_files`            | `15`    | Per-commit / per-diff file cap. Affects both the historical-baseline filter (commits with > this many files don't contribute to coupling priors) and the working-tree bulk-self-filter (diffs over the cap silence HOTSPOT/COUPLING). v0.6 made this overridable from `mokumokuren.toml`. **Repos with naturally wider feature-commit grain** (workspace projects, infra repos, scaffold-heavy histories) need to bump this — at the default 15, every wide-grain commit gets dropped from the analyzer's commit set, leaving cross-cutting files reading as "no analyzable history" even with rich edit history. |
| `max_lines`            | `1000`  | Per-commit / per-diff line cap. Same dual purpose as `max_files`. v0.6 made this overridable. Agent-context guardrail; the number is internal calibration, not a published threshold. |
| `greenfield_threshold` | `0.5`   | When the working-tree diff's new-file fraction exceeds this, `mmk review` emits one explicit greenfield-acknowledgement Info finding so the agent reads HOTSPOT/COUPLING/DRIFT silence as expected, not as "mmk decided to be quiet." |
| `ignore_for_budget`    | `[]`    | (v0.6) Globs whose paths are excluded from diff-time BUDGET accounting (bulk-self-filter, over-cap trigger, under-cap ramp). Generated-file regenerations no longer trip BUDGET on every edit and silence HOTSPOT/COUPLING. The full diff still appears in `review.diff.files[]`; the optional `review.diff.budget` JSON sub-block surfaces gross / net counts plus the active glob list so silent dropping never recurs. |

To diagnose whether your repo's natural commit grain is hitting the
cap, run `git log --shortstat` and look at how many commits regularly
clear `max_files`. If most real feature commits are ≥30 files (common
on workspace projects), bump `max_files` to 50; the line cap rarely
needs tuning for the same reason.

## `[sensor.cohesion]`

Controls the COHESION sensor (v0.6) — tangled-diff detection on the
historical co-change graph. Fires Warn when a working-tree diff
decomposes into ≥2 disjoint connected components. **Lineage:**
*inspired by* the tangled-change literature (Herzig & Zeller 2013) —
those papers untangle at AST granularity, mmk operates at diff
granularity, so the sensor is a structural-fingerprint proxy rather
than a direct reproduction. Severity calibrated by MSR 2026 LGTM
auto-merge data (focused PRs auto-merge more readily). Edge metric
is the max-symmetrized Wilson 95 % lower bound on the directional
conditional co-change probability — same statistical primitive as
COUPLING. See `connected_components_by_wilson` in
`mmk-core/src/coupling.rs` for the derivation.

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

The absolute caps below the delta knobs — `nesting_absolute_max
= 6`, `loc_absolute_max = 80` — are **internal calibration from
the v0.8 N=20 cohort**, not values cited in any paper. The delta
knobs are what actually trim the noise on pre-existing oversized
functions; the absolute caps are the floor those knobs act over.
`mmk eval --learn` retunes them per-repo when calibration data
exists.

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
TypeScript / JavaScript files. Three patterns ship today, each
targeting a documented LLM-coding failure mode.

```toml
[health.ts]
enabled  = true
patterns = ["test_pair", "broad_exception", "test_weakening"]
```

| Field      | Default      | Notes                                                                                                                  |
| ---------- | ------------ | ---------------------------------------------------------------------------------------------------------------------- |
| `enabled`  | `false`      | Off by default outside the `js-ts` profile so non-TS users aren't surprised.                                           |
| `patterns` | all three    | Subset of `"test_pair"`, `"broad_exception"`, `"test_weakening"`. Unknown tokens are dropped silently. Delta-mode patterns (`broad_exception`, `test_weakening`) are silently dropped in `audit` (no HEAD baseline to compare against). |

Pattern semantics:

| Pattern           | Trigger                                                                                                                                                                                                                                                                                                          | Severity                                                                  | Emitted by         |
| ----------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------- | ------------------ |
| `test_pair`       | Implementation file with a sibling `*.test.{ts,tsx,js,jsx}` / `*.spec.{ts,tsx,js,jsx}`. Pairs across the TS family (`.ts` ↔ `.tsx`) and the JS family (`.js` ↔ `.jsx`); cross-family rejected.                                                                                                                  | Warn in review when the test partner isn't in the diff; Info in pre-edit. | review, audit      |
| `broad_exception` | Working tree adds a non-top-level broad TS/JS catch handler not present at HEAD: empty body, no parameter, typed `any`/`unknown`/`Error`, or log-and-swallow shape (single-statement body of member-call expressions on a configured set of log identifiers, no rethrow). v0.7, log-and-swallow extension v0.12. Targets arXiv:2509.13941. | Warn in review; suppressed in pre-edit (no working-vs-HEAD diff yet).     | review             |
| `test_weakening`  | Working tree erodes an existing test file vs HEAD: `+1+ .skip` / `.only` / `xit` / `xtest` / `xdescribe`, `−1+ expect(...)`, `+1+ jest.mock` / `vi.mock`, `+1+ @ts-expect-error` / `@ts-ignore`, or `−1+ it` / `test` / `describe` case. Only test files are eligible subjects. v0.13. Targets arXiv:2503.15223 *"Are 'Solved Issues' in SWE-bench Really Solved Correctly?"* | Warn in review; suppressed in pre-edit (no diff yet).                     | review             |

(v0.13 dropped `registration` and `service` — VSCode-specific
hardcoded patterns that fired 0 times on non-VSCode codebases in
the empirical experiments — and `broad_catch_debt` — the audit-mode
counterpart subsumed by `broad_exception` in the delta-mode review
loop.)

### `[health.ts.broad_exception]`

Sub-block consumed by `broad_exception` (delta-mode EVASION).
Single knob today — `log_identifiers` — controls the set of object
identifiers recognised as a "logger" for the log-and-swallow shape
(`catch (e) { logger.warn(...); }`). The default covers
pino-style (`logger`), winston-style (`log`), and the browser
fallback (`console`); add project-specific names to extend it.

```toml
[health.ts.broad_exception]
log_identifiers = ["logger", "log", "console", "appLog", "tracer"]
```

Setting `log_identifiers` replaces the default list — copy the
shipped defaults in if you want to extend rather than override.

## CLI flags vs file config

| Source              | When it wins                                          |
| ------------------- | ----------------------------------------------------- |
| `--coupling-threshold` etc. | Always wins for the call.                     |
| `mokumokuren.toml`  | Loaded if discovered at the repo root (or via `--config <PATH>`). |
| Built-in defaults   | Apply when neither of the above sets the field.       |

The effective `Config` is echoed in JSON output under `config`, so a
consumer can see exactly what produced the result.
