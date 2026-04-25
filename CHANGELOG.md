# Changelog

All notable changes to Mokumokuren are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project
follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

[Unreleased]: https://github.com/mlitton/mmk/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/mlitton/mmk/releases/tag/v0.1.0
