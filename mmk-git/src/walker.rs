//! Repo discovery + revwalk.

use anyhow::{Context, Result};
use gix::bstr::ByteSlice;
use mmk_core::types::CommitInfo;
use std::path::Path;

/// How `RepoWalker::resolve_base` arrived at the returned commit.
/// The CLI surfaces this in the JSON `session.base_resolved_via`
/// field so harnesses can refuse to trust a synthetic base.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BaseResolvedVia {
    Explicit,
    SinceCommit,
    MergeBaseOriginMain,
    MergeBaseMain,
    MergeBaseOriginMaster,
    MergeBaseMaster,
    HeadMinusOne,
}

impl BaseResolvedVia {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Explicit => "explicit",
            Self::SinceCommit => "since_commit",
            Self::MergeBaseOriginMain => "merge_base_origin_main",
            Self::MergeBaseMain => "merge_base_main",
            Self::MergeBaseOriginMaster => "merge_base_origin_master",
            Self::MergeBaseMaster => "merge_base_master",
            Self::HeadMinusOne => "head_minus_one",
        }
    }

    /// Whether the resolution method is "synthetic" (the user didn't
    /// pick it; we fell back). Synthetic resolutions warn through
    /// `analysis.warnings`.
    #[must_use]
    pub const fn is_synthetic(self) -> bool {
        matches!(self, Self::HeadMinusOne)
    }
}

#[derive(Debug, Clone)]
pub struct BaseResolution {
    pub oid: gix::ObjectId,
    pub via: BaseResolvedVia,
    pub label: Option<String>,
}

#[allow(missing_debug_implementations)]
pub struct RepoWalker {
    pub(crate) repo: gix::Repository,
}

impl RepoWalker {
    pub fn open(path: &Path) -> Result<Self> {
        let repo = gix::discover(path)
            .with_context(|| format!("not a git repository: {}", path.display()))?;
        Ok(Self { repo })
    }

    pub fn is_shallow(&self) -> bool {
        self.repo.is_shallow()
    }

    /// Return the HEAD commit's sha and committer timestamp (seconds since
    /// the Unix epoch), or `None` if the repo has no HEAD yet (unborn /
    /// empty).
    pub fn head_sha_and_time(&self) -> Result<Option<(String, i64)>> {
        let Ok(commit) = self.repo.head_commit() else {
            return Ok(None);
        };
        let time = commit.time().context("failed to decode HEAD commit time")?;
        Ok(Some((commit.id.to_string(), time.seconds)))
    }

    /// Resolve the base commit for `mmk session` using the cascade
    /// from the v0.2.0 plan:
    ///
    /// 1. `since_commit_sha` — exact match wins.
    /// 2. `base_hint` (a ref name like `main`, `origin/main`).
    /// 3. `merge_base(HEAD, origin/main)`.
    /// 4. `merge_base(HEAD, main)`.
    /// 5. `merge_base(HEAD, origin/master)`.
    /// 6. `merge_base(HEAD, master)`.
    /// 7. `HEAD~1` (synthetic — caller should warn).
    ///
    /// Returns `Ok(None)` if HEAD itself is unborn.
    pub fn resolve_base(
        &self,
        base_hint: Option<&str>,
        since_commit_sha: Option<&str>,
    ) -> Result<Option<BaseResolution>> {
        let Ok(head_ref) = self.repo.head_id() else {
            return Ok(None);
        };
        let head_oid = head_ref.detach();

        if let Some(sha) = since_commit_sha {
            let id = self
                .repo
                .rev_parse_single(sha.as_bytes().as_bstr())
                .with_context(|| format!("failed to parse --since-commit '{sha}'"))?;
            return Ok(Some(BaseResolution {
                oid: id.detach(),
                via: BaseResolvedVia::SinceCommit,
                label: Some(sha.to_string()),
            }));
        }

        if let Some(hint) = base_hint {
            // Try as a ref/spec; if it resolves directly, use the
            // merge-base of HEAD and that ref. This matches the
            // semantic of "since I branched off `main`", not "since
            // `main`'s tip".
            if let Ok(spec_id) = self.repo.rev_parse_single(hint.as_bytes().as_bstr()) {
                if let Ok(mb) = self.repo.merge_base(head_oid, spec_id.detach()) {
                    return Ok(Some(BaseResolution {
                        oid: mb.detach(),
                        via: BaseResolvedVia::Explicit,
                        label: Some(hint.to_string()),
                    }));
                }
            }
        }

        for (refname, via) in [
            ("origin/main", BaseResolvedVia::MergeBaseOriginMain),
            ("main", BaseResolvedVia::MergeBaseMain),
            ("origin/master", BaseResolvedVia::MergeBaseOriginMaster),
            ("master", BaseResolvedVia::MergeBaseMaster),
        ] {
            if let Ok(rid) = self.repo.rev_parse_single(refname.as_bytes().as_bstr()) {
                if let Ok(mb) = self.repo.merge_base(head_oid, rid.detach()) {
                    if mb.detach() != head_oid {
                        return Ok(Some(BaseResolution {
                            oid: mb.detach(),
                            via,
                            label: Some(refname.to_string()),
                        }));
                    }
                }
            }
        }

        // HEAD~1 fallback. If HEAD has no parent (root commit only),
        // there is no base — return None and let the caller treat the
        // session as the entire window.
        let head_commit = self
            .repo
            .find_commit(head_oid)
            .context("failed to load HEAD commit")?;
        if let Some(parent) = head_commit.parent_ids().next() {
            return Ok(Some(BaseResolution {
                oid: parent.detach(),
                via: BaseResolvedVia::HeadMinusOne,
                label: None,
            }));
        }
        Ok(None)
    }

