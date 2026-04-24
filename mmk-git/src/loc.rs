//! HEAD LOC map: walk the HEAD tree, counting lines in each non-binary blob.

use ahash::AHashMap;
use anyhow::{Context, Result};
use gix::bstr::ByteSlice;
use globset::GlobSet;
use rayon::prelude::*;
use std::path::PathBuf;

use crate::binary::{count_lines, is_binary};

/// Returns an empty map if HEAD is unborn. Blob reads are parallelized
/// across rayon workers; each worker gets its own thread-local
/// `gix::Repository` via `ts_repo.to_thread_local()`.
pub fn head_loc_map(
    repo: &gix::Repository,
    ts_repo: &gix::ThreadSafeRepository,
    ignores: &GlobSet,
) -> Result<AHashMap<PathBuf, u32>> {
    let Ok(commit) = repo.head_commit() else {
        return Ok(AHashMap::new());
    };
    let tree = commit.tree().context("load HEAD tree")?;
    let entries = tree
        .traverse()
        .breadthfirst
        .files()
        .context("traverse HEAD tree")?;

    let candidates: Vec<_> = entries
        .into_iter()
        .filter(|e| e.mode.is_blob())
        .filter_map(|e| {
            let path_str = e.filepath.to_str_lossy().into_owned();
            if !ignores.is_empty() && ignores.is_match(&path_str) {
                return None;
            }
            Some((path_str, e.oid))
        })
        .collect();

    let pairs: Vec<(PathBuf, u32)> = candidates
        .par_iter()
        .map_init(
            || {
                let mut r = ts_repo.to_thread_local();
                r.object_cache_size_if_unset(8 * 1024 * 1024);
                r
            },
            |r, (path_str, oid)| -> Result<Option<(PathBuf, u32)>> {
                let blob = r
                    .find_blob(*oid)
                    .with_context(|| format!("load blob for {path_str}"))?;
                if is_binary(&blob.data) {
                    return Ok(None);
                }
                Ok(Some((PathBuf::from(path_str), count_lines(&blob.data))))
            },
        )
        .filter_map(Result::transpose)
        .collect::<Result<Vec<_>>>()?;

    Ok(pairs.into_iter().collect())
}
