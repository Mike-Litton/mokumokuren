//! Property tests for `session::compute_delta`. Locks the
//! universal post-conditions on the session-vs-window comparison and
//! the entropy / churn-of-churn helpers it surfaces. The internal
//! `commit_entropy` and `churn_of_churn` aren't pub, so the tests
//! drive `compute_delta` and read the published fields.

use mmk_core::hotspot::HotspotEntry;
use mmk_core::session::compute_delta;
use mmk_core::types::{Commit, CommitInfo, FileDelta};
use proptest::collection::vec;
use proptest::prelude::*;
use std::path::PathBuf;

/// Production rankings come from a HashMap keyed on `PathBuf`, so
/// each path appears at most once per ranking. The strategy mirrors
/// that invariant by drawing a path subsequence from a fixed pool —
/// arbitrary `path_strategy + vec` would produce duplicate paths and
/// fail invariants the production code never violates.
const PATH_POOL: [&str; 6] = ["a", "b", "c", "d", "e", "f"];

fn delta_strategy() -> impl Strategy<Value = FileDelta> {
    (
        proptest::sample::select(PATH_POOL.to_vec()),
        0u32..50,
        0u32..50,
    )
        .prop_map(|(path, added, deleted)| FileDelta {
            path: PathBuf::from(path),
            added,
            deleted,
        })
}

fn commit_strategy() -> impl Strategy<Value = Commit> {
    (100i64..1_000_000i64, vec(delta_strategy(), 0..6)).prop_map(|(ts, deltas)| Commit {
        info: CommitInfo {
            sha: format!("{ts:040x}"),
            parent_sha: None,
            timestamp: ts,
            author_email: "t@example.com".into(),
        },
        deltas,
    })
}

fn ranking_strategy() -> impl Strategy<Value = Vec<HotspotEntry>> {
    // Pick a subset (no duplicates) and assign sequential ranks.
    proptest::sample::subsequence(PATH_POOL.to_vec(), 1..=PATH_POOL.len()).prop_map(|paths| {
        paths
            .into_iter()
            .enumerate()
            .map(|(i, p)| {
                let rank = u32::try_from(i + 1).unwrap_or(u32::MAX);
                HotspotEntry {
                    path: PathBuf::from(p),
                    loc: 100,
                    weighted_churn: 1.0,
                    relative_churn: 0.01,
                    hotspot_score: f64::from(1_000 - rank),
                    hotspot_rank: rank,
                    commits_touching: 1,
                    last_modified: 0,
                    top_couples: Vec::new(),
                }
            })
            .collect()
    })
}