    /// Walk commits reachable from HEAD with committer time `>= since_ts`.
    /// Returns an empty vec if HEAD doesn't exist.
    pub fn walk_commits_since(&self, since_ts: i64) -> Result<Vec<CommitInfo>> {
        let Ok(head_id) = self.repo.head_id() else {
            return Ok(Vec::new());
        };
        self.walk_commits_from(head_id.detach(), since_ts)
    }

    /// Walk commits reachable from `start` with committer time
    /// `>= since_ts`. Generalized form of [`walk_commits_since`] used
    /// by `analyze_at` to anchor on an arbitrary commit (drift
    /// snapshots).
    pub fn walk_commits_from(
        &self,
        start: gix::ObjectId,
        since_ts: i64,
    ) -> Result<Vec<CommitInfo>> {
        let walk = self
            .repo
            .rev_walk(std::iter::once(start))
            .sorting(gix::revision::walk::Sorting::ByCommitTimeCutoff {
                order: gix::traverse::commit::simple::CommitTimeOrder::NewestFirst,
                seconds: since_ts,
            })
            .use_commit_graph(true)
            .all()
            .context("failed to start revision walk")?;

        let mut out = Vec::new();
        for info in walk {
            let info =
                info.with_context(|| format!("revision walk error starting from {start}"))?;
            let commit = info.object().context("failed to load commit object")?;
            let ts = commit
                .time()
                .context("failed to decode commit time")?
                .seconds;
            let parent_sha = commit.parent_ids().next().map(|id| id.detach().to_string());
            let author_email = commit
                .author()
                .ok()
                .map(|a| a.email.to_string())
                .unwrap_or_default();
            out.push(CommitInfo {
                sha: info.id.to_string(),
                parent_sha,
                timestamp: ts,
                author_email,
            });
        }
        Ok(out)
    }

    /// Find K session boundaries on the current HEAD's history.
    ///
    /// Default heuristic: walk back from HEAD, take the most recent
    /// K merge commits (any commit with > 1 parent — corresponds to
    /// a PR-style merge into the base ref). Fallback: if fewer than
    /// K merges exist, split the linear walk into K equal chunks
    /// and return the boundary commit at each chunk start.
    ///
    /// Returns oldest-first so callers can iterate `analyze_at` in
    /// chronological order to feed `mmk_core::drift::compute_drift`.
    pub fn find_session_boundaries(&self, k: usize) -> Result<Vec<gix::ObjectId>> {
        if k == 0 {
            return Ok(Vec::new());
        }
        let Ok(head_id) = self.repo.head_id() else {
            return Ok(Vec::new());
        };

        let walk = self
            .repo
            .rev_walk(std::iter::once(head_id.detach()))
            .use_commit_graph(true)
            .all()
            .context("failed to start boundary walk")?;

        let mut all: Vec<gix::ObjectId> = Vec::new();
        let mut merges: Vec<gix::ObjectId> = Vec::new();
        for info in walk {
            let info = info.context("boundary walk error")?;
            let oid = info.id;
            all.push(oid);
            if info.parent_ids.len() > 1 {
                merges.push(oid);
                if merges.len() >= k {
                    // K merges found — the linear-chunk fallback below
                    // won't fire, so `all` won't be read past this
                    // point. On long-history repos (~150k commits)
                    // this drops the boundary walk from full history
                    // to whatever distance the K-th most recent
                    // merge sits at.
                    break;
                }
            }
        }

        let chosen: Vec<gix::ObjectId> = if merges.len() >= k {
            // Newest-first iteration produced merges in newest-first
            // order; take the K most recent then reverse to oldest-first.
            merges.into_iter().take(k).rev().collect()
        } else if all.is_empty() {
            Vec::new()
        } else {
            // Linear-chunk fallback: split the walk into K equal
            // segments, return the boundary commit at each segment
            // start. `all` is newest-first; segment[0] = HEAD,
            // segment[K-1] = oldest. Reverse to oldest-first for the
            // caller.
            let n = all.len();
            let mut picks: Vec<gix::ObjectId> = (0..k)
                .map(|i| {
                    let idx = (i * (n.saturating_sub(1))) / k.max(1);
                    all[idx]
                })
                .collect();
            picks.reverse();
            picks.dedup();
            picks
        };

        Ok(chosen)
    }
}
