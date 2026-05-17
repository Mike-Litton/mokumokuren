//! Coupling surfaced through the `analyze` CLI: per-file `top_couples`
//! arrays in JSON. (v0.13 dropped the `--couples-of` / `--couples`
//! flags — JSON `top_couples[]` carries the same data and the text
//! `couples:` block was pure human-convenience sugar that broke
//! grep.)

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
fn default_text_table_stays_grep_friendly() {
    // v0.13 dropped the `--couples` indented render; the text table
    // must never carry a `couples:` block now. Locks the choice so a
    // future regression doesn't bring it back.
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    build_coupling_fixture(dir.path(), now);

    let mut args = json_args();
    args.format = Format::Text;
    let stdout = run_in(dir.path(), args);
    let text = String::from_utf8(stdout).unwrap();
    assert!(
        !text.contains("couples:"),
        "text output must stay grep-friendly (no `couples:` block): {text}"
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
