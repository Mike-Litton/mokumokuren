//! `mmk review` — the v0.3 headline. Compares a diff (working-tree
//! by default, or `--staged`/`--range`/`--commit`) against the
//! historical baseline and emits layer-labeled findings.
//!
//! Orthogonality tag: protects **agent mode** (the
//! `PostToolUse:Edit` hook reads JSON findings) and **human mode**
//! (line-by-line text the reviewer scans before commit).

mod common;

use common::{build_coupling_fixture, commit_all, init_repo, write, CWD_LOCK, DAY};
use mokumokuren::args::{Format, ReviewArgs};
use serde_json::Value;
use tempfile::TempDir;

fn review_args() -> ReviewArgs {
    ReviewArgs {
        staged: false,
        range: None,
        commit: None,
        since: "60days".into(),
        top: 20,
        format: Format::Text,
        ignores: Vec::new(),
        config: None,
        verbose: false,
        blast_radius_threshold: None,
    }
}

fn run_in(repo: &std::path::Path, args: ReviewArgs) -> (Vec<u8>, Vec<u8>) {
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
    (stdout, stderr)
}

#[test]
fn review_silent_on_clean_working_tree() {
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    build_coupling_fixture(dir.path(), now);

    let (stdout, _) = run_in(dir.path(), review_args());
    let text = String::from_utf8(stdout).unwrap();
    assert!(
        text.is_empty(),
        "clean working tree must produce no text output (no findings); got: {text}"
    );

    let mut json_args = review_args();
    json_args.format = Format::Json;
    let (stdout, _) = run_in(dir.path(), json_args);
    let v: Value = serde_json::from_slice(&stdout).expect("clean tree still emits valid JSON");
    let findings = v["findings"].as_array().expect("findings array present");
    assert!(
        findings.is_empty(),
        "clean working tree must emit empty findings array; got: {findings:?}"
    );
}

#[test]
fn review_emits_hotspot_for_changed_top_n_file() {
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    build_coupling_fixture(dir.path(), now);

    // Edit core/a.rs (the canonical hotspot in the coupling fixture)
    // without committing — the working tree now has one changed file
    // that ranks #1.
    write(dir.path(), "core/a.rs", "a1\na2\na3\na4\na5\nNEW\n");

    let mut args = review_args();
    args.format = Format::Json;
    let (stdout, _) = run_in(dir.path(), args);
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");

    let findings = v["findings"].as_array().expect("findings array");
    let hotspot: Vec<&Value> = findings
        .iter()
        .filter(|f| f["layer"] == "hotspot")
        .collect();
    assert!(
        !hotspot.is_empty(),
        "editing a top-N file must emit at least one HOTSPOT finding; got findings: {findings:?}"
    );
    let mentions_a = hotspot
        .iter()
        .any(|f| f["message"].as_str().unwrap_or("").contains("core/a.rs"));
    assert!(
        mentions_a,
        "HOTSPOT finding must mention core/a.rs; got: {hotspot:?}"
    );
}

#[test]
fn review_emits_coupling_miss_on_uncommitted_diff() {
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    build_coupling_fixture(dir.path(), now);

    // Edit core/a.rs but leave its historical partner core/b.rs
    // untouched. jaccard(a,b) = 0.75 in this fixture; the COUPLING
    // finding fires for the missed partner.
    write(dir.path(), "core/a.rs", "a1\na2\na3\na4\na5\nNEW\n");

    let mut args = review_args();
    args.format = Format::Json;
    let (stdout, _) = run_in(dir.path(), args);
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");

    let findings = v["findings"].as_array().expect("findings array");
    let coupling: Vec<&Value> = findings
        .iter()
        .filter(|f| f["layer"] == "coupling")
        .collect();
    assert!(
        !coupling.is_empty(),
        "editing core/a.rs without core/b.rs must emit a COUPLING miss; got: {findings:?}"
    );
    let mentions_b = coupling
        .iter()
        .any(|f| f["message"].as_str().unwrap_or("").contains("core/b.rs"));
    assert!(
        mentions_b,
        "COUPLING finding must mention the missed partner core/b.rs; got: {coupling:?}"
    );
}

