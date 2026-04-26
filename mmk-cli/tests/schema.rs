//! Stable JSON schema versioning. The `schema_version` field tracks the
//! `mmk` minor release; consumers (LLM harnesses) pin against it. The
//! `crate_version` field is the Cargo crate version and is reported
//! purely for diagnostics.

mod common;

use common::{
    build_canonical_fixture, build_coupling_fixture, build_session_fixture, write, CWD_LOCK,
};
use mokumokuren::args::{AnalyzeArgs, DriftArgs, Format, PreEditArgs, ReviewArgs, SessionArgs};
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::PathBuf;
use tempfile::TempDir;

fn run_in(repo: &std::path::Path, args: AnalyzeArgs) -> Vec<u8> {
    let _g = CWD_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let orig = std::env::current_dir().unwrap();
    std::env::set_current_dir(repo).unwrap();
    let mut stdout: Vec<u8> = Vec::new();
    let mut stderr: Vec<u8> = Vec::new();
    let res = mokumokuren::commands::analyze::run(&args, &mut stdout, &mut stderr);
    std::env::set_current_dir(orig).unwrap();
    res.expect("analyze should succeed on fixture");
    stdout
}

fn json_args() -> AnalyzeArgs {
    AnalyzeArgs {
        since: "30days".into(),
        top: 20,
        format: Format::Json,
        ignores: Vec::new(),
        config: None,
        verbose: false,
        couples_of: None,
        couples: false,
        blast_radius: None,
        blast_radius_threshold: None,
    }
}

#[test]
fn schema_version_present_and_pinned() {
    let dir = TempDir::new().unwrap();
    let now: i64 = 1_700_000_000;
    build_canonical_fixture(dir.path(), now);

    let stdout = run_in(dir.path(), json_args());
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");
    assert_eq!(
        v["schema_version"], "0.4.0",
        "schema_version should be pinned to the mmk minor release"
    );
}

#[test]
fn crate_version_distinct_from_schema_version() {
    let dir = TempDir::new().unwrap();
    let now: i64 = 1_700_000_000;
    build_canonical_fixture(dir.path(), now);

    let stdout = run_in(dir.path(), json_args());
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");

    let schema = v["schema_version"]
        .as_str()
        .expect("schema_version should be a string");
    let crate_v = v["crate_version"]
        .as_str()
        .expect("crate_version should be a string");

    // The legacy `version` field (which conflated crate + schema) must be
    // gone — consumers should not be tempted to read it.
    assert!(
        v.get("version").is_none(),
        "legacy `version` field must not appear once we have crate_version + schema_version"
    );
    assert!(!schema.is_empty());
    assert!(!crate_v.is_empty());
}

// --- Structural lock: assert key sets at every documented level. ---
//
// These tests are the runtime of the contract in `docs/schema.md`.
// Renaming, adding, or removing a documented field without updating
// both the docs *and* this test fails the build. Additive optional
// fields (e.g. `blast_radius` when not requested) live behind
// `expect_optional_keys`.
//
// Orthogonality tag: protects **agent mode** — the LLM harness's JSON
// parser depends on this contract; humans reading text output don't.

fn keys(v: &Value) -> BTreeSet<String> {
    v.as_object()
        .expect("expected an object at this level")
        .keys()
        .cloned()
        .collect()
}

fn expect_required_keys(value: &Value, expected: &[&str], context: &str) {
    let actual = keys(value);
    let expected_set: BTreeSet<String> = expected.iter().map(|s| (*s).to_string()).collect();
    let missing: Vec<&String> = expected_set.difference(&actual).collect();
    let extra: Vec<&String> = actual.difference(&expected_set).collect();
    assert!(
        missing.is_empty() && extra.is_empty(),
        "{context}: schema-shape mismatch.\n\
             missing required: {missing:?}\n\
             unexpected extra: {extra:?}\n\
             expected exactly: {expected_set:?}\n\
             got:               {actual:?}\n\
             If you added a field on purpose: update docs/schema.md AND this test together."
    );
}

