//! Git-layer for Mokumokuren: discovery, revwalk, per-commit diff, HEAD LOC.

use ahash::AHashMap;
use anyhow::{Context, Result};
use globset::{Glob, GlobSet, GlobSetBuilder};
use mmk_config::Config;
use mmk_core::types::Commit;
use rayon::prelude::*;
use std::path::{Path, PathBuf};

pub mod binary;
pub mod diff;
pub mod loc;
pub mod walker;

pub use walker::{BaseResolution, BaseResolvedVia};

/// Discover the work-tree root of the Git repo containing `start`, or
/// `None` if `start` is not inside a Git repo. Re-exports gix's
/// discovery without requiring the caller to depend on gix directly.
#[must_use]
pub fn discover_work_dir(start: &Path) -> Option<PathBuf> {
    let repo = gix::discover(start).ok()?;
    repo.workdir().map(Path::to_path_buf)
}

#[derive(Debug, Clone, Default)]
pub struct AnalysisCounts {
    /// Commits returned by the revwalk (before bulk filter).
    pub commits_seen: u64,
    /// Commits kept after bulk filter — these feed the metrics.
    pub commits_analyzed: u64,
    /// Commits dropped by the bulk filter (>max_files OR >max_lines).
    pub commits_filtered_bulk: u64,
    /// Change-events on paths that don't exist at HEAD, summed across
    /// analyzed commits. Counts events, not distinct paths — computing
    /// distinct paths would require collecting them downstream of the
    /// inflate-skip fast path that excludes them in the first place.
    /// Includes Additions of later-renamed/deleted files, Deletions of
    /// unreachable files, and Modifications where the path was later
    /// removed. Also includes paths filtered by ignore globs.
    pub non_head_events: u64,
    /// HEAD-tree files dropped by ignore globs. Distinct count, not
    /// events. Surfacing this separately from `non_head_events` lets a
    /// user see at a glance whether their ignore list is actually doing
    /// something on this repo.
    pub head_paths_ignored: u64,
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
    // Phase timing: set MMK_TRACE=1 to print per-phase wall times to
    // stderr. Useful for perf investigation; off by default.
    let trace = std::env::var_os("MMK_TRACE").is_some();
    let phase = |name: &str, t: std::time::Instant| {
        if trace {
            eprintln!(
                "[mmk] {name}: {:>6.1} ms",
                t.elapsed().as_secs_f64() * 1000.0
            );
        }
    };

    let t = std::time::Instant::now();
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
    phase("open+head", t);

    let t = std::time::Instant::now();
    let commit_infos = walker.walk_commits_since(since_ts)?;
    phase("revwalk", t);

    let mut counts = AnalysisCounts {
        commits_seen: commit_infos.len() as u64,
        ..Default::default()
    };

    let ts_repo = walker.repo.clone().into_sync();
    let rename_similarity = cfg.rename_similarity;

    // Stage 1: enumerate HEAD paths (cheap tree walk — no blob loads).
    // This set gates inflate work inside diff_commit: paths deleted
    // before HEAD or ignored don't have their historical blobs loaded.
    //
    // NOTE: `head_entries` lossy-converts non-UTF-8 path bytes via
    // `to_str_lossy`, then we reconstitute those bytes here via
    // `as_encoded_bytes`. For the rare repo with invalid UTF-8 in
    // paths, the reconstituted bytes won't byte-match gix's raw
    // `location.as_bytes()` inside `diff_commit`, and such paths will
    // be treated as non-HEAD even when they exist at HEAD. We accept
    // this: real Git repositories with non-UTF-8 paths are vanishingly
    // rare and the failure mode is a silent undercount, not incorrect
    // ranking on the typical input.
    let t = std::time::Instant::now();
    let (head_entries, head_paths_ignored) = loc::head_entries(&walker.repo, &ignores)?;
    counts.head_paths_ignored = head_paths_ignored;
    let head_paths: diff::HeadPathBytes = head_entries
        .iter()
        .map(|e| e.path.as_os_str().as_encoded_bytes().to_vec())
        .collect();
    phase("head path enum", t);

    let bulk_limits = (cfg.bulk.max_files, cfg.bulk.max_lines);

    let t = std::time::Instant::now();
    let raw: Vec<(Commit, diff::DiffStats)> = commit_infos
        .par_iter()
        .map_init(
            || {
                let mut repo = ts_repo.to_thread_local();
                repo.object_cache_size_if_unset(64 * 1024 * 1024);
                repo.objects.set_pack_cache(|| {
                    Box::new(gix::odb::pack::cache::lru::MemoryCappedHashmap::new(
                        256 * 1024 * 1024,
                    ))
                });
                let cache =
                    diff::make_resource_cache(&repo).expect("failed to build diff resource cache");
                (repo, cache)
            },
            |(repo, cache), info| -> Result<(Commit, diff::DiffStats)> {
                let (deltas, stats) = diff::diff_commit(
                    repo,
                    cache,
                    info,
                    rename_similarity,
                    Some(&head_paths),
                    bulk_limits,
                )?;
                Ok((
                    Commit {
                        info: info.clone(),
                        deltas,
                    },
                    stats,
                ))
            },
        )
        .collect::<Result<Vec<_>>>()?;
    phase("per-commit diff", t);

    // Only compute LOC for paths that actually churned in the window.
    // Saves inflating tens of thousands of HEAD blobs for untouched files
    // that wouldn't contribute to ranking anyway.
    let t = std::time::Instant::now();
    let touched: ahash::AHashSet<PathBuf> = raw
        .iter()
        .filter(|(_, s)| !s.bulk_filtered)
        .flat_map(|(c, _)| c.deltas.iter().map(|d| d.path.clone()))
        .collect();
    let touched_entries: Vec<_> = head_entries
        .iter()
        .filter(|e| touched.contains(&e.path))
        .cloned()
        .collect();
    let loc = loc::count_loc(&ts_repo, &touched_entries)?;
    phase("loc (touched only)", t);

