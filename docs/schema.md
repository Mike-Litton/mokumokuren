# `mmk` JSON schema

Every `mmk` subcommand that takes `--format json` emits a single
JSON object whose shape is documented here. The
**`schema_version`** field is the only contract consumers should pin
against; `crate_version` is the Cargo version of the producing build
and is diagnostic only.

Subcommands at v0.5.0:

- `mmk analyze` — ranked hotspots over a window.
- `mmk session-summary` (alias: `mmk session`) — window + session
  ranking, delta block, plus a `findings[]` overlay (DRIFT, BUDGET,
  ANCHOR on empty session).
- `mmk review` — diff against history (working tree by default;
  `--staged`, `--range A..B`, `--commit <SHA>`). Surfaces STRUCTURE
  / COMPLEXITY alongside HOTSPOT / COUPLING / HEALTH / BUDGET.
- `mmk pre-edit <PATH>` — historical context for a path before edit.
  Absolute path inputs are normalized against the discovered repo
  root.
- `mmk drift --sessions K` — climb signal across K session boundaries.
- `mmk eval --sample N` — sampled noise-floor report (adoption tool).
  `--learn` adds suggestion blocks; `--replay` adds a per-layer
  histogram (cross-repo aggregation).

## Stability contract

`schema_version` tracks the `mmk` minor release: every `0.5.x` build
emits `0.5.0`.

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
| `schema_version` | string  | Pinned to the `mmk` minor (`"0.5.0"`).                                               |
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
| `bulk.max_files`               | uint     | Bulk-filter file threshold.                                  |
| `bulk.max_lines`               | uint     | Bulk-filter line threshold.                                  |
| `blast_radius.threshold`       | float    | Min Jaccard for `--blast-radius` neighborhood.               |
| `coupling.threshold`           | float    | (deprecated) Legacy v0.3 jaccard threshold; v0.4 silently maps it to `confidence_threshold`. |
| `coupling.confidence_threshold`| float    | (v0.4) Min Wilson 95% lower bound on `P(partner | subject)` for COUPLING findings (default 0.20). |
| `coupling.min_sample_size`     | uint     | (v0.4) Min `commits_touching(subject)` before COUPLING fires (default 5). |
| `coupling.ignore_partners`     | string[] | Globs that never fire as the missed partner in COUPLING.     |
| `health.ts.enabled`            | bool     | (v0.4) Whether the TypeScript Health adapter runs.           |
| `health.ts.patterns`           | string[] | (v0.4) Pattern tokens (`registration`, `service`, `test_pair`). |
| `sensor.structure.enabled`     | bool     | (v0.5) Whether the STRUCTURE convention sensor runs.         |
| `sensor.structure.import_majority` | float | (v0.5) Sibling fraction needed for an import to count as the directory's convention (default 0.85). |
| `sensor.complexity.enabled`    | bool     | (v0.5) Whether the COMPLEXITY per-function sensor runs.      |
| `sensor.budget_ramp.enabled`   | bool     | (v0.5) Whether the under-cap BUDGET ramp emits Info @ ≥50% / Warn @ ≥75% (default `true`). |
| `rename_similarity`            | float    | Diff-engine rename threshold.                                |
| `ignores`                      | string[] | Final ignore globs after merging file + CLI sources.         |

### `blast_radius` (present when `--blast-radius <PATH>` is set)

| Field       | Type     | Notes                                                       |
| ----------- | -------- | ----------------------------------------------------------- |
| `root`      | string   | Echo of the `--blast-radius` path.                          |
| `hops`      | uint     | Always `1` in v0.5.0.                                       |
| `threshold` | float    | Effective Jaccard threshold applied. Resolved as `--blast-radius-threshold` → `[blast_radius] threshold` in `mokumokuren.toml` → built-in default `0.10`. Echoed so consumers can see what filter produced the listed nodes. |
| `nodes`     | array    | `[{ "path": string, "jaccard": float, "co_change_count": uint, "hops": uint }]` sorted desc by jaccard. |

## v0.5 envelopes

### `mmk review`

Compares a diff against the historical baseline and emits findings
before any commit lands.

| Field            | Type    | Notes                                                                                  |
| ---------------- | ------- | -------------------------------------------------------------------------------------- |
| `schema_version` | string  | `"0.5.0"`.                                                                             |
| `crate_version`  | string  |                                                                                        |
| `repo`           | object  | Same as analyze. Present only when there are changes (clean tree skips analyze).       |
| `config`         | object  | Same as analyze. Present only when there are changes.                                  |
| `analysis`       | object  | Same as analyze. Present only when there are changes.                                  |
| `review`         | object  | `mode` + per-file diff numstat. Always present.                                        |
| `findings`       | array   | Layer-labeled findings (HOTSPOT, COUPLING, BUDGET, HEALTH v0.4, STRUCTURE / COMPLEXITY v0.5). Always present (possibly empty). |
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

