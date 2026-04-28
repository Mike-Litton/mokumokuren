//! Property tests for `hotspot::rank`. The example tests in
//! `tests/hotspot.rs` cover specific score-formula and ordering cases;
//! the properties here lock the universal post-conditions ranking
//! consumers downstream rely on.

use ahash::AHashMap;
use mmk_core::hotspot::{rank, RankInputs};
use proptest::collection::vec;
use proptest::prelude::*;
use std::path::PathBuf;

fn path_strategy() -> impl Strategy<Value = PathBuf> {
    "[a-z]{1,3}\\.rs".prop_map(PathBuf::from)
}

/// Generate a coherent (weighted, loc, commits_touching, last_modified)
/// quartet keyed on the same path set, so the test exercises the
/// "loc must be present" intersection rule.
fn rank_inputs_strategy() -> impl Strategy<
    Value = (
        AHashMap<PathBuf, f64>,
        AHashMap<PathBuf, u32>,
        AHashMap<PathBuf, u32>,
    ),
> {
    vec(
        (path_strategy(), 0.0f64..10_000.0, 1u32..1_000, 0u32..200),
        0..30,
    )
    .prop_map(|rows| {
        let mut weighted = AHashMap::new();
        let mut loc = AHashMap::new();
        let mut cts = AHashMap::new();
        for (path, w, lines, c) in rows {
            weighted.insert(path.clone(), w);
            loc.insert(path.clone(), lines);
            cts.insert(path, c);
        }
        (weighted, loc, cts)
    })
}

proptest! {
    /// Ranks are dense in `[1, n]` with no gaps and no duplicates.
    /// Locks the load-bearing invariant for everything downstream
    /// that joins on `hotspot_rank` (drift, session deltas).
    #[test]
    fn ranks_are_dense_with_no_gaps_or_duplicates(
        (weighted, loc, cts) in rank_inputs_strategy(),
    ) {
        let relative = AHashMap::new();
        let lm = AHashMap::new();
        let entries = rank(
            RankInputs {
                weighted: &weighted,
                relative: &relative,
                loc: &loc,
                commits_touching: &cts,
                last_modified: &lm,
            },
            0,
        );
        let mut ranks: Vec<u32> = entries.iter().map(|e| e.hotspot_rank).collect();
        ranks.sort_unstable();
        let expected: Vec<u32> = (1..=ranks.len() as u32).collect();
        prop_assert_eq!(
            ranks, expected,
            "ranks must be a contiguous 1..=n sequence",
        );
    }

    /// Output is sorted by `hotspot_score` descending, with path as
    /// the lex tiebreaker. Pulled from the example `ranking_orders_by_score_desc`
    /// test and generalized.
    #[test]
    fn output_sorted_by_score_descending(
        (weighted, loc, cts) in rank_inputs_strategy(),
    ) {
        let relative = AHashMap::new();
        let lm = AHashMap::new();
        let entries = rank(
            RankInputs {
                weighted: &weighted,
                relative: &relative,
                loc: &loc,
                commits_touching: &cts,
                last_modified: &lm,
            },
            0,
        );
        for w in entries.windows(2) {
            // f64 partial_cmp: NaN is excluded by the f64 generator
            // (0.0..10_000.0), so unwrap is safe here.
            prop_assert!(
                w[0].hotspot_score >= w[1].hotspot_score,
                "scores out of order: {} then {} for {:?} then {:?}",
                w[0].hotspot_score, w[1].hotspot_score, w[0].path, w[1].path,
            );
            // Path lex tiebreaker when scores tie.
            if (w[0].hotspot_score - w[1].hotspot_score).abs() < 1e-12 {
                prop_assert!(
                    w[0].path <= w[1].path,
                    "path tiebreaker broken on equal scores: {:?} then {:?}",
                    w[0].path, w[1].path,
                );
            }
        }
    }

    /// Files missing from `loc` are excluded from the output (deleted
    /// from HEAD). Equivalent: the output's path-set is a subset of
    /// `loc`'s key-set.
    #[test]
    fn entries_have_loc_present(
        (weighted, loc, cts) in rank_inputs_strategy(),
    ) {
        let relative = AHashMap::new();
        let lm = AHashMap::new();
        let entries = rank(
            RankInputs {
                weighted: &weighted,
                relative: &relative,
                loc: &loc,
                commits_touching: &cts,
                last_modified: &lm,
            },
            0,
        );
        for e in &entries {
            prop_assert!(
                loc.contains_key(&e.path),
                "entry for {:?} survived even though loc has no record",
                e.path,
            );
        }
    }

    /// `top_n > 0` clamps output length. Catches a refactor that
    /// truncates before sorting (would produce wrong top-N).
    #[test]
    fn top_n_truncation_bounds_output(
        (weighted, loc, cts) in rank_inputs_strategy(),
        top_n in 1usize..50,
    ) {
        let relative = AHashMap::new();
        let lm = AHashMap::new();
        let entries = rank(
            RankInputs {
                weighted: &weighted,
                relative: &relative,
                loc: &loc,
                commits_touching: &cts,
                last_modified: &lm,
            },
            top_n,
        );
        prop_assert!(
            entries.len() <= top_n,
            "got {} entries with top_n={top_n}",
            entries.len(),
        );
    }

    /// Determinism: two calls on the same inputs produce identical
    /// rankings. The function reads from `AHashMap` (whose iteration
    /// order isn't deterministic), so the explicit sort + tiebreaker
    /// is what carries this property.
    #[test]
    fn rank_is_deterministic(
        (weighted, loc, cts) in rank_inputs_strategy(),
    ) {
        let relative = AHashMap::new();
        let lm = AHashMap::new();
        let inputs = RankInputs {
            weighted: &weighted,
            relative: &relative,
            loc: &loc,
            commits_touching: &cts,
            last_modified: &lm,
        };
        let a = rank(inputs, 0);
        let b = rank(inputs, 0);
        prop_assert_eq!(a.len(), b.len());
        for (ea, eb) in a.iter().zip(b.iter()) {
            prop_assert_eq!(&ea.path, &eb.path);
            prop_assert_eq!(ea.hotspot_rank, eb.hotspot_rank);
            prop_assert!((ea.hotspot_score - eb.hotspot_score).abs() < 1e-12);
        }
    }
}
