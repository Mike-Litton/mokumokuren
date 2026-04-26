//! Git-layer for Mokumokuren: discovery, revwalk, per-commit diff, HEAD LOC.

use ahash::AHashMap;
use anyhow::{Context, Result};
use globset::{Glob, GlobSet, GlobSetBuilder};
use mmk_config::Config;
use mmk_core::types::Commit;
use rayon::prelude::*;
use std::path::{Path, PathBuf};

pub mod binary;
pub mod cache;
pub mod diff;
pub mod loc;
pub mod untracked;
pub mod walker;

pub use untracked::{list_untracked, UntrackedFile};
pub use walker::{BaseResolution, BaseResolvedVia, RepoWalker};

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
    analyze_inner(path, cfg, None)
}

/// Run the analyze pipeline anchored at `anchor_oid` instead of HEAD.
///
/// Used by `mmk drift` to take K snapshots at K historical commits
/// without needing a checkout per snapshot. The recency epoch (the
/// `now` for `weighted_churn` decay) becomes the anchor commit's
/// time, not wall-clock now — so each snapshot's churn is weighted
/// against its own historical "present."
pub fn analyze_at(path: &Path, cfg: &Config, anchor_oid: gix::ObjectId) -> Result<AnalyzeOutput> {
    analyze_inner(path, cfg, Some(anchor_oid))
}

