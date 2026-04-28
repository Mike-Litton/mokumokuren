//! Wording rules for the unified findings surface — pure formatting
//! tests for `mokumokuren::output::messages`.
//!
//! Each formatter has an exact-string equality fixture pinning the
//! wording. Drift caught at the test boundary. The wording-design
//! rationale (descriptive over prescriptive, indicator implications,
//! Code-Red-grounded language) lives in doc-comments on the
//! formatters themselves — that's the readable source of truth, not
//! a list of banned substrings.

use mokumokuren::output::messages as msg;
use std::path::{Path, PathBuf};

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

// ---- coupling_pre_edit ----------------------------------------------------

#[test]
fn coupling_pre_edit_pins_factual_wording() {
    let s = msg::coupling_pre_edit(Path::new("core/a.rs"), Path::new("core/b.rs"), 3, 5);
    assert_eq!(
        s,
        "core/a.rs co-edited with core/b.rs in 3 of 5 prior commits"
    );
}

// ---- hotspot --------------------------------------------------------------

#[test]
fn hotspot_pins_factual_wording() {
    let s = msg::hotspot(Path::new("core/a.rs"), 2, 20);
    assert_eq!(s, "core/a.rs: rank #2 of top-20");
}

// ---- budget_files / budget_lines ------------------------------------------

#[test]
fn budget_files_normal_pins_factual_wording() {
    let s = msg::budget_files(120, 100, None, false);
    assert_eq!(
        s,
        "diff touches 120 files; cap 100; large diffs concentrate rollback risk and slow review"
    );
}

#[test]
fn budget_files_suppressed_names_the_skipped_layers() {
    // The bulk-self path: the diff itself blew the cap, so the
    // history-graph layers (HOTSPOT/COUPLING) didn't run. Wording
    // names *what* and *why* so an agent reads the silence
    // correctly — without it, an empty HOTSPOT/COUPLING block
    // could be misread as "all clear" rather than "uncomputed."
    let s = msg::budget_files(120, 100, None, true);
    assert_eq!(
        s,
        "diff touches 120 files; cap 100, HOTSPOT/COUPLING skipped (partners co-touched by construction); large diffs concentrate rollback risk and slow review"
    );
}

#[test]
fn budget_files_with_gross_split_carries_both_counts() {
    // v0.6 net-with-honest-gross: when ignore_for_budget excluded
    // some files, the prose surfaces both totals so the agent can
    // see what was excluded — silent dropping was the failure mode
    // this calibration ships to avoid.
    let s = msg::budget_files(20, 15, Some(45), false);
    assert_eq!(
        s,
        "diff touches 20 files (45 gross, ignore_for_budget excluded 25); cap 15; large diffs concentrate rollback risk and slow review"
    );
}

#[test]
fn budget_files_with_equal_gross_omits_split() {
    // Defensive: when gross == net (globs configured but matched
    // nothing on this diff), the prose stays terse rather than
    // emitting "(N gross, ignore_for_budget excluded 0)".
    let s = msg::budget_files(20, 15, Some(20), false);
    assert_eq!(
        s,
        "diff touches 20 files; cap 15; large diffs concentrate rollback risk and slow review"
    );
}

#[test]
fn budget_lines_normal_pins_factual_wording() {
    let s = msg::budget_lines(6000, 1000, None, false);
    assert_eq!(
        s,
        "diff is 6000 lines; cap 1000; large diffs concentrate rollback risk and slow review"
    );
}

#[test]
fn budget_lines_suppressed_names_the_skipped_layers() {
    let s = msg::budget_lines(6000, 1000, None, true);
    assert_eq!(
        s,
        "diff is 6000 lines; cap 1000, HOTSPOT/COUPLING skipped (partners co-touched by construction); large diffs concentrate rollback risk and slow review"
    );
}

