//! Edge-case lock for `mmk_core::budget` — the diff-budget pure
//! checks reused by `mmk review` (per-edit) and `mmk session-summary`
//! (aggregate).

use ahash::AHashSet;
use mmk_config::BulkCfg;
use mmk_core::budget::{
    budget_progress, budget_tier, check_diff_budget, check_session_aggregate, new_files_at_head,
    BudgetCheck, BudgetTier, BudgetTrigger,
};
use rstest::rstest;
use std::path::PathBuf;

const fn cfg(max_files: u32, max_lines: u32) -> BulkCfg {
    BulkCfg {
        max_files,
        max_lines,
        review_quality_lines: 0,
        greenfield_threshold: mmk_config::DEFAULT_GREENFIELD_THRESHOLD,
        ignore_for_budget: Vec::new(),
    }
}

const fn cfg_with_floor(max_files: u32, max_lines: u32, review_quality_lines: u32) -> BulkCfg {
    BulkCfg {
        max_files,
        max_lines,
        review_quality_lines,
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

// ---- new_files_at_head ----------------------------------------------------

fn at_head(paths: &[&str]) -> AHashSet<PathBuf> {
    paths.iter().map(|p| PathBuf::from(*p)).collect()
}

#[test]
fn new_files_at_head_empty_diff_is_zero() {
    let h = at_head(&[]);
    let (count, frac) = new_files_at_head(&[], &h);
    assert_eq!(count, 0);
    assert!(frac.abs() < 1e-12);
}

#[test]
fn new_files_at_head_all_new_is_one() {
    // None of the changed paths exist at HEAD → every path is new.
    let h = at_head(&[]);
    let changed = vec![
        PathBuf::from("a.rs"),
        PathBuf::from("b.rs"),
        PathBuf::from("c.rs"),
    ];
    let (count, frac) = new_files_at_head(&changed, &h);
    assert_eq!(count, 3);
    assert!((frac - 1.0).abs() < 1e-12);
}

#[test]
fn new_files_at_head_all_existing_is_zero() {
    let h = at_head(&["a.rs", "b.rs", "c.rs"]);
    let changed = vec![
        PathBuf::from("a.rs"),
        PathBuf::from("b.rs"),
        PathBuf::from("c.rs"),
    ];
    let (count, frac) = new_files_at_head(&changed, &h);
    assert_eq!(count, 0);
    assert!(frac.abs() < 1e-12);
}

#[test]
fn new_files_at_head_half_and_half() {
    let h = at_head(&["a.rs", "b.rs"]);
    let changed = vec![
        PathBuf::from("a.rs"),
        PathBuf::from("b.rs"),
        PathBuf::from("new1.rs"),
        PathBuf::from("new2.rs"),
    ];
    let (count, frac) = new_files_at_head(&changed, &h);
    assert_eq!(count, 2);
    assert!(
        (frac - 0.5).abs() < 1e-12,
        "two of four paths absent from HEAD — fraction must be 0.5; got {frac}"
    );
}

#[test]
fn new_files_at_head_cold_file_at_head_is_not_new() {
    // A file present at HEAD but never touched in the analysis
    // window must NOT count as new — the predicate is HEAD presence,
    // not in-window churn.
    let h = at_head(&["cold.rs"]);
    let changed = vec![PathBuf::from("cold.rs"), PathBuf::from("brand_new.rs")];
    let (count, frac) = new_files_at_head(&changed, &h);
    assert_eq!(count, 1);
    assert!(
        (frac - 0.5).abs() < 1e-12,
        "cold file at HEAD must not be new; got {frac}"
    );
}

// ---- ReviewQuality floor (review-effectiveness) ---------------------------

/// The review-effectiveness floor sits *below* `Approaching`: a
/// diff under 50% of the per-diff cap fires `ReviewQuality` once
/// it crosses the absolute LOC threshold. Above 50% the
/// proportional `Approaching` tier wins (precedence).
#[rstest]
#[case::just_under_floor(199, BudgetTier::Quiet)]
#[case::at_floor(200, BudgetTier::ReviewQuality)]
#[case::between_floor_and_50pct(214, BudgetTier::ReviewQuality)]
#[case::approaching_wins_above_50pct(500, BudgetTier::Approaching)]
fn review_quality_tier_transitions(#[case] lines: u64, #[case] expect: BudgetTier) {
    // 200 LOC floor, 1000 LOC cap → 50% of cap (500 LOC) is where
    // Approaching takes over.
    let progress = budget_progress(
        &BudgetCheck {
            files_changed: 0,
            lines_changed: lines,
        },
        &cfg_with_floor(15, 1000, 200),
    );
    assert_eq!(
        budget_tier(&progress),
        expect,
        "{lines} LOC against 200/1000 floor/cap"
    );
}

/// `review_quality_lines = 0` disables the floor — a 199-or-200 LOC
/// diff under 50% of the cap stays Quiet. The test catches an
/// accidental "always fire when lines >= 0" condition where the
/// disabled-knob branch and the active-knob branch share code.
#[test]
fn review_quality_floor_disabled_stays_quiet() {
    let progress = budget_progress(
        &BudgetCheck {
            files_changed: 0,
            lines_changed: 250,
        },
        &cfg_with_floor(15, 1000, 0),
    );
    assert_eq!(budget_tier(&progress), BudgetTier::Quiet);
}
