//! Wording rules for the unified findings surface — pure formatting
//! tests for `mokumokuren::output::messages`.
//!
//! Each formatter has an exact-string snapshot pinning the wording.
//! `cargo insta review` is the diff-acceptance UI; `cargo insta accept`
//! adopts pending snapshots wholesale. Drift caught at the test
//! boundary. The wording-design rationale (descriptive over
//! prescriptive, indicator implications, Code-Red-grounded language)
//! lives in doc-comments on the formatters themselves — that's the
//! readable source of truth, not a list of banned substrings.
//!
//! Snapshots live under `tests/snapshots/`; one file per `#[test]` fn.

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
    insta::assert_snapshot!(s);
}

#[test]
fn coupling_review_missed_handles_large_n() {
    let s = msg::coupling_review_missed(Path::new("core/a.rs"), Path::new("core/b.rs"), 54, 203);
    insta::assert_snapshot!(s);
}

// ---- coupling_pre_edit ----------------------------------------------------

#[test]
fn coupling_pre_edit_pins_factual_wording() {
    let s = msg::coupling_pre_edit(Path::new("core/a.rs"), Path::new("core/b.rs"), 3, 5);
    insta::assert_snapshot!(s);
}

// ---- hotspot --------------------------------------------------------------

#[test]
fn hotspot_pins_factual_wording() {
    let s = msg::hotspot(Path::new("core/a.rs"), 2, 20);
    insta::assert_snapshot!(s);
}

// ---- budget_files / budget_lines ------------------------------------------

#[test]
fn budget_files_normal_pins_factual_wording() {
    let s = msg::budget_files(120, 100, None, false);
    insta::assert_snapshot!(s);
}

#[test]
fn budget_files_suppressed_names_the_skipped_layers() {
    // The bulk-self path: the diff itself blew the cap, so the
    // history-graph layers (HOTSPOT/COUPLING) didn't run. Wording
    // names *what* and *why* so an agent reads the silence
    // correctly — without it, an empty HOTSPOT/COUPLING block
    // could be misread as "all clear" rather than "uncomputed."
    let s = msg::budget_files(120, 100, None, true);
    insta::assert_snapshot!(s);
}

#[test]
fn budget_files_with_gross_split_carries_both_counts() {
    // v0.6 net-with-honest-gross: when ignore_for_budget excluded
    // some files, the prose surfaces both totals so the agent can
    // see what was excluded — silent dropping was the failure mode
    // this calibration ships to avoid.
    let s = msg::budget_files(20, 15, Some(45), false);
    insta::assert_snapshot!(s);
}

#[test]
fn budget_files_with_equal_gross_omits_split() {
    // Defensive: when gross == net (globs configured but matched
    // nothing on this diff), the prose stays terse rather than
    // emitting "(N gross, ignore_for_budget excluded 0)".
    let s = msg::budget_files(20, 15, Some(20), false);
    insta::assert_snapshot!(s);
}

#[test]
fn budget_lines_normal_pins_factual_wording() {
    let s = msg::budget_lines(6000, 1000, None, false);
    insta::assert_snapshot!(s);
}

#[test]
fn budget_lines_suppressed_names_the_skipped_layers() {
    let s = msg::budget_lines(6000, 1000, None, true);
    insta::assert_snapshot!(s);
}

#[test]
fn budget_lines_with_gross_split_carries_both_counts() {
    let s = msg::budget_lines(80, 1000, Some(1500), false);
    insta::assert_snapshot!(s);
}

#[test]
fn budget_ramp_approaching_emits_meter_only() {
    let s = msg::budget_ramp(8, 15, 600, 1000, false);
    insta::assert_snapshot!(s);
}

#[test]
fn budget_ramp_near_appends_decision_clause() {
    let s = msg::budget_ramp(12, 15, 900, 1000, true);
    insta::assert_snapshot!(s);
}

// ---- drift ----------------------------------------------------------------

#[test]
fn drift_pins_factual_wording() {
    let s = msg::drift(Path::new("core/a.rs"), 3, 4, 2);
    insta::assert_snapshot!(s);
}

// ---- health_* -------------------------------------------------------------