    let t = std::time::Instant::now();
    let mut commits = Vec::with_capacity(raw.len());
    for (commit, stats) in raw {
        if stats.bulk_filtered {
            // Discard the commit and its skip tally — it didn't
            // contribute to metrics.
            counts.commits_filtered_bulk += 1;
            continue;
        }
        counts.non_head_events += stats.skipped;
        commits.push(commit);
    }
    counts.commits_analyzed = commits.len() as u64;
    phase("bulk filter", t);

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

/// Result of `analyze_session`.
///
/// Contains the full window output plus the subset of commits that
/// are reachable from HEAD but *not* from the resolved base. The
/// subset is what `mmk_core::session::compute_delta` runs the second
/// `rank()` over.
///
/// **LOC epoch contract**: two LOC maps are exposed because they
/// belong to two different epochs.
///
/// - `window.loc` is **HEAD-LOC**: line counts in the working tree
///   right now. Used by the top-level `files[]` ranking.
/// - `session_loc` is **base-LOC with a HEAD-LOC fallback for files
///   the session introduced**. For each file touched in the session:
///   if it existed at the resolved base commit, the value is its LOC
///   *there*; otherwise (the file was introduced mid-session) the
///   value is its current HEAD-LOC. The semantic becomes "size at
///   the start of the session if applicable, else current size" —
///   the only sensible choice, since a file that didn't exist at
///   base has no meaningful base-epoch denominator. The fallback
///   keeps session-introduced files visible in `entered_top_n`.
#[derive(Debug)]
pub struct SessionAnalyzeOutput {
    pub window: AnalyzeOutput,
    pub session_commits: Vec<mmk_core::types::Commit>,
    pub base: Option<BaseResolution>,
    /// LOC at the resolved session base, scoped to files touched in
    /// `session_commits`. See struct-level docs for the epoch
    /// contract. Empty when `base` is `None`.
    pub session_loc: ahash::AHashMap<PathBuf, u32>,
}

/// Run the analyze pipeline plus split commits into a session subset.
///
/// Walks the full window once, then partitions the commit list into
/// "session" commits (reachable from HEAD but not from the resolved
/// base). Returning the split rather than a second rank lets the CLI
/// compose its own `rank()` call without the git layer needing to
/// know about scoring.
pub fn analyze_session(
    path: &Path,
    cfg: &Config,
    base_hint: Option<&str>,
    since_commit_sha: Option<&str>,
) -> Result<SessionAnalyzeOutput> {
    let mut window = analyze(path, cfg)?;

    let walker = walker::RepoWalker::open(path)?;
    let resolution = walker.resolve_base(base_hint, since_commit_sha)?;

    let Some(resolution) = resolution else {
        // No HEAD or no parents — session = entire window. No base
        // to compute LOC against; consumers should treat session_loc
        // as empty (which means session ranking will be empty too,
        // since rank() filters by loc.contains_key()).
        return Ok(SessionAnalyzeOutput {
            session_commits: window.commits.clone(),
            window,
            base: None,
            session_loc: ahash::AHashMap::new(),
        });
    };

    if resolution.via.is_synthetic() {
        window.warnings.push(format!(
            "session base resolved via fallback ({}); harnesses may want to refuse this",
            resolution.via.as_str()
        ));
    }

    // Collect ancestor SHAs of the base (including base itself) so
    // commits reachable from base get filtered out of the session
    // window. This is bounded by the existing `--since` window we
    // already walked; we only need ancestors *within* that window.
    let ancestors: ahash::AHashSet<String> = walk_ancestors(&walker, resolution.oid)?;

    let session_commits: Vec<_> = window
        .commits
        .iter()
        .filter(|c| !ancestors.contains(&c.info.sha))
        .cloned()
        .collect();

    // Compute LOC at the session base, scoped to paths touched in
    // session. Files that didn't exist at base (introduced
    // mid-session) fall back to HEAD-LOC — see the SessionAnalyzeOutput
    // doc comment for the epoch contract.
    let session_paths: ahash::AHashSet<PathBuf> = session_commits
        .iter()
        .flat_map(|c| c.deltas.iter().map(|d| d.path.clone()))
        .collect();
    let ts_repo = walker.repo.into_sync();
    let mut session_loc = loc::count_loc_at(&ts_repo, resolution.oid, &session_paths)
        .context("failed to compute session-base LOC")?;
    for path in &session_paths {
        if !session_loc.contains_key(path) {
            if let Some(&head_loc) = window.loc.get(path) {
                session_loc.insert(path.clone(), head_loc);
            }
        }
    }

    Ok(SessionAnalyzeOutput {
        window,
        session_commits,
        base: Some(resolution),
        session_loc,
    })
}

fn walk_ancestors(
    walker: &walker::RepoWalker,
    start: gix::ObjectId,
) -> Result<ahash::AHashSet<String>> {
    let mut out: ahash::AHashSet<String> = ahash::AHashSet::new();
    let walk = walker
        .repo
        .rev_walk(std::iter::once(start))
        .all()
        .context("failed to start ancestor walk")?;
    for info in walk {
        let info = info.context("ancestor walk error")?;
        out.insert(info.id.to_string());
    }
    Ok(out)
}

fn build_globset(patterns: &[String]) -> Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for pat in patterns {
        let glob = Glob::new(pat).with_context(|| format!("invalid ignore glob: {pat}"))?;
        builder.add(glob);
    }
    builder.build().context("failed to build glob set")
}
