//! Stable JSON schema version.
//!
//! Tracks the `mmk` minor release: every `0.3.x` build emits
//! `0.3.0`. Additive changes (new optional fields, new top-level
//! blocks) do not bump the schema version; renames, removals, type
//! changes, and semantic changes do.
//!
//! v0.3.0 introduces:
//! - new top-level subcommands `review`, `pre-edit`, `drift`, each
//!   with their own envelope plus a `findings[]` array;
//! - the unified findings format with `layer` (hotspot, coupling,
//!   drift, budget — plus reserved `health`/`anchor` slots for v0.4),
//!   `severity` (warn, info, ok), `message`;
//! - `findings[]` overlay on `mmk session-summary` (the renamed
//!   `mmk session` — `session` remains as a CLI alias);
//! - the `review` block (`mode`, per-file `diff` numstat) and the
//!   `drift` block (`base`, `sessions`, `snapshot_labels`).
//!
//! Consumers (LLM harnesses, CI scripts) pin against this rather
//! than the crate version, which they should treat as diagnostic
//! only.

pub const SCHEMA_VERSION: &str = "0.3.0";
