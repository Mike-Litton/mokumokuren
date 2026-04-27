//! Per-commit tree diff with rename detection.

use ahash::AHashSet;
use anyhow::{Context, Result};
use gix::bstr::ByteSlice;
use gix::diff::Rewrites;
use gix::object::tree::diff::{Action, Change};
use mmk_core::types::{CommitInfo, FileDelta};
use std::path::PathBuf;

use crate::binary::count_lines;

fn bstr_to_pathbuf(bytes: &[u8]) -> PathBuf {
    PathBuf::from(String::from_utf8_lossy(bytes).into_owned())
}

/// Type alias for the HEAD-path filter: byte-slice keys so we can probe
/// with a `&[u8]` slice from the diff change's `location` without
/// allocating a `PathBuf`.
pub type HeadPathBytes = AHashSet<Vec<u8>>;

/// Build a reusable blob-diff resource cache. Expensive (constructs the
/// filter pipeline and attribute stack from repo config); call once per
/// worker thread and reuse across commits via [`diff_commit`].
pub fn make_resource_cache(repo: &gix::Repository) -> Result<gix::diff::blob::Platform> {
    repo.diff_resource_cache(
        gix::diff::blob::pipeline::Mode::ToGit,
        gix::diff::blob::pipeline::WorktreeRoots::default(),
    )
    .context("failed to build diff resource cache")
}

#[derive(Debug, Default)]
pub struct DiffStats {
    pub skipped: u64,
    pub bulk_filtered: bool,
}

