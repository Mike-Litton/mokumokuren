//! Wording rules for the unified findings surface — pure formatting
//! tests for `mokumokuren::output::messages`.
//!
//! Two assertion classes:
//!   1. Positive fixtures: exact-string equality on a known input,
//!      so the new wording is pinned against accidental drift.
//!   2. Negative oracles: forbidden internal-vocabulary tokens that
//!      must never appear in any rendered message, applied across a
//!      wide range of inputs so they catch regressions independent of
//!      the specific positive fixture.

use mokumokuren::output::messages as msg;
use std::path::{Path, PathBuf};

/// Tokens that must never appear in any human-readable finding
/// message. Internal vocabulary (algorithm names, config tokens),
/// suggestive language, and the percentage framing the user dropped.
const FORBIDDEN: &[&str] = &[
    "Wilson",
    "expected partner",
    "= 100%",
    "= 50%",
    "= 75%",
    "= 25%",
    "min_sample_size",
    "likely a sweep",
    "pre-edit consulted",
    "historical co-edit",
    "historically co-changes",
];

fn assert_no_forbidden(message: &str, context: &str) {
    for token in FORBIDDEN {
        assert!(
            !message.contains(token),
            "{context}: forbidden token {token:?} appeared in: {message:?}"
        );
    }
}

// ---- coupling_review_missed -----------------------------------------------

#[test]
fn coupling_review_missed_pins_factual_wording() {
    let s = msg::coupling_review_missed(
        Path::new("src/left/index.tsx"),
        Path::new("src/-components/edge.tsx"),
        1,
        1,
    );
    assert_eq!(
        s,
        "src/left/index.tsx edited; src/-components/edge.tsx co-edited 1 of 1 prior commits, not in diff"
    );
}

#[test]
fn coupling_review_missed_handles_large_n() {
    let s = msg::coupling_review_missed(Path::new("core/a.rs"), Path::new("core/b.rs"), 54, 203);
    assert_eq!(
        s,
        "core/a.rs edited; core/b.rs co-edited 54 of 203 prior commits, not in diff"
    );
}

#[test]
fn coupling_review_missed_no_forbidden_tokens_across_inputs() {
    for (k, n) in [(1u32, 1u32), (3, 4), (2, 10), (54, 203), (5, 5), (1, 2)] {
        let s = msg::coupling_review_missed(Path::new("core/a.rs"), Path::new("core/b.rs"), k, n);
        assert_no_forbidden(&s, &format!("review_missed k={k} n={n}"));
    }
}

// ---- coupling_pre_edit ----------------------------------------------------

#[test]
fn coupling_pre_edit_pins_factual_wording() {
    let s = msg::coupling_pre_edit(Path::new("core/a.rs"), Path::new("core/b.rs"), 3, 5);
    assert_eq!(
        s,
        "core/a.rs co-edited with core/b.rs in 3 of 5 prior commits"
    );
}

#[test]
fn coupling_pre_edit_no_forbidden_tokens_across_inputs() {
    for (k, n) in [(1u32, 1u32), (3, 4), (54, 203), (1, 2)] {
        let s = msg::coupling_pre_edit(Path::new("a.rs"), Path::new("b.rs"), k, n);
        assert_no_forbidden(&s, &format!("pre_edit k={k} n={n}"));
    }
}

// ---- hotspot --------------------------------------------------------------

#[test]
fn hotspot_pins_factual_wording() {
    let s = msg::hotspot(Path::new("core/a.rs"), 2, 20);
    assert_eq!(s, "core/a.rs: rank #2 of top-20");
}

#[test]
fn hotspot_no_forbidden_tokens_across_inputs() {
    for (rank, top) in [(1u32, 5usize), (2, 20), (10, 50), (1, 1)] {
        let s = msg::hotspot(Path::new("core/a.rs"), rank, top);
        assert_no_forbidden(&s, &format!("hotspot rank={rank} top={top}"));
    }
}

// ---- budget_files / budget_lines ------------------------------------------

#[test]
fn budget_files_normal_pins_factual_wording() {
    let s = msg::budget_files(120, 100, false);
    assert_eq!(s, "diff touches 120 files; cap 100");
}

#[test]
fn budget_files_suppressed_appends_analysis_suppressed() {
    let s = msg::budget_files(120, 100, true);
    assert_eq!(s, "diff touches 120 files; cap 100, analysis suppressed");
}

#[test]
fn budget_lines_normal_pins_factual_wording() {
    let s = msg::budget_lines(6000, 1000, false);
    assert_eq!(s, "diff is 6000 lines; cap 1000");
}

#[test]
fn budget_lines_suppressed_appends_analysis_suppressed() {
    let s = msg::budget_lines(6000, 1000, true);
    assert_eq!(s, "diff is 6000 lines; cap 1000, analysis suppressed");
}

#[test]
fn budget_no_forbidden_tokens_across_inputs() {
    for suppressed in [false, true] {
        let s = msg::budget_files(120, 100, suppressed);
        assert_no_forbidden(&s, &format!("budget_files suppressed={suppressed}"));
        let s = msg::budget_lines(6000, 1000, suppressed);
        assert_no_forbidden(&s, &format!("budget_lines suppressed={suppressed}"));
    }
}