fn assert_string_or_null(v: &Value, label: &str) {
    assert!(
        v.is_string() || v.is_null(),
        "{label}: expected string or null, got {v:?}"
    );
}

fn ranking_entry_keys() -> Vec<&'static str> {
    vec![
        "path",
        "loc",
        "weighted_churn",
        "relative_churn",
        "hotspot_score",
        "hotspot_rank",
        "commits_touching",
        "last_modified",
        "top_couples",
    ]
}

/// Strict shape for `top_couples[]` entries. Tested via the coupling
/// fixture (the canonical fixture has no co-changes so its
/// top_couples arrays come back empty and don't exercise this).
fn coupling_entry_keys() -> Vec<&'static str> {
    vec![
        "partner",
        "jaccard",
        "co_change_count",
        "conditional_probability",
        "wilson_lower_95",
    ]
}

#[test]
fn schema_shape_matches_docs_contract() {
    let dir = TempDir::new().unwrap();
    let now: i64 = 1_700_000_000;
    build_canonical_fixture(dir.path(), now);

    let stdout = run_in(dir.path(), json_args());
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");

    // Top-level: blast_radius is optional (only present when --blast-radius
    // was requested). All others are required. session/session_files are
    // session-only and must NOT appear here.
    expect_required_keys(
        &v,
        &[
            "schema_version",
            "crate_version",
            "repo",
            "config",
            "analysis",
            "files",
        ],
        "top-level analyze output",
    );
    assert!(
        v.get("session").is_none(),
        "session block must not appear in `analyze` output"
    );
    assert!(
        v.get("blast_radius").is_none(),
        "blast_radius must be absent when --blast-radius was not requested"
    );

    // repo block.
    expect_required_keys(
        &v["repo"],
        &["head_sha", "head_timestamp", "is_shallow", "warnings"],
        "repo",
    );
    assert_string_or_null(&v["repo"]["head_sha"], "repo.head_sha");
    assert_string_or_null(&v["repo"]["head_timestamp"], "repo.head_timestamp");
    assert!(v["repo"]["is_shallow"].is_boolean(), "repo.is_shallow");
    assert!(v["repo"]["warnings"].is_array(), "repo.warnings");

    // analysis block.
    expect_required_keys(
        &v["analysis"],
        &[
            "commits_seen",
            "commits_analyzed",
            "commits_filtered",
            "files_ignored",
            "duration_ms",
        ],
        "analysis",
    );
    expect_required_keys(
        &v["analysis"]["commits_filtered"],
        &["bulk"],
        "analysis.commits_filtered",
    );
    expect_required_keys(
        &v["analysis"]["files_ignored"],
        &["deleted_from_head", "head_paths_ignored"],
        "analysis.files_ignored",
    );

    // files[] entries.
    let files = v["files"].as_array().expect("files is an array");
    assert!(
        !files.is_empty(),
        "fixture should produce at least one entry"
    );
    for (i, entry) in files.iter().enumerate() {
        expect_required_keys(entry, &ranking_entry_keys(), &format!("files[{i}]"));
        // The canonical fixture intentionally has no co-changes so
        // top_couples arrays come back empty here. The strict
        // entry-shape lock lives in
        // [`schema_top_couples_entry_shape_against_coupling_fixture`],
        // which uses build_coupling_fixture.
        let couples = entry["top_couples"]
            .as_array()
            .unwrap_or_else(|| panic!("files[{i}].top_couples must be an array"));
        for (j, c) in couples.iter().enumerate() {
            expect_required_keys(
                c,
                &coupling_entry_keys(),
                &format!("files[{i}].top_couples[{j}]"),
            );
        }
    }
}

