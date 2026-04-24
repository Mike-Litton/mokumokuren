//! Repo discovery + revwalk.

use anyhow::{Context, Result};
use mmk_core::types::CommitInfo;
use std::path::Path;

#[allow(missing_debug_implementations)]
pub struct RepoWalker {
    pub repo: gix::Repository,
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

    /// Walk commits reachable from HEAD with committer time `>= since_ts`.
    /// Returns an empty vec if HEAD doesn't exist.
    pub fn walk_commits_since(&self, since_ts: i64) -> Result<Vec<CommitInfo>> {
        let Ok(head_id) = self.repo.head_id() else {
            return Ok(Vec::new());
        };

        let walk = self
            .repo
            .rev_walk(std::iter::once(head_id.detach()))
            .sorting(gix::revision::walk::Sorting::ByCommitTimeCutoff {
                order: gix::traverse::commit::simple::CommitTimeOrder::NewestFirst,
                seconds: since_ts,
            })
            .use_commit_graph(true)
            .all()
            .context("failed to start revision walk")?;

        let mut out = Vec::new();
        for info in walk {
            let info = info.context("revision walk error")?;
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
}
