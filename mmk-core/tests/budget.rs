//! Edge-case lock for `mmk_core::budget` — the diff-budget pure
//! checks reused by `mmk review` (per-edit) and `mmk session-summary`
//! (aggregate).

use ahash::AHashMap;
use mmk_config::BulkCfg;
use mmk_core::budget::{
    check_diff_budget, check_session_aggregate, new_file_fraction, BudgetCheck, BudgetTrigger,
};
use std::path::PathBuf;

const fn cfg(max_files: u32, max_lines: u32) -> BulkCfg {
    BulkCfg {
        max_files,
        max_lines,
        greenfield_threshold: mmk_config::DEFAULT_GREENFIELD_THRESHOLD,
        ignore_for_budget: Vec::new(),
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

// ---- new_file_fraction ----------------------------------------------------

fn touched(map: &[(&str, u32)]) -> AHashMap<PathBuf, u32> {
    map.iter().map(|(p, n)| (PathBuf::from(*p), *n)).collect()
}

#[test]
fn new_file_fraction_empty_diff_is_zero() {
    let m = touched(&[]);
    assert!((new_file_fraction(&[], &m) - 0.0).abs() < 1e-12);
}

#[test]
fn new_file_fraction_all_new_is_one() {
    // None of the changed paths appear in the commits_touching map →
    // every path is "new" by the function's definition.
    let m = touched(&[]);
    let changed = vec![
        PathBuf::from("a.rs"),
        PathBuf::from("b.rs"),
        PathBuf::from("c.rs"),
    ];
    assert!((new_file_fraction(&changed, &m) - 1.0).abs() < 1e-12);
}

#[test]
fn new_file_fraction_all_existing_is_zero() {
    let m = touched(&[("a.rs", 5), ("b.rs", 2), ("c.rs", 9)]);
    let changed = vec![
        PathBuf::from("a.rs"),
        PathBuf::from("b.rs"),
        PathBuf::from("c.rs"),
    ];
    assert!((new_file_fraction(&changed, &m) - 0.0).abs() < 1e-12);
}

#[test]
fn new_file_fraction_half_and_half() {
    let m = touched(&[("a.rs", 3), ("b.rs", 1)]);
    let changed = vec![
        PathBuf::from("a.rs"),
        PathBuf::from("b.rs"),
        PathBuf::from("new1.rs"),
        PathBuf::from("new2.rs"),
    ];
    let f = new_file_fraction(&changed, &m);
    assert!(
        (f - 0.5).abs() < 1e-12,
        "two of four paths are new — fraction must be 0.5; got {f}"
    );
}

#[test]
fn new_file_fraction_is_order_independent() {
    let m = touched(&[("a.rs", 3)]);
    let one = vec![
        PathBuf::from("a.rs"),
        PathBuf::from("new.rs"),
        PathBuf::from("other.rs"),
    ];
    let two = vec![
        PathBuf::from("other.rs"),
        PathBuf::from("a.rs"),
        PathBuf::from("new.rs"),
    ];
    assert!((new_file_fraction(&one, &m) - new_file_fraction(&two, &m)).abs() < 1e-12);
}

#[test]
fn new_file_fraction_zero_count_partner_treated_as_existing() {
    // A path with an explicit zero count in commits_touching has been
    // *seen* in the window even though it didn't churn — that's not
    // greenfield. Only "absent from the map" qualifies as new.
    let m = touched(&[("seen.rs", 0)]);
    let changed = vec![PathBuf::from("seen.rs"), PathBuf::from("new.rs")];
    let f = new_file_fraction(&changed, &m);
    assert!(
        (f - 0.5).abs() < 1e-12,
        "seen-but-zero must not count as new; got {f}"
    );
}
