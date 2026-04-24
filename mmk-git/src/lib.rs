//! Git-layer for Mokumokuren: discovery, revwalk, per-commit diff, HEAD LOC.

use ahash::AHashMap;
use anyhow::{Context, Result};
use globset::{Glob, GlobSetBuilder};
use mmk_config::Config;
use mmk_core::types::Commit;
use rayon::prelude::*;
use std::path::{Path, PathBuf};

pub mod binary;
pub mod diff;
pub mod loc;
pub mod walker;

#[derive(Debug, Clone, Default)]
pub struct AnalysisCounts {
    /// Commits returned by the revwalk (before bulk filter).
    pub commits_seen: u64,
    /// Commits kept after bulk filter — these feed the metrics.
    pub commits_analyzed: u64,
    /// Commits dropped by the bulk filter (>max_files OR >max_lines).
    pub commits_filtered_bulk: u64,
    /// Paths deleted from HEAD (excluded from hotspot ranking).
    pub files_deleted_from_head: u64,
}

#[derive(Debug)]
pub struct AnalyzeOutput {
    pub commits: Vec<Commit>,
    pub loc: AHashMap<PathBuf, u32>,
    pub counts: AnalysisCounts,
    pub is_shallow: bool,
    pub head_sha: Option<String>,
    /// Committer time of HEAD, used as the reference point for recency
    /// weighting. `None` if HEAD is unborn.
    pub head_timestamp: Option<i64>,
    pub warnings: Vec<String>,
}

pub fn analyze(path: &Path, cfg: &Config) -> Result<AnalyzeOutput> {
    let walker = walker::RepoWalker::open(path)?;
    let is_shallow = walker.is_shallow();

    let ignores = build_globset(&cfg.ignores)?;

    let head = walker.head_sha_and_time()?;
    let (head_sha, head_ts) = match head {
        Some((sha, ts)) => (Some(sha), Some(ts)),
        None => (None, None),
    };

    let now_ts = head_ts.unwrap_or(0);
    let since_ts = now_ts.saturating_sub(cfg.window_seconds());
    let commit_infos = walker.walk_commits_since(since_ts)?;

    let mut counts = AnalysisCounts {
        commits_seen: commit_infos.len() as u64,
        ..Default::default()
    };

    let ts_repo = walker.repo.clone().into_sync();
    let rename_similarity = cfg.rename_similarity;

    let raw: Vec<(Commit, u32, u32)> = commit_infos
        .par_iter()
        .map_init(
            || {
                let mut repo = ts_repo.to_thread_local();
                // Keep recently-decoded blobs/trees around so rename
                // detection and parent-tree lookups don't re-read from the
                // odb on every commit.
                repo.object_cache_size_if_unset(16 * 1024 * 1024);
                let cache =
                    diff::make_resource_cache(&repo).expect("failed to build diff resource cache");
                (repo, cache)
            },
            |(repo, cache), info| -> Result<(Commit, u32, u32)> {
                let deltas = diff::diff_commit(repo, cache, info, rename_similarity)?;
                let files = u32::try_from(deltas.len()).unwrap_or(u32::MAX);
                let lines: u32 = deltas
                    .iter()
                    .map(|d| d.added.saturating_add(d.deleted))
                    .fold(0u32, u32::saturating_add);
                Ok((
                    Commit {
                        info: info.clone(),
                        deltas,
                    },
                    files,
                    lines,
                ))
            },
        )
        .collect::<Result<Vec<_>>>()?;

    let max_files = cfg.bulk.max_files;
    let max_lines = cfg.bulk.max_lines;
    let mut commits = Vec::with_capacity(raw.len());
    for (commit, files, lines) in raw {
        if files > max_files || lines > max_lines {
            counts.commits_filtered_bulk += 1;
            continue;
        }
        commits.push(commit);
    }
    counts.commits_analyzed = commits.len() as u64;

    let loc = loc::head_loc_map(&walker.repo, &ts_repo, &ignores)?;

    if !ignores.is_empty() {
        commits.iter_mut().for_each(|c| {
            c.deltas.retain(|d| {
                let s = d.path.to_string_lossy();
                !ignores.is_match(s.as_ref())
            });
        });
    }

    let touched_paths: ahash::AHashSet<PathBuf> = commits
        .iter()
        .flat_map(|c| c.deltas.iter().map(|d| d.path.clone()))
        .collect();
    counts.files_deleted_from_head = touched_paths
        .iter()
        .filter(|p| !loc.contains_key(p.as_path()))
        .count() as u64;

    let mut warnings = Vec::new();
    if is_shallow {
        warnings.push(
            "repository is a shallow clone — history before the shallow boundary is not analyzed"
                .into(),
        );
    }

    Ok(AnalyzeOutput {
        commits,
        loc,
        counts,
        is_shallow,
        head_sha,
        head_timestamp: head_ts,
        warnings,
    })
}

fn build_globset(patterns: &[String]) -> Result<globset::GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for pat in patterns {
        let glob = Glob::new(pat).with_context(|| format!("invalid ignore glob: {pat}"))?;
        builder.add(glob);
    }
    builder.build().context("failed to build glob set")
}