#[test]
fn review_silent_when_partner_also_touched() {
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    build_coupling_fixture(dir.path(), now);

    // Both partners touched — no COUPLING miss.
    write(dir.path(), "core/a.rs", "a1\na2\na3\na4\na5\nNEW\n");
    write(dir.path(), "core/b.rs", "b1\nb2\nb3\nNEW\n");

    let mut args = review_args();
    args.format = Format::Json;
    let (stdout, _) = run_in(dir.path(), args);
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");

    let findings = v["findings"].as_array().expect("findings array");
    let coupling_misses: Vec<&Value> = findings
        .iter()
        .filter(|f| f["layer"] == "coupling")
        .collect();
    assert!(
        coupling_misses.is_empty(),
        "touching both partners must NOT emit a COUPLING miss; got: {coupling_misses:?}"
    );
}

#[test]
fn review_staged_only_reads_index() {
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    build_coupling_fixture(dir.path(), now);

    // Stage core/a.rs (will be seen by --staged), modify core/b.rs
    // unstaged (will NOT be seen by --staged).
    write(dir.path(), "core/a.rs", "a1\na2\na3\na4\na5\nNEW\n");
    common::git(dir.path(), &["add", "core/a.rs"]);
    write(dir.path(), "core/b.rs", "b1\nb2\nb3\nNEW\n");

    let mut args = review_args();
    args.staged = true;
    args.format = Format::Json;
    let (stdout, _) = run_in(dir.path(), args);
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");

    let changed_paths: Vec<&str> = v["review"]["diff"]["files"]
        .as_array()
        .expect("review.diff.files array")
        .iter()
        .map(|f| f["path"].as_str().unwrap())
        .collect();
    assert_eq!(
        changed_paths,
        vec!["core/a.rs"],
        "--staged should only see staged files; got {changed_paths:?}"
    );
}

#[test]
fn review_range_uses_committed_diff() {
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    build_coupling_fixture(dir.path(), now);

    // Add a known committed change on top of the fixture so we can
    // diff a parent-child range.
    write(dir.path(), "core/a.rs", "a1\na2\na3\na4\na5\na6\n");
    commit_all(dir.path(), "extra: bump a.rs", now);
    let head = String::from_utf8(
        std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(dir.path())
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap()
    .trim()
    .to_string();
    let parent = String::from_utf8(
        std::process::Command::new("git")
            .args(["rev-parse", "HEAD^"])
            .current_dir(dir.path())
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap()
    .trim()
    .to_string();

    let mut args = review_args();
    args.range = Some(format!("{parent}..{head}"));
    args.format = Format::Json;
    let (range_out, _) = run_in(dir.path(), args);

    let mut args = review_args();
    args.commit = Some(head);
    args.format = Format::Json;
    let (commit_out, _) = run_in(dir.path(), args);

    let r: Value = serde_json::from_slice(&range_out).unwrap();
    let c: Value = serde_json::from_slice(&commit_out).unwrap();
    assert_eq!(
        r["review"]["diff"]["files"], c["review"]["diff"]["files"],
        "--range parent..head and --commit head must produce identical diff.files"
    );
}

#[test]
fn review_emits_budget_when_diff_exceeds() {
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    init_repo(dir.path());
    write(dir.path(), "seed.rs", "x\n");
    commit_all(dir.path(), "seed", now - DAY);

    // Working-tree blast: 1 file, many lines added — exceeds default
    // bulk.max_lines (1000 in mmk-config).
    let mut huge = String::with_capacity(6000 * 8);
    for i in 0..6000 {
        use std::fmt::Write as _;
        writeln!(huge, "line{i}").unwrap();
    }
    write(dir.path(), "seed.rs", &huge);

    let mut args = review_args();
    args.format = Format::Json;
    let (stdout, _) = run_in(dir.path(), args);
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");

    let any_budget = v["findings"]
        .as_array()
        .expect("findings array")
        .iter()
        .any(|f| f["layer"] == "budget");
    assert!(
        any_budget,
        "a 6000-line edit must trip the BUDGET finding; got findings: {:?}",
        v["findings"]
    );
}
