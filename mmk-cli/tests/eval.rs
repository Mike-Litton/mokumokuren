//! `mmk eval` — sample N recent commits, run review against each,
//! aggregate a noise-floor report. Adoption tool.

mod common;

use common::{build_coupling_fixture, CWD_LOCK};
use mokumokuren::args::{EvalArgs, Format};
use serde_json::Value;
use tempfile::TempDir;

fn eval_args() -> EvalArgs {
    EvalArgs {
        sample: 50,
        since: "60days".into(),
        top: 20,
        format: Format::Json,
        config: None,
        verbose: false,
    }
}

fn run_in(repo: &std::path::Path, args: EvalArgs) -> Vec<u8> {
    let _g = CWD_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let orig = std::env::current_dir().unwrap();
    std::env::set_current_dir(repo).unwrap();
    let mut stdout: Vec<u8> = Vec::new();
    let mut stderr: Vec<u8> = Vec::new();
    let res = mokumokuren::commands::eval::run(&args, &mut stdout, &mut stderr);
    std::env::set_current_dir(orig).unwrap();
    res.expect("eval should succeed on fixture");
    stdout
}

#[test]
fn eval_aggregates_findings_across_sampled_commits() {
    // The coupling fixture has 5 commits. Eval samples them and runs
    // review against each. The fixture's a+b co-changes mean every
    // commit that touches a or b without the other should fire
    // COUPLING — but the third commit "2: a only" is the one historic
    // miss inside the window. Either way: aggregation must produce a
    // valid report with positive sample count.
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    build_coupling_fixture(dir.path(), now);

    let stdout = run_in(dir.path(), eval_args());
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON report");

    assert!(
        v["commits_sampled"].as_u64().unwrap_or(0) > 0,
        "must have sampled at least one commit; got: {v}"
    );
    assert!(
        v["threshold"].is_number(),
        "report must echo effective coupling threshold; got: {v}"
    );
    assert!(v["by_layer"].is_object());
    assert!(v["jaccard_buckets"].is_object());
}

#[test]
fn eval_text_mode_emits_firing_rate_line() {
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    build_coupling_fixture(dir.path(), now);

    let mut args = eval_args();
    args.format = Format::Text;
    let stdout = run_in(dir.path(), args);
    let text = String::from_utf8(stdout).unwrap();
    assert!(
        text.contains("firing rate:"),
        "text report must include firing rate line: {text}"
    );
    assert!(
        text.contains("layer mix:"),
        "text report must include layer mix line: {text}"
    );
}
