mod common;

use common::build_canonical_fixture;
use mokumokuren::args::{AnalyzeArgs, Format};
use serde_json::Value;
use serial_test::serial;
use tempfile::TempDir;

fn run_in(repo: &std::path::Path, args: AnalyzeArgs) -> Vec<u8> {
    let (res, stdout, _) = common::with_cwd(repo, |so, se| {
        mokumokuren::commands::analyze::run(&args, so, se)
    });
    res.expect("analyze should succeed on fixture");
    stdout
}

fn default_args() -> AnalyzeArgs {
    AnalyzeArgs {
        since: "30days".into(),
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
fn json_output_on_canonical_fixture() {
    let dir = TempDir::new().unwrap();
    let now: i64 = 1_700_000_000;
    build_canonical_fixture(dir.path(), now);

    let stdout = run_in(dir.path(), default_args());
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");
    assert_eq!(v["analysis"]["commits_seen"], 4);
    assert_eq!(v["analysis"]["commits_analyzed"], 4);

    let files = v["files"].as_array().expect("files array");
    assert!(!files.is_empty(), "expected at least one hotspot");
    let top = &files[0];
    assert_eq!(top["path"], "a.rs");
    assert_eq!(top["hotspot_rank"], 1);
    assert!(top["weighted_churn"].as_f64().unwrap() > 0.0);
}

#[serial(cwd)]
#[test]
fn text_output_on_canonical_fixture() {
    let dir = TempDir::new().unwrap();
    let now: i64 = 1_700_000_000;
    build_canonical_fixture(dir.path(), now);

    let mut args = default_args();
    args.format = Format::Text;
    let stdout = run_in(dir.path(), args);
    let text = String::from_utf8(stdout).unwrap();
    assert!(text.contains("a.rs"), "text output should mention a.rs");
    assert!(
        text.contains("rank") && text.contains("hotspot"),
        "text output should have header row"
    );
}
