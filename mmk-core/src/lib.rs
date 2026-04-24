//! Pure metrics engine for Mokumokuren. No I/O, no git — consumes a
//! `Vec<Commit>` produced by `mmk-git` and returns ranked hotspots.

pub mod churn;
pub mod hotspot;
pub mod types;

pub use hotspot::HotspotEntry;
pub use types::{Commit, CommitInfo, FileDelta};

/// Compute the most recent commit timestamp per path within a window.
#[must_use]
pub fn last_modified(commits: &[Commit]) -> ahash::AHashMap<std::path::PathBuf, i64> {
    let mut out: ahash::AHashMap<std::path::PathBuf, i64> = ahash::AHashMap::new();
    for commit in commits {
        for delta in &commit.deltas {
            let entry = out.entry(delta.path.clone()).or_insert(i64::MIN);
            if commit.info.timestamp > *entry {
                *entry = commit.info.timestamp;
            }
        }
    }
    out
}
