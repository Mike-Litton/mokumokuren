//! Shared helpers for `mmk-health` integration tests.
//!
//! Most pattern tests construct `Vec<PathBuf>` peers and feed them
//! to `analyze_ts`. The only genuine duplication today is the `p()`
//! shorthand; richer scaffolding lives inline because the
//! per-pattern shapes diverge (Pattern B reads peer bodies from
//! disk; Pattern A and C work in-memory).

use std::path::PathBuf;

/// `PathBuf::from(s)` shorthand. Tests construct enough peer paths
/// that the shorter name is worth the import line.
#[allow(dead_code)]
pub fn p(s: &str) -> PathBuf {
    PathBuf::from(s)
}
