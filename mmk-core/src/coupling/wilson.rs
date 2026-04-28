//! Wilson score lower bound for the conditional probability `P(B | A)`.
//!
//! The COUPLING decision rule asks "given the agent edited file A,
//! what's the chance file B is *also* the right thing to touch?" That
//! is `P(B | A) = co_change(A, B) / commits_touching(A)` — a binomial
//! proportion estimated from `n = commits_touching(A)` Bernoulli
//! trials, each with success indicator `B ∈ commit`.
//!
//! Point estimates collapse for small `n`: a 1-of-1 hit looks like
//! 100 % until you ask how confident the estimate is. The Wilson score
//! interval (`Wilson, 1927`) gives a closed-form CI that stays
//! well-defined at the boundaries (unlike the normal-approximation
//! "Wald" interval which underestimates badly for `p` near 0 or 1) and
//! handles small `n` honestly.
//!
//! We surface only the **lower** bound at 95 % confidence: that's the
//! "you can confidently expect at least this much conditional
//! probability" number the agent should gate on. Pairing the lower
//! bound with a `min_sample_size` floor (handled by the caller) is the
//! standard "don't infer from too few observations" guard.

const Z_95: f64 = 1.96;

/// Wilson 95 % lower bound for a binomial proportion.
///
/// Returns `0.0` when `n == 0` (no observations → no inference).
/// Saturates `k` to `n` if a caller passes a malformed pair so the
/// formula stays in `[0, 1]`.
///
/// # Examples
///
/// The hot-file calibration target — 54 of 203 commits is the
/// motivating "real partner" case the COUPLING gate must surface:
///
/// ```
/// use mmk_core::coupling::wilson::wilson_lower_95;
/// let lo = wilson_lower_95(54, 203);
/// // scipy says ≈ 0.21097; our Z_95 is rounded to 1.96 so we land
/// // a few permille tighter (0.20998).
/// assert!(lo > 0.205 && lo < 0.215, "got {lo}");
/// ```
///
/// Empty observations collapse to zero, never NaN:
///
/// ```
/// # use mmk_core::coupling::wilson::wilson_lower_95;
/// assert_eq!(wilson_lower_95(0, 0), 0.0);
/// ```
#[must_use]
pub fn wilson_lower_95(k: u32, n: u32) -> f64 {
    if n == 0 {
        return 0.0;
    }
    let k = k.min(n);
    let n_f = f64::from(n);
    let p_hat = f64::from(k) / n_f;
    let z2 = Z_95 * Z_95;
    let denom = 1.0 + z2 / n_f;
    let center = (p_hat + z2 / (2.0 * n_f)) / denom;
    let margin = Z_95 * (p_hat * (1.0 - p_hat) / n_f + z2 / (4.0 * n_f * n_f)).sqrt() / denom;
    (center - margin).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::wilson_lower_95;

    #[test]
    fn zero_n_yields_zero() {
        assert!((wilson_lower_95(0, 0) - 0.0).abs() < 1e-12);
    }

    #[test]
    fn zero_k_yields_zero_lower_bound() {
        // k=0/n=10 → point estimate 0; Wilson lower bound is 0 (clamped).
        let lo = wilson_lower_95(0, 10);
        assert!(lo.abs() < 1e-12, "expected ~0, got {lo}");
    }

    #[test]
    fn full_proportion_lower_bound_is_below_one() {
        // 5/5 → point estimate 1.0; Wilson lower bound is < 1 because
        // the sample is small. Verifies the interval stays bounded.
        let lo = wilson_lower_95(5, 5);
        assert!(lo > 0.4 && lo < 1.0, "expected (0.4, 1.0), got {lo}");
    }
}
