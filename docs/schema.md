# `mmk` JSON schema

Every `mmk` subcommand that takes `--format json` emits a single
JSON object whose shape is documented here. The
**`schema_version`** field is the only contract consumers should pin
against; `crate_version` is the Cargo version of the producing build
and is diagnostic only.

Subcommands at v0.10.0:

- `mmk analyze` — ranked hotspots over a window.
- `mmk session-summary` (alias: `mmk session`) — window + session
  ranking, delta block, plus a `findings[]` overlay (DRIFT, BUDGET,
  ANCHOR on empty session). v0.8 surfaces window-truncation as
  `analysis.window_truncation` metadata instead of a finding —
  operational BUDGET (session-aggregate overrun) is unchanged. On
  empty sessions the WINDOW ranking collapses (the `files` key is
  omitted) so the ANCHOR pointer isn't buried under
  generated-artefact rankings.
- `mmk review` — diff against history (working tree by default;
  `--staged`, `--range A..B`, `--commit <SHA>`). Surfaces HOTSPOT,
  COUPLING, COHESION (Warn since v0.7), STRUCTURE, COMPLEXITY,
  HEALTH (incl. EVASION / `broad_exception` since v0.7), and BUDGET
  findings. v0.7 adds an optional `cohesion` top-level block with
  the per-cluster path decomposition.
- `mmk pre-edit <PATH>` — historical context for a path before edit.
  Absolute path inputs are normalized against the discovered repo
  root. The path is optional when invoked via a Claude Code hook
  (the JSON envelope on stdin supplies it).
- `mmk drift --sessions K` — climb signal across K session boundaries.
- `mmk eval --sample N` — sampled noise-floor report (adoption tool).
  `--learn` adds suggestion blocks; `--replay` adds a per-layer
  histogram (cross-repo aggregation).

