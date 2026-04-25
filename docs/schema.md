# `mmk` JSON schema

`mmk analyze --format json` and `mmk session --format json` emit a
single JSON object whose shape is documented here. The
**`schema_version`** field is the only contract consumers should pin
against; `crate_version` is the Cargo version of the producing build
and is diagnostic only.

## Stability contract

`schema_version` tracks the `mmk` minor release: every `0.2.x` build
emits `0.2.0`, every `0.3.x` build emits `0.3.0`.

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
| `schema_version` | string  | Pinned to the `mmk` minor (`"0.2.0"`).                                               |
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

| Field             | Type   | Notes                                                       |
| ----------------- | ------ | ----------------------------------------------------------- |
| `partner`         | string | Repo-relative path of the co-changing file.                 |
| `jaccard`         | float  | Symmetric coupling: `co_change / (touches_a + touches_b - co_change)`. |
| `co_change_count` | uint   | Distinct commits where both files were modified.            |

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

### `blast_radius` (present when `--blast-radius <PATH>` is set)

| Field       | Type     | Notes                                                       |
| ----------- | -------- | ----------------------------------------------------------- |
| `root`      | string   | Echo of the `--blast-radius` path.                          |
| `hops`      | uint     | Always `1` in v0.2.0.                                       |
| `threshold` | float    | Effective Jaccard threshold applied. Resolved as `--blast-radius-threshold` → `[blast_radius] threshold` in `mokumokuren.toml` → built-in default `0.10`. Echoed so consumers can see what filter produced the listed nodes. |
| `nodes`     | array    | `[{ "path": string, "jaccard": float, "co_change_count": uint, "hops": uint }]` sorted desc by jaccard. |
