# Changelog

All notable changes to Mokumokuren are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project
follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.4.0] - 2026-04-26

Closes the empirical gaps surfaced by validating v0.3 against the
four-repo example set in `~/Code/js-examples`. Three semantic shifts
in COUPLING behavior plus a new structural-pattern adapter.

### Added

- **Wilson 95 % lower-bound COUPLING gate.** `mmk review` and
  `mmk pre-edit` now fire COUPLING when the Wilson 95 % lower bound on
  `P(partner | subject) = co_change / commits_touching(subject)` clears
  `[coupling] confidence_threshold` (default `0.20`), gated by a
  `min_sample_size` floor (default `5`). The legacy symmetric jaccard
  rule was statistically miscalibrated for hot files; the asymmetric
  conditional-probability view fires correctly on the
  `runInTerminalTool.ts ↔ *.test.ts` 54/203 pair while continuing to
  suppress the `chatWidget.ts ↔ chatInputPart.ts` 27/133 borderline.
- **Quiet-file fall-through.** `mmk pre-edit` on a file with
  `commits_touching < min_sample_size` and no other firing layer emits
  one `Severity::Ok` finding so the agent can distinguish "mmk had
  nothing to say" from "mmk wasn't run."
- **`mmk eval --learn`.** Synthesizes a suggested
  `[coupling] ignore_partners` block from the sampled findings using
  partner breadth + inverse conditional probability — surfaces
  system-level noise files (e.g. `CHANGELOG.md`) that fire across many
  unrelated subjects.
- **Health layer (TypeScript).** New `mmk-health` crate with a
  tree-sitter-driven structural-pattern adapter. Three patterns ship:
  - `registration` — `*.contribution.ts` files; surfaces sibling
    contribution files in the same `contrib/` subtree.
  - `service` — `interface IFoo` + `registerSingleton(IFoo, ...)`;
    surfaces consumers importing the interface.
  - `test_pair` — `<base>.ts` paired with `<base>.test.ts` /
    `<base>.spec.ts`.
  Configured via `[health.ts] enabled = true`; `js-ts` profile flips
  it on with all three patterns. `findings[].layer = "health"` —
  previously reserved — is now populated, with optional top-level
  `health` block for structured consumption.
- New `mmk-cli` shared module `commands/common.rs` consolidates
  `load_config_file`, `apply_coupling_file` (now pure — returns a
  list of `CouplingDeprecation`s), `apply_health_file`,
  `coupling_findings` (single emission point for review/pre-edit),
  `health_to_finding`, and `health_severity_for_review`.

### Changed

- `CouplingEntry` gains `conditional_probability` and `wilson_lower_95`
  fields. `jaccard` is preserved — it still drives `--blast-radius`,
  the symmetric "what's near this file" surface where symmetry is the
  right question.
- `compute_findings` in review and pre-edit now accepts the
  pre-computed `commits_touching` map instead of re-deriving it.
- `mmk eval` reports COUPLING distribution as `wilson_lower_buckets`
  (renamed from `jaccard_buckets`; the v0.3 buckets had jaccard
  semantics that no longer apply).
- `[coupling] threshold` is silently mapped to
  `[coupling] confidence_threshold` for back-compat;
  `--verbose` surfaces a deprecation note.
- Schema bumped to `0.4.0`.

### Performance

- `top_couples_for` and `compute_conditional_couples_for` now share
  an internal `collect_couples_for` collector; the previous
  implementation called `top_couples_for` then re-sorted the same
  vectors, paying for two `O(N log N)` sorts per target.
- `mmk-health/src/ts/service.rs` caps the consumer-search read loop
  at 500 peers to avoid scanning the whole workbench when the
  declared interface has no consumers within the first batch.

### Measured behavior vs v0.3 (paired eval, four repos)

v0.4 is strictly ≥ v0.3 on every measured axis.

**Recall on real fix-shaped commits (32 commits, 61 actual partners):**
v0.3 surfaced 23 (38 %); v0.4 surfaces 30 (49 %). Per-repo (sample
size in commits): better on the ~16k repo (1→2), ~18k repo (12→13),
~140k repo (3→8); tied on the ~10k repo (7→7).

**Aggregate firing rate (`mmk eval --sample 200`):** lower noise on
the ~16k repo (44 % → 32 %), ~10k repo (54 % → 41 %), ~18k repo
(68 % → 44 %); slight rise on the ~140k repo (56 % → 58 %, where
the new test partners caught outweigh the v0.3 false leads
suppressed).

The combined effect is "more real co-edits surfaced, fewer false
leads emitted" — what the metric swap was designed for. See
`docs/coupling.md` for the full read-out.

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
- `findings[].severity = "ok"` now actually appears (pre-edit
  quiet-file fall-through).
- `findings[].layer = "health"` is now populated.

## [0.1.0] - 2026-04-25

The first vertical slice: `mmk analyze` ranks files by how much maintenance
effort they consume, using only Git history. Ignore patterns are
configured per-repo via `mokumokuren.toml` — the tool ships with no
ecosystem-specific defaults because there is no ecosystem-neutral right
answer.

### Added

- `mmk analyze` command: walks Git history, computes recency-weighted churn,
  ranks files by hotspot score (weighted churn × log LOC). JSON and
  human-readable text output.
- `mmk init` command: scaffolds a starter `mokumokuren.toml` in the
  current directory with commented-out examples covering the common
  ecosystem cases (translations, vendored, lockfiles, generated, engine
  assets). `--force` to overwrite an existing file.
- Repo-local `mokumokuren.toml` config file with an `ignore = [...]`
  glob list. Auto-discovered at the Git work-tree root; override with
  `--config <path>`. CLI `--ignore` unions with the file.
- Strict TOML parsing: unknown keys are an error so typos like
  `ignores = ...` surface immediately.
- Rename detection during per-commit diff (configurable similarity
  threshold; pure renames don't count as a touch).
- Bulk-commit filter: commits exceeding `max_files` or `max_lines` are
  dropped from the analysis with an early-abort fast path that avoids
  inflating blobs for commits about to be discarded.
- `--verbose` reports which config file (if any) was loaded and how
  many HEAD paths were excluded by ignore globs.
- Numstat-oracle regression tests: per-commit `(added, deleted)` counts
  are pinned to `git diff --numstat` values, guarding against silent
  divergence if the diff algorithm or normalization pipeline ever
  changes.

### Performance

- HEAD-path filter: paths deleted before HEAD or filtered by ignore
  globs don't have their historical blobs inflated during per-commit
  diff.
- LOC counting only runs for paths that actually churned in-window —
  not every blob in HEAD.
- Cache hoisting: per-thread blob cache + pack cache are reused across
  commits inside the rayon worker.
- 685 ms on the kindling repo (647 commits), 1.7 s on godot (3,139
  commits), 442 ms on git's own source (1,793 commits in-window).

[Unreleased]: https://github.com/mlitton/mmk/compare/v0.4.0...HEAD
[0.4.0]: https://github.com/mlitton/mmk/compare/v0.1.0...v0.4.0
[0.1.0]: https://github.com/mlitton/mmk/releases/tag/v0.1.0