// ---- drift ----------------------------------------------------------------

#[test]
fn drift_pins_factual_wording() {
    let s = msg::drift(Path::new("core/a.rs"), 3, 4, 2);
    assert_eq!(s, "core/a.rs: climbed 3 of 4 transitions; latest rank #2");
}

#[test]
fn drift_no_forbidden_tokens_across_inputs() {
    for (climb, total, rank) in [(3u32, 4u32, 2u32), (1, 1, 5), (10, 10, 1)] {
        let s = msg::drift(Path::new("core/a.rs"), climb, total, rank);
        assert_no_forbidden(
            &s,
            &format!("drift climb={climb} total={total} rank={rank}"),
        );
    }
}

// ---- health_* -------------------------------------------------------------

#[test]
fn health_test_pair_single_related_pins_wording() {
    let related = vec![PathBuf::from("src/foo.test.ts")];
    let s = msg::health_test_pair(Path::new("src/foo.ts"), &related);
    assert_eq!(s, "src/foo.ts: test partner src/foo.test.ts not in diff");
}

#[test]
fn health_test_pair_multiple_related_joins_with_comma() {
    let related = vec![
        PathBuf::from("src/foo.test.ts"),
        PathBuf::from("src/foo.spec.ts"),
    ];
    let s = msg::health_test_pair(Path::new("src/foo.ts"), &related);
    assert_eq!(
        s,
        "src/foo.ts: test partner src/foo.test.ts, src/foo.spec.ts not in diff"
    );
}

#[test]
fn health_registration_pins_wording() {
    let related = vec![
        PathBuf::from("src/actions/one.ts"),
        PathBuf::from("src/actions/two.ts"),
    ];
    let s = msg::health_registration(Path::new("src/actions/new.ts"), &related);
    assert_eq!(
        s,
        "src/actions/new.ts: action-registration; precedents: src/actions/one.ts, src/actions/two.ts"
    );
}

#[test]
fn health_service_pins_wording() {
    let related = vec![
        PathBuf::from("src/consumers/one.ts"),
        PathBuf::from("src/consumers/two.ts"),
    ];
    let s = msg::health_service(Path::new("src/services/foo.ts"), &related);
    assert_eq!(
        s,
        "src/services/foo.ts: service-decl; consumers: src/consumers/one.ts, src/consumers/two.ts"
    );
}

#[test]
fn health_no_forbidden_tokens_across_inputs() {
    let one = vec![PathBuf::from("src/foo.test.ts")];
    let many = vec![
        PathBuf::from("src/a.ts"),
        PathBuf::from("src/b.ts"),
        PathBuf::from("src/c.ts"),
    ];
    for related in [&one, &many] {
        for f in [
            msg::health_test_pair as fn(&Path, &[PathBuf]) -> String,
            msg::health_registration as fn(&Path, &[PathBuf]) -> String,
            msg::health_service as fn(&Path, &[PathBuf]) -> String,
        ] {
            let s = f(Path::new("src/x.ts"), related);
            assert_no_forbidden(&s, "health");
        }
    }
}

// ---- quiet_file -----------------------------------------------------------

#[test]
fn quiet_file_without_rank_pins_wording() {
    let s = msg::quiet_file(Path::new("quiet.rs"), 2, 60, None);
    assert_eq!(s, "quiet.rs: no signal (2 commits in 60-day window)");
}

#[test]
fn quiet_file_with_rank_appends_rank_clause() {
    let s = msg::quiet_file(Path::new("quiet.rs"), 2, 60, Some(7));
    assert_eq!(
        s,
        "quiet.rs: no signal (2 commits in 60-day window, rank #7)"
    );
}

#[test]
fn quiet_file_no_forbidden_tokens_across_inputs() {
    for rank in [None, Some(1), Some(7), Some(50)] {
        let s = msg::quiet_file(Path::new("q.rs"), 0, 30, rank);
        assert_no_forbidden(&s, &format!("quiet_file rank={rank:?}"));
        let s = msg::quiet_file(Path::new("q.rs"), 99, 90, rank);
        assert_no_forbidden(&s, &format!("quiet_file 99 rank={rank:?}"));
    }
}

// ---- session_budget / session_overrun -------------------------------------

#[test]
fn session_budget_pins_wording() {
    let s = msg::session_budget(7, 12, 100, 1000);
    assert_eq!(s, "7 of 12 commits dropped (>100 files or >1000 lines)");
}

#[test]
fn session_overrun_pins_wording() {
    let s = msg::session_overrun(8000, 4, 8000);
    assert_eq!(s, "session is 8000 lines across 4 commits; cap 8000");
}

#[test]
fn session_no_forbidden_tokens_across_inputs() {
    let s = msg::session_budget(1, 50, 100, 1000);
    assert_no_forbidden(&s, "session_budget small");
    let s = msg::session_budget(99, 100, 100, 1000);
    assert_no_forbidden(&s, "session_budget large");
    let s = msg::session_overrun(2000, 1, 1000);
    assert_no_forbidden(&s, "session_overrun");
}
