//! Property tests for `mmk_core::budget`. The example tests pin
//! specific cap-trigger transitions; the properties here lock the
//! universal post-conditions on the ramp / fraction helpers.

use ahash::AHashMap;
use mmk_config::BulkCfg;
use mmk_core::budget::{
    budget_progress, budget_tier, new_file_fraction, BudgetCheck, BudgetProgress, BudgetTier,
};
use proptest::collection::vec;
use proptest::prelude::*;
use proptest::sample::Index;
use std::path::PathBuf;

const fn tier_ordinal(t: BudgetTier) -> u8 {
    match t {
        BudgetTier::Quiet => 0,
        BudgetTier::Approaching => 1,
        BudgetTier::Near => 2,
        BudgetTier::Over => 3,
    }
}

fn path_strategy() -> impl Strategy<Value = PathBuf> {
    "[a-z]{1,3}\\.rs".prop_map(PathBuf::from)
}

const fn cfg(max_files: u32, max_lines: u32) -> BulkCfg {
    BulkCfg {
        max_files,
        max_lines,
        greenfield_threshold: mmk_config::DEFAULT_GREENFIELD_THRESHOLD,
        ignore_for_budget: Vec::new(),
    }
}

proptest! {
    /// `new_file_fraction` is always in `[0.0, 1.0]`. Empty diff is
    /// 0.0 (defined that way to avoid 0/0); full-greenfield is 1.0.
    #[test]
    fn new_file_fraction_in_unit_interval(
        changed in vec(path_strategy(), 0..30),
        seen in vec(path_strategy(), 0..30),
    ) {
        let map: AHashMap<PathBuf, u32> = seen.into_iter().map(|p| (p, 1)).collect();
        let f = new_file_fraction(&changed, &map);
        prop_assert!(
            (0.0..=1.0).contains(&f),
            "new_file_fraction {f} outside [0, 1]",
        );
        prop_assert!(f.is_finite());
    }

    /// Empty `changed` ⇒ fraction is exactly 0. Defined this way to
    /// avoid 0/0; example test pins the value, property locks it
    /// against an arbitrary `commits_touching` map.
    #[test]
    fn new_file_fraction_empty_changed_is_zero(
        seen in vec(path_strategy(), 0..30),
    ) {
        let map: AHashMap<PathBuf, u32> = seen.into_iter().map(|p| (p, 1)).collect();
        let f = new_file_fraction(&[], &map);
        prop_assert!(f.abs() < 1e-12, "expected 0.0, got {f}");
    }

    /// Fraction is order-independent. Ordering of `changed` is a
    /// caller-side artefact; the proportion of new files isn't.
    #[test]
    fn new_file_fraction_order_independent(
        mut changed in vec(path_strategy(), 1..15),
        seen in vec(path_strategy(), 0..15),
        swaps in vec((any::<Index>(), any::<Index>()), 0..32),
    ) {
        let map: AHashMap<PathBuf, u32> = seen.into_iter().map(|p| (p, 1)).collect();
        let f_canonical = new_file_fraction(&changed, &map);
        let n = changed.len();
        for (a, b) in &swaps {
            changed.swap(a.index(n), b.index(n));
        }
        let f_shuffled = new_file_fraction(&changed, &map);
        prop_assert!((f_canonical - f_shuffled).abs() < 1e-12);
    }

    /// `budget_tier` matches the documented ramp boundaries:
    /// `[0, 0.5)` ⇒ Quiet, `[0.5, 0.75)` ⇒ Approaching,
    /// `[0.75, 1.0)` ⇒ Near, `[1.0, ∞)` ⇒ Over. The property
    /// checks the transitions are monotone non-decreasing in
    /// "severity ordinal" — Quiet < Approaching < Near < Over.
    #[test]
    fn budget_tier_monotone_in_ratio(r1 in 0.0f64..2.0, r2 in 0.0f64..2.0) {
        let (lo, hi) = if r1 <= r2 { (r1, r2) } else { (r2, r1) };
        let p_lo = BudgetProgress { files: (0, 0), lines: (0, 0), peak_ratio: lo };
        let p_hi = BudgetProgress { files: (0, 0), lines: (0, 0), peak_ratio: hi };
        prop_assert!(
            tier_ordinal(budget_tier(&p_lo)) <= tier_ordinal(budget_tier(&p_hi)),
            "tier non-monotone: ratio {lo} -> {:?}, ratio {hi} -> {:?}",
            budget_tier(&p_lo), budget_tier(&p_hi),
        );
    }

    /// `budget_progress` produces a non-negative `peak_ratio`;
    /// `peak_ratio` is the max of the two directional ratios, which
    /// are themselves non-negative.
    #[test]
    fn budget_progress_peak_ratio_non_negative(
        files in 0u32..1_000,
        lines in 0u64..1_000_000,
        max_files in 1u32..200,
        max_lines in 100u32..50_000,
    ) {
        let p = budget_progress(&BudgetCheck { files_changed: files, lines_changed: lines }, &cfg(max_files, max_lines));
        prop_assert!(p.peak_ratio >= 0.0, "peak_ratio {} < 0", p.peak_ratio);
        prop_assert!(p.peak_ratio.is_finite());
    }
}
