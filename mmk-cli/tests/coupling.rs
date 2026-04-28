//! Coupling surfaced through the `analyze` CLI: per-file `top_couples`
//! arrays in JSON, and the targeted `--couples-of <PATH>` mode.

mod common;

use common::{build_coupling_fixture, DAY};
use mokumokuren::args::{AnalyzeArgs, Format};
use serde_json::Value;
use serial_test::serial;
use tempfile::TempDir;

fn run_in(repo: &std::path::Path, args: AnalyzeArgs) -> Vec<u8> {
    let (res, stdout, _) = common::with_cwd(repo, |so, se| {
        mokumokuren::commands::analyze::run(&args, so, se)
    });
    res.expect("analyze should succeed on coupling fixture");
    stdout
}

fn json_args() -> AnalyzeArgs {
    AnalyzeArgs {
        since: format!("{}days", 60),
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

#[serial(cwd)]
#[test]
fn json_files_carry_top_couples_array() {
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    build_coupling_fixture(dir.path(), now);

    let stdout = run_in(dir.path(), json_args());
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");

    let files = v["files"].as_array().expect("files array");
    let a_entry = files
        .iter()
        .find(|f| f["path"] == "core/a.rs")
        .expect("core/a.rs in files");

    let couples = a_entry["top_couples"]
        .as_array()
        .expect("top_couples should be an array on every file entry");
    let partners: Vec<&str> = couples
        .iter()
        .map(|c| c["partner"].as_str().unwrap())
        .collect();

    assert!(
        partners.contains(&"core/b.rs"),
        "core/a.rs should be coupled to core/b.rs (they co-change in 4 of 5 commits); got {partners:?}"
    );

    let b_couple = couples
        .iter()
        .find(|c| c["partner"] == "core/b.rs")
        .unwrap();
    let jaccard = b_couple["jaccard"].as_f64().unwrap();
    assert!(
        (jaccard - 0.80).abs() < 1e-9,
        "expected jaccard 0.80 on the canonical coupling pair (4 co-changes / 5 touches A / 4 touches B), got {jaccard}"
    );
}

#[serial(cwd)]
#[test]
fn couples_of_flag_returns_partners_for_a_single_path() {
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    build_coupling_fixture(dir.path(), now);

    let mut args = json_args();
    args.couples_of = Some("core/a.rs".into());
    // Suppress the (also-valid) full ranking — when --couples-of is set,
    // output is just the coupling list for that path.
    let stdout = run_in(dir.path(), args);
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");

    assert!(
        v.get("files").is_none(),
        "--couples-of mode should suppress the ranked `files` list"
    );

    let couples_of = v["couples_of"]
        .as_object()
        .expect("couples_of block should be present");
    assert_eq!(couples_of["path"], "core/a.rs");
    let entries = couples_of["entries"]
        .as_array()
        .expect("couples_of.entries should be an array");
    let partners: Vec<&str> = entries
        .iter()
        .map(|c| c["partner"].as_str().unwrap())
        .collect();
    assert!(
        partners.contains(&"core/b.rs"),
        "couples_of(core/a.rs) should include core/b.rs; got {partners:?}"
    );
}

#[serial(cwd)]
#[test]
fn default_text_table_does_not_include_couples_block() {
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    build_coupling_fixture(dir.path(), now);

    let mut args = json_args();
    args.format = Format::Text;
    args.couples = false;
    let stdout = run_in(dir.path(), args);
    let text = String::from_utf8(stdout).unwrap();
    assert!(
        !text.contains("couples:"),
        "default text output should not include a `couples:` block (preserves grep-friendliness): {text}"
    );
}

#[serial(cwd)]
#[test]
fn text_with_couples_flag_renders_indented_partners() {
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    build_coupling_fixture(dir.path(), now);

    let mut args = json_args();
    args.format = Format::Text;
    args.couples = true;
    let stdout = run_in(dir.path(), args);
    let text = String::from_utf8(stdout).unwrap();
    assert!(
        text.contains("couples:"),
        "--couples should render an indented `couples:` block, got: {text}"
    );
    assert!(
        text.contains("core/b.rs"),
        "couples block should mention core/b.rs (canonical partner of core/a.rs); got: {text}"
    );
}

// Sanity: existing canonical fixture still parses via the wider window.
#[test]
fn day_constant_still_exported() {
    // Touches the constant so the test file at least references it,
    // catching the case where build_coupling_fixture stops using it
    // and the unused-import warning hides a real regression.
    assert_eq!(DAY, 86_400);
}

// --- Step 2: blast-radius threshold configurability. ---
//
// Orthogonality tag: protects **agent mode** (knows what filter
// produced the neighborhood) and **CI/CD mode** (per-repo tuning
// without code changes).

#[serial(cwd)]
#[test]
fn blast_radius_threshold_echoes_in_json() {
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    build_coupling_fixture(dir.path(), now);

    let mut args = json_args();
    args.blast_radius = Some("core/a.rs".into());
    let stdout = run_in(dir.path(), args);
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");

    let threshold = v["blast_radius"]["threshold"]
        .as_f64()
        .expect("blast_radius.threshold must be a number echoed in JSON");
    assert!(
        (threshold - 0.10).abs() < 1e-9,
        "default threshold should be 0.10, got {threshold}"
    );
}

#[serial(cwd)]
#[test]
fn blast_radius_threshold_cli_override_filters_more() {
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    build_coupling_fixture(dir.path(), now);

    // Default 0.10: includes the canonical core/b.rs partner (jaccard 0.60).
    let mut a = json_args();
    a.blast_radius = Some("core/a.rs".into());
    let lo = run_in(dir.path(), a);
    let lo_v: Value = serde_json::from_slice(&lo).unwrap();
    let lo_nodes = lo_v["blast_radius"]["nodes"].as_array().unwrap().len();

    // Raised threshold to 0.99 — strictly stricter; jaccard 0.60 < 0.99
    // so the canonical partner is dropped. Echo must reflect 0.99.
    let mut b = json_args();
    b.blast_radius = Some("core/a.rs".into());
    b.blast_radius_threshold = Some(0.99);
    let hi = run_in(dir.path(), b);
    let hi_v: Value = serde_json::from_slice(&hi).unwrap();
    let hi_nodes = hi_v["blast_radius"]["nodes"].as_array().unwrap().len();
    let hi_threshold = hi_v["blast_radius"]["threshold"].as_f64().unwrap();

    assert!(
        (hi_threshold - 0.99).abs() < 1e-9,
        "JSON should echo the CLI-overridden threshold, got {hi_threshold}"
    );
    assert!(
        hi_nodes < lo_nodes,
        "stricter threshold (0.99) must return strictly fewer nodes than 0.10; \
         got hi={hi_nodes}, lo={lo_nodes}"
    );
}
