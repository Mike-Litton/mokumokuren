# Changelog

All notable changes to Mokumokuren are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project
follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.9.0] - 2026-04-29

Calibration II: tighten attribution, surface confidence, harmonize
silence. No new sensors.

### Changed

- **COMPLEXITY HEAD baseline matches by qualified function identity.**
  `FunctionFact` carries a new `qualified_name` field
  (`ClassName::methodName` for methods inside a class declaration or
  expression; bare `functionName` at top level). The `mmk review`
  HEAD-baseline filter now matches by `qualified_name`, closing off
  the v0.8 cross-class collision where the first AST match by bare
  name (`constructor`, `dispose`, `init`, …) won. The user-visible
  `<P>::<fn>` rendering now reads `<P>::Inner::constructor` for
  methods, identifying both the file and the enclosing class.
- **COUPLING confidence inline on prose.** `coupling_review_missed` /
  `coupling_pre_edit` append ` [low-confidence n=N]` when the fire
  cleared the gate near the floor (`n ≤ min_sample_size + 1` or
  `wilson_lower_95 < 2 × confidence_threshold`). High-confidence
  fires render silently — the suffix is the override-discipline
  signal, not a tier label.
- **Manual `mmk review` text mode prints one canonical clean-state
  line.** `[no actionable signal] no findings (HEAD <sha7>)` on a
  clean working tree (was silent in v0.8). Hook mode and JSON
  envelopes already carried equivalent surfaces; text mode converges.
- **Canonical `[no actionable signal] ` prefix on every fall-through.**
  Six distinct v0.8 wordings ("no signal", "new file", "history
  priors don't apply", "session contains 0 commits", "mmk: no
  findings", "present in HEAD but no analyzable history") all gain
  the same prefix. The reason follows after the prefix unchanged.

### Schema

`schema_version` → `0.9.0`. No JSON shape changes:
`ComplexityFinding.function` field shape is unchanged but its
content shifts from bare-name to qualified-name strings; v0.8
readers iterating findings see longer strings, no parse breakage.
The fall-through prose and the COUPLING confidence suffix are
text-only changes inside `findings[].message`. See
[`docs/schema.md`](docs/schema.md).

## [0.8.0] - 2026-04-29

Calibration pass: tighten existing sensors, no new ones. Driven by
the N=20 vscode multi-agent experiment and Run-3 follow-up.

### Added

- `[sensor.structure] role_patterns`. Files whose stem matches a
  shipped pattern (factory / contribution / registration / module
  / routes / config) emit STRUCTURE divergence as Info, not Warn.
- `[sensor.complexity] delta_warn_pct` / `delta_warn_abs`.
  Severity knobs for delta-weighted COMPLEXITY (see *Changed*).
- `analysis.window_truncation` JSON block on `mmk session-summary`
  with `commits_dropped`, `total_commits`, `max_files`, `max_lines`.
- `docs/agent-claude-md-template.md` — canonical CLAUDE.md content
  for agents using mmk.

### Changed

- COMPLEXITY severity is delta-weighted: pre-existing functions
  emit Info unless `Δ ≥ 50 %` of HEAD or `Δ ≥ 20` absolute. New
  files and new functions still emit Warn.
- TestPair partner discovery covers mirrored `test/` layouts at
  any ancestor (vscode-style nested test trees).
- Pre-edit "new file" wording fires only when the path actually
  isn't in HEAD. Pre-v0.8 the predicate misclassified mature files
  with no recent churn as new.
- Window-truncation moves out of `findings[]` into
  `analysis.window_truncation`. Operational BUDGET (diff-vs-cap,
  ramp, session-aggregate overrun) is unchanged.

### Schema

`schema_version` → `0.8.0`. Additive on field shapes. v0.7 readers
parse v0.8 output unchanged; session-summary envelopes that
previously fired the window-truncation Warn finding now emit one
fewer `findings[]` entry. See [`docs/schema.md`](docs/schema.md).

## [0.7.0] - 2026-04-28

Three composable changes that together make agent failure modes
detectable at commit time on the sub-second hot path. Each change
carries peer-reviewed empirical grounding.

### Added

- **EVASION sensor (`broad_exception` Health pattern).** Detects
  newly-added non-top-level broad TS/JS catch handlers (empty body,
  no parameter, or `any` / `unknown` / `Error` type) by comparing the
  working tree against HEAD. Severity Warn in review; skipped under
  pre-edit (no diff yet). Targets the *"evasive repairs with
  try-except blocks"* failure mode named in arXiv:2509.13941
  *(An Empirical Study on Failures in Automated Issue Solving)* and
  corroborated by FSE 2025 *Suppressed Static Analysis Warnings*
  (broad-except = 18.4 % of Python suppressions across 46 projects).
  Default-on under the `js-ts` profile.
- **Structured `cohesion` block on `mmk review` JSON.** Carries the
  full per-cluster path decomposition (`tangles[].clusters[]`) so
  harnesses can render the split as a commit-split proposal without
  re-parsing finding messages. Absent when no tangle qualifies —
  shape stays additive.
- **HEAD-blob fetch helper (`mmk_git::read_head_blob` /
  `read_head_bodies`).** Reads a path's blob bytes at HEAD without
  callers needing to depend on `gix` directly. Foundation for
  EVASION's working-vs-HEAD comparison.

### Changed

- **COHESION severity Info → Warn.** Empirical grounding: MSR 2026
  *"LGTM! Characteristics of Auto-Merged LLM-based Agentic PRs"*
  (Canelas et al.) — across the AIDev corpus, auto-merged PRs are
  smaller and more focused than non-auto-merged ones. Consumers
  running `--gate warn` now exit 2 on tangled diffs.
- **COHESION fires gain MonotonicSignal dedup.** Key derives from a
  canonical signature over the qualifying clusters; axes are
  `[cluster_count, total_files]`. A re-save of the same tangled diff
  no longer re-fires; adding an unrelated file to one cluster does.
- **TestPair extended to `.js` / `.jsx`.** Implementation-test
  pairing fires on JS sources, not just TS. Cross-language pairing
  (`.js → .test.ts`) is now rejected as a non-match.
- **TSX grammar dispatch fixed.** Pre-v0.7 every `.tsx` file was
  parsed by the non-TSX TypeScript grammar — JSX-bearing files
  silently degraded. v0.7 selects `LANGUAGE_TSX` for `.tsx` and
  `.jsx`, which removes the latent bug across all Health detectors.
- **`mmk-health` adapter coverage extends to `.js` / `.jsx`.**
  Structural detectors (Service, TestPair, EVASION) now run on JS,
  matching the JS/TS-first deployment target.

### Schema

- `SCHEMA_VERSION` bumps to `0.7.0`. All changes are additive — a
  v0.6 reader parses v0.7 output without modification.

### Fixed (in-flight, before tag)

- **COMPLEXITY prose names the absolute cap that fired** instead of
  emitting "directory median unknown" when no siblings exist. The
  `Option<u32>` median was leaking mmk's data state into the agent
  output. Now: `<P>::<fn>: 320 LOC exceeds cap 80; correlates with
  slower comprehension and issue resolution`. When median is also
  available, it appears as enrichment after the cap clause.
  `ComplexityFinding` now carries `cap: u32` so the formatter can
  name the breach.
- **COMPLEXITY suppresses unchanged pre-existing functions.** Filter
  the per-function findings against a HEAD-baseline lookup: keep a
  finding only when the function is newly added, the file is new,
  or the metric strictly worsened vs. HEAD. A pre-existing over-cap
  function that the agent didn't reshape no longer fires noise on
  every fresh agent's first review. Known weakness: a function
  rename leaves the working-tree function with no HEAD match and
  fires as if newly added — a small false-fire cost trade-off
  preferred over structural cross-rename matching.
- **TestPair pairs across the TS family (`.ts` ↔ `.tsx`) and the JS
  family (`.js` ↔ `.jsx`)**, not strict same-extension. Cross-family
  (TS ↔ JS) still rejected. The earlier same-extension rule made
  `.tsx` impl + `.test.ts` partner invisible — a common React
  pattern where the impl needs JSX support and the test doesn't
  render. Now visible.
- **TestPair sees stable test partners** by augmenting `peer_paths`
  with the subject's working-tree directory listing (and any
  `test/` subdirectory). Pre-fix, `peer_paths` came from
  `analysis.loc.keys()`, which only contains files with recent
  churn — an untouched test partner was invisible to the detector
  even when both files existed in HEAD's tree.
