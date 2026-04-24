//! Churn metrics. Raw `weighted_churn` feeds the hotspot score; the
//! LOC-normalized `relative_churn` is reported alongside but not used in
//! ranking (Nagappan-Ball predictor, emitted for observers).

use ahash::AHashMap;
use std::path::PathBuf;

use crate::types::Commit;

/// Recency-weighted churn per file.
///
/// For each commit, each file's `added + deleted` is multiplied by
/// `exp(-age_seconds / tau_seconds)` and summed across the window.
/// `now_ts` should be the analysis-start timestamp (typically HEAD's committer
/// time, or wall-clock "now"); commits newer than `now_ts` are treated as
/// `age = 0`.
#[must_use]
pub fn weighted_churn(commits: &[Commit], now_ts: i64, tau_seconds: f64) -> AHashMap<PathBuf, f64> {
    let mut out: AHashMap<PathBuf, f64> = AHashMap::new();
    if tau_seconds <= 0.0 {
        return out;
    }
    for commit in commits {
        let age_i = (now_ts - commit.info.timestamp).max(0);
        // Timestamps are well within f64's safe-integer range (2^53 ≈ 285 Myr).
        #[allow(clippy::cast_precision_loss)]
        let age = age_i as f64;
        let weight = (-age / tau_seconds).exp();
        for delta in &commit.deltas {
            let churn = f64::from(delta.added + delta.deleted) * weight;
            *out.entry(delta.path.clone()).or_insert(0.0) += churn;
        }
    }
    out
}

/// Weighted churn divided by current LOC at HEAD.
///
/// Files missing from `loc` (deleted at HEAD) are skipped; files with zero
/// LOC are skipped to avoid division by zero.
#[must_use]
pub fn relative_churn(
    weighted: &AHashMap<PathBuf, f64>,
    loc: &AHashMap<PathBuf, u32>,
) -> AHashMap<PathBuf, f64> {
    let mut out = AHashMap::with_capacity(weighted.len());
    for (path, &w) in weighted {
        if let Some(&lines) = loc.get(path) {
            if lines > 0 {
                out.insert(path.clone(), w / f64::from(lines));
            }
        }
    }
    out
}

/// Count of distinct commits that touched each file.
#[must_use]
pub fn commits_touching(commits: &[Commit]) -> AHashMap<PathBuf, u32> {
    let mut out: AHashMap<PathBuf, u32> = AHashMap::new();
    for commit in commits {
        for delta in &commit.deltas {
            *out.entry(delta.path.clone()).or_insert(0) += 1;
        }
    }
    out
}
