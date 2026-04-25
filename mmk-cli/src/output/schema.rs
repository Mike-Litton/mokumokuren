//! Stable JSON schema version.
//!
//! Tracks the `mmk` minor release: every `0.2.x` build emits
//! `0.2.0`. Additive changes (new optional fields, new top-level
//! blocks) do not bump the schema version; renames, removals, type
//! changes, and semantic changes do.
//!
//! Consumers (LLM harnesses, CI scripts) pin against this rather
//! than the crate version, which they should treat as diagnostic
//! only.

pub const SCHEMA_VERSION: &str = "0.2.0";