#[test]
fn schema_top_couples_entry_shape_against_coupling_fixture() {
    // Pin the `top_couples[]` entry shape on a fixture that
    // *actually populates* the array. The canonical fixture's empty
    // arrays meant the contract test would silently pass with the
    // wrong field set — this closes that hole.
    let dir = TempDir::new().unwrap();
    let now: i64 = 1_700_000_000;
    build_coupling_fixture(dir.path(), now);

    let mut args = json_args();
    args.since = "60days".into();
    let stdout = run_in(dir.path(), args);
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");

    let files = v["files"].as_array().expect("files array");
    let a_entry = files
        .iter()
        .find(|f| f["path"] == "core/a.rs")
        .expect("core/a.rs in files");
    let couples = a_entry["top_couples"]
        .as_array()
        .expect("top_couples on core/a.rs");
    assert!(
        !couples.is_empty(),
        "coupling fixture must populate top_couples[] on core/a.rs"
    );
    for (j, c) in couples.iter().enumerate() {
        expect_required_keys(c, &coupling_entry_keys(), &format!("top_couples[{j}]"));
    }

    let b_couple = couples
        .iter()
        .find(|c| c["partner"] == "core/b.rs")
        .expect("canonical core/b.rs partner");
    // Lock the populated values too — the contract is "right keys,
    // right values," not just keys. P(B|A) = 3/5 = 0.60; jaccard = 0.60.
    let p = b_couple["conditional_probability"].as_f64().unwrap();
    assert!(
        (p - 0.60).abs() < 1e-9,
        "conditional_probability for canonical pair = 3/5; got {p}"
    );
    let w = b_couple["wilson_lower_95"].as_f64().unwrap();
    assert!(
        w > 0.20 && w < 0.30,
        "Wilson 95% lower for 3/5, n=5 should be ≈0.231; got {w}"
    );
    let co = b_couple["co_change_count"].as_u64().unwrap();
    assert_eq!(co, 3, "co_change_count of canonical pair");
}

#[test]
fn schema_blast_radius_block_shape_matches_docs() {
    let dir = TempDir::new().unwrap();
    let now: i64 = 1_700_000_000;
    build_canonical_fixture(dir.path(), now);

    let mut args = json_args();
    args.blast_radius = Some("a.rs".into());
    let stdout = run_in(dir.path(), args);
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");

    // `threshold` is part of the documented blast_radius shape.
    // Lock mirrors docs/schema.md.
    expect_required_keys(
        &v["blast_radius"],
        &["root", "hops", "threshold", "nodes"],
        "blast_radius",
    );

    let nodes = v["blast_radius"]["nodes"]
        .as_array()
        .expect("blast_radius.nodes is an array");
    for (i, n) in nodes.iter().enumerate() {
        expect_required_keys(
            n,
            &["path", "jaccard", "co_change_count", "hops"],
            &format!("blast_radius.nodes[{i}]"),
        );
    }
}

fn session_args() -> SessionArgs {
    SessionArgs {
        since: "60days".into(),
        top: 5,
        format: Format::Json,
        base: Some("main".into()),
        since_commit: None,
        ignores: Vec::new(),
        config: None,
        verbose: false,
        blast_radius: None,
        blast_radius_threshold: None,
        drift_sessions: 0,
        gate: mokumokuren::args::Gate::None,
    }
}

fn run_session_in(repo: &std::path::Path, args: SessionArgs) -> Vec<u8> {
    let _g = CWD_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let orig = std::env::current_dir().unwrap();
    std::env::set_current_dir(repo).unwrap();
    let mut stdout: Vec<u8> = Vec::new();
    let mut stderr: Vec<u8> = Vec::new();
    let res = mokumokuren::commands::session::run(&args, &mut stdout, &mut stderr);
    std::env::set_current_dir(orig).unwrap();
    res.expect("session should succeed on fixture");
    stdout
}

