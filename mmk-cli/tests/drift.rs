//! `mmk drift --base <REF> --sessions <K>` — re-run analyze at K
//! historical session boundaries, surface files climbing in a
//! majority of K-1 transitions. Pure function of git state — no
//! persistence, no shared cache.

mod common;

use common::{commit_all, init_repo, write, DAY};
use mokumokuren::args::{DriftArgs, Format};
use serde_json::Value;
use serial_test::serial;
use tempfile::TempDir;

fn drift_args() -> DriftArgs {
    DriftArgs {
        sessions: 5,
        base: Some("HEAD".into()),
        since: "180days".into(),
        top: 10,
        format: Format::Json,
        ignores: Vec::new(),
        config: None,
        verbose: false,
    }
}

fn run_in(repo: &std::path::Path, args: DriftArgs) -> Vec<u8> {
    let (res, stdout, _) = common::with_cwd(repo, |so, se| {
        mokumokuren::commands::drift::run(&args, so, se)
    });
    res.expect("drift should succeed on fixture");
    stdout
}

/// Build a fixture with 8 linear churn commits across 4 files. No
/// merges → exercises the linear-chunk fallback path of
/// `find_session_boundaries`.
fn build_drift_fixture(repo: &std::path::Path, now: i64) {
    use std::fmt::Write as _;
    init_repo(repo);
    for i in 0..8 {
        let mut body = String::new();
        for n in 0..(i + 5) {
            writeln!(body, "a{n}").unwrap();
        }
        write(repo, "a.rs", &body);
        write(repo, "b.rs", &body);
        write(repo, "c.rs", &body);
        write(repo, "d.rs", &body);
        commit_all(repo, &format!("churn {i}"), now - (10 - i64::from(i)) * DAY);
    }
}

#[serial(cwd)]
#[test]
fn drift_recomputes_at_session_boundaries() {
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    build_drift_fixture(dir.path(), now);

    let stdout = run_in(dir.path(), drift_args());
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");

    let labels = v["drift"]["snapshot_labels"]
        .as_array()
        .expect("drift.snapshot_labels array");
    assert!(
        !labels.is_empty(),
        "drift must produce at least one snapshot; got: {}",
        v["drift"]
    );
    // K=5 requested; the linear-chunk fallback may dedup adjacent
    // boundaries on small fixtures, so allow ≤ K but require ≥ 2 so
    // there's at least one transition.
    assert!(
        labels.len() >= 2 && labels.len() <= 5,
        "expected 2..=5 snapshot labels, got {}: {labels:?}",
        labels.len()
    );

    assert!(v["findings"].is_array(), "findings array must be present");
    assert!(
        v["schema_version"].is_string() && v["crate_version"].is_string(),
        "envelope must carry schema_version + crate_version"
    );
}

#[serial(cwd)]
#[test]
fn drift_no_persistence_same_git_state_same_output() {
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    build_drift_fixture(dir.path(), now);

    let a = run_in(dir.path(), drift_args());
    let b = run_in(dir.path(), drift_args());

    let va: Value = serde_json::from_slice(&a).unwrap();
    let vb: Value = serde_json::from_slice(&b).unwrap();

    assert_eq!(
        va["drift"]["snapshot_labels"], vb["drift"]["snapshot_labels"],
        "same git state must yield identical snapshot labels (pure function)"
    );
    assert_eq!(
        va["findings"], vb["findings"],
        "same git state must yield identical findings (pure function)"
    );
}

#[serial(cwd)]
#[test]
fn drift_zero_sessions_emits_no_snapshots() {
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    build_drift_fixture(dir.path(), now);

    let mut args = drift_args();
    args.sessions = 0;
    let stdout = run_in(dir.path(), args);
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");

    let labels = v["drift"]["snapshot_labels"]
        .as_array()
        .expect("snapshot_labels array (possibly empty)");
    assert!(
        labels.is_empty(),
        "--sessions 0 must produce no snapshots; got: {labels:?}"
    );
    let findings = v["findings"].as_array().expect("findings array");
    assert!(
        findings.is_empty(),
        "no snapshots → no findings; got: {findings:?}"
    );
}
