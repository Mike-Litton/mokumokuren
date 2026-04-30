//! HEAD-blob fetch helper.
//!
//! Reads the bytes of a path's blob at HEAD without requiring the
//! caller to dance with gix's tree-walk API. Returns `None` for paths
//! not present at HEAD (new files); returns `Some(bytes)` for any
//! path that resolves to a blob.
//!
//! Hot-path use case is EVASION: for each changed file, we want the
//! HEAD body to compute a working-vs-HEAD broad-handler delta. The
//! per-call cost is one `peel_to_entry_by_path` walk plus one blob
//! inflate; sub-millisecond for typical files. The caller is
//! responsible for batching across the changed-set.

use ahash::{AHashMap, AHashSet};
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// Read the bytes of `path` from HEAD's tree.
///
/// `Ok(None)` means the path is not present at HEAD (new file), or
/// the path resolved to a non-blob entry (a directory). `Ok(Some(_))`
/// is the raw blob bytes — UTF-8 conversion is the caller's
/// responsibility.
pub fn read_head_blob(repo: &gix::Repository, path: &Path) -> Result<Option<Vec<u8>>> {
    let Ok(commit) = repo.head_commit() else {
        // Unborn HEAD: no tree to walk.
        return Ok(None);
    };
    let mut tree = commit.tree().context("load HEAD tree")?;
    let Some(entry) = tree
        .peel_to_entry_by_path(path)
        .with_context(|| format!("peel HEAD entry for {}", path.display()))?
    else {
        return Ok(None);
    };
    if !entry.mode().is_blob() {
        return Ok(None);
    }
    let object = entry.object().with_context(|| {
        format!(
            "fetch HEAD blob object for {} ({})",
            path.display(),
            entry.id()
        )
    })?;
    Ok(Some(object.data.clone()))
}

/// `true` iff `path` resolves to a blob in HEAD's tree.
///
/// Cheap predicate for distinguishing "truly new file (absent from
/// HEAD)" from "in HEAD but with no analyzable history" — the
/// pre-edit fall-through wording depends on the difference.
/// `analyze.loc.contains_key()` is *not* a substitute: that map only
/// covers paths that churned within the analysis window, so any
/// existing file with no recent churn (or whose touches all fell to
/// the bulk filter) falsely registers as new.
#[must_use]
pub fn path_in_head(work_dir: &Path, path: &Path) -> bool {
    let Ok(repo) = gix::open(work_dir) else {
        return false;
    };
    matches!(read_head_blob(&repo, path), Ok(Some(_)))
}

/// Batched HEAD-blob fetch over `paths` rooted at `work_dir`.
///
/// Returns a map from path → UTF-8-decoded body for every path that
/// resolves to a text blob at HEAD. Paths absent at HEAD, paths that
/// fail UTF-8 decode (binaries surfacing under a text extension), or
/// any per-path error are silently dropped — EVASION is opportunistic
/// (no HEAD body just means new-file semantics on that subject).
///
/// Opening the repo once and reusing it across the batch is the cost
/// win that makes EVASION sub-millisecond per call. Letting callers
/// in mmk-cli reach into gix would force a transitive gix dep on
/// mmk-cli for one function — this wrapper keeps the dependency
/// graph clean.
#[must_use]
pub fn read_head_bodies(work_dir: &Path, paths: &[PathBuf]) -> AHashMap<PathBuf, String> {
    let mut out = AHashMap::new();
    let Ok(repo) = gix::open(work_dir) else {
        return out;
    };
    for p in paths {
        let Ok(Some(bytes)) = read_head_blob(&repo, p) else {
            continue;
        };
        if let Ok(body) = String::from_utf8(bytes) {
            out.insert(p.clone(), body);
        }
    }
    out
}

/// Batched [`path_in_head`] — opens the repo once and returns the
/// subset of `paths` that resolve to a blob in HEAD's tree.
///
/// Unlike [`read_head_bodies`], binary blobs and non-UTF-8 entries
/// count as present: the predicate is "does HEAD know this path,"
/// not "can we read it as text." Callers doing greenfield-style
/// classification should prefer this over `read_head_bodies` to skip
/// the inflate cost when bodies aren't needed.
#[must_use]
pub fn paths_in_head(work_dir: &Path, paths: &[PathBuf]) -> AHashSet<PathBuf> {
    let mut out = AHashSet::new();
    let Ok(repo) = gix::open(work_dir) else {
        return out;
    };
    for p in paths {
        if matches!(read_head_blob(&repo, p), Ok(Some(_))) {
            out.insert(p.clone());
        }
    }
    out
}
