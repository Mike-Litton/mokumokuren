//! Edge-case lock for `mmk_core::budget` — the diff-budget pure
//! checks reused by `mmk review` (per-edit) and `mmk session-summary`
//! (aggregate).

use mmk_config::BulkCfg;
use mmk_core::budget::{check_diff_budget, check_session_aggregate, BudgetCheck, BudgetTrigger};

const fn cfg(max_files: u32, max_lines: u32) -> BulkCfg {
    BulkCfg {
        max_files,
        max_lines,
    }
}

#[test]
fn diff_budget_silent_under_thresholds() {
    let triggers = check_diff_budget(
        &BudgetCheck {
            files_changed: 5,
            lines_changed: 200,
        },
        &cfg(15, 1000),
    );
    assert!(
        triggers.is_empty(),
        "under-threshold diff produces no triggers"
    );
}

#[test]
fn diff_budget_files_exceeded() {
    let triggers = check_diff_budget(
        &BudgetCheck {
            files_changed: 20,
            lines_changed: 100,
        },
        &cfg(15, 1000),
    );
    assert!(
        triggers.contains(&BudgetTrigger::FilesExceeded {
            actual: 20,
            max: 15
        }),
        "20 > max_files 15 must fire FilesExceeded; got: {triggers:?}"
    );
}

#[test]
fn diff_budget_lines_exceeded() {
    let triggers = check_diff_budget(
        &BudgetCheck {
            files_changed: 1,
            lines_changed: 6000,
        },
        &cfg(15, 1000),
    );
    assert!(
        triggers.contains(&BudgetTrigger::LinesExceeded {
            actual: 6000,
            max: 1000
        }),
        "6000 > max_lines 1000 must fire LinesExceeded; got: {triggers:?}"
    );
}

#[test]
fn diff_budget_both_can_fire() {
    let triggers = check_diff_budget(
        &BudgetCheck {
            files_changed: 50,
            lines_changed: 6000,
        },
        &cfg(15, 1000),
    );
    assert_eq!(
        triggers.len(),
        2,
        "both files and lines exceeded must produce two distinct triggers; got: {triggers:?}"
    );
}

#[test]
fn session_aggregate_silent_with_zero_commits() {
    assert_eq!(check_session_aggregate(99_999, 0, &cfg(15, 1000)), None);
}

#[test]
fn session_aggregate_silent_under_2x_threshold() {
    // 5 commits × 1000 max_lines × 2 = 10_000 budget; 8000 lines < budget.
    assert_eq!(check_session_aggregate(8000, 5, &cfg(15, 1000)), None);
}

#[test]
fn session_aggregate_fires_above_2x_threshold() {
    // 5 commits × 1000 × 2 = 10_000 budget; 11_000 > budget.
    let r = check_session_aggregate(11_000, 5, &cfg(15, 1000));
    assert_eq!(
        r,
        Some(10_000),
        "session_lines exceeding 2× max_lines × commits must return the soft budget; got: {r:?}"
    );
}
