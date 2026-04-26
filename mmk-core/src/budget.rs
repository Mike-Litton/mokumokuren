//! Diff-budget thresholds — pure checks reused by `mmk review`
//! (per-edit working-tree diff) and `mmk session-summary` (aggregate
//! over a session window).
//!
//! The per-commit `bulk` filter in `mmk-git` already drops any
//! single commit that exceeds `bulk.max_files` / `bulk.max_lines`
//! before metrics see it. These checks surface the *budget* signal
//! at a different granularity (the live edit / the session window)
//! without changing what the analyzer ranks.

use ahash::AHashMap;
use mmk_config::BulkCfg;
use std::path::{Path, PathBuf};

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

/// Fraction of `changed` paths that have no entry in `commits_touching`.
///
/// I.e., paths the historical analyzer has never seen. The "new"
/// definition is *absence from the map*; an explicit zero-count entry
/// means the file was visible to the walk but didn't churn, which is
/// structurally different from a brand-new file.
///
/// Returns a value in `[0.0, 1.0]`. An empty diff returns `0.0`.
#[must_use]
pub fn new_file_fraction(changed: &[PathBuf], commits_touching: &AHashMap<PathBuf, u32>) -> f64 {
    if changed.is_empty() {
        return 0.0;
    }
    let new = changed
        .iter()
        .filter(|p| !commits_touching.contains_key(p.as_path()))
        .count();
    new as f64 / changed.len() as f64
}

/// Slice-of-`&Path` form of [`new_file_fraction`] for callers that
/// already have references handy and don't want to allocate
/// `PathBuf`s.
#[must_use]
pub fn new_file_fraction_paths(
    changed: &[&Path],
    commits_touching: &AHashMap<PathBuf, u32>,
) -> f64 {
    if changed.is_empty() {
        return 0.0;
    }
    let new = changed
        .iter()
        .filter(|p| !commits_touching.contains_key(**p))
        .count();
    new as f64 / changed.len() as f64
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
