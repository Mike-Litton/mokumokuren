//! Wilson 95 % lower-bound reference values, locked against scipy's
//! `proportion_confint(method='wilson', alpha=0.05)`. The CI's center
//! comes out tighter at large `n` and wider at small `n`; these
//! reference points span both regimes so a future tweak that breaks
//! either side trips the test.
//!
//! Tolerance is 5e-3 because scipy uses `z_{α/2} = 1.95996398...`
//! while we hard-code `Z_95 = 1.96`. Closer would over-pin to a
//! particular `Z_95` digit.

use mmk_core::coupling::wilson::wilson_lower_95;
use rstest::rstest;

const TOL: f64 = 5e-3;

/// Reference table — `(k, n, scipy-expected)` per row. Each row
/// labels a regime so test names stay grep-friendly while the body
/// is shared. (k, n) pairs come from histograms of real
/// large-monorepo commit data, scipy values from
/// `statsmodels.stats.proportion.proportion_confint`.
#[rstest]
// 200+ commit hot file with a strong real partner.
#[case::hot_file_strong_partner(54, 203, 0.211)]
// Borderline pair just under the default 0.20 confidence floor.
#[case::borderline_under_default(27, 133, 0.142)]
// Pair clearly below the default threshold.
#[case::well_below_default(22, 133, 0.111)]
// Mid-sample pair near 50% proportion.
#[case::mid_sample_balanced(10, 20, 0.299)]
// Quiet-file pair: high proportion (3/3) above 0.20 threshold but
// filtered by `min_sample_size = 5` downstream.
#[case::quiet_file_full_proportion(3, 3, 0.439)]
// 0/100 → scipy ≈ 0.0 (clamped).
#[case::zero_proportion(0, 100, 0.000)]
fn matches_scipy_reference(#[case] k: u32, #[case] n: u32, #[case] expected: f64) {
    let actual = wilson_lower_95(k, n);
    assert!(
        (actual - expected).abs() < TOL,
        "{k}/{n}: expected ≈ {expected}, got {actual} (|delta|={:.6})",
        (actual - expected).abs()
    );
}

#[test]
fn n_zero_returns_zero_not_nan() {
    // Defensive: 0/0 must not be NaN.
    let lo = wilson_lower_95(0, 0);
    assert!(
        lo.is_finite(),
        "wilson_lower_95(0,0) must be finite, got {lo}"
    );
    assert!(lo.abs() < 1e-12, "expected 0.0, got {lo}");
}

// Monotonicity-in-k at fixed n is now property-tested across the
// input domain in `tests/wilson_properties.rs::monotone_in_k_at_fixed_n`.