#[test]
fn health_test_pair_single_related_pins_wording() {
    let related = vec![PathBuf::from("src/foo.test.ts")];
    let s = msg::health_test_pair(Path::new("src/foo.ts"), &related);
    insta::assert_snapshot!(s);
}

#[test]
fn health_test_pair_multiple_related_joins_with_comma() {
    let related = vec![
        PathBuf::from("src/foo.test.ts"),
        PathBuf::from("src/foo.spec.ts"),
    ];
    let s = msg::health_test_pair(Path::new("src/foo.ts"), &related);
    insta::assert_snapshot!(s);
}

#[test]
fn health_registration_pins_wording() {
    let related = vec![
        PathBuf::from("src/actions/one.ts"),
        PathBuf::from("src/actions/two.ts"),
    ];
    let s = msg::health_registration(Path::new("src/actions/new.ts"), &related);
    insta::assert_snapshot!(s);
}

#[test]
fn health_service_pins_wording() {
    let related = vec![
        PathBuf::from("src/consumers/one.ts"),
        PathBuf::from("src/consumers/two.ts"),
    ];
    let s = msg::health_service(Path::new("src/services/foo.ts"), &related);
    insta::assert_snapshot!(s);
}

// ---- quiet_file -----------------------------------------------------------

#[test]
fn quiet_file_without_rank_pins_wording() {
    let s = msg::quiet_file(Path::new("quiet.rs"), 2, 60, None, true);
    insta::assert_snapshot!(s);
}

#[test]
fn quiet_file_with_rank_appends_rank_clause() {
    let s = msg::quiet_file(Path::new("quiet.rs"), 2, 60, Some(7), true);
    insta::assert_snapshot!(s);
}

#[test]
fn quiet_file_zero_commits_not_in_head_says_truly_new() {
    // The agent reads this as "create-from-scratch territory; no
    // historical risk." Distinct wording so they don't conflate it
    // with the history-was-filtered case below.
    let s = msg::quiet_file(Path::new("brand-new.rs"), 0, 60, None, false);
    insta::assert_snapshot!(s);
}

#[test]
fn quiet_file_zero_commits_in_head_signals_filtered_history() {
    // The dangerous case: file is committed and has been edited
    // before, but every commit was filtered as bulk. Without this
    // wording the agent reads silence as "no risk" when the
    // file's actual maintenance history is just invisible to mmk.
    let s = msg::quiet_file(Path::new("docs/configuration.md"), 0, 180, None, true);
    insta::assert_snapshot!(s);
}

#[test]
fn quiet_file_zero_commits_with_rank_drops_rank_clause() {
    // A new file can't have a hotspot rank — guard against the
    // wrong rank surfacing if a caller wires `rank` through anyway.
    let s = msg::quiet_file(Path::new("brand-new.rs"), 0, 60, Some(7), false);
    insta::assert_snapshot!(s);
}

// ---- greenfield_signal ----------------------------------------------------

#[test]
fn greenfield_signal_pins_factual_wording() {
    let s = msg::greenfield_signal(10, 15);
    insta::assert_snapshot!(s);
}

#[test]
fn greenfield_signal_handles_full_greenfield() {
    let s = msg::greenfield_signal(7, 7);
    insta::assert_snapshot!(s);
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
    insta::assert_snapshot!(s);
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
    insta::assert_snapshot!(s);
}

#[test]
fn structure_review_divergent_lists_missing() {
    let missing = vec!["zod".to_string(), "react".to_string()];
    let templates = vec!["Create*Dialog".to_string()];
    let s = msg::structure_review_divergent(Path::new("dlg/d.tsx"), &missing, 6, &templates);
    insta::assert_snapshot!(s);
}

#[test]
fn structure_review_conforming_pins_wording() {
    let s = msg::structure_review_conforming(Path::new("dlg/d.tsx"), Path::new("dlg"), 4);
    insta::assert_snapshot!(s);
}

#[test]
fn complexity_review_nesting_pins_wording() {
    // Ratio gate path: actual=8, cap=6, median=2 → ratio fired
    // (the 8 > 6 absolute breach is also true, so the new wording
    // anchors on the cap; median enrichment follows).
    let s = msg::complexity_review_nesting(
        Path::new("src/dialogs/resume/sections/job-tracker.tsx"),
        "parseApplication",
        8,
        6,
        None,
        Some(2),
    );
    insta::assert_snapshot!(s);
}