proptest! {
    /// `commit_entropy` is normalized by `log(n)` and therefore
    /// lives in `[0.0, 1.0]`. The Shannon-entropy upper bound at n
    /// equiprobable bins is exactly `log(n)`, so the normalized form
    /// can't exceed 1.
    #[test]
    fn commit_entropy_in_unit_interval(
        window in ranking_strategy(),
        session in ranking_strategy(),
        commits in vec(commit_strategy(), 0..15),
    ) {
        let d = compute_delta(&window, &session, &commits);
        prop_assert!(
            (0.0..=1.0).contains(&d.commit_entropy),
            "commit_entropy = {} outside [0, 1]",
            d.commit_entropy,
        );
        prop_assert!(d.commit_entropy.is_finite());
    }

    /// Less than two commits ⇒ entropy is exactly zero (degenerate
    /// case; a single observation has no distribution to measure).
    #[test]
    fn commit_entropy_zero_for_degenerate_input(
        window in ranking_strategy(),
        session in ranking_strategy(),
        commits in vec(commit_strategy(), 0..2),
    ) {
        let d = compute_delta(&window, &session, &commits);
        prop_assert!(d.commit_entropy.abs() < 1e-12, "expected 0.0, got {}", d.commit_entropy);
    }

    /// `churn_of_churn[*].ratio ∈ [0, 1]`. The formula is
    /// `min(a, d) * 2 / (a + d)`. With non-negative `a, d` and
    /// `total > 0`, `min * 2 ≤ total`, so the ratio is bounded.
    #[test]
    fn churn_of_churn_ratio_in_unit_interval(
        window in ranking_strategy(),
        session in ranking_strategy(),
        commits in vec(commit_strategy(), 0..10),
    ) {
        let d = compute_delta(&window, &session, &commits);
        for c in &d.churn_of_churn {
            prop_assert!(
                (0.0..=1.0).contains(&c.ratio),
                "{:?}: ratio {} outside [0, 1]",
                c.path, c.ratio,
            );
        }
    }

    /// `churn_of_churn` is sorted by ratio descending, with path as
    /// the lex tiebreaker. The session-summary table reads in this
    /// order.
    #[test]
    fn churn_of_churn_sorted_by_ratio_desc(
        window in ranking_strategy(),
        session in ranking_strategy(),
        commits in vec(commit_strategy(), 0..10),
    ) {
        let d = compute_delta(&window, &session, &commits);
        for w in d.churn_of_churn.windows(2) {
            prop_assert!(
                w[0].ratio >= w[1].ratio,
                "ratio order: {} then {} for {:?} then {:?}",
                w[0].ratio, w[1].ratio, w[0].path, w[1].path,
            );
        }
    }

    /// `entered_top_n` and `rank_climbs` cover the session ranking
    /// completely:
    /// - `entered_top_n` = session paths absent from window.
    /// - `rank_climbs`   = session paths present in window with
    ///                     `window_rank > session_rank` (delta > 0).
    /// - Everything else (session paths present in window with
    ///   `window_rank ≤ session_rank`) is correctly absent from
    ///   both blocks.
    ///
    /// This property locks all three rules in one place. The earlier
    /// version of this property only checked subset relations and
    /// claimed "partition" without verifying the completeness leg —
    /// a regression that silently dropped a true `entered` entry
    /// would have passed.
    #[test]
    fn entered_and_climbs_cover_session_ranking_correctly(
        window in ranking_strategy(),
        session in ranking_strategy(),
        commits in vec(commit_strategy(), 0..5),
    ) {
        let d = compute_delta(&window, &session, &commits);
        let window_ranks: std::collections::HashMap<_, u32> =
            window.iter().map(|e| (e.path.clone(), e.hotspot_rank)).collect();
        let entered_set: std::collections::HashSet<_> = d.entered_top_n.iter().cloned().collect();
        let climb_set: std::collections::HashMap<_, i32> =
            d.rank_climbs.iter().map(|c| (c.path.clone(), c.delta)).collect();

        // For every session entry, exactly one of three states holds.
        for s in &session {
            match window_ranks.get(&s.path) {
                None => {
                    // Not in window ⇒ must be in entered_top_n.
                    prop_assert!(
                        entered_set.contains(&s.path),
                        "session entry {:?} absent from window must appear in entered_top_n",
                        s.path,
                    );
                    prop_assert!(
                        !climb_set.contains_key(&s.path),
                        "session entry {:?} absent from window must NOT appear in rank_climbs",
                        s.path,
                    );
                }
                Some(&w_rank) if w_rank > s.hotspot_rank => {
                    // Climbed (window_rank > session_rank). Must be
                    // in rank_climbs with the right delta.
                    // Both ranks are bounded by the ranking length
                    // (≤ 5 in this strategy), well within i32.
                    #[allow(clippy::cast_possible_wrap)]
                    let expected_delta = w_rank as i32 - s.hotspot_rank as i32;
                    prop_assert_eq!(
                        climb_set.get(&s.path).copied(),
                        Some(expected_delta),
                        "session entry {:?} climbed must appear in rank_climbs with delta {}",
                        s.path, expected_delta,
                    );
                    prop_assert!(!entered_set.contains(&s.path));
                }
                Some(_) => {
                    // Same or worse rank ⇒ neither block.
                    prop_assert!(
                        !entered_set.contains(&s.path),
                        "session entry {:?} unchanged/worse must NOT be in entered_top_n",
                        s.path,
                    );
                    prop_assert!(
                        !climb_set.contains_key(&s.path),
                        "session entry {:?} unchanged/worse must NOT be in rank_climbs",
                        s.path,
                    );
                }
            }
        }

        // Reverse direction: entries in entered_top_n / rank_climbs
        // must come from the session ranking (no synthesized paths).
        let session_paths: std::collections::HashSet<_> =
            session.iter().map(|e| &e.path).collect();
        for p in &d.entered_top_n {
            prop_assert!(
                session_paths.contains(p),
                "{:?} in entered_top_n but absent from session ranking",
                p,
            );
        }
        for c in &d.rank_climbs {
            prop_assert!(
                session_paths.contains(&c.path),
                "{:?} in rank_climbs but absent from session ranking",
                c.path,
            );
            prop_assert!(c.delta > 0, "{:?}: delta {} should be > 0", c.path, c.delta);
        }
    }

    /// Determinism: two calls on identical inputs produce identical
    /// `SessionDelta`s. Output stability matters because the
    /// session-summary JSON envelope is consumed by harnesses that
    /// diff the output across invocations.
    #[test]
    fn compute_delta_is_deterministic(
        window in ranking_strategy(),
        session in ranking_strategy(),
        commits in vec(commit_strategy(), 0..5),
    ) {
        let a = compute_delta(&window, &session, &commits);
        let b = compute_delta(&window, &session, &commits);
        prop_assert_eq!(&a.entered_top_n, &b.entered_top_n);
        prop_assert_eq!(&a.rank_climbs, &b.rank_climbs);
        prop_assert!((a.commit_entropy - b.commit_entropy).abs() < 1e-12);
        prop_assert_eq!(a.churn_of_churn.len(), b.churn_of_churn.len());
        for (ea, eb) in a.churn_of_churn.iter().zip(b.churn_of_churn.iter()) {
            prop_assert_eq!(&ea.path, &eb.path);
            prop_assert!((ea.ratio - eb.ratio).abs() < 1e-12);
        }
    }
}
