# Changelog

All notable changes to Mokumokuren are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project
follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.6.0] - 2026-04-28

Calibration plus the COHESION sensor and a documented Claude Code
hook contract.

### Added

- COHESION sensor — tangled-diff detection on the co-change graph.
  Default on. See [`docs/metrics.md`](docs/metrics.md) and
  [`docs/configuration.md`](docs/configuration.md).
- Claude Code stdin-JSON hook contract on `mmk pre-edit` /
  `mmk review`; output through `hookSpecificOutput`,
  `decision`/`reason` (under `--gate warn`), and `systemMessage`.
  See [`docs/claude-code.md`](docs/claude-code.md) and
  [`docs/schema.md`](docs/schema.md).
- `bulk.max_files` / `bulk.max_lines` overridable from
  `mokumokuren.toml` so wide-grain repos can tune the
  per-commit / per-diff cap. See
  [`docs/configuration.md`](docs/configuration.md).
- `bulk.ignore_for_budget` glob list excluded from diff-time BUDGET
  accounting; surfaces gross / net via optional
  `review.diff.budget` JSON sub-block.
- Per-key monotonic-worsening dedup generalized from COMPLEXITY to
  COUPLING and STRUCTURE; LRU cap at 10 000 entries on save.
- `mmk eval --learn` cohesion calibration:
  `cohesion_tangled_diffs_seen` / `cohesion_components_p95` and a
  suggested `[sensor.cohesion]` block when fire rate > 10 %.

### Changed

- COUPLING gate defaults: `confidence_threshold` 0.20 → 0.30,
  `min_sample_size` 1 → 3. Opt back in via explicit
  `[coupling]` settings.
- BUDGET wording on the bulk-self path now names the skipped
  layers ("HOTSPOT/COUPLING skipped (partners co-touched by
  construction)") instead of "analysis suppressed".
- `quiet_file` fall-through distinguishes "new file (not yet in
  HEAD)" from "present in HEAD but no analyzable history (file may
  be stale or prior touches were filtered as bulk commits)" so
  wide-grain repos don't read filtered-history as "no risk".
- Empty-session WINDOW collapse on `mmk session-summary` (JSON
  `files` key omitted; text emits a suppression notice).
- Schema bumped to `0.6.0`.

### Schema (0.6.0)

See [`docs/schema.md`](docs/schema.md) for the full envelope
contract. Additive changes:

- `findings[].layer = "cohesion"` populated.
- `mmk session-summary`: `files` key now optional (empty session).
- `mmk review`: optional `review.diff.budget` sub-block.
- `config.bulk.{max_files, max_lines, ignore_for_budget}` honoured
  from `mokumokuren.toml`.
- `config.sensor.cohesion` block in the `config` echo.
- `learn_sensor_stats` adds cohesion fields.
- Hook output envelope (`hookSpecificOutput.*` plus optional
  top-level `decision` / `reason` / `systemMessage`).
- COUPLING firing thresholds shift with the new defaults.

## [0.5.0] - 2026-04-27

Two new sensors (STRUCTURE, COMPLEXITY), a continuous BUDGET ramp,
working-tree completeness fixes, per-fire dedup, and a calibration
pass driven by an agent-eval session.

### Added

- **STRUCTURE** sensor: directory-convention finding when ≥3 siblings
  share imports / export shape and a subject diverges. Configurable
  via `[sensor.structure]`. Layer `"structure"` populated.
- **COMPLEXITY** sensor: per-function nesting + LOC budget,
  AST-driven (TypeScript full; other languages refuse rather than
  emit line-scan-quality signal). Configurable via
  `[sensor.complexity]`. Layer `"complexity"` populated.
- **BUDGET ramp** (`[sensor.budget_ramp]`, default on): progressive
  Info @ ≥50% / Warn @ ≥75% of cap on `mmk review` and
  `mmk pre-edit` so the meter is visible before it trips.
- **`mmk eval --replay`**: per-layer fire-rate histogram across the
  sample (composable with `--learn`).
- **Greenfield acknowledgement**: `mmk review` emits one Info
  finding when most of the diff is paths the analyzer hasn't seen,
  so the agent reads silence on history-based layers as expected,
  not as broken.
- **Per-fire dedup**: identical findings against the same HEAD
  within `MMK_DEDUP_TTL_SECONDS` (default 1800) are suppressed.
  `--no-dedup` bypasses.
- **Working-tree untracked files** are now part of `mmk review`'s
  diff (binary / ignore-matched files filtered out).
- **Empty-session ANCHOR nudge**: `mmk session-summary` with no
  commits since base emits an Info finding pointing to `mmk review`
  instead of silently reporting "0 files." Layer `"anchor"`
  populated.

