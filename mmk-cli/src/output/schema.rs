//! Stable JSON schema version.
//!
//! Tracks the `mmk` minor release: every `0.4.x` build emits
//! `0.4.0`. Additive changes (new optional fields, new top-level
//! blocks) do not bump the schema version; renames, removals, type
//! changes, and semantic changes do.
//!
//! ## v0.4.0 — what's new
//!
//! - `top_couples[]` entries gain `conditional_probability` and
//!   `wilson_lower_95`. The COUPLING decision rule in `mmk review` /
//!   `mmk pre-edit` now reads Wilson lower bound on conditional
//!   probability rather than symmetric jaccard.
//! - `config.coupling` gains `confidence_threshold` and
//!   `min_sample_size`. Legacy `threshold` is silently mapped to
//!   `confidence_threshold` with a deprecation warning in `--verbose`.
//! - Optional top-level `health` block on `mmk review` / `mmk pre-edit`
//!   when the structural-pattern adapter (mmk-health) fires.
//!   `findings[].layer = "health"` — previously reserved — is now
//!   populated.
//! - `mmk eval` reports COUPLING distribution as `wilson_lower_buckets`
//!   (renamed from `jaccard_buckets`; v0.3 buckets had jaccard
//!   semantics that no longer apply).
//! - `mmk eval --learn` produces a `learn_suggestions[]` array of
//!   high-breadth partners with structured evidence.
//! - `Severity::Ok` now actually appears: pre-edit emits one OK
//!   finding for files with insufficient history under
//!   `coupling.min_sample_size`.
//!
//! ## v0.3.0 — recap
//!
//! - subcommands `review`, `pre-edit`, `drift`, each with their own
//!   envelope plus a `findings[]` array;
//! - the unified findings format with `layer` (hotspot, coupling,
//!   drift, budget — plus reserved `health`/`anchor` slots),
//!   `severity` (warn, info, ok), `message`;
//! - `findings[]` overlay on `mmk session-summary`;
//! - the `review` block (`mode`, per-file `diff` numstat) and the
//!   `drift` block (`base`, `sessions`, `snapshot_labels`).
//!
//! Consumers (LLM harnesses, CI scripts) pin against this rather
//! than the crate version, which they should treat as diagnostic
//! only.

pub const SCHEMA_VERSION: &str = "0.4.0";
