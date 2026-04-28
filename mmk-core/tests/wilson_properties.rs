//! Property tests for `wilson_lower_95`.
//!
//! Example tests in `wilson.rs` lock specific scipy reference points;
//! the properties here lock invariants that must hold across the input
//! domain. Search-space caps are explicit (n ≤ 10_000) per the
//! rustprojectprimer guidance — any larger range is "vacuously
//! distinct" without surfacing real bugs.

use mmk_core::coupling::wilson::wilson_lower_95;
use proptest::prelude::*;

proptest! {
    /// Output is always in the unit interval. Wilson is documented
    /// to clamp to `[0.0, 1.0]`; this is the load-bearing
    /// post-condition that any future precision-tweak must preserve.
    #[test]
    fn output_in_unit_interval(
        n in 0u32..10_000,
        k in 0u32..10_000,
    ) {
        let lo = wilson_lower_95(k, n);
        prop_assert!(
            (0.0..=1.0).contains(&lo),
            "wilson({k}, {n}) = {lo} outside [0, 1]",
        );
        prop_assert!(lo.is_finite(), "wilson({k}, {n}) = {lo} not finite");
    }

    /// The lower bound is conservative: it never exceeds the point
    /// estimate `k/n`. (For `n == 0` we return 0.0 and skip — there
    /// is no point estimate to compare against.)
    #[test]
    fn lower_bound_below_point_estimate(
        n in 1u32..10_000,
        k in 0u32..10_000,
    ) {
        let k = k.min(n);
        let lo = wilson_lower_95(k, n);
        let p_hat = f64::from(k) / f64::from(n);
        // Allow a tiny epsilon — the Wilson centre + margin
        // arithmetic can land 1e-16 above p_hat at boundary cases
        // due to f64 rounding, which is not a real violation.
        prop_assert!(
            lo <= p_hat + 1e-12,
            "wilson({k}, {n}) = {lo} > p_hat = {p_hat}",
        );
    }

    /// `k = 0` always yields exactly 0. The formula's centre and
    /// margin both vanish at p_hat = 0, and the clamp pulls any
    /// rounding negative to 0.
    #[test]
    fn zero_k_yields_zero(n in 0u32..u32::MAX) {
        let lo = wilson_lower_95(0, n);
        prop_assert!(lo.abs() < 1e-12, "wilson(0, {n}) = {lo}");
    }

    /// `k = n` (full proportion) is bounded strictly below 1 for any
    /// finite sample — Wilson stays honest about uncertainty even at
    /// the boundary, unlike the Wald interval which collapses there.
    #[test]
    fn full_proportion_below_one(n in 1u32..10_000) {
        let lo = wilson_lower_95(n, n);
        prop_assert!(lo < 1.0, "wilson({n}, {n}) = {lo} ≥ 1.0");
    }

    /// Monotone in `k` for fixed `n`: increasing co-changes only
    /// raises the lower bound. Subsumes the hand-rolled
    /// `for k in 1..=100` loop in the example tests.
    #[test]
    fn monotone_in_k_at_fixed_n(
        n in 1u32..1_000,
        k_pair in any::<(u32, u32)>(),
    ) {
        let (a, b) = k_pair;
        let k_lo = a.min(b).min(n);
        let k_hi = a.max(b).min(n);
        let lo_k = wilson_lower_95(k_lo, n);
        let lo_h = wilson_lower_95(k_hi, n);
        prop_assert!(
            lo_h >= lo_k - 1e-12,
            "monotonicity broken: wilson({k_hi}, {n}) = {lo_h} < wilson({k_lo}, {n}) = {lo_k}",
        );
    }

    /// At fixed proportion `k/n = 0.5`, increasing `n` only tightens
    /// the bound (lower bound rises toward the point estimate).
    /// Pinned at p = 0.5 because mixed-precision approximation
    /// behaviour is well-defined there; the property is still the
    /// invariant Wilson needs to satisfy at any fixed p.
    #[test]
    fn lower_bound_tightens_with_more_evidence(n_small in 4u32..20) {
        let n_big = n_small * 10;
        let k_small = n_small / 2;
        let k_big = n_big / 2;
        let lo_small = wilson_lower_95(k_small, n_small);
        let lo_big = wilson_lower_95(k_big, n_big);
        prop_assert!(
            lo_big >= lo_small - 1e-12,
            "more evidence at p≈0.5 should not loosen the bound: \
             wilson({k_small}, {n_small}) = {lo_small}, \
             wilson({k_big}, {n_big}) = {lo_big}",
        );
    }
}
