//! Shared fixture helpers for `mmk-core` integration tests.
//!
//! Pulled from `tests/coupling.rs` so the new property tests
//! (`tests/coupling_properties.rs`) and any future co-change /
//! conditional-probability tests can share the construction
//! primitives without copy-paste.

use mmk_core::coupling::CouplingEntry;
use mmk_core::types::{Commit, CommitInfo, FileDelta};
use std::path::PathBuf;

/// Build a synthetic commit. Each touched path gets a `(added,
/// deleted)` delta via the [`IntoDelta`] trait — `&str` shortcuts to
/// `(added=1, deleted=0)` for co-change-only tests, and
/// `(&str, u32, u32)` carries real LOC counts for churn tests. The
/// helper replaces the two divergent `commit()` definitions that
/// previously lived in `coupling.rs` and `churn.rs`.
#[allow(dead_code)]
pub fn commit<D: IntoDelta>(ts: i64, files: &[D]) -> Commit {
    Commit {
        info: CommitInfo {
            sha: format!("{ts:040x}"),
            parent_sha: None,
            timestamp: ts,
            author_email: "t@example.com".into(),
        },
        deltas: files.iter().map(IntoDelta::as_delta).collect(),
    }
}

/// Convert a path-shorthand into a `FileDelta`. Implemented for
/// `&str` (no-LOC shortcut) and `(&str, u32, u32)` (full counts).
/// `as_delta` (not `into_delta`) because Clippy's `wrong_self_convention`
/// flags `into_*` taking `&self`.
pub trait IntoDelta {
    fn as_delta(&self) -> FileDelta;
}

impl IntoDelta for &str {
    fn as_delta(&self) -> FileDelta {
        FileDelta {
            path: PathBuf::from(self),
            added: 1,
            deleted: 0,
        }
    }
}

impl IntoDelta for (&str, u32, u32) {
    fn as_delta(&self) -> FileDelta {
        FileDelta {
            path: PathBuf::from(self.0),
            added: self.1,
            deleted: self.2,
        }
    }
}

/// `PathBuf::from(s)` shorthand. Tests call this enough that the
/// shorter name is worth the import line.
#[allow(dead_code)]
pub fn p(s: &str) -> PathBuf {
    PathBuf::from(s)
}

/// Look up a partner inside a `Vec<CouplingEntry>` by partner path.
#[allow(dead_code)]
pub fn entry_for<'a>(entries: &'a [CouplingEntry], partner: &str) -> Option<&'a CouplingEntry> {
    entries.iter().find(|e| e.partner == p(partner))
}