#[test]
fn schema_session_shape_matches_docs_contract() {
    let dir = TempDir::new().unwrap();
    let now: i64 = 1_700_000_000;
    build_session_fixture(dir.path(), now);

    let stdout = run_session_in(dir.path(), session_args());
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");

    expect_required_keys(
        &v,
        &[
            "schema_version",
            "crate_version",
            "repo",
            "config",
            "analysis",
            "files",
            "session_files",
            "session",
            "findings",
        ],
        "top-level session-summary output",
    );
    assert!(
        v["findings"].is_array(),
        "session-summary findings must be an array (possibly empty), got {:?}",
        v["findings"]
    );
    assert!(
        v.get("blast_radius").is_none(),
        "blast_radius must be absent when --blast-radius was not requested"
    );

    // session block.
    expect_required_keys(
        &v["session"],
        &[
            "base_ref",
            "base_sha",
            "base_resolved_via",
            "entered_top_n",
            "rank_climbs",
            "churn_of_churn",
            "commit_entropy",
        ],
        "session",
    );
    assert_string_or_null(&v["session"]["base_ref"], "session.base_ref");
    assert_string_or_null(&v["session"]["base_sha"], "session.base_sha");
    assert!(v["session"]["base_resolved_via"].is_string());
    assert!(v["session"]["entered_top_n"].is_array());
    assert!(v["session"]["rank_climbs"].is_array());
    assert!(v["session"]["churn_of_churn"].is_array());
    assert!(v["session"]["commit_entropy"].is_number());

    // rank_climbs[] entries.
    for (i, rc) in v["session"]["rank_climbs"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        expect_required_keys(rc, &["path", "delta"], &format!("rank_climbs[{i}]"));
    }
    // churn_of_churn[] entries.
    for (i, c) in v["session"]["churn_of_churn"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        expect_required_keys(c, &["path", "ratio"], &format!("churn_of_churn[{i}]"));
    }

    // files[] and session_files[] both follow the ranking-entry contract.
    for (label, arr) in [
        ("files", &v["files"]),
        ("session_files", &v["session_files"]),
    ] {
        for (i, entry) in arr.as_array().unwrap().iter().enumerate() {
            expect_required_keys(entry, &ranking_entry_keys(), &format!("{label}[{i}]"));
        }
    }
}

// --- review / pre-edit / drift envelope locks. ---

fn run_review_in(repo: &std::path::Path, args: ReviewArgs) -> Vec<u8> {
    let _g = CWD_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let orig = std::env::current_dir().unwrap();
    std::env::set_current_dir(repo).unwrap();
    let mut stdout: Vec<u8> = Vec::new();
    let mut stderr: Vec<u8> = Vec::new();
    let res = mokumokuren::commands::review::run(&args, &mut stdout, &mut stderr);
    std::env::set_current_dir(orig).unwrap();
    res.expect("review should succeed on fixture");
    stdout
}

fn run_pre_edit_in(repo: &std::path::Path, args: PreEditArgs) -> Vec<u8> {
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

fn run_drift_in(repo: &std::path::Path, args: DriftArgs) -> Vec<u8> {
    let _g = CWD_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let orig = std::env::current_dir().unwrap();
    std::env::set_current_dir(repo).unwrap();
    let mut stdout: Vec<u8> = Vec::new();
    let mut stderr: Vec<u8> = Vec::new();
    let res = mokumokuren::commands::drift::run(&args, &mut stdout, &mut stderr);
    std::env::set_current_dir(orig).unwrap();
    res.expect("drift should succeed on fixture");
    stdout
}

fn finding_keys() -> Vec<&'static str> {
    vec!["layer", "severity", "message"]
}

