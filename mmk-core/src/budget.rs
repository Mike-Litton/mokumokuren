//! Diff-budget thresholds — pure checks reused by `mmk review`
//! (per-edit working-tree diff) and `mmk session-summary` (aggregate
//! over a session window).
//!
//! The per-commit `bulk` filter in `mmk-git` already drops any
//! single commit that exceeds `bulk.max_files` / `bulk.max_lines`
//! before metrics see it. These checks surface the *budget* signal
//! at a different granularity (the live edit / the session window)
//! without changing what the analyzer ranks.

use ahash::AHashSet;
use mmk_config::BulkCfg;
use std::path::PathBuf;

/// What an edit / session looked like, for budget evaluation.
#[derive(Debug, Clone, Copy)]
pub struct BudgetCheck {
    pub files_changed: u32,
    pub lines_changed: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetTrigger {
    /// Files exceeded `bulk.max_files`.
    FilesExceeded { actual: u32, max: u32 },
    /// Lines exceeded `bulk.max_lines`.
    LinesExceeded { actual: u64, max: u64 },
}

/// Continuous-feedback ramp progress.
///
/// Distinct from `BudgetTrigger`: the trigger fires only when the
/// cap is *exceeded*, but the agent loses visibility of the cap
/// climbing toward the limit. The ramp surfaces an Info from 50%, a
/// Warn from 75%, and the existing over-cap finding takes over above
/// 100%. Motivation: the Gloaguen 2026 finding that minimal,
/// only-essential context helps agents — a meter that climbs is one
/// essential signal; flat silence followed by a binary "you blew it"
/// is not.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BudgetProgress {
    /// `(actual_files, max_files)`.
    pub files: (u32, u32),
    /// `(actual_lines, max_lines)`.
    pub lines: (u64, u64),
    /// The peak ratio across files / lines, capped at 1.0 for the
    /// ramp logic. A ratio > 1.0 means the over-cap branch should
    /// fire instead.
    pub peak_ratio: f64,
}

/// Severity tier of the budget ramp. Maps to `Severity::Info` /
/// `Severity::Warn` at the CLI boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetTier {
    /// Below 50% — emit nothing (noise floor).
    Quiet,
    /// 50–74% — Info: meter climbing, agent should know.
    Approaching,
    /// 75–100% — Warn: cap is close, decision point.
    Near,
    /// Above 100% — handled by `BudgetTrigger`; this variant
    /// exists so callers can distinguish "ramp at cap" from
    /// "ramp under cap" without re-deriving.
    Over,
}

/// Compute the ramp tier for a diff against the bulk caps.
///
/// Returns `Quiet` (no finding) below 50%; `Approaching` Info from
/// 50–74%; `Near` Warn from 75–99%; `Over` ≥100%. Callers use
/// `Over` as a signal to skip the ramp message and emit the
/// existing `BudgetTrigger`-based one instead so we don't
/// double-count.
#[must_use]
pub fn budget_progress(check: &BudgetCheck, cfg: &BulkCfg) -> BudgetProgress {
    let max_files = cfg.max_files.max(1);
    let max_lines = u64::from(cfg.max_lines).max(1);
    let r_files = f64::from(check.files_changed) / f64::from(max_files);
    let r_lines = check.lines_changed as f64 / max_lines as f64;
    let peak_ratio = r_files.max(r_lines);
    BudgetProgress {
        files: (check.files_changed, max_files),
        lines: (check.lines_changed, max_lines),
        peak_ratio,
    }
}

/// Map a [`BudgetProgress`] to its tier.
///
/// `Approaching` / `Near` / `Over` gate purely on `peak_ratio`.
///
/// # Examples
///
/// The 50% / 75% / 100% transition points the ramp emits at:
///
/// ```
/// use mmk_core::budget::{budget_tier, BudgetProgress, BudgetTier};
///
/// fn tier_at(r: f64) -> BudgetTier {
///     budget_tier(&BudgetProgress {
///         files: (0, 0),
///         lines: (0, 0),
///         peak_ratio: r,
///     })
/// }
/// assert_eq!(tier_at(0.40), BudgetTier::Quiet);
/// assert_eq!(tier_at(0.60), BudgetTier::Approaching);
/// assert_eq!(tier_at(0.80), BudgetTier::Near);
/// assert_eq!(tier_at(1.10), BudgetTier::Over);
/// ```
#[must_use]
pub fn budget_tier(progress: &BudgetProgress) -> BudgetTier {
    let r = progress.peak_ratio;
    if r >= 1.0 {
        BudgetTier::Over
    } else if r >= 0.75 {
        BudgetTier::Near
    } else if r >= 0.50 {
        BudgetTier::Approaching
    } else {
        BudgetTier::Quiet
    }
}

/// Per-edit / per-range budget. Used by `mmk review` against the
/// working-tree (or `--range` / `--commit`) diff. Returns every
/// trigger that fired so callers can render distinct findings.
#[must_use]
pub fn check_diff_budget(check: &BudgetCheck, cfg: &BulkCfg) -> Vec<BudgetTrigger> {
    let mut triggers = Vec::new();
    if check.files_changed > cfg.max_files {
        triggers.push(BudgetTrigger::FilesExceeded {
            actual: check.files_changed,
            max: cfg.max_files,
        });
    }
    if check.lines_changed > u64::from(cfg.max_lines) {
        triggers.push(BudgetTrigger::LinesExceeded {
            actual: check.lines_changed,
            max: u64::from(cfg.max_lines),
        });
    }
    triggers
}

/// `(count, fraction)` of `changed` paths absent from `at_head`.
///
/// `at_head` is the set of paths present in HEAD's tree, typically
/// from `mmk_git::paths_in_head`. A file in `changed` but not in
/// `at_head` is genuinely new (no HEAD blob); a file present at
/// HEAD is part of the codebase regardless of how recently it
/// churned. This is the predicate the greenfield gate uses.
///
/// `fraction` is in `[0.0, 1.0]`. An empty diff returns `(0, 0.0)`.
#[must_use]
pub fn new_files_at_head(changed: &[PathBuf], at_head: &AHashSet<PathBuf>) -> (usize, f64) {
    if changed.is_empty() {
        return (0, 0.0);
    }
    let new = changed
        .iter()
        .filter(|p| !at_head.contains(p.as_path()))
        .count();
    (new, new as f64 / changed.len() as f64)
}

/// Session-aggregate trigger.
///
/// Returns the soft budget when surviving session lines exceed
/// `2 × max_lines × commits`. The 2× multiplier exists because the
/// per-commit bulk filter already capped each surviving commit at
/// `max_lines`, so a strict `> max_lines × commits` threshold is
/// unreachable in practice. The 2× form fires on a session of many
/// borderline-large commits.
#[must_use]
pub fn check_session_aggregate(
    session_lines: u64,
    session_commits: u32,
    cfg: &BulkCfg,
) -> Option<u64> {
    if session_commits == 0 {
        return None;
    }
    let soft_budget = u64::from(cfg.max_lines) * u64::from(session_commits) * 2;
    if session_lines > soft_budget {
        Some(soft_budget)
    } else {
        None
    }
}