#[test]
fn complexity_review_size_pins_wording() {
    // Same shape as nesting: actual=120 LOC, cap=80 (default),
    // median=20. Absolute fires; median+ratio enrich.
    let s = msg::complexity_review_size(Path::new("a.ts"), "longFn", 120, 80, None, Some(20));
    insta::assert_snapshot!(s);
}

#[test]
fn complexity_review_size_singleton_directory_names_cap_not_unknown_median() {
    // Regression oracle for the v0.7 fix: when median is None
    // (no siblings to compute a directory baseline), the prose must
    // name the absolute cap that fired — not confess "directory
    // median unknown" back to the reader. The reader needs an
    // actionable instruction (reduce below 80 LOC), not
    // meta-information about mmk's missing data.
    let s = msg::complexity_review_size(
        Path::new("src/routes/widget/-components/dialog.tsx"),
        "WidgetDialog",
        320,
        80,
        None,
        None,
    );
    assert!(
        !s.contains("median unknown"),
        "must not leak the median-unknown state into prose; got: {s}"
    );
    assert!(
        s.contains("exceeds cap 80"),
        "must name the absolute cap that fired; got: {s}"
    );
    insta::assert_snapshot!(s);
}

#[test]
fn complexity_review_nesting_singleton_directory_names_cap() {
    let s =
        msg::complexity_review_nesting(Path::new("src/foo.ts"), "deeplyNested", 9, 6, None, None);
    assert!(
        !s.contains("median unknown"),
        "must not leak the median-unknown state into prose; got: {s}"
    );
    assert!(
        s.contains("exceeds cap 6"),
        "must name the absolute cap that fired; got: {s}"
    );
    insta::assert_snapshot!(s);
}

#[test]
fn complexity_review_size_renders_delta_when_head_baseline_known() {
    // Real-world failure mode (data point #3): an agent grew
    // `parse` from 363 → 366 LOC. Without the delta clause the
    // prose reads "366 LOC exceeds cap 80" — agent's perception
    // is "this problem mostly existed before me." With the delta
    // it reads "366 LOC exceeds cap 80 (+3 vs HEAD)" — the agent
    // can judge their contribution at a glance.
    let s = msg::complexity_review_size(
        Path::new("src/integrations/import/reactive-resume-v4-json.tsx"),
        "parse",
        366,
        80,
        Some(363),
        None,
    );
    assert!(
        s.contains("(+3 vs HEAD)"),
        "must render the +N vs HEAD clause when head_actual is known and worsened; got: {s}"
    );
    assert!(
        s.contains("exceeds cap 80"),
        "must keep the cap-naming anchor alongside the delta; got: {s}"
    );
    insta::assert_snapshot!(s);
}

#[test]
fn complexity_review_size_omits_delta_when_head_baseline_absent() {
    // New file or new function: head_actual is None, so the prose
    // shouldn't fabricate a delta. Reader gets the same
    // cap-anchored message the singleton-directory case produces.
    let s = msg::complexity_review_size(Path::new("src/new.ts"), "newFn", 200, 80, None, None);
    assert!(
        !s.contains("vs HEAD"),
        "must not render `vs HEAD` when no baseline; got: {s}"
    );
}

#[test]
fn complexity_review_nesting_renders_delta() {
    let s = msg::complexity_review_nesting(Path::new("src/foo.ts"), "deepFn", 9, 6, Some(7), None);
    assert!(
        s.contains("(+2 vs HEAD)"),
        "nesting variant must also render the delta; got: {s}"
    );
}

// ---- session_budget / session_overrun -------------------------------------

#[test]
fn session_budget_pins_wording() {
    let s = msg::session_budget(7, 12, 100, 1000);
    insta::assert_snapshot!(s);
}

#[test]
fn session_overrun_pins_wording() {
    let s = msg::session_overrun(8000, 4, 8000);
    insta::assert_snapshot!(s);
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
    insta::assert_snapshot!(s);
}
