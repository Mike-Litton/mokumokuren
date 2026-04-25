//! Session vs. window deltas.
//!
//! A *window* ranking is the full `--since`-bounded view; a
//! *session* ranking is the subset of commits since the resolved
//! base ref. The agent uses `SessionDelta` to ask "what shifted
//! since I started?"
//!
//! Per the v0.2.0 plan, this is the monotonicity proxy from the
//! SCAFFOLD-CEGIS finding that LLM-driven iteration silently
//! *degrades* code quality across rounds (correlation with raw churn
//! is only 0.09 — the signal lives in the rate of change of churn,
//! not its volume).

use ahash::AHashMap;
use serde::Serialize;
use std::path::PathBuf;

use crate::hotspot::HotspotEntry;
use crate::types::Commit;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RankClimb {
    pub path: PathBuf,
    /// Window-rank minus session-rank, when both are known. Positive
    /// means the file is *higher* in the session ranking than the
    /// window — i.e. the session is amplifying it.
    pub delta: i32,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ChurnOfChurn {
    pub path: PathBuf,
    /// `min(added, deleted) × 2 / (added + deleted)` over the session
    /// commits. 0 = pure adds or pure deletes; 1 = perfectly balanced
    /// add/delete (a thrash signature). Files with zero churn are
    /// excluded.
    pub ratio: f64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Default)]
pub struct SessionDelta {
    pub entered_top_n: Vec<PathBuf>,
    pub rank_climbs: Vec<RankClimb>,
    pub churn_of_churn: Vec<ChurnOfChurn>,
    /// Shannon entropy of files-touched-per-commit-in-session,
    /// normalized by `log(commits)`. 0 if the session has < 2
    /// commits. 1 = perfectly uniform across commits.
    pub commit_entropy: f64,
}

#[must_use]
pub fn compute_delta(
    window: &[HotspotEntry],
    session_entries: &[HotspotEntry],
    session_commits: &[Commit],
) -> SessionDelta {
    let window_ranks: AHashMap<&PathBuf, u32> =
        window.iter().map(|e| (&e.path, e.hotspot_rank)).collect();

    let mut entered_top_n: Vec<PathBuf> = Vec::new();
    let mut rank_climbs: Vec<RankClimb> = Vec::new();
    for s in session_entries {
        match window_ranks.get(&s.path).copied() {
            None => entered_top_n.push(s.path.clone()),
            Some(window_rank) => {
                let delta = i32::try_from(window_rank).unwrap_or(i32::MAX)
                    - i32::try_from(s.hotspot_rank).unwrap_or(i32::MAX);
                if delta > 0 {
                    rank_climbs.push(RankClimb {
                        path: s.path.clone(),
                        delta,
                    });
                }
            }
        }
    }
    entered_top_n.sort();
    rank_climbs.sort_by(|a, b| b.delta.cmp(&a.delta).then_with(|| a.path.cmp(&b.path)));

    SessionDelta {
        entered_top_n,
        rank_climbs,
        churn_of_churn: churn_of_churn(session_commits),
        commit_entropy: commit_entropy(session_commits),
    }
}

fn churn_of_churn(commits: &[Commit]) -> Vec<ChurnOfChurn> {
    let mut totals: AHashMap<PathBuf, (u64, u64)> = AHashMap::new();
    for c in commits {
        for d in &c.deltas {
            let entry = totals.entry(d.path.clone()).or_insert((0, 0));
            entry.0 += u64::from(d.added);
            entry.1 += u64::from(d.deleted);
        }
    }
    let mut out: Vec<ChurnOfChurn> = totals
        .into_iter()
        .filter_map(|(path, (a, d))| {
            let total = a + d;
            if total == 0 {
                return None;
            }
            let min = a.min(d);
            #[allow(clippy::cast_precision_loss)]
            let ratio = (min as f64) * 2.0 / (total as f64);
            Some(ChurnOfChurn { path, ratio })
        })
        .collect();
    out.sort_by(|a, b| {
        b.ratio
            .partial_cmp(&a.ratio)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.path.cmp(&b.path))
    });
    out
}

fn commit_entropy(commits: &[Commit]) -> f64 {
    if commits.len() < 2 {
        return 0.0;
    }
    let n = commits.len();
    let mut h = 0.0;
    let total: usize = commits.iter().map(|c| c.deltas.len().max(1)).sum();
    if total == 0 {
        return 0.0;
    }
    #[allow(clippy::cast_precision_loss)]
    let total_f = total as f64;
    for c in commits {
        let count = c.deltas.len().max(1);
        #[allow(clippy::cast_precision_loss)]
        let p = (count as f64) / total_f;
        if p > 0.0 {
            h -= p * p.ln();
        }
    }
    #[allow(clippy::cast_precision_loss)]
    let norm = (n as f64).ln();
    if norm > 0.0 {
        h / norm
    } else {
        0.0
    }
}
