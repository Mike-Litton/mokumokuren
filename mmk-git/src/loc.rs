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
use std::sync::Mutex;

use crate::binary::{count_lines, is_binary};
use crate::cache::LocCache;

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
    tree_entries(repo, commit.id, ignores)
}

/// Same shape as [`head_entries`], but anchored on an arbitrary
/// commit. Used by `analyze_at` to enumerate the file set at a
/// historical commit (drift snapshots).
pub fn tree_entries(
    repo: &gix::Repository,
    commit_oid: gix::ObjectId,
    ignores: &GlobSet,
) -> Result<(Vec<HeadEntry>, u64)> {
    let commit = repo
        .find_commit(commit_oid)
        .with_context(|| format!("anchor commit {commit_oid} not found"))?;
    let tree = commit.tree().context("load anchor tree")?;
    let entries = tree
        .traverse()
        .breadthfirst
        .files()
        .context("traverse anchor tree")?;

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
    let (loc, _) = count_loc_at_cached(ts_repo, commit_oid, paths, None)?;
    Ok(loc)
}

/// Cached form of [`count_loc_at`].
///
/// Shares the same blob-OID-keyed cache used by analyze; a blob's
/// line count is a property of its bytes, not of which tree
/// referenced it, so an entry written by an analyze pass is reusable
/// here and vice versa.
pub fn count_loc_at_cached(
    ts_repo: &gix::ThreadSafeRepository,
    commit_oid: gix::ObjectId,
    paths: &ahash::AHashSet<PathBuf>,
    cache: Option<&Mutex<LocCache>>,
) -> Result<(AHashMap<PathBuf, u32>, bool)> {
    if paths.is_empty() {
        return Ok((AHashMap::new(), false));
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

    count_loc_cached(ts_repo, &scoped, cache)
}

/// Inflate each blob in `entries` and count its non-binary lines,
/// without touching the persistent LOC cache.
pub fn count_loc(
    ts_repo: &gix::ThreadSafeRepository,
    entries: &[HeadEntry],
) -> Result<AHashMap<PathBuf, u32>> {
    let (loc, _) = count_loc_cached(ts_repo, entries, None)?;
    Ok(loc)
}

/// Same as [`count_loc`] but consults `cache` (per-blob LOC cache,
/// keyed by blob OID) before inflating. Hits return immediately;
/// misses run the parallel inflate path and write back through.
///
/// Returns `(loc_map, dirty)` where `dirty == true` iff at least one
/// new entry was inserted into `cache` — the caller's signal to call
/// `cache.save(...)`. Mirrors the `*_dirty` bool tracked by the
/// other persistent caches in `analyze_inner`.
pub fn count_loc_cached(
    ts_repo: &gix::ThreadSafeRepository,
    entries: &[HeadEntry],
    cache: Option<&Mutex<LocCache>>,
) -> Result<(AHashMap<PathBuf, u32>, bool)> {
    let mut hits: Vec<(PathBuf, u32)> = Vec::new();
    let mut misses: Vec<&HeadEntry> = Vec::with_capacity(entries.len());

    if let Some(cache_mu) = cache {
        let cache_ref = cache_mu.lock().expect("loc cache poisoned");
        for entry in entries {
            // gix's `ObjectId::as_bytes` returns 20 raw bytes for SHA-1.
            let mut key = [0u8; 20];
            key.copy_from_slice(entry.oid.as_bytes());
            match cache_ref.entries.get(&key) {
                // Binary blobs cached as `None`: hit, but no map entry.
                Some(e) => {
                    if let Some(n) = e.lines {
                        hits.push((entry.path.clone(), n));
                    }
                }
                None => misses.push(entry),
            }
        }
    } else {
        misses.extend(entries.iter());
    }

    let computed: Vec<(PathBuf, [u8; 20], Option<u32>)> = misses
        .par_iter()
        .map_init(
            || {
                let mut r = ts_repo.to_thread_local();
                r.object_cache_size_if_unset(8 * 1024 * 1024);
                r
            },
            |r, entry| -> Result<(PathBuf, [u8; 20], Option<u32>)> {
                let blob = r
                    .find_blob(entry.oid)
                    .with_context(|| format!("load blob for {}", entry.path.display()))?;
                let mut key = [0u8; 20];
                key.copy_from_slice(entry.oid.as_bytes());
                let lines = if is_binary(&blob.data) {
                    None
                } else {
                    Some(count_lines(&blob.data))
                };
                Ok((entry.path.clone(), key, lines))
            },
        )
        .collect::<Result<Vec<_>>>()?;

    let dirty = cache.is_some() && !computed.is_empty();
    let mut out: AHashMap<PathBuf, u32> = AHashMap::with_capacity(hits.len() + computed.len());
    for (path, n) in hits {
        out.insert(path, n);
    }
    if let Some(cache_mu) = cache {
        let mut cache_ref = cache_mu.lock().expect("loc cache poisoned");
        for (path, key, lines) in computed {
            cache_ref.insert(key, lines);
            if let Some(n) = lines {
                out.insert(path, n);
            }
        }
        drop(cache_ref);
    } else {
        for (path, _key, lines) in computed {
            if let Some(n) = lines {
                out.insert(path, n);
            }
        }
    }
    Ok((out, dirty))
}
