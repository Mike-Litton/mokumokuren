//! Per-commit tree diff with rename detection.

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

/// Diff `commit_info` against its first parent.
///
/// If the commit has no parent, diff against the empty tree. Binary files
/// are excluded; renames are tracked at `rename_similarity` (0.0–1.0) and
/// reported as a single `FileDelta` on the new path. The caller-supplied
/// `resource_cache` is reused across calls and cleared between commits.
pub fn diff_commit(
    repo: &gix::Repository,
    resource_cache: &mut gix::diff::blob::Platform,
    commit_info: &CommitInfo,
    rename_similarity: f32,
) -> Result<Vec<FileDelta>> {
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

    let mut deltas: Vec<FileDelta> = Vec::new();
    let mut platform = old_tree
        .changes()
        .context("failed to build tree-diff platform")?;
    platform.options(|opts| {
        opts.track_path();
        opts.track_rewrites(Some(rewrites));
    });

    platform
        .for_each_to_obtain_tree(
            &new_tree,
            |change: Change<'_, '_, '_>| -> Result<Action, anyhow::Error> {
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
                        let path = bstr_to_pathbuf(location.as_bytes());
                        let blob = id.object().context("blob load")?;
                        if crate::binary::is_binary(&blob.data) {
                            return Ok(Action::Continue);
                        }
                        let added = count_lines(&blob.data);
                        if added > 0 {
                            deltas.push(FileDelta {
                                path,
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
                        let path = bstr_to_pathbuf(location.as_bytes());
                        let blob = id.object().context("blob load")?;
                        if crate::binary::is_binary(&blob.data) {
                            return Ok(Action::Continue);
                        }
                        let deleted = count_lines(&blob.data);
                        if deleted > 0 {
                            deltas.push(FileDelta {
                                path,
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
                        let path = bstr_to_pathbuf(location.as_bytes());
                        let counts = change
                            .diff(resource_cache)
                            .ok()
                            .and_then(|mut p| p.line_counts().ok().flatten());
                        resource_cache.clear_resource_cache_keep_allocation();
                        if let Some(c) = counts {
                            if c.insertions > 0 || c.removals > 0 {
                                deltas.push(FileDelta {
                                    path,
                                    added: c.insertions,
                                    deleted: c.removals,
                                });
                            }
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
                        let path = bstr_to_pathbuf(location.as_bytes());
                        let (added, deleted) =
                            blob_diff.map_or((0, 0), |d| (d.insertions, d.removals));
                        deltas.push(FileDelta {
                            path,
                            added,
                            deleted,
                        });
                    }
                }
                Ok(Action::Continue)
            },
        )
        .context("tree diff traversal failed")?;

    Ok(deltas)
}