#### `mmk review` bulk-self-filter envelope

When the input diff itself trips `bulk.max_files` or
`bulk.max_lines`, review skips the (expensive) hotspot/coupling
analysis and emits a slimmer envelope. v0.5: STRUCTURE and
COMPLEXITY findings now appear here too — they don't depend on the
analyzer pass and surface alongside BUDGET on over-cap diffs.

| Field            | Type    | Notes                                                            |
| ---------------- | ------- | ---------------------------------------------------------------- |
| `schema_version` | string  | `"0.5.0"`.                                                       |
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
| `schema_version` | string  | `"0.5.0"`.                                                                             |
| `crate_version`  | string  |                                                                                        |
| `repo`           | object  |                                                                                        |
| `config`         | object  |                                                                                        |
| `analysis`       | object  |                                                                                        |
| `pre_edit.path`  | string  | Echo of the queried path. Absolute paths are normalized against the discovered repo root before lookup so hook integrations passing `tool_input.file_path` produce the same signal as relative-path manual invocations. |
| `findings`       | array   | HOTSPOT (Warn), COUPLING (Info), HEALTH (Info, v0.4), STRUCTURE (Info, v0.5), DRIFT (Warn), BUDGET ramp (Info @ ≥50% / Warn @ ≥75%, v0.5). May contain a single OK finding if every layer is silent and the file's history is below `coupling.min_sample_size`. |
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
| `learn_sensor_stats`   | object?  | (v0.5) Present only with `--learn`. Per-sensor distribution: `structure_dir_shapes_seen`, `structure_dir_shapes_above_floor`, `structure_commits_with_fire`, `complexity_functions_seen`, `complexity_nesting_{median,p90,p99}`, `complexity_loc_{median,p90,p99}`. |
| `replay_histogram`     | object?  | (v0.5) Present only with `--replay`. `{ commits_sampled: uint, layers: [{ layer: string, commits_with_fire: uint, fire_rate: float, distinct_paths: uint, total_findings: uint, severity: { ok: uint, info: uint, warn: uint } }] }`. Designed for cross-repo aggregation. |
| `duration_ms`          | uint     |                                                                    |

### `mmk drift`

K snapshot labels + the climb-majority findings.

| Field                       | Type     | Notes                                                                          |
| --------------------------- | -------- | ------------------------------------------------------------------------------ |
| `schema_version`            | string   | `"0.5.0"`.                                                                     |
| `crate_version`             | string   |                                                                                |
| `drift.base`                | string?  | Echo of `--base`.                                                              |
| `drift.sessions`            | uint     | Echo of `--sessions K`.                                                        |
| `drift.snapshot_labels`     | string[] | One commit OID per snapshot, oldest-first.                                     |
| `findings`                  | array    | DRIFT findings. Each has `layer`, `severity`, `path`, `climb_transitions`, `total_transitions`, `latest_rank`. |
| `duration_ms`               | uint     | Wall time for the K analyze runs + drift compute.                              |

### `findings[]` (unified, used by review / pre-edit / session-summary)

| Field      | Type   | Notes                                                                                                |
| ---------- | ------ | ---------------------------------------------------------------------------------------------------- |
| `layer`    | string | One of `"hotspot"`, `"coupling"`, `"drift"`, `"budget"`, `"health"` (v0.4 — populated), `"structure"` (v0.5 — populated), `"complexity"` (v0.5 — populated), `"anchor"` (v0.5 — populated; previously reserved). |
| `severity` | string | `"warn"`, `"info"`, `"ok"`. v0.4 actually populates `"ok"` (pre-edit quiet-file fall-through).      |
| `message`  | string | Human-readable, terse, one-line. The structured detail lives in `layer` / `severity`.                |

### `health` block (v0.4)

Present on `mmk review` and `mmk pre-edit` when
`config.health.ts.enabled = true` AND the Health adapter returned
at least one match. Mirrors the `findings[]` Health entries in
structured form so consumers can read `pattern` + `related` without
parsing the message text.

| Field                  | Type     | Notes                                                              |
| ---------------------- | -------- | ------------------------------------------------------------------ |
| `patterns_evaluated`   | string[] | Pattern tokens the adapter ran (`registration`, `service`, `test_pair`). Echo of `config.health.ts.patterns`. |
| `matches[].pattern`    | string   | Which pattern fired (`registration`, `service`, `test_pair`).      |
| `matches[].subject`    | string   | The file the analysis was about — typically the changed/queried path. |
| `matches[].related`    | string[] | Architectural neighbors / partners surfaced for this match.        |

The `mmk drift` envelope inlines per-finding bookkeeping fields
(`path`, `climb_transitions`, etc.) directly into each finding object;
review / pre-edit / session-summary keep `findings[]` to the unified
shape above.
