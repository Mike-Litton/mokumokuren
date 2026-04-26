//! Diff-budget thresholds — pure checks reused by `mmk review`
//! (per-edit working-tree diff) and `mmk session-summary` (aggregate
//! over a session window).
//!
//! The per-commit `bulk` filter in `mmk-git` already drops any
//! single commit that exceeds `bulk.max_files` / `bulk.max_lines`
//! before metrics see it. These checks surface the *budget* signal
//! at a different granularity (the live edit / the session window)
//! without changing what the analyzer ranks.

use mmk_config::BulkCfg;

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
