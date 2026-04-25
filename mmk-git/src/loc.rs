//! HEAD tree enumeration + per-blob line counting.
//!
//! Splitting this in two is a performance win: the path enumeration (a
//! tree walk with no blob loads) is cheap enough to run synchronously
//! before per-commit diff, and its output is the path set we filter
//! against inside `diff_commit` to avoid inflating blobs for paths that
//! won't rank. The expensive half — counting newlines in every HEAD
//! blob — then runs in parallel with per-commit diff via `rayon::join`.

use ahash::AHashMap;
use anyhow::{Context, Result};
use gix::bstr::ByteSlice;
use globset::GlobSet;
use rayon::prelude::*;
use std::path::PathBuf;

use crate::binary::{count_lines, is_binary};

#[derive(Debug, Clone)]
pub struct HeadEntry {
    pub path: PathBuf,
    pub oid: gix::ObjectId,
}

/// Fast: walk HEAD tree, collect blob entries, apply ignore globs. No
/// blob loads. Empty vec if HEAD is unborn.
///
/// Returns `(entries, head_paths_ignored)` where the second value is
/// the count of HEAD blobs that matched an ignore glob and were
/// excluded.
pub fn head_entries(repo: &gix::Repository, ignores: &GlobSet) -> Result<(Vec<HeadEntry>, u64)> {
    let Ok(commit) = repo.head_commit() else {
        return Ok((Vec::new(), 0));
    };
    let tree = commit.tree().context("load HEAD tree")?;
    let entries = tree
        .traverse()
        .breadthfirst
        .files()
        .context("traverse HEAD tree")?;

    let mut out = Vec::with_capacity(entries.len());
    let mut head_paths_ignored: u64 = 0;
    for entry in entries {
        if !entry.mode.is_blob() {
            continue;
        }
        let path_str = entry.filepath.to_str_lossy().into_owned();
        if !ignores.is_empty() && ignores.is_match(&path_str) {
            head_paths_ignored += 1;
            continue;
        }
        out.push(HeadEntry {
            path: PathBuf::from(path_str),
            oid: entry.oid,
        });
    }
    Ok((out, head_paths_ignored))
}

/// Tree walk at an arbitrary commit OID, scoped to a fixed path allowlist.
///
/// Used by `mmk session` to compute LOC at the session base (not at HEAD),
/// so `session.relative_churn` divides by the file's size at the start of
/// the session rather than its size now.
///
/// Returns `path -> u32` LOC. Files in `paths` that don't exist at
/// `commit_oid` are silently absent from the output (consistent with
/// `count_loc`'s "missing blob → no entry" semantics — `rank()`
/// filters by `loc.contains_key()`).
pub fn count_loc_at(
    ts_repo: &gix::ThreadSafeRepository,
    commit_oid: gix::ObjectId,
    paths: &ahash::AHashSet<PathBuf>,
) -> Result<AHashMap<PathBuf, u32>> {
    if paths.is_empty() {
        return Ok(AHashMap::new());
    }
    let repo = ts_repo.to_thread_local();
    let commit = repo
        .find_commit(commit_oid)
        .with_context(|| format!("load base commit {commit_oid}"))?;
    let tree = commit.tree().context("load base tree")?;
    let entries = tree
        .traverse()
        .breadthfirst
        .files()
        .context("traverse base tree")?;

    let scoped: Vec<HeadEntry> = entries
        .iter()
        .filter_map(|entry| {
            if !entry.mode.is_blob() {
                return None;
            }
            let path_str = entry.filepath.to_str_lossy().into_owned();
            let path = PathBuf::from(&path_str);
            if !paths.contains(&path) {
                return None;
            }
            Some(HeadEntry {
                path,
                oid: entry.oid,
            })
        })
        .collect();

    count_loc(ts_repo, &scoped)
}

/// Slow: inflate each blob in `entries` and count its non-binary lines.
pub fn count_loc(
    ts_repo: &gix::ThreadSafeRepository,
    entries: &[HeadEntry],
) -> Result<AHashMap<PathBuf, u32>> {
    let pairs: Vec<(PathBuf, u32)> = entries
        .par_iter()
        .map_init(
            || {
                let mut r = ts_repo.to_thread_local();
                r.object_cache_size_if_unset(8 * 1024 * 1024);
                r
            },
            |r, entry| -> Result<Option<(PathBuf, u32)>> {
                let blob = r
                    .find_blob(entry.oid)
                    .with_context(|| format!("load blob for {}", entry.path.display()))?;
                if is_binary(&blob.data) {
                    return Ok(None);
                }
                Ok(Some((entry.path.clone(), count_lines(&blob.data))))
            },
        )
        .filter_map(Result::transpose)
        .collect::<Result<Vec<_>>>()?;

    Ok(pairs.into_iter().collect())
}