### Changed

- Bulk-self-filter no longer suppresses STRUCTURE / COMPLEXITY:
  cheap per-file sensors surface alongside BUDGET on over-cap diffs.
  HOTSPOT / COUPLING still gate on bulk because they need the
  expensive analyze pass.
- `mmk pre-edit` normalizes absolute path inputs against the
  discovered repo root. Previously, hook integrations passing
  absolute `tool_input.file_path` silently degraded to the OK
  fall-through.
- STRUCTURE majority defaults raised 0.66 → 0.85 after agent-eval
  showed 0.66 fired on directories with heterogeneous file roles.
- Finding wording shifted to indicator-style ("correlates with
  elevated defect rate") — descriptive rather than prescriptive.
- Schema bumped to `0.5.0`.

### Schema (0.5.0)

- `findings[].layer` populates `"structure"`, `"complexity"`, and
  `"anchor"` (last was reserved in v0.4).
- `config.sensor.{structure,complexity,budget_ramp}` blocks added.
- `config.bulk.greenfield_threshold` added.
- `mmk review` envelope: optional `diff.new_file_fraction` (greenfield
  fraction).
- `mmk eval` envelope: optional `replay_histogram` (under `--replay`)
  and `learn_sensor_stats` (under `--learn`).
- `mmk review` bulk-self-filter envelope carries STRUCTURE /
  COMPLEXITY findings in addition to BUDGET.

## [0.4.0] - 2026-04-26

A corrected COUPLING gate plus a structural-pattern adapter for
TypeScript, calibrated against a four-repo benchmark eval.

### Added

- **Wilson 95% lower-bound COUPLING gate.** `mmk review` and
  `mmk pre-edit` fire COUPLING when the Wilson 95% lower bound on
  `P(partner | subject) = co_change / commits_touching(subject)`
  clears `[coupling] confidence_threshold` (default `0.20`), gated
  by a `min_sample_size` floor. The asymmetric conditional-probability
  view fires correctly on real implementation ↔ test pairs while
  suppressing coincidental hot-file neighbors that the symmetric
  jaccard rule used to surface.
- **Quiet-file fall-through.** `mmk pre-edit` on a file with
  insufficient history emits one OK finding so the agent can tell
  "mmk had nothing to say" from "mmk wasn't run."
- **`mmk eval --learn`** synthesizes a suggested
  `[coupling] ignore_partners` block from the sampled findings,
  surfacing system-level noise files that fire across many
  unrelated subjects.
- **Health layer (TypeScript).** New `mmk-health` crate with a
  tree-sitter-driven structural-pattern adapter. Three patterns:
  - `registration` — `*.contribution.ts` files; surfaces sibling
    contribution files.
  - `service` — `interface IFoo` + `registerSingleton(IFoo, ...)`;
    surfaces consumers importing the interface.
  - `test_pair` — `<base>.ts` paired with `<base>.test.ts` /
    `<base>.spec.ts`.
  Configured via `[health.ts] enabled = true`. Layer `"health"`
  populated, with optional top-level `health` block on review /
  pre-edit envelopes.

### Changed

- `top_couples[]` entries gain `conditional_probability` and
  `wilson_lower_95`. `jaccard` is preserved — it still drives
  `--blast-radius`, where the symmetric "what's near this file"
  question is the right one.
- `mmk eval` reports COUPLING distribution as `wilson_lower_buckets`
  (renamed from `jaccard_buckets`).
- `[coupling] threshold` is silently mapped to
  `[coupling] confidence_threshold` for back-compat; `--verbose`
  surfaces a deprecation note.
- Schema bumped to `0.4.0`.

### Measured behavior vs v0.3

Paired eval, four reference repos.

- **Recall** on real fix-shaped commits: 23/61 (38%) → 30/61 (49%)
  partners surfaced.
- **Aggregate firing rate** dropped on three of four repos
  (44%→32%, 54%→41%, 68%→44%); slight rise on the fourth as new
  test partners outweighed suppressed false leads.

The combined effect is "more real co-edits surfaced, fewer false
leads emitted." See `docs/coupling.md` for the full read-out.

### Schema (0.4.0)

- `top_couples[]`: new fields `conditional_probability`,
  `wilson_lower_95`.
- `config.coupling`: new fields `confidence_threshold`,
  `min_sample_size`.
- `config.health.ts`: new sub-block (`enabled`, `patterns`).
- New optional top-level `health` block on `mmk review` and
  `mmk pre-edit` envelopes.
- `mmk eval` envelope: `wilson_lower_buckets` replaces
  `jaccard_buckets`; new optional `learn_suggestions` array under
  `--learn`.
- `findings[].severity = "ok"` populated (pre-edit quiet-file
  fall-through).
- `findings[].layer = "health"` populated.

## [0.3.0] - 2026-04-26

The agent edit loop. Three new subcommands with a unified
layer-labeled findings format, plus `mmk eval` and `--gate` for
adoption and CI use.

### Added

- **`mmk review`** — diff against history. Working tree by default;
  `--staged`, `--range A..B`, `--commit <SHA>`. The `PostToolUse:Edit`
  hook target.
- **`mmk pre-edit <PATH>`** — historical context (rank, expected
  partners, drift) for a path before edit. The `PreToolUse:Edit`
  hook target.
- **`mmk drift --sessions K`** — rank-climb signal across K
  session boundaries.
- **`mmk eval --sample N`** — sampled noise-floor report; the
  adoption tool for calibrating `[coupling]` against a real repo.
- **Unified findings format** with `layer`
  (hotspot, coupling, drift, budget; reserved health/anchor),
  `severity` (warn, info, ok), `message`. Consumed by
  review / pre-edit / drift / session-summary.
- **`--gate {none, warn, error}`** on review / pre-edit /
  session-summary for CI use. Exits non-zero when policy trips.
- **`mmk init --profile {default, js-ts, rust, python, go}`** —
  ecosystem-specific starter configs.
- **`[coupling]` config block** separated from `[blast_radius]`,
  with `ignore_partners` glob list to suppress noisy partners.

### Schema (0.3.0)

- New top-level `findings[]` array on review / pre-edit / drift /
  session-summary envelopes. Shape:
  `{layer: string, severity: string, message: string}`.
- `mmk review` envelope: `review` block (`mode`, per-file `diff`
  numstat).
- `mmk drift` envelope: `drift` block (`base`, `sessions`,
  `snapshot_labels`).

## [0.2.0] - 2026-04-25

Change coupling and the session view, plus a locked JSON schema.

### Added

- **`mmk session-summary`** (alias `mmk session`) — window vs
  session ranking, with `entered_top_n` / `rank_climbs` /
  `churn_of_churn` showing what shifted while you were working.
- **`--blast-radius <PATH>`** on `mmk analyze` — one-hop change-coupling
  neighborhood (jaccard ≥ threshold). Each node is a co-changing
  partner with its jaccard, co-change count, and hop distance.
- **`top_couples[]`** on each ranked file: per-file co-change
  partners.

### Schema (0.2.0)

- New top-level `schema_version` field locks the JSON envelope shape
  for downstream consumers (LLM harnesses, CI scripts) so they can
  pin against the schema rather than the crate version.
- New optional top-level `session` and `blast_radius` blocks.

## [0.1.0] - 2026-04-25

The first vertical slice: `mmk analyze` ranks files by how much
maintenance effort they consume, using only Git history. Ignore
patterns are configured per-repo via `mokumokuren.toml` — the tool
ships with no ecosystem-specific defaults because there is no
ecosystem-neutral right answer.

### Added

- **`mmk analyze`** walks Git history, computes recency-weighted
  churn, and ranks files by hotspot score (weighted churn × log LOC).
  JSON and human-readable text output.
- **`mmk init`** scaffolds a starter `mokumokuren.toml` with
  commented examples covering common ecosystem cases (translations,
  vendored, lockfiles, generated, engine assets). `--force` to
  overwrite.
- Repo-local `mokumokuren.toml` config with an `ignore = [...]`
  glob list. Auto-discovered at the Git work-tree root; override
  with `--config <path>`. CLI `--ignore` unions with the file.
- Strict TOML parsing: unknown keys are an error so typos surface
  immediately.
- Rename detection during per-commit diff (configurable similarity
  threshold; pure renames don't count as a touch).
- Bulk-commit filter: commits exceeding `max_files` or `max_lines`
  are dropped from the analysis.
- Numstat-oracle regression tests pin per-commit `(added, deleted)`
  counts to `git diff --numstat`, guarding against silent divergence
  if the diff pipeline ever changes.

### Performance

- 685 ms on a ~650-commit reference repo, 1.7 s on a ~3.1k-commit
  reference repo, 442 ms on a ~1.8k-commit (in-window) reference repo.

[Unreleased]: https://github.com/mlitton/mmk/compare/v0.5.0...HEAD
[0.5.0]: https://github.com/mlitton/mmk/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/mlitton/mmk/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/mlitton/mmk/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/mlitton/mmk/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/mlitton/mmk/releases/tag/v0.1.0