#[test]
fn budget_lines_with_gross_split_carries_both_counts() {
    let s = msg::budget_lines(80, 1000, Some(1500), false);
    assert_eq!(
        s,
        "diff is 80 lines (1500 gross, ignore_for_budget excluded 1420); cap 1000; large diffs concentrate rollback risk and slow review"
    );
}

#[test]
fn budget_ramp_approaching_emits_meter_only() {
    let s = msg::budget_ramp(8, 15, 600, 1000, false);
    assert_eq!(s, "diff at 8 of 15 files, 600 of 1000 lines (60% of cap)");
}

#[test]
fn budget_ramp_near_appends_decision_clause() {
    let s = msg::budget_ramp(12, 15, 900, 1000, true);
    assert_eq!(
        s,
        "diff at 12 of 15 files, 900 of 1000 lines (90% of cap); approaching review cap"
    );
}

// ---- drift ----------------------------------------------------------------

#[test]
fn drift_pins_factual_wording() {
    let s = msg::drift(Path::new("core/a.rs"), 3, 4, 2);
    assert_eq!(s, "core/a.rs: climbed 3 of 4 transitions; latest rank #2");
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

// ---- quiet_file -----------------------------------------------------------

#[test]
fn quiet_file_without_rank_pins_wording() {
    let s = msg::quiet_file(Path::new("quiet.rs"), 2, 60, None, true);
    assert_eq!(s, "quiet.rs: no signal (2 commits in 60-day window)");
}

#[test]
fn quiet_file_with_rank_appends_rank_clause() {
    let s = msg::quiet_file(Path::new("quiet.rs"), 2, 60, Some(7), true);
    assert_eq!(
        s,
        "quiet.rs: no signal (2 commits in 60-day window, rank #7)"
    );
}

#[test]
fn quiet_file_zero_commits_not_in_head_says_truly_new() {
    // The agent reads this as "create-from-scratch territory; no
    // historical risk." Distinct wording so they don't conflate it
    // with the history-was-filtered case below.
    let s = msg::quiet_file(Path::new("brand-new.rs"), 0, 60, None, false);
    assert_eq!(s, "brand-new.rs: new file (not yet in HEAD)");
}

#[test]
fn quiet_file_zero_commits_in_head_signals_filtered_history() {
    // The dangerous case: file is committed and has been edited
    // before, but every commit was filtered as bulk. Without this
    // wording the agent reads silence as "no risk" when the
    // file's actual maintenance history is just invisible to mmk.
    let s = msg::quiet_file(Path::new("docs/configuration.md"), 0, 180, None, true);
    assert_eq!(
        s,
        "docs/configuration.md: present in HEAD but no analyzable history \
         (file may be stale or prior touches were filtered as bulk commits)"
    );
}

#[test]
fn quiet_file_zero_commits_with_rank_drops_rank_clause() {
    // A new file can't have a hotspot rank — guard against
    // mis-rendering by callers that wire `rank` through anyway.
    let s = msg::quiet_file(Path::new("brand-new.rs"), 0, 60, Some(7), false);
    assert_eq!(s, "brand-new.rs: new file (not yet in HEAD)");
}

// ---- greenfield_signal ----------------------------------------------------

#[test]
fn greenfield_signal_pins_factual_wording() {
    let s = msg::greenfield_signal(10, 15);
    assert_eq!(s, "diff is 10 of 15 new files; history priors don't apply");
}

#[test]
fn greenfield_signal_handles_full_greenfield() {
    let s = msg::greenfield_signal(7, 7);
    assert_eq!(s, "diff is 7 of 7 new files; history priors don't apply");
}

// ---- structure_* / complexity_* -------------------------------------------

#[test]
fn structure_pre_edit_new_pins_factual_wording() {
    let imports = vec![
        "zod".to_string(),
        "@lingui/react/macro".to_string(),
        "useResumeStore".to_string(),
        "useDialogStore".to_string(),
        "useForm".to_string(),
        "@/components/ui/form".to_string(),
    ];
    let templates = vec!["Create*Dialog".to_string(), "Update*Dialog".to_string()];
    let s = msg::structure_pre_edit_new(&msg::StructurePreEdit {
        path: Path::new("src/dialogs/resume/sections/job-tracker.tsx"),
        dir: Path::new("src/dialogs/resume/sections"),
        sibling_count: 15,
        shape_ext: "tsx",
        shape_suffix: "",
        common_imports: &imports,
        total_common_imports: 11,
        cap: 6,
        majority_pct: 66,
        common_templates: &templates,
    });
    assert_eq!(
        s,
        "src/dialogs/resume/sections/job-tracker.tsx: new file in src/dialogs/resume/sections/; \
         15 sibling *.tsx files share imports {zod, @lingui/react/macro, useResumeStore, \
         useDialogStore, useForm, @/components/ui/form} (showing 6 of 11 ≥66%) and export \
         template Create*Dialog, Update*Dialog"
    );
}

#[test]
fn structure_pre_edit_existing_pins_factual_wording() {
    let imports = vec!["zod".to_string()];
    let s = msg::structure_pre_edit_existing(&msg::StructurePreEdit {
        path: Path::new("dlg/foo.tsx"),
        dir: Path::new("dlg"),
        sibling_count: 4,
        shape_ext: "tsx",
        shape_suffix: "",
        common_imports: &imports,
        total_common_imports: 1,
        cap: 6,
        majority_pct: 66,
        common_templates: &[],
    });
    assert_eq!(
        s,
        "dlg/foo.tsx: existing file in dlg/; 4 sibling *.tsx files share imports {zod}"
    );
}

#[test]
fn structure_review_divergent_lists_missing() {
    let missing = vec!["zod".to_string(), "react".to_string()];
    let templates = vec!["Create*Dialog".to_string()];
    let s = msg::structure_review_divergent(Path::new("dlg/d.tsx"), &missing, 6, &templates);
    assert_eq!(
        s,
        "dlg/d.tsx: missing 2 of 6 directory-common imports {zod, react}; \
         not exporting expected Create*Dialog"
    );
}

#[test]
fn structure_review_conforming_pins_wording() {
    let s = msg::structure_review_conforming(Path::new("dlg/d.tsx"), Path::new("dlg"), 4);
    assert_eq!(s, "dlg/d.tsx: matches dlg/ convention (4 sibling baseline)");
}

#[test]
fn complexity_review_nesting_pins_wording() {
    let s = msg::complexity_review_nesting(
        Path::new("src/dialogs/resume/sections/job-tracker.tsx"),
        "parseApplication",
        8,
        Some(2),
    );
    assert_eq!(
        s,
        "src/dialogs/resume/sections/job-tracker.tsx::parseApplication: nesting 8, \
         directory median 2 (ratio 4.0); correlates with elevated defect rate (Code Red 2022)"
    );
}

#[test]
fn complexity_review_size_pins_wording() {
    let s = msg::complexity_review_size(Path::new("a.ts"), "longFn", 120, Some(20));
    assert_eq!(
        s,
        "a.ts::longFn: 120 LOC, directory median 20 LOC (ratio 6.0); \
         correlates with slower comprehension and issue resolution"
    );
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

// ---- quiet_file with paths that happen to contain interesting substrings --

#[test]
fn quiet_file_with_path_containing_special_chars_renders_path_unchanged() {
    // The formatter doesn't rewrite paths. If a real path contains
    // characters that look like config tokens, they survive into the
    // output verbatim — which is correct, since they're path-derived,
    // not formatter-introduced. Locks this so a future "sanitize the
    // path" change can't silently break it.
    let s = msg::quiet_file(Path::new("a/min_sample_size_dir/x.rs"), 0, 60, None, false);
    assert_eq!(s, "a/min_sample_size_dir/x.rs: new file (not yet in HEAD)");
}
