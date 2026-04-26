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

const TOL: f64 = 5e-3;

fn assert_close(actual: f64, expected: f64, label: &str) {
    assert!(
        (actual - expected).abs() < TOL,
        "{label}: expected ≈ {expected}, got {actual} (|delta|={:.6})",
        (actual - expected).abs()
    );
}

#[test]
fn matches_scipy_on_hot_file_real_partner() {
    // runInTerminalTool.ts: 54/203 → scipy says ≈ 0.21097.
    assert_close(wilson_lower_95(54, 203), 0.211, "54/203");
}

#[test]
fn matches_scipy_on_chat_widget_borderline() {
    // chatWidget.ts → chatInputPart.ts: 27/133 → scipy says ≈ 0.14224.
    assert_close(wilson_lower_95(27, 133), 0.142, "27/133");
}

#[test]
fn matches_scipy_on_chat_widget_below_threshold() {
    // chatWidget.ts → chatServiceImpl.ts: 22/133 → scipy ≈ 0.11146.
    assert_close(wilson_lower_95(22, 133), 0.111, "22/133");
}

#[test]
fn matches_scipy_on_album_service_pair() {
    // album.service.ts → album.service.spec.ts: 10/20 → scipy ≈ 0.2993.
    assert_close(wilson_lower_95(10, 20), 0.299, "10/20");
}

#[test]
fn matches_scipy_on_breakpoints_view_quiet_file() {
    // breakpointsView.ts → debugViewlet.css: 3/3 → scipy ≈ 0.4385.
    // Above the 0.20 confidence threshold but should be filtered
    // by the min_sample_size = 5 floor downstream.
    assert_close(wilson_lower_95(3, 3), 0.439, "3/3");
}

#[test]
fn matches_scipy_on_zero_proportion() {
    // 0/100 → scipy ≈ 0.0 (clamped).
    assert_close(wilson_lower_95(0, 100), 0.000, "0/100");
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

#[test]
fn lower_bound_strictly_increases_with_k_at_fixed_n() {
    // For fixed n=100, increasing k must strictly increase the lower
    // bound. This locks the monotonicity scipy guarantees.
    let n = 100;
    let mut prev = wilson_lower_95(0, n);
    for k in 1..=n {
        let cur = wilson_lower_95(k, n);
        assert!(
            cur >= prev,
            "wilson_lower_95({k},{n}) = {cur} regressed below {prev}"
        );
        prev = cur;
    }
}
