//! Property tests for `compute_drift`. Locks the universal
//! invariants on the K-snapshots transition walk: climb counts can't
//! exceed the transition count, total_transitions equals K-1, and
//! degenerate inputs produce empty output.

use mmk_core::drift::{compute_drift, Snapshot};
use mmk_core::HotspotEntry;
use proptest::collection::vec;
use proptest::prelude::*;
use std::path::PathBuf;

/// Fixed pool of distinct paths. Production rankings come from a
/// `HashMap<PathBuf, _>` and are always path-unique within one
/// snapshot; the strategy mirrors that invariant by drawing a
/// permutation of this pool rather than an arbitrary `Vec` that
/// could repeat paths.
const PATH_POOL: [&str; 8] = [
    "a.rs", "b.rs", "c.rs", "d.rs", "e.rs", "f.rs", "g.rs", "h.rs",
];

fn entry(path: &str, rank: u32) -> HotspotEntry {
    HotspotEntry {
        path: PathBuf::from(path),
        loc: 100,
        weighted_churn: 1.0,
        relative_churn: 0.01,
        hotspot_score: f64::from(1_000 - rank),
        hotspot_rank: rank,
        commits_touching: 1,
        last_modified: 0,
        top_couples: Vec::new(),
    }
}

fn snapshot_strategy() -> impl Strategy<Value = Snapshot> {
    // Pick a subset size 1..=PATH_POOL.len() and permute that many
    // pool entries. Each path appears at most once.
    (
        1usize..=PATH_POOL.len(),
        proptest::sample::subsequence(PATH_POOL.to_vec(), 1..=PATH_POOL.len()),
    )
        .prop_map(|(_, paths)| {
            let ranking: Vec<HotspotEntry> = paths
                .into_iter()
                .enumerate()
                .map(|(i, p)| entry(p, u32::try_from(i + 1).unwrap_or(u32::MAX)))
                .collect();
            Snapshot {
                label: format!("s{}", ranking.len()),
                ranking,
            }
        })
}

fn snapshots_strategy() -> impl Strategy<Value = Vec<Snapshot>> {
    vec(snapshot_strategy(), 0..6)
}

proptest! {
    /// `total_transitions` on every finding equals `K - 1` where K is
    /// the input length. Locks the bookkeeping; a refactor that
    /// counted only "real" transitions (skipping empty rankings)
    /// would silently desync the prose ("3 of 4 transitions").
    #[test]
    fn total_transitions_equals_k_minus_one(snapshots in snapshots_strategy()) {
        if snapshots.len() < 2 {
            prop_assert!(compute_drift(&snapshots).is_empty());
            return Ok(());
        }
        let expected = u32::try_from(snapshots.len() - 1).unwrap_or(u32::MAX);
        for f in compute_drift(&snapshots) {
            prop_assert_eq!(
                f.total_transitions, expected,
                "finding for {:?} reports {} transitions; expected K-1 = {}",
                f.path, f.total_transitions, expected,
            );
        }
    }

    /// `climb_transitions ≤ total_transitions`. A path can only climb
    /// in as many transitions as exist; the claim is what makes the
    /// "majority of transitions" gate meaningful.
    #[test]
    fn climbs_bounded_by_transitions(snapshots in snapshots_strategy()) {
        for f in compute_drift(&snapshots) {
            prop_assert!(
                f.climb_transitions <= f.total_transitions,
                "{:?}: climb_transitions {} > total_transitions {}",
                f.path, f.climb_transitions, f.total_transitions,
            );
        }
    }

    /// Every finding's `path` exists in the latest snapshot; you
    /// can't drift into top-N without being in the latest ranking.
    /// Locks the `latest_rank` lookup that the prose depends on.
    #[test]
    fn finding_paths_present_in_latest_snapshot(snapshots in snapshots_strategy()) {
        if snapshots.len() < 2 {
            return Ok(());
        }
        let latest_paths: std::collections::HashSet<_> = snapshots
            .last()
            .unwrap()
            .ranking
            .iter()
            .map(|e| &e.path)
            .collect();
        for f in compute_drift(&snapshots) {
            prop_assert!(
                latest_paths.contains(&f.path),
                "finding for {:?} but path absent from latest snapshot",
                f.path,
            );
        }
    }

    /// Strict majority gate: every emitted finding satisfies
    /// `climb_transitions > total_transitions / 2`. (Floor-division
    /// majority — `1 of 2` is not a majority; `2 of 3` is.)
    #[test]
    fn emitted_findings_satisfy_strict_majority_gate(snapshots in snapshots_strategy()) {
        for f in compute_drift(&snapshots) {
            prop_assert!(
                f.climb_transitions > f.total_transitions / 2,
                "{:?}: climbs {} not strict majority of {}",
                f.path, f.climb_transitions, f.total_transitions,
            );
        }
    }

    /// Empty / single-snapshot inputs produce no findings.
    #[test]
    fn degenerate_inputs_return_empty(snapshots in vec(snapshot_strategy(), 0..2)) {
        prop_assert!(compute_drift(&snapshots).is_empty());
    }

    /// Identical-snapshot input (steady state) produces no findings.
    /// Locks the noise floor.
    #[test]
    fn steady_state_produces_no_findings(
        snap in snapshot_strategy(),
        copies in 2usize..6,
    ) {
        let snaps: Vec<Snapshot> = (0..copies).map(|i| Snapshot {
            label: format!("s{i}"),
            ranking: snap.ranking.clone(),
        }).collect();
        prop_assert!(
            compute_drift(&snaps).is_empty(),
            "steady-state ranking across {copies} snapshots produced findings",
        );
    }

    /// Determinism: same input ⇒ same findings.
    #[test]
    fn compute_drift_is_deterministic(snapshots in snapshots_strategy()) {
        let a = compute_drift(&snapshots);
        let b = compute_drift(&snapshots);
        prop_assert_eq!(a, b);
    }
}
