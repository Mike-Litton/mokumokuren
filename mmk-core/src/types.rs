//! Shared data types for the metrics pipeline.
//!
//! `mmk-core` is pure: no I/O, no git, no parallelism. The `mmk-git` crate
//! constructs `Vec<Commit>` from a real repository and hands it to the metric
//! functions here.

use std::path::PathBuf;

/// Metadata for a single commit in the analysis window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitInfo {
    pub sha: String,
    pub parent_sha: Option<String>,
    /// Committer timestamp, seconds since the Unix epoch, UTC.
    pub timestamp: i64,
    pub author_email: String,
}

/// One file's contribution to a commit: absolute added/deleted line counts.
///
/// Renames are folded into a single `FileDelta` on the *new* path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDelta {
    pub path: PathBuf,
    pub added: u32,
    pub deleted: u32,
}

/// A commit plus its per-file deltas.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    pub info: CommitInfo,
    pub deltas: Vec<FileDelta>,
}