/// Diff `commit_info` against its first parent.
///
/// If the commit has no parent, diff against the empty tree. Binary files
/// are excluded; renames are tracked at `rename_similarity` (0.0–1.0) and
/// reported as a single `FileDelta` on the new path. The caller-supplied
/// `resource_cache` is reused across calls and cleared between commits.
///
/// `head_paths` — if `Some`, only paths present in this set contribute to
/// the returned deltas. Paths not in `head_paths` (files that no longer
/// exist at HEAD, or paths filtered by ignore globs) are counted in the
/// returned `DiffStats::skipped` instead of having their blobs inflated.
/// This is the largest single perf lever available: inflate dominates
/// CPU, and every churn event for a non-HEAD path is discarded downstream
/// anyway.
///
/// `bulk_limits` — if `(max_files, max_lines)` is exceeded during the
/// tree walk, we abort early and return with `bulk_filtered = true`. The
/// partial deltas are still returned (the caller will discard them) so we
/// don't need to re-run the walk; the point is to stop inflating *more*
/// blobs for a commit we're about to throw away.
pub fn diff_commit(
    repo: &gix::Repository,
    resource_cache: &mut gix::diff::blob::Platform,
    commit_info: &CommitInfo,
    rename_similarity: f32,
    head_paths: Option<&HeadPathBytes>,
    bulk_limits: (u32, u32),
) -> Result<(Vec<FileDelta>, DiffStats)> {
    let oid: gix::ObjectId = gix::ObjectId::from_hex(commit_info.sha.as_bytes())
        .with_context(|| format!("invalid commit sha: {}", commit_info.sha))?;
    let commit = repo
        .find_commit(oid)
        .with_context(|| format!("commit not found: {}", commit_info.sha))?;
    let new_tree = commit.tree().context("failed to load commit tree")?;

    let empty = repo.empty_tree();
    let old_tree = match commit_info.parent_sha.as_ref() {
        Some(psha) => {
            let pid = gix::ObjectId::from_hex(psha.as_bytes())
                .with_context(|| format!("invalid parent sha: {psha}"))?;
            repo.find_commit(pid)
                .with_context(|| format!("parent commit not found: {psha}"))?
                .tree()
                .context("failed to load parent tree")?
        }
        None => empty.clone(),
    };

    let rewrites = Rewrites {
        copies: None,
        percentage: Some(rename_similarity.clamp(0.0, 1.0)),
        limit: 1000,
        track_empty: false,
    };

    resource_cache.clear_resource_cache_keep_allocation();

    let (max_files, max_lines) = bulk_limits;
    let mut deltas: Vec<FileDelta> = Vec::new();
    let mut skipped: u64 = 0;
    let mut bulk_filtered = false;
    let mut running_files: u32 = 0;
    let mut running_lines: u32 = 0;
    let mut platform = old_tree
        .changes()
        .context("failed to build tree-diff platform")?;
    platform.options(|opts| {
        opts.track_path();
        opts.track_rewrites(Some(rewrites));
    });

    // Returns true if the path should contribute to metrics. Probes the
    // HEAD-path set by byte-slice so we don't allocate a `PathBuf` just
    // to do the lookup — at multi-thousand-commit scale, that allocation
    // was visible in profiles.
    let at_head = |bytes: &[u8]| -> bool { head_paths.map_or(true, |set| set.contains(bytes)) };

    let result = platform.for_each_to_obtain_tree(
        &new_tree,
        |change: Change<'_, '_, '_>| -> Result<Action, anyhow::Error> {
            // Each arm either returns early (not a blob / not at HEAD /
            // binary / no-op) or pushes one delta. After the match, we
            // update running totals and early-abort if we've crossed the
            // bulk threshold. This keeps us from inflating blobs for a
            // commit we're going to throw away downstream.
            let pre_len = deltas.len();
            match change {
                Change::Addition {
                    location,
                    entry_mode,
                    id,
                    ..
                } => {
                    if !entry_mode.is_blob() {
                        return Ok(Action::Continue);
                    }
                    if !at_head(location.as_bytes()) {
                        skipped += 1;
                        return Ok(Action::Continue);
                    }
                    let blob = id.object().context("blob load")?;
                    if crate::binary::is_binary(&blob.data) {
                        return Ok(Action::Continue);
                    }
                    let added = count_lines(&blob.data);
                    if added > 0 {
                        deltas.push(FileDelta {
                            path: bstr_to_pathbuf(location.as_bytes()),
                            added,
                            deleted: 0,
                        });
                    }
                }
                Change::Deletion {
                    location,
                    entry_mode,
                    id,
                    ..
                } => {
                    if !entry_mode.is_blob() {
                        return Ok(Action::Continue);
                    }
                    if !at_head(location.as_bytes()) {
                        skipped += 1;
                        return Ok(Action::Continue);
                    }
                    let blob = id.object().context("blob load")?;
                    if crate::binary::is_binary(&blob.data) {
                        return Ok(Action::Continue);
                    }
                    let deleted = count_lines(&blob.data);
                    if deleted > 0 {
                        deltas.push(FileDelta {
                            path: bstr_to_pathbuf(location.as_bytes()),
                            added: 0,
                            deleted,
                        });
                    }
                }
                Change::Modification {
                    location,
                    previous_entry_mode,
                    entry_mode,
                    ..
                } => {
                    if !entry_mode.is_blob() || !previous_entry_mode.is_blob() {
                        return Ok(Action::Continue);
                    }
                    if !at_head(location.as_bytes()) {
                        skipped += 1;
                        return Ok(Action::Continue);
                    }
                    // gix's LCS-based `line_counts` returns `None` for
                    // binary blobs (either side), so binary detection is
                    // inline here without a separate `is_binary` check.
                    let counts = change
                        .diff(resource_cache)
                        .ok()
                        .and_then(|mut p| p.line_counts().ok().flatten());
                    resource_cache.clear_resource_cache_keep_allocation();
                    let Some(c) = counts else {
                        return Ok(Action::Continue);
                    };
                    if c.insertions > 0 || c.removals > 0 {
                        deltas.push(FileDelta {
                            path: bstr_to_pathbuf(location.as_bytes()),
                            added: c.insertions,
                            deleted: c.removals,
                        });
                    }
                }
                Change::Rewrite {
                    location,
                    diff: blob_diff,
                    entry_mode,
                    ..
                } => {
                    if !entry_mode.is_blob() {
                        return Ok(Action::Continue);
                    }
                    if !at_head(location.as_bytes()) {
                        skipped += 1;
                        return Ok(Action::Continue);
                    }
                    // Pure rename carries no content churn. gix sets
                    // `blob_diff = None` iff source_id == id (100%
                    // identical content). A non-None blob_diff can still
                    // be zero — same bytes but similarity computation
                    // produced (0, 0). Both cases: skip the push to keep
                    // `commits_touching` honest about the destination.
                    let (added, deleted) = match blob_diff {
                        Some(d) if d.insertions > 0 || d.removals > 0 => (d.insertions, d.removals),
                        _ => return Ok(Action::Continue),
                    };
                    deltas.push(FileDelta {
                        path: bstr_to_pathbuf(location.as_bytes()),
                        added,
                        deleted,
                    });
                }
            }
            if deltas.len() > pre_len {
                let last = &deltas[pre_len];
                running_files = running_files.saturating_add(1);
                running_lines = running_lines
                    .saturating_add(last.added)
                    .saturating_add(last.deleted);
                if running_files > max_files || running_lines > max_lines {
                    bulk_filtered = true;
                    return Ok(Action::Cancel);
                }
            }
            Ok(Action::Continue)
        },
    );
    // `Action::Cancel` from our closure surfaces from gix as
    // `Err(Error::Diff(tree::Error::Cancelled))`. Treat our own
    // deliberate cancel as success; let any other error through.
    if let Err(e) = result {
        if !bulk_filtered {
            return Err(e).context("tree diff traversal failed");
        }
    }

    Ok((
        deltas,
        DiffStats {
            skipped,
            bulk_filtered,
        },
    ))
}
