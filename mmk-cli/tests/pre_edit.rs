//! `mmk pre-edit <PATH>` — emit findings *before* editing a file.
//! The PreToolUse:Edit hook target. Composes hotspot rank +
//! coupling lookup into the unified findings format.
//!
//! DRIFT findings are wired in once Step 4 lands `compute_drift`.

mod common;

use common::{build_coupling_fixture, commit_all, init_repo, write, CWD_LOCK, DAY};
use mokumokuren::args::{Format, Gate, PreEditArgs};
use serde_json::Value;
use std::path::PathBuf;
use tempfile::TempDir;

fn pre_edit_args(path: &str) -> PreEditArgs {
    PreEditArgs {
        path: PathBuf::from(path),
        since: "60days".into(),
        top: 20,
        format: Format::Json,
        ignores: Vec::new(),
        config: None,
        verbose: false,
        coupling_threshold: None,
        blast_radius_threshold: None,
        drift_sessions: 0,
        gate: Gate::None,
    }
}

fn run_in(repo: &std::path::Path, args: PreEditArgs) -> Vec<u8> {
    let _g = CWD_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let orig = std::env::current_dir().unwrap();
    std::env::set_current_dir(repo).unwrap();
    let mut stdout: Vec<u8> = Vec::new();
    let mut stderr: Vec<u8> = Vec::new();
    let res = mokumokuren::commands::pre_edit::run(&args, &mut stdout, &mut stderr);
    std::env::set_current_dir(orig).unwrap();
    res.expect("pre-edit should succeed on fixture");
    stdout
}

#[test]
fn pre_edit_emits_hotspot_when_path_is_top_n() {
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    build_coupling_fixture(dir.path(), now);

    let stdout = run_in(dir.path(), pre_edit_args("core/a.rs"));
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");

    let hotspot: Vec<&Value> = v["findings"]
        .as_array()
        .expect("findings array")
        .iter()
        .filter(|f| f["layer"] == "hotspot")
        .collect();
    assert!(
        !hotspot.is_empty(),
        "core/a.rs is the canonical fixture hotspot — must fire HOTSPOT; got: {}",
        v["findings"]
    );
    assert!(hotspot
        .iter()
        .any(|f| f["message"].as_str().unwrap_or("").contains("core/a.rs")));
}

#[test]
fn pre_edit_emits_coupling_for_partners_above_threshold() {
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    build_coupling_fixture(dir.path(), now);

    let stdout = run_in(dir.path(), pre_edit_args("core/a.rs"));
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");

    let coupling: Vec<&Value> = v["findings"]
        .as_array()
        .expect("findings array")
        .iter()
        .filter(|f| f["layer"] == "coupling")
        .collect();
    assert!(
        !coupling.is_empty(),
        "core/a.rs has jaccard 0.75 with core/b.rs — must fire COUPLING informational; got: {}",
        v["findings"]
    );
    let mentions_b = coupling
        .iter()
        .any(|f| f["message"].as_str().unwrap_or("").contains("core/b.rs"));
    assert!(
        mentions_b,
        "COUPLING finding should list core/b.rs as the historical partner; got: {coupling:?}"
    );
}

#[test]
fn pre_edit_silent_on_quiet_file() {
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    init_repo(dir.path());
    // quiet.rs lives in its own seed commit so it never co-changes
    // with anything — zero couples. noisy.rs gets all the churn,
    // which makes it the top hotspot, leaving quiet.rs well outside
    // the rank-1 floor.
    write(dir.path(), "quiet.rs", "q\n");
    commit_all(dir.path(), "seed quiet", now - 31 * DAY);
    write(dir.path(), "noisy.rs", "n\n");
    commit_all(dir.path(), "seed noisy", now - 30 * DAY);
    for i in 0..6 {
        write(dir.path(), "noisy.rs", &format!("n{i}\n"));
        commit_all(dir.path(), &format!("noisy {i}"), now - (29 - i) * DAY);
    }

    // Look up quiet.rs with a tight top — well below the rank it'd
    // claim against noisy.rs.
    let mut args = pre_edit_args("quiet.rs");
    args.top = 1;
    let stdout = run_in(dir.path(), args);
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");

    let findings = v["findings"].as_array().expect("findings array");
    assert!(
        findings.is_empty(),
        "quiet, low-rank, non-coupled file must produce no findings; got: {findings:?}"
    );
}

#[test]
fn pre_edit_with_drift_sessions_runs_and_shapes_findings() {
    use std::fmt::Write as _;
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    init_repo(dir.path());
    // Reasonable churn fixture so find_session_boundaries can pick K
    // distinct linear-chunk boundaries. We assert the wiring works
    // (no error, drift findings — when present — are filtered to
    // args.path); pure-function climb correctness is locked in
    // mmk-core/tests/drift.rs.
    write(dir.path(), "target.rs", "t\n");
    write(dir.path(), "other.rs", "o\n");
    commit_all(dir.path(), "seed", now - 30 * DAY);
    for i in 0..10 {
        let mut body = String::new();
        for n in 0..(5 + i) {
            writeln!(body, "target{n}-r{i}").unwrap();
        }
        write(dir.path(), "target.rs", &body);
        commit_all(
            dir.path(),
            &format!("c{i}"),
            now - (28 - i64::from(i)) * DAY,
        );
    }

    let mut args = pre_edit_args("target.rs");
    args.drift_sessions = 5;
    args.top = 5;
    let stdout = run_in(dir.path(), args);
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");

    let findings = v["findings"].as_array().expect("findings array");
    // Any drift finding emitted must mention target.rs (pre-edit
    // filters DRIFT to the queried path; other files' drift signals
    // are out of scope for this view).
    for f in findings.iter().filter(|f| f["layer"] == "drift") {
        assert!(
            f["message"].as_str().unwrap_or("").contains("target.rs"),
            "DRIFT findings in pre-edit must concern the queried path; got: {f}"
        );
    }
}

#[test]
fn pre_edit_json_envelope_has_path_and_findings() {
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    build_coupling_fixture(dir.path(), now);

    let stdout = run_in(dir.path(), pre_edit_args("core/a.rs"));
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");

    assert_eq!(
        v["pre_edit"]["path"], "core/a.rs",
        "pre_edit.path must echo the queried path; got: {}",
        v["pre_edit"]
    );
    assert!(
        v["findings"].is_array(),
        "top-level findings array must be present; got: {v}"
    );
    assert!(v["schema_version"].is_string());
    assert!(v["crate_version"].is_string());
}
