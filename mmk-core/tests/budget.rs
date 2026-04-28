//! Edge-case lock for `mmk_core::budget` — the diff-budget pure
//! checks reused by `mmk review` (per-edit) and `mmk session-summary`
//! (aggregate).

use ahash::AHashMap;
use mmk_config::BulkCfg;
use mmk_core::budget::{
    check_diff_budget, check_session_aggregate, new_file_fraction, BudgetCheck, BudgetTrigger,
};
use rstest::rstest;
use std::path::PathBuf;

const fn cfg(max_files: u32, max_lines: u32) -> BulkCfg {
    BulkCfg {
        max_files,
        max_lines,
        greenfield_threshold: mmk_config::DEFAULT_GREENFIELD_THRESHOLD,
        ignore_for_budget: Vec::new(),
    }
}

/// Per-edit budget table. The cap is fixed at (15 files, 1000 lines)
/// for every case; what varies is the diff size and which triggers
/// must / must not fire. `(expect_files, expect_lines)` is the
/// expectation matrix.
#[rstest]
#[case::silent_under_thresholds(5, 200, false, false)]
#[case::files_only(20, 100, true, false)]
#[case::lines_only(1, 6000, false, true)]
#[case::both(50, 6000, true, true)]
fn diff_budget_cases(
    #[case] files_changed: u32,
    #[case] lines_changed: u64,
    #[case] expect_files: bool,
    #[case] expect_lines: bool,
) {
    let triggers = check_diff_budget(
        &BudgetCheck {
            files_changed,
            lines_changed,
        },
        &cfg(15, 1000),
    );
    let saw_files = triggers
        .iter()
        .any(|t| matches!(t, BudgetTrigger::FilesExceeded { .. }));
    let saw_lines = triggers
        .iter()
        .any(|t| matches!(t, BudgetTrigger::LinesExceeded { .. }));
    assert_eq!(
        saw_files, expect_files,
        "FilesExceeded mismatch ({files_changed} files): got {triggers:?}"
    );
    assert_eq!(
        saw_lines, expect_lines,
        "LinesExceeded mismatch ({lines_changed} lines): got {triggers:?}"
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