#[test]
fn schema_review_shape_with_changes() {
    // Build the coupling fixture, then dirty the working tree so
    // review takes the with-changes (full envelope) code path.
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    build_coupling_fixture(dir.path(), now);
    write(dir.path(), "core/a.rs", "a1\na2\na3\na4\na5\nNEW\n");

    let args = ReviewArgs {
        staged: false,
        range: None,
        commit: None,
        since: "60days".into(),
        top: 20,
        format: Format::Json,
        ignores: Vec::new(),
        config: None,
        verbose: false,
        coupling_threshold: None,
        blast_radius_threshold: None,
        gate: mokumokuren::args::Gate::None,
    };
    let stdout = run_review_in(dir.path(), args);
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");

    expect_required_keys(
        &v,
        &[
            "schema_version",
            "crate_version",
            "repo",
            "config",
            "analysis",
            "review",
            "findings",
        ],
        "top-level review (with-changes)",
    );
    expect_required_keys(&v["review"], &["mode", "diff"], "review");
    expect_required_keys(
        &v["review"]["diff"],
        &[
            "files_changed",
            "lines_added",
            "lines_deleted",
            "files",
            "new_file_fraction",
        ],
        "review.diff",
    );
    let files = v["review"]["diff"]["files"]
        .as_array()
        .expect("review.diff.files array");
    for (i, f) in files.iter().enumerate() {
        expect_required_keys(
            f,
            &["path", "added", "deleted"],
            &format!("review.diff.files[{i}]"),
        );
    }
    let findings = v["findings"].as_array().expect("findings array");
    for (i, f) in findings.iter().enumerate() {
        expect_required_keys(f, &finding_keys(), &format!("findings[{i}]"));
    }
}

#[test]
fn schema_review_clean_tree_minimal_envelope() {
    // Clean working tree → review skips analyze and emits the
    // minimal envelope. The contract: schema_version, crate_version,
    // review (mode + empty diff), and an empty findings array.
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    build_coupling_fixture(dir.path(), now);

    let args = ReviewArgs {
        staged: false,
        range: None,
        commit: None,
        since: "60days".into(),
        top: 20,
        format: Format::Json,
        ignores: Vec::new(),
        config: None,
        verbose: false,
        coupling_threshold: None,
        blast_radius_threshold: None,
        gate: mokumokuren::args::Gate::None,
    };
    let stdout = run_review_in(dir.path(), args);
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");

    expect_required_keys(
        &v,
        &["schema_version", "crate_version", "review", "findings"],
        "top-level review (clean tree)",
    );
    assert!(
        v["findings"].as_array().unwrap().is_empty(),
        "clean tree must emit empty findings"
    );
}

#[test]
fn schema_pre_edit_shape() {
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    build_coupling_fixture(dir.path(), now);

    let args = PreEditArgs {
        path: PathBuf::from("core/a.rs"),
        since: "60days".into(),
        top: 20,
        format: Format::Json,
        ignores: Vec::new(),
        config: None,
        verbose: false,
        coupling_threshold: None,
        blast_radius_threshold: None,
        drift_sessions: 0,
        gate: mokumokuren::args::Gate::None,
    };
    let stdout = run_pre_edit_in(dir.path(), args);
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");

    expect_required_keys(
        &v,
        &[
            "schema_version",
            "crate_version",
            "repo",
            "config",
            "analysis",
            "pre_edit",
            "findings",
        ],
        "top-level pre-edit",
    );
    expect_required_keys(&v["pre_edit"], &["path"], "pre_edit");
    let findings = v["findings"].as_array().expect("findings array");
    for (i, f) in findings.iter().enumerate() {
        expect_required_keys(f, &finding_keys(), &format!("findings[{i}]"));
    }
}

#[test]
fn schema_drift_shape() {
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    build_session_fixture(dir.path(), now);

    let args = DriftArgs {
        sessions: 3,
        base: Some("HEAD".into()),
        since: "180days".into(),
        top: 5,
        format: Format::Json,
        ignores: Vec::new(),
        config: None,
        verbose: false,
    };
    let stdout = run_drift_in(dir.path(), args);
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");

    expect_required_keys(
        &v,
        &[
            "schema_version",
            "crate_version",
            "drift",
            "findings",
            "duration_ms",
        ],
        "top-level drift",
    );
    expect_required_keys(
        &v["drift"],
        &["base", "sessions", "snapshot_labels"],
        "drift",
    );
    let findings = v["findings"].as_array().expect("findings array");
    for (i, f) in findings.iter().enumerate() {
        expect_required_keys(
            f,
            &[
                "layer",
                "severity",
                "path",
                "climb_transitions",
                "total_transitions",
                "latest_rank",
            ],
            &format!("drift.findings[{i}]"),
        );
    }
}
