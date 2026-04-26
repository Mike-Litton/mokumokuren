//! Historical-pattern drift across recent sessions.
//!
//! Pure function: given K snapshots of a repo's hotspot ranking
//! (oldest first, K-1 adjacent transitions), surface every path that
//! climbs in a *majority* of those transitions. The git layer
//! produces the snapshots by re-running `analyze` at each base oid
//! `find_session_boundaries` returns; this module knows nothing
//! about git.
//!
//! The signal is a loose analog of SCAFFOLD-CEGIS's
//! "safety-monotonicity" (their version is static-analysis-based
//! between iterations; ours is rank-based across recent sessions).
//! Same spirit — "catch a degenerate trend before it lands" — but
//! different mechanism. The DRIFT finding wording stays honest about
//! that.

use ahash::AHashMap;
use std::path::PathBuf;

use crate::HotspotEntry;

/// One ranking snapshot. `label` is for diagnostics (a short SHA or
/// session tag); `compute_drift` does not interpret it.
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub label: String,
    pub ranking: Vec<HotspotEntry>,
}

/// One emitted drift signal. `climb_transitions` of `total_transitions`
/// adjacent transitions showed `path` either improving rank or newly
/// entering the ranking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriftFinding {
    pub path: PathBuf,
    pub climb_transitions: u32,
    pub total_transitions: u32,
    pub latest_rank: u32,
}

/// Walk adjacent snapshot pairs, count per-path climbs, emit a
/// finding for every path that climbed in a majority of transitions.
/// Snapshots are oldest-first; "latest" is the last element.
#[must_use]
pub fn compute_drift(snapshots: &[Snapshot]) -> Vec<DriftFinding> {
    if snapshots.len() < 2 {
        return Vec::new();
    }

    let total_transitions = u32::try_from(snapshots.len() - 1).unwrap_or(u32::MAX);
    let mut climbs: AHashMap<PathBuf, u32> = AHashMap::new();

    for window in snapshots.windows(2) {
        let prev = rank_map(&window[0]);
        let next_ranking = &window[1].ranking;
        for entry in next_ranking {
            // Newly-entered (no prev rank) is treated as a climb by
            // convention — first appearance is direction-of-interest.
            let climbed = prev
                .get(&entry.path)
                .map_or(true, |&prev_r| entry.hotspot_rank < prev_r);
            if climbed {
                *climbs.entry(entry.path.clone()).or_insert(0) += 1;
            }
        }
    }

    let latest = rank_map(snapshots.last().expect("len >= 2 checked"));
    let majority_floor = total_transitions / 2; // strict majority is `> floor`

    let mut findings: Vec<DriftFinding> = climbs
        .into_iter()
        .filter(|(_, c)| *c > majority_floor)
        .filter_map(|(path, climb_transitions)| {
            let latest_rank = *latest.get(&path)?;
            Some(DriftFinding {
                path,
                climb_transitions,
                total_transitions,
                latest_rank,
            })
        })
        .collect();
    findings.sort_by(|a, b| {
        a.latest_rank
            .cmp(&b.latest_rank)
            .then_with(|| a.path.cmp(&b.path))
    });
    findings
}

fn rank_map(snap: &Snapshot) -> AHashMap<PathBuf, u32> {
    snap.ranking
        .iter()
        .map(|e| (e.path.clone(), e.hotspot_rank))
        .collect()
}