When `mmk review` or `mmk pre-edit` detects a Claude Code hook
envelope on stdin, it switches to a hook-shape output envelope
(`hookSpecificOutput.additionalContext` etc.) instead of the
`--format json` shape documented below. See the
[Hook output envelope](#hook-output-envelope-v06) section.

## Stability contract

`schema_version` tracks the `mmk` minor release: every `0.10.x`
build emits `0.10.0`.

### v0.10.0 — breaking removals

v0.13 prunes the research-thin and empirically-dead sensors. The
JSON shape loses these fields:

- `config.bulk.review_quality_lines` and the
  `BudgetTier::ReviewQuality` Info finding it produced.
- `couples_of` top-level block on `mmk analyze` (the `--couples-of`
  / `--couples` CLI flags are gone too). `top_couples[]` per file
  remains.
- `health.matches[].pattern` values `registration`, `service`,
  `broad_catch_debt`. New value: `test_weakening`.
- `health.matches[].detail.kind = "broad_catch_debt"`. New variant:
  `kind = "test_weakening"` with `skips_added`, `assertions_lost`,
  `mocks_added`, `ts_suppressions_added`, `tests_removed` fields.

A consumer pinning to `0.9.0` keeps reading older builds; reading
`0.10.0` requires dropping the field references above.

### v0.9.0 — content-shape changes inside string fields

v0.9 introduces no top-level JSON shape changes for `--format json`.
One hook-envelope routing change (see the last bullet) and four
`findings[].message` content shifts are worth calling out for
harnesses that parse the prose:

- **COMPLEXITY findings carry class-qualified function identity.**
  v0.8 emitted `<P>::methodName`; v0.9 emits
  `<P>::ClassName::methodName` for methods inside a class
  declaration or class expression. Top-level function declarations
  still render as `<P>::functionName`. Reason: the v0.8 HEAD-baseline
  filter cross-attributed methods that shared a bare name across
  classes in the same file (`constructor`, `dispose`, `init`, …); the
  qualified rendering disambiguates and the underlying matcher is
  qualified, too.
- **Fall-through findings carry the canonical `[no actionable
  signal] ` prefix.** Six v0.8 wordings ("no signal", "new file",
  "history priors don't apply", "session contains 0 commits",
  "present in HEAD but no analyzable history", "mmk: no findings")
  now compose `[no actionable signal] {reason}` so an agent can scan
  for one prefix instead of six. The reason follows after the prefix
  unchanged.
- **COUPLING fires near the gate floor append `
  [low-confidence n=N]`.** Triggered when `n ≤ min_sample_size + 1`
  or `wilson_lower_95 < 2 × confidence_threshold`; high-confidence
  fires are silent. Two-tier surface: any suffix is the canonical
  low-confidence form; no other tier wording exists.
- **COMPLEXITY Info renders the `[info]` text marker, not the `ⓘ`
  glyph.** Affects the text body in
  `hookSpecificOutput.additionalContext` for hook mode and the
  `mmk review --format text` output for CLI mode. The JSON
  `findings[].severity` field is unchanged.
- **Empty-findings line shifts from `systemMessage` to
  `additionalContext`** (hook envelopes only). When a PostToolUse
  or PreToolUse run produces zero findings, the canonical
  `[no actionable signal] no findings (...)` line travels via
  `hookSpecificOutput.additionalContext`. v0.8 placed this line
  on the top-level `systemMessage`. Real-finding routing is
  unchanged. Dedup-suppress notices stay on `systemMessage`.

v0.8 readers parse v0.9 output without modification, with one
caveat: harnesses that pinned to "the empty-findings line is in
`systemMessage`" should switch to reading
`hookSpecificOutput.additionalContext`.

| Change kind                                             | Schema bump? |
| ------------------------------------------------------- | :----------: |
| New optional field                                      |      No      |
| New top-level block (e.g. `session`, `blast_radius`)    |      No      |
| Rename of an existing field                             |     Yes      |
| Removal of an existing field                            |     Yes      |
| Type change (`string` → `int`, `array` → `object`, …)   |     Yes      |
| Semantic change (same name and type, different meaning) |     Yes      |

Consumers should treat unknown fields as forward-compatible additions
and ignore them rather than fail.

## Top-level fields

| Field            | Type    | Notes                                                                                |
| ---------------- | ------- | ------------------------------------------------------------------------------------ |
| `schema_version` | string  | Pinned to the `mmk` minor (`"0.10.0"`).                                               |
| `crate_version`  | string  | `CARGO_PKG_VERSION` of the producing build. Diagnostic only — do not pin against.    |
| `repo`           | object  | HEAD metadata + repo-level warnings.                                                 |
| `config`         | object  | Effective `Config` after merging file + CLI sources.                                 |
| `analysis`       | object  | Walk counters and timing.                                                            |
| `files`          | array   | Ranked hotspot entries.                                                              |
| `session`        | object? | Present only on `mmk session`.                                                       |
| `blast_radius`   | object? | Present only when `--blast-radius <PATH>` is set.                                    |

### `repo`

| Field            | Type     | Notes                                                  |
| ---------------- | -------- | ------------------------------------------------------ |
| `head_sha`       | string?  | `null` if HEAD is unborn.                              |
| `head_timestamp` | string?  | RFC 3339, UTC, second precision. `null` if unborn.     |
| `is_shallow`     | boolean  | `true` if `.git/shallow` exists.                       |
| `warnings`       | string[] | Repo-level diagnostics (shallow clone, base fallback). |

### `analysis`

| Field                              | Type   | Notes                                                       |
| ---------------------------------- | ------ | ----------------------------------------------------------- |
| `commits_seen`                     | uint   | Walk output before the bulk filter.                         |
| `commits_analyzed`                 | uint   | Commits kept after the bulk filter.                         |
| `commits_filtered.bulk`            | uint   | Commits dropped by `bulk.max_files` / `bulk.max_lines`.     |
| `files_ignored.deleted_from_head`  | uint   | Diff events on paths that don't exist at HEAD.              |
| `files_ignored.head_paths_ignored` | uint   | HEAD-tree paths matched by an ignore glob.                  |
| `duration_ms`                      | uint   | Wall time for the analyze pipeline.                         |
| `window_truncation`                | object? | (v0.8) Present on `mmk session-summary` envelopes when `commits_filtered.bulk > 0`. Carries `commits_dropped` (uint), `total_commits` (uint), `max_files` (uint), `max_lines` (uint). Descriptive metadata about what the analyzer saw — operational BUDGET (diff-vs-cap) still fires under `findings[]`. Replaces the v0.7 `findings[]` entry on `Layer::Budget / Severity::Warn` that emitted the same wording. |

### `files[]` (per hotspot entry)

| Field              | Type     | Notes                                                                                |
| ------------------ | -------- | ------------------------------------------------------------------------------------ |
| `path`             | string   | Repo-relative.                                                                       |
| `loc`              | uint     | Line count at HEAD.                                                                  |
| `weighted_churn`   | float    | Recency-weighted `added + deleted`.                                                  |
| `relative_churn`   | float    | `weighted_churn / loc`. Reported, not used in ranking.                               |
| `hotspot_score`    | float    | `log(1 + weighted_churn) × log(1 + loc)`.                                            |
| `hotspot_rank`     | uint     | 1-indexed position in the ranked output.                                             |
| `commits_touching` | uint     | Distinct commits modifying this file in the window.                                  |
| `last_modified`    | string?  | RFC 3339 of latest in-window commit touching this file.                              |
| `top_couples`      | array    | (v0.2.0+) Top co-changing partners. Empty unless the file is in the ranked top-N.    |

### `session_files[]` (`mmk session` only — same shape as `files[]`, different LOC epoch)

Same field set as `files[]`, but the `loc` and `relative_churn`
denominators come from a **different epoch**:

- `session_files[].loc` is the line count at the **resolved session
  base commit** for files that existed there. For files introduced
  *during* the session (no base counterpart), it falls back to
  HEAD-LOC. This way `session_files[].relative_churn =
  session_weighted_churn / loc` reflects "fraction of the file the
  session touched, relative to its size at session start" — not
  "relative to whatever's left at HEAD."
- `files[].loc` (the top-level window ranking) keeps using HEAD-LOC.

The two-epoch model is intentional: the window ranking answers "what
does the working tree look like right now?" while the session
ranking answers "what shifted while the agent was working?"

### `files[].top_couples[]`

| Field                     | Type   | Notes                                                       |
| ------------------------- | ------ | ----------------------------------------------------------- |
| `partner`                 | string | Repo-relative path of the co-changing file.                 |
| `jaccard`                 | float  | Symmetric coupling: `co_change / (touches_a + touches_b - co_change)`. Drives `--blast-radius`. |
| `co_change_count`         | uint   | Distinct commits where both files were modified.            |
| `conditional_probability` | float  | (v0.4) `co_change / commits_touching(subject)`. Direct point estimate of `P(partner | subject)`. |
| `wilson_lower_95`         | float  | (v0.4) Wilson 95% lower bound for `conditional_probability`. Drives the COUPLING decision in `mmk review` / `mmk pre-edit`. |

### `session` (present on `mmk session`)

| Field                | Type     | Notes                                                                          |
| -------------------- | -------- | ------------------------------------------------------------------------------ |
| `base_ref`           | string?  | The user-supplied or resolved ref label (e.g. `"origin/main"`).                |
| `base_sha`           | string?  | The resolved base commit SHA.                                                  |
| `base_resolved_via`  | string   | One of `"explicit"`, `"since_commit"`, `"merge_base_origin_main"`, `"merge_base_main"`, `"merge_base_origin_master"`, `"merge_base_master"`, `"head_minus_one"`. |
| `entered_top_n`      | string[] | Files in session top-N not in window top-N.                                    |
| `rank_climbs`        | array    | `[{ "path": string, "delta": int }]` where positive = climbed.                 |
| `churn_of_churn`     | array    | `[{ "path": string, "ratio": float }]`. Symmetric thrash ratio in `[0, 1]`.    |
| `commit_entropy`     | float    | Shannon entropy of files-touched-per-commit, normalized by `log(commits)`.     |

### `config` (effective configuration echo)

The `config` block is the merged in-memory `Config` after applying
`mokumokuren.toml`, profile, and CLI overrides. Consumers can read
it to understand exactly what produced the result.

| Field                          | Type     | Notes                                                        |
| ------------------------------ | -------- | ------------------------------------------------------------ |
| `window.days`                  | uint     | Effective `--since` in days.                                 |
| `window.tau_days`              | uint     | Recency-decay 1/e point.                                     |
| `hotspot.top_n`                | uint     | Effective `--top`.                                           |
| `bulk.max_files`               | uint     | Bulk-filter file threshold (default `15`; v0.6 made this overridable from `mokumokuren.toml`). |
| `bulk.max_lines`               | uint     | Per-diff line cap (default `1000`; v0.6 made this overridable). Agent-context guardrail; the number is internal calibration, not a published threshold. (v0.13 dropped the `bulk.review_quality_lines` knob — see `CHANGELOG.md`.) |
| `bulk.greenfield_threshold`    | float    | Fraction of changed paths the analyzer must not have seen before `mmk review` emits the greenfield-acknowledgement Info finding (default `0.5`). |
| `bulk.ignore_for_budget`       | string[] | (v0.6) Glob patterns excluded from diff-time BUDGET accounting. The full diff still appears in `review.diff.files[]`. |
| `blast_radius.threshold`       | float    | Min Jaccard for `--blast-radius` neighborhood.               |
| `coupling.threshold`           | float    | (deprecated) Legacy v0.3 jaccard threshold; v0.4+ silently maps it to `confidence_threshold`. |
| `coupling.confidence_threshold`| float    | Min Wilson 95% lower bound on `P(partner | subject)` for COUPLING findings (default `0.30` since v0.6; was `0.20` in v0.4–v0.5). |
| `coupling.min_sample_size`     | uint     | Min `commits_touching(subject)` before COUPLING fires (default `3` since v0.6; was `5` in v0.4 and `1` in v0.5). |
| `coupling.ignore_partners`     | string[] | Globs that never fire as the missed partner in COUPLING.     |
| `health.ts.enabled`            | bool     | (v0.4) Whether the TypeScript Health adapter runs.           |
| `health.ts.patterns`           | string[] | Pattern tokens. Current set: `test_pair` (v0.4), `broad_exception` (v0.7 EVASION), `test_weakening` (v0.13). v0.13 dropped `registration` / `service` / `broad_catch_debt` — see `CHANGELOG.md`. |
| `sensor.structure.enabled`     | bool     | (v0.5) Whether the STRUCTURE convention sensor runs.         |
| `sensor.structure.import_majority` | float | (v0.5) Sibling fraction needed for an import to count as the directory's convention (default 0.85). |
| `sensor.structure.role_patterns` | string[] | (v0.8) Stem-suffix patterns (`*<suffix>`) marking architectural-role files. Matching files have their `ReviewDivergent` finding demoted from `Warn` to `Info` and the prose reframed. Defaults to `["*.contribution", "*Factory", "*.action", "*.actions", "*Registry", "*.module", "*Module", "*.routes", "*.config"]`. |
| `sensor.complexity.enabled`    | bool     | (v0.5) Whether the COMPLEXITY per-function sensor runs.      |
| `sensor.complexity.delta_warn_pct` | float | (v0.8) Δ-percent threshold above which a pre-existing function's finding earns `Warn`. Below this AND below `delta_warn_abs`, the finding demotes to `Info`. New files / new functions still emit `Warn`. Default `0.50`. |
| `sensor.complexity.delta_warn_abs` | uint  | (v0.8) Δ-absolute threshold above which a pre-existing function's finding earns `Warn`. Default `20`. |
| `sensor.budget_ramp.enabled`   | bool     | (v0.5) Whether the under-cap BUDGET ramp emits Info @ ≥50% / Warn @ ≥75% (default `true`). |
| `sensor.cohesion.enabled`      | bool     | (v0.6) Whether the COHESION tangled-diff sensor runs (default `true`). |
| `sensor.cohesion.confidence_threshold` | float | (v0.6) Wilson 95% lower bound floor for the symmetrized edge metric. Default `0.20` — looser than COUPLING's `0.30` because cohesion gates a graph-connectivity question, not a missed-partner one. |
| `sensor.cohesion.min_sample_size` | uint  | (v0.6) Min `max(commits_touching(A), commits_touching(B))` for an edge to admit. Default `3`, mirroring COUPLING. |
| `sensor.cohesion.min_files_per_cluster` | uint | (v0.6) Min cluster size for a component to count toward the fire decision. Default `2` — singletons aren't clusters. |
| `rename_similarity`            | float    | Diff-engine rename threshold.                                |
| `ignores`                      | string[] | Final ignore globs after merging file + CLI sources.         |

### `blast_radius` (present when `--blast-radius <PATH>` is set)

| Field       | Type     | Notes                                                       |
| ----------- | -------- | ----------------------------------------------------------- |
| `root`      | string   | Echo of the `--blast-radius` path.                          |
| `hops`      | uint     | Always `1` in v0.6.0.                                       |
| `threshold` | float    | Effective Jaccard threshold applied. Resolved as `--blast-radius-threshold` → `[blast_radius] threshold` in `mokumokuren.toml` → built-in default `0.10`. Echoed so consumers can see what filter produced the listed nodes. |
| `nodes`     | array    | `[{ "path": string, "jaccard": float, "co_change_count": uint, "hops": uint }]` sorted desc by jaccard. |

## Envelopes by subcommand

### `mmk review`

Compares a diff against the historical baseline and emits findings
before any commit lands.

| Field            | Type    | Notes                                                                                  |
| ---------------- | ------- | -------------------------------------------------------------------------------------- |
| `schema_version` | string  | `"0.10.0"`.                                                                             |
| `crate_version`  | string  |                                                                                        |
| `repo`           | object  | Same as analyze. Present only when there are changes (clean tree skips analyze).       |
| `config`         | object  | Same as analyze. Present only when there are changes.                                  |
| `analysis`       | object  | Same as analyze. Present only when there are changes.                                  |
| `review`         | object  | `mode` + per-file diff numstat. Always present.                                        |
| `findings`       | array   | Layer-labeled findings (HOTSPOT, COUPLING, COHESION v0.6, BUDGET, HEALTH v0.4, STRUCTURE / COMPLEXITY v0.5). Always present (possibly empty). |
| `health`         | object? | (v0.4) Present when the Health adapter ran AND returned matches. See [`health` block](#health-block-v04). |

#### `review`

| Field                  | Type   | Notes                                                                          |
| ---------------------- | ------ | ------------------------------------------------------------------------------ |
| `mode`                 | string | One of `"working_tree"`, `"staged"`, `"range"`, `"commit"`.                    |
| `diff.files_changed`   | uint   |                                                                                |
| `diff.lines_added`     | uint   |                                                                                |
| `diff.lines_deleted`   | uint   |                                                                                |
| `diff.files[]`         | array  | `[{ "path": string, "added": uint, "deleted": uint }]`. Binary files omitted. Working-tree mode includes untracked-but-not-ignored files (added = line count, deleted = 0). |
| `diff.new_file_fraction` | float? | (v0.4) Fraction of changed paths the historical analyzer hasn't seen. Optional: present on the with-changes envelope, omitted on bulk-self / clean-tree envelopes. Lets a consumer reason about why HOTSPOT/COUPLING/DRIFT are silent on greenfield diffs. |
| `diff.budget`            | object? | (v0.6) Present only when `bulk.ignore_for_budget` matched at least one file in the diff. Carries `files_gross`, `files_net`, `lines_gross`, `lines_net`, and `ignored_for_budget` (the active glob list). Surfaces gross / net divergence so silent file-dropping never recurs. |

#### `mmk review` bulk-self-filter envelope

When the input diff itself trips `bulk.max_files` or
`bulk.max_lines`, review skips the (expensive) hotspot/coupling
analysis and emits a slimmer envelope. STRUCTURE and COMPLEXITY
findings appear here alongside BUDGET (since v0.5) — they don't
depend on the analyzer pass and stay meaningful on over-cap diffs.
The BUDGET wording on this path names *which* layers were skipped
and *why* (since v0.6) so silence on HOTSPOT/COUPLING reads as
"uncomputed at this scale" rather than "all clear."

| Field            | Type    | Notes                                                            |
| ---------------- | ------- | ---------------------------------------------------------------- |
| `schema_version` | string  | `"0.10.0"`.                                                       |
| `crate_version`  | string  |                                                                  |
| `review`         | object  | `mode` + per-file diff numstat.                                  |
| `findings`       | array   | One BUDGET finding plus any STRUCTURE / COMPLEXITY findings on the changed paths; HOTSPOT/COUPLING skipped. |
| `duration_ms`    | uint    | Wall time (no analyze ran).                                      |

The clean-tree envelope (no diff) keeps its existing shape:
`schema_version`, `crate_version`, `review` with empty `diff.files`,
empty `findings`.

### `mmk pre-edit`

Pre-edit context: rank, expected partners, optional drift for the
queried path.

| Field            | Type    | Notes                                                                                  |
| ---------------- | ------- | -------------------------------------------------------------------------------------- |
| `schema_version` | string  | `"0.10.0"`.                                                                             |
| `crate_version`  | string  |                                                                                        |
| `repo`           | object  |                                                                                        |
| `config`         | object  |                                                                                        |
| `analysis`       | object  |                                                                                        |
| `pre_edit.path`  | string  | Echo of the queried path. Absolute paths are normalized against the discovered repo root before lookup so hook integrations passing `tool_input.file_path` produce the same signal as relative-path manual invocations. |
| `findings`       | array   | HOTSPOT (Warn), COUPLING (Info), HEALTH (Info, v0.4), STRUCTURE (Info, v0.5), DRIFT (Warn), BUDGET ramp (Info @ ≥50% / Warn @ ≥75%, v0.5). May contain a single OK finding if every layer is silent. The fall-through wording distinguishes (v0.6) "new file (not yet in HEAD)" from "present in HEAD but no analyzable history (file may be stale or prior touches were filtered as bulk commits)" so an agent reading silence on a wide-grain repo can't misread the latter as "no risk." |
| `health`         | object? | (v0.4) Present when the Health adapter ran AND returned matches. See [`health` block](#health-block-v04). |

### `mmk eval`

Aggregated noise-floor report from sampling N recent commits. Built
for `mmk eval --sample N`; not on the agent edit-loop hot path.

| Field                  | Type     | Notes                                                              |
| ---------------------- | -------- | ------------------------------------------------------------------ |
| `commits_sampled`      | uint     | Number of non-merge commits actually sampled.                      |
| `commits_with_findings`| uint     | Of those, how many fired at least one finding.                     |
| `total_findings`       | uint     | Sum of findings across the sample.                                 |
| `by_layer`             | object   | `{layer: count}` over `hotspot`, `coupling`, `drift`, `budget`, ...|
| `noisy_partners`       | object   | `{path: count}` — partner paths most often blamed in COUPLING.    |
| `wilson_lower_buckets` | object   | (v0.4) `{ "0.00-0.20": uint, "0.20-0.40": uint, "0.40+": uint }`. Wilson 95% lower bound buckets (was `jaccard_buckets` in v0.3). |
| `threshold`            | float    | Effective `[coupling] confidence_threshold` for context (v0.4 semantics; was jaccard threshold in v0.3). |
| `learn_suggestions`    | array?   | (v0.4) Present only with `--learn`. Each entry: `{ partner: string, subject_count: uint, mean_inverse_conditional_probability: float }`. High-breadth, low-inverse-prob partners flagged for `[coupling] ignore_partners`. |
| `learn_sensor_stats`   | object?  | (v0.5) Present only with `--learn`. Per-sensor distribution: `structure_dir_shapes_seen`, `structure_dir_shapes_above_floor`, `structure_commits_with_fire`, `complexity_functions_seen`, `complexity_nesting_{median,p90,p99}`, `complexity_loc_{median,p90,p99}`, plus (v0.6) `cohesion_tangled_diffs_seen`, `cohesion_components_p95`. The text-mode writer additionally emits a suggested `[sensor.cohesion]` block when > 10 % of sampled commits would fire COHESION. |
| `replay_histogram`     | object?  | (v0.5) Present only with `--replay`. `{ commits_sampled: uint, layers: [{ layer: string, commits_with_fire: uint, fire_rate: float, distinct_paths: uint, total_findings: uint, severity: { ok: uint, info: uint, warn: uint } }] }`. Designed for cross-repo aggregation. |
| `duration_ms`          | uint     |                                                                    |

### `mmk explain`

Per-commit evidence behind a finding's claim. Pass `--finding <id>`
(the `[id=…]` tag from a `mmk review` / `mmk pre-edit` finding);
get back the chronological co-change commits + aggregate timeline.
Currently scoped to `coupling:<subject>:<partner>` ids only.

| Field                       | Type     | Notes                                                                          |
| --------------------------- | -------- | ------------------------------------------------------------------------------ |
| `finding`                   | string   | Echo of the requested fingerprint.                                             |
| `co_change_count`           | uint     | Commits that touched both pair members.                                        |
| `commits_touching_either`   | uint     | Commits in the window that touched at least one pair member. Difference from `co_change_count` is the partner-only count. |
| `co_change_span_days`       | uint     | Days between first and last co-change. `0` when there are no co-changes.       |
| `co_change_first_ts`        | int?     | Unix timestamp of the earliest co-change. `null` when there are no co-changes. |
| `co_change_last_ts`         | int?     | Unix timestamp of the latest co-change. `null` when there are no co-changes.   |
| `evidence`                  | array    | One entry per co-change commit, sorted newest-first. Each entry: `{ sha, ts, deltas: [{ path, added, deleted }] }`. |

### `mmk drift`

K snapshot labels + the climb-majority findings.

| Field                       | Type     | Notes                                                                          |
| --------------------------- | -------- | ------------------------------------------------------------------------------ |
| `schema_version`            | string   | `"0.10.0"`.                                                                     |
| `crate_version`             | string   |                                                                                |
| `drift.base`                | string?  | Echo of `--base`.                                                              |
| `drift.sessions`            | uint     | Echo of `--sessions K`.                                                        |
| `drift.snapshot_labels`     | string[] | One commit OID per snapshot, oldest-first.                                     |
| `findings`                  | array    | DRIFT findings. Each has `layer`, `severity`, `path`, `climb_transitions`, `total_transitions`, `latest_rank`. |
| `duration_ms`               | uint     | Wall time for the K analyze runs + drift compute.                              |

### `findings[]` (unified, used by review / pre-edit / session-summary)

| Field      | Type   | Notes                                                                                                |
| ---------- | ------ | ---------------------------------------------------------------------------------------------------- |
| `layer`    | string | One of `"hotspot"`, `"coupling"`, `"cohesion"` (v0.6 — populated), `"drift"`, `"budget"`, `"health"` (v0.4 — populated), `"structure"` (v0.5 — populated), `"complexity"` (v0.5 — populated), `"anchor"` (v0.5 — populated; previously reserved). |
| `severity` | string | `"warn"`, `"info"`, `"ok"`. v0.4 actually populates `"ok"` (pre-edit quiet-file fall-through).      |
| `message`  | string | Human-readable, terse, one-line. The structured detail lives in `layer` / `severity`.                |
| `id`       | string? | (v0.11) Stable fingerprint for `mmk explain --finding <id>`. Populated for COUPLING as `coupling:<subject>:<partner>`; `null` for layers that don't yet support explain. Always present in the JSON envelope (explicit absence beats inferring from a missing key). |

### `health` block (v0.4)

Present on `mmk review` and `mmk pre-edit` when
`config.health.ts.enabled = true` AND the Health adapter returned
at least one match. Mirrors the `findings[]` Health entries in
structured form so consumers can read `pattern` + `related` without
parsing the message text.

| Field                  | Type     | Notes                                                              |
| ---------------------- | -------- | ------------------------------------------------------------------ |
| `patterns_evaluated`   | string[] | Pattern tokens the adapter ran (`test_pair`, `broad_exception`, `test_weakening`). Echo of `config.health.ts.patterns`. |
| `matches[].pattern`    | string   | Which pattern fired (`test_pair`, `broad_exception`, `test_weakening`). |
| `matches[].subject`    | string   | The file the analysis was about — typically the changed/queried path. |
| `matches[].related`    | string[] | Architectural neighbors / partners surfaced for this match.        |

The `mmk drift` envelope inlines per-finding bookkeeping fields
(`path`, `climb_transitions`, etc.) directly into each finding object;
review / pre-edit / session-summary keep `findings[]` to the unified
shape above.

## Hook output envelope (v0.6)

When `mmk pre-edit` or `mmk review` is invoked from a Claude Code
hook (detected by the presence of a JSON envelope on stdin with a
non-empty `hook_event_name`), the output shape switches from the
`--format json` envelope above to the documented Claude Code
hook-output schema. The body of any findings flows through the
text renderer (single source of truth with the CLI text path) and
is wrapped in:

```json
{
  "hookSpecificOutput": {
    "hookEventName": "PreToolUse" | "PostToolUse" | "Stop",
    "additionalContext": "<rendered findings text, optional>"
  },
  "decision": "block",
  "reason": "<gated wording, optional>",
  "systemMessage": "<dedup-suppress or no-findings note, optional>"
}
```

| Field                                  | Type    | When present                                                                                                          |
| -------------------------------------- | ------- | --------------------------------------------------------------------------------------------------------------------- |
| `hookSpecificOutput.hookEventName`     | string  | Always.                                                                                                               |
| `hookSpecificOutput.additionalContext` | string? | Present when there is a findings body to inject and the run isn't dedup-suppressed and isn't blocking on Warn.        |
| `decision`                             | string? | Set to `"block"` only when the caller passed `--gate warn` AND at least one Warn-severity finding fires. Hard yield to the agent. |
| `reason`                               | string? | The block reason, including the rendered findings body. Present iff `decision = "block"`.                             |
| `systemMessage`                        | string? | Present on dedup-suppress (`mmk: prior findings unchanged since HEAD <sha7>`) or no-findings-to-deliver.              |

`PreToolUse` invocations never carry `decision: "block"` — Claude
Code's hook contract reserves Edit-phase blocking for `PostToolUse`
/ `Stop`, which is also where `--gate warn` takes effect.

The argv-fallback path is preserved: invoking `mmk review` /
`mmk pre-edit` without a stdin envelope produces the
`--format text` or `--format json` shape documented above.

## Findings vs. Diagnostics

`findings[]` carries signals the agent should **act on** — the
agent's diff is over the BUDGET cap, a COUPLING partner is missing,
a function structurally crosses a COMPLEXITY threshold. Each entry
has a `layer` and `severity`; consumers gate on these.

**Diagnostics** are descriptive metadata about *what the analyzer
ran*. They aren't actionable — telling the agent "your edit caused
this" would be wrong, since the diagnostic describes the analysis
window, not the agent's diff. v0.8 introduces the first explicit
diagnostic surface: `analysis.window_truncation`. Operational
BUDGET (the diff-vs-cap signal) stays in `findings[]` because it
*is* about the agent's diff.

The split exists because the v0.7 calibration pass found agents
treating window-truncation Warn fires as actionable when they
described nothing the agent could do. Diagnostics are the right
home for "here's what the analyzer saw"; Findings are the right
home for "here's what your work just did."

## Schema-version history

- **`0.8.0`** — additive: `analysis.window_truncation` block,
  `sensor.structure.role_patterns`,
  `sensor.complexity.delta_warn_pct` / `delta_warn_abs` echoed in
  `config`. Severity semantics shift on COMPLEXITY (delta-weighted)
  and STRUCTURE (role demotion). Session-summary's v0.7
  window-truncation `findings[]` entry retires; the data moves to
  the metadata block. v0.7 readers parse v0.8 output without
  modification. See [CHANGELOG](../CHANGELOG.md) for the rationale.
- **`0.7.0`** — `findings[].layer = "cohesion"` severity is **Warn**
  (was Info; consumers running `--gate warn` now exit 2 on tangled
  diffs); optional top-level `cohesion` block with
  `tangles[].clusters[]` per-cluster decomposition;
  `findings[].layer = "health"` `pattern = "broad_exception"`
  (EVASION) added; `health.matches[].pattern = "test_pair"` extends
  to `.js` / `.jsx` (cross-extension within TS / JS family); TSX
  grammar dispatch fix (no shape change to schema). All changes
  additive — `0.6` consumers parse `0.7` without modification.
- **`0.6.0`** — Layer `"cohesion"` populated; `sensor.cohesion`
  config block; `review.diff.budget` sub-block (gross / net BUDGET
  accounting); `bulk.max_files` / `bulk.max_lines` honoured from
  `mokumokuren.toml`; `learn_sensor_stats` cohesion fields; hook
  output envelope shape; COUPLING gate defaults shifted
  (`confidence_threshold` 0.20 → 0.30, `min_sample_size` 1 → 3);
  empty-session WINDOW collapse (`files` key omitted).
- **`0.5.0`** — Layers `"structure"`, `"complexity"`, `"anchor"`
  populated; `sensor.{structure,complexity,budget_ramp}` config
  blocks; greenfield acknowledgement
  (`review.diff.new_file_fraction`); per-fire dedup; eval gains
  `replay_histogram` and `learn_sensor_stats`.
- **`0.4.0`** — Wilson-gated COUPLING: `top_couples[]` gains
  `conditional_probability`, `wilson_lower_95`;
  `coupling.confidence_threshold` / `min_sample_size`; Health
  layer (TypeScript) populates `"health"` and the optional
  `health` block; eval renames `jaccard_buckets` →
  `wilson_lower_buckets` and gains `learn_suggestions`;
  `Severity::Ok` populated.
- **`0.3.0`** — Unified `findings[]` array on review / pre-edit /
  drift / session-summary envelopes.
- **`0.2.0`** — `schema_version` field locked; optional
  `session` and `blast_radius` blocks.
