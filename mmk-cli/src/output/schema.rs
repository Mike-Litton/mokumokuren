//! Stable JSON schema version.
//!
//! Tracks the `mmk` minor release: every `0.5.x` build emits
//! `0.5.0`. Additive changes (new optional fields, new top-level
//! blocks) do not bump the schema version; renames, removals, type
//! changes, and semantic changes do.
//!
//! ## v0.5.0 — what's new
//!
//! - `findings[].layer = "structure"` and
//!   `findings[].layer = "complexity"` are populated. Two new
//!   directory-aggregated sensors (mmk-core::sensors::structure /
//!   complexity) ship: STRUCTURE surfaces convention divergence
//!   when ≥3 siblings share imports / export shape; COMPLEXITY
//!   fires per-function on nesting and LOC. Both are on by default
//!   under `[sensor]` config.
//! - `findings[].layer = "anchor"` — previously reserved — is now
//!   populated. `mmk session-summary` emits an Anchor Info finding
//!   when the session is empty (typically `--base HEAD`), nudging
//!   the user toward `mmk review` for working-tree state.
//! - New `[sensor.budget_ramp]` block (default `enabled = true`):
//!   `mmk review` and `mmk pre-edit` emit progressive Info @ ≥50%
//!   of cap and Warn @ ≥75% of cap so the agent sees the budget
//!   meter climb before it snaps over. Set `enabled = false` to
//!   silence.
//! - `mmk eval --replay` populates an optional `replay_histogram`
//!   block with per-layer fire rate, distinct paths surfaced, and
//!   severity mix across the sampled commits — designed for
//!   cross-repo aggregation.
//! - Bulk-self-filter no longer suppresses the per-file sensors:
//!   STRUCTURE and COMPLEXITY findings now surface alongside
//!   BUDGET on over-cap diffs (HOTSPOT/COUPLING still gated on the
//!   bulk path because they need the expensive analyze pass).
//! - `mmk pre-edit` normalizes absolute path inputs against the
//!   discovered repo root before lookup. Hook integrations passing
//!   `tool_input.file_path` (Claude Code) no longer silently
//!   degrade to the OK fall-through.
//! - STRUCTURE majority defaults raised from 0.66 → 0.85 after
//!   first-run agent calibration showed 0.66 fired too readily on
//!   directories with heterogeneous file roles.
//!
//! ## v0.4.0 — recap
//!
//! - Wilson 95 % lower-bound COUPLING gate; `top_couples[]` gains
//!   `conditional_probability` and `wilson_lower_95`.
//! - `config.coupling` gains `confidence_threshold` /
//!   `min_sample_size`; legacy `threshold` mapped with deprecation.
//! - Health adapter (TypeScript) ships;
//!   `findings[].layer = "health"` populated; optional
//!   `health` block on review / pre-edit envelopes.
//! - `mmk eval` renames `jaccard_buckets` → `wilson_lower_buckets`;
//!   `--learn` emits `learn_suggestions[]`.
//! - `Severity::Ok` populated by pre-edit's quiet-file fall-through.
//!
//! Consumers (LLM harnesses, CI scripts) pin against this rather
//! than the crate version, which they should treat as diagnostic
//! only.

pub const SCHEMA_VERSION: &str = "0.5.0";