- **BUDGET fires get per-finding MonotonicSignal dedup** keyed on
  `budget::files` / `budget::lines` (over-cap path), `budget::ramp::*`
  (under-cap ramp), with axes `[actual]`. Fixes the failure mode
  where a generated artifact (e.g. drizzle's ~3.5k-line
  `snapshot.json`) caused the same BUDGET warning to re-fire on
  every subsequent Edit/Write hook. Now: re-fires only when the
  offending count strictly worsens past the prior emission. The
  bulk-self path goes through the same gate as the under-cap path.
- **COMPLEXITY prose names the `+N vs HEAD` delta** when the agent
  worsens a pre-existing over-cap function. Reads
  `<P>::<fn>: 366 LOC exceeds cap 80 (+3 vs HEAD); ...` — agents
  can judge their contribution at a glance (small additions to an
  inherited problem vs. substantial worsening). `head_actual: Option<u32>`
  on `ComplexityFinding` carries the baseline; the formatter renders
  the clause when present. Omitted on new files / new functions so
  no false delta is fabricated.
- **HEALTH findings get per-key MonotonicSignal dedup** keyed on
  `health::<pattern>::review::<subject>` with constant axes `[1]`.
  Fixes the failure mode where an agent doing N Edits in a row sees
  the same TestPair / EVASION warning N times: envelope-level dedup
  keys on the whole findings hash, which changes whenever the
  broader diff grows (BUDGET ramp tier moves, COMPLEXITY actuals
  shift, etc.); the per-key gate suppresses identical re-fires
  until TTL expires or the partner is touched. Same dedup
  discipline already applied to BUDGET / COUPLING / COHESION /
  COMPLEXITY / STRUCTURE.

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
- `mmk cache` adds a `loc` scope for the per-blob LOC cache
  (`loc.bincode.v1`), shared by analyze / review / pre-edit /
  session-summary / drift. The prior `loc` scope, which targeted
  head-tree enumeration, is renamed `head-tree`.
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

[Unreleased]: https://github.com/Mike-Litton/mokumokuren/compare/v0.9.0...HEAD
[0.9.0]: https://github.com/Mike-Litton/mokumokuren/compare/v0.8.0...v0.9.0
[0.8.0]: https://github.com/Mike-Litton/mokumokuren/compare/v0.7.0...v0.8.0
[0.7.0]: https://github.com/Mike-Litton/mokumokuren/compare/v0.6.0...v0.7.0
[0.6.0]: https://github.com/Mike-Litton/mokumokuren/compare/v0.5.0...v0.6.0
[0.5.0]: https://github.com/Mike-Litton/mokumokuren/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/Mike-Litton/mokumokuren/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/Mike-Litton/mokumokuren/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/Mike-Litton/mokumokuren/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/Mike-Litton/mokumokuren/releases/tag/v0.1.0