fn analyze_inner(
    path: &Path,
    cfg: &Config,
    anchor_oid: Option<gix::ObjectId>,
) -> Result<AnalyzeOutput> {
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

    let (head_sha, head_ts, start_oid) = if let Some(oid) = anchor_oid {
        let commit = walker
            .repo
            .find_commit(oid)
            .with_context(|| format!("anchor commit {oid} not found"))?;
        let ts = commit
            .time()
            .context("failed to decode anchor commit time")?
            .seconds;
        (Some(oid.to_string()), Some(ts), Some(oid))
    } else {
        let head = walker.head_sha_and_time()?;
        let (sha, ts) = head.map_or((None, None), |(sha, ts)| (Some(sha), Some(ts)));
        (sha, ts, None)
    };

    let now_ts = head_ts.unwrap_or(0);
    let since_ts = now_ts.saturating_sub(cfg.window_seconds());
    phase("open+head", t);

    // Revwalk cache: the set of commits reachable from a fixed anchor
    // with committer-time >= since_ts is determined by the immutable
    // commit graph. Same key = same output, modulo new commits being
    // fetched after caching. New commits change HEAD's sha, which
    // makes a new key — the cached entry stays correct, just unused.
    let revwalk_cache_path = cache::revwalk_cache_path(walker.repo.git_dir())?;
    let mut revwalk_cache = cache::RevwalkCache::load(&revwalk_cache_path).unwrap_or_else(|err| {
        if trace {
            eprintln!("[mmk] revwalk cache load failed: {err:#}; starting empty");
        }
        cache::RevwalkCache::empty()
    });
    let revwalk_key = head_sha.as_ref().map(|sha| cache::RevwalkKey {
        anchor_sha: sha.clone(),
        since_ts,
    });
    let mut revwalk_dirty = false;

    let t = std::time::Instant::now();
    let commit_infos: Vec<mmk_core::types::CommitInfo> = if let Some(commits) = revwalk_key
        .as_ref()
        .and_then(|k| revwalk_cache.entries.get(k).map(|e| e.commits.clone()))
    {
        commits
    } else {
        let computed = if let Some(oid) = start_oid {
            walker.walk_commits_from(oid, since_ts)?
        } else {
            walker.walk_commits_since(since_ts)?
        };
        if let Some(key) = revwalk_key {
            revwalk_cache.insert(key, computed.clone());
            revwalk_dirty = true;
        }
        computed
    };
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
    //
    // Head-tree cache: keyed by `(commit_sha, ignores_hash)`. A tree's
    // blob list is immutable once the tree exists; the ignore-glob
    // hash partitions distinct ignore configs into separate entries.
    let head_tree_cache_path = cache::head_tree_cache_path(walker.repo.git_dir())?;
    let mut head_tree_cache =
        cache::HeadTreeCache::load(&head_tree_cache_path).unwrap_or_else(|err| {
            if trace {
                eprintln!("[mmk] head-tree cache load failed: {err:#}; starting empty");
            }
            cache::HeadTreeCache::empty()
        });
    let ignores_hash = cache::ignores_hash(&cfg.ignores);
    let head_tree_key = head_sha.as_ref().map(|sha| cache::HeadTreeKey {
        commit_sha: sha.clone(),
        ignores_hash,
    });
    let mut head_tree_dirty = false;

    let t = std::time::Instant::now();
    let (head_entries, head_paths_ignored) = if let Some(cached) = head_tree_key
        .as_ref()
        .and_then(|k| head_tree_cache.entries.get(k))
    {
        let entries = cached
            .entries
            .iter()
            .map(|e| -> Result<loc::HeadEntry> {
                let oid = gix::ObjectId::from_hex(e.oid_hex.as_bytes())
                    .with_context(|| format!("invalid cached oid hex: {}", e.oid_hex))?;
                Ok(loc::HeadEntry {
                    path: e.path.clone(),
                    oid,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        (entries, cached.head_paths_ignored)
    } else {
        let (entries, ignored_count) = match start_oid {
            Some(oid) => loc::tree_entries(&walker.repo, oid, &ignores)?,
            None => loc::head_entries(&walker.repo, &ignores)?,
        };
        if let Some(key) = head_tree_key {
            let cached_entries = entries
                .iter()
                .map(|e| cache::CachedTreeEntry {
                    path: e.path.clone(),
                    oid_hex: e.oid.to_string(),
                })
                .collect();
            head_tree_cache.insert(key, cached_entries, ignored_count);
            head_tree_dirty = true;
        }
        (entries, ignored_count)
    };
    counts.head_paths_ignored = head_paths_ignored;
    let head_paths: diff::HeadPathBytes = head_entries
        .iter()
        .map(|e| e.path.as_os_str().as_encoded_bytes().to_vec())
        .collect();
    phase("head path enum", t);

    let bulk_limits = (cfg.bulk.max_files, cfg.bulk.max_lines);

    // Persistent cache: per-commit deltas keyed by SHA. Survives across
    // invocations; populated as gix-LCS results land. The hot path for
    // calls 2-N is "every commit hits the cache, gix is never invoked".
    let cache_path = cache::cache_path(walker.repo.git_dir())?;
    let t = std::time::Instant::now();
    let mut commit_cache = cache::Cache::load(&cache_path).unwrap_or_else(|err| {
        if trace {
            eprintln!("[mmk] cache load failed: {err:#}; starting empty");
        }
        cache::Cache::empty()
    });
    phase("cache load", t);

    // Partition commits: cached (delta lookup is free) vs missing
    // (must run gix-LCS). The missing set is what the parallel diff
    // phase actually has to do work for.
    let t = std::time::Instant::now();
    let missing: Vec<&mmk_core::types::CommitInfo> = commit_infos
        .iter()
        .filter(|info| !commit_cache.entries.contains_key(&info.sha))
        .collect();
    if trace {
        eprintln!(
            "[mmk] cache: {} cached / {} missing of {} commits",
            commit_infos.len() - missing.len(),
            missing.len(),
            commit_infos.len(),
        );
    }
    phase("cache partition", t);

    let t = std::time::Instant::now();
    let computed: Vec<(Commit, diff::DiffStats)> = missing
        .par_iter()
        .map_init(
            || -> Result<(gix::Repository, gix::diff::blob::Platform)> {
                let mut repo = ts_repo.to_thread_local();
                repo.object_cache_size_if_unset(64 * 1024 * 1024);
                repo.objects.set_pack_cache(|| {
                    Box::new(gix::odb::pack::cache::lru::MemoryCappedHashmap::new(
                        256 * 1024 * 1024,
                    ))
                });
                let cache = diff::make_resource_cache(&repo)?;
                Ok((repo, cache))
            },
            |init, info| -> Result<(Commit, diff::DiffStats)> {
                let (repo, cache) = init
                    .as_mut()
                    .map_err(|e| anyhow::anyhow!("worker init failed: {e:#}"))?;
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
                        info: (*info).clone(),
                        deltas,
                    },
                    stats,
                ))
            },
        )
        .collect::<Result<Vec<_>>>()?;
    phase("per-commit diff (missing only)", t);

    // Insert freshly-computed entries into the cache.
    for (commit, stats) in &computed {
        commit_cache.entries.insert(
            commit.info.sha.clone(),
            cache::CommitDeltas {
                deltas: commit.deltas.clone(),
                skipped: stats.skipped,
                bulk_filtered: stats.bulk_filtered,
            },
        );
    }

    // Persist if we added anything new. Atomic via tmp-rename.
    if !computed.is_empty() {
        let t = std::time::Instant::now();
        if let Err(err) = commit_cache.save(&cache_path) {
            if trace {
                eprintln!("[mmk] cache save failed: {err:#}; continuing");
            }
        }
        phase("cache save", t);
    }
    if revwalk_dirty {
        let t = std::time::Instant::now();
        if let Err(err) = revwalk_cache.save(&revwalk_cache_path) {
            if trace {
                eprintln!("[mmk] revwalk cache save failed: {err:#}; continuing");
            }
        }
        phase("revwalk cache save", t);
    }
    if head_tree_dirty {
        let t = std::time::Instant::now();
        if let Err(err) = head_tree_cache.save(&head_tree_cache_path) {
            if trace {
                eprintln!("[mmk] head-tree cache save failed: {err:#}; continuing");
            }
        }
        phase("head-tree cache save", t);
    }

    // Materialize the full result: cached entries → freshly-built
    // (Commit, DiffStats), interleaved with the just-computed ones in
    // commit_infos order.
    let t = std::time::Instant::now();
    let mut computed_by_sha: AHashMap<String, (Commit, diff::DiffStats)> = computed
        .into_iter()
        .map(|(c, s)| (c.info.sha.clone(), (c, s)))
        .collect();
    let raw: Vec<(Commit, diff::DiffStats)> = commit_infos
        .iter()
        .map(|info| {
            if let Some(v) = computed_by_sha.remove(&info.sha) {
                v
            } else {
                let cd = commit_cache
                    .entries
                    .get(&info.sha)
                    .expect("cache hit must be present");
                (
                    Commit {
                        info: info.clone(),
                        deltas: cd.deltas.clone(),
                    },
                    diff::DiffStats {
                        skipped: cd.skipped,
                        bulk_filtered: cd.bulk_filtered,
                    },
                )
            }
        })
        .collect();
    phase("cache materialize", t);

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
    // window. Bounded by the same `since_ts` cutoff as the window
    // walk: ancestors older than the cutoff cannot match a window
    // commit's sha, so walking them is wasted work — and on
    // long-history repos that's the dominant cost of `session-summary`.
    let head_ts = window.head_timestamp.unwrap_or(0);
    let since_ts = head_ts.saturating_sub(cfg.window_seconds());
    let ancestors: ahash::AHashSet<String> = walk_ancestors(&walker, resolution.oid, since_ts)?;

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
    since_ts: i64,
) -> Result<ahash::AHashSet<String>> {
    let mut out: ahash::AHashSet<String> = ahash::AHashSet::new();
    let walk = walker
        .repo
        .rev_walk(std::iter::once(start))
        .sorting(gix::revision::walk::Sorting::ByCommitTimeCutoff {
            order: gix::traverse::commit::simple::CommitTimeOrder::NewestFirst,
            seconds: since_ts,
        })
        .use_commit_graph(true)
        .all()
        .context("failed to start ancestor walk")?;
    for info in walk {
        let info = info.context("ancestor walk error")?;
        out.insert(info.id.to_string());
    }
    Ok(out)
}

pub fn build_globset(patterns: &[String]) -> Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for pat in patterns {
        let glob = Glob::new(pat).with_context(|| format!("invalid ignore glob: {pat}"))?;
        builder.add(glob);
    }
    builder.build().context("failed to build glob set")
}
