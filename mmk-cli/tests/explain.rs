//! `mmk explain --finding <id>` — addressable per-commit evidence
//! behind a borderline COUPLING claim.
//!
//! Oracle: a fixture where all 8 co-changes between `core/a.rs` and
//! `core/b.rs` cluster within a single 14-day window 8 months before
//! HEAD. The K-of-N summary in the COUPLING message can't tell that
//! story; `explain` must — or the agent has no way to verify the
//! claim before acting.

mod common;

use common::{commit_all, init_repo, with_cwd, write, DAY};
use mokumokuren::args::{ExplainArgs, Format};
use serde_json::Value;
use serial_test::serial;
use tempfile::TempDir;

fn explain_args(id: &str) -> ExplainArgs {
    ExplainArgs {
        finding: id.to_string(),
        since: "365days".into(),
        format: Format::Json,
        ignores: Vec::new(),
        config: None,
        verbose: false,
    }
}

fn run_explain(
    repo: &std::path::Path,
    args: ExplainArgs,
) -> (anyhow::Result<()>, Vec<u8>, Vec<u8>) {
    with_cwd(repo, |so, se| {
        mokumokuren::commands::explain::run(&args, so, se)
    })
}

/// Build a repo where the K-of-N summary destroys temporal information.
///
/// 12 commits total. 8 co-changes between `core/a.rs` and `core/b.rs`
/// land inside a single 13-day window 8 months before HEAD; the
/// remaining 4 commits touch `core/a.rs` alone, spread across the
/// 12-month history so HEAD sees a long-running file with a single
/// burst of paired activity in the middle.
fn build_clustered_coupling_fixture(repo: &std::path::Path, now: i64) {
    init_repo(repo);

    // 4 a-only commits spread across the 12-month history. These set
    // commits_touching(a) = 12 once the burst lands, with 4 of them
    // *not* touching b — the partner-only count `explain` surfaces.
    let a_only_offsets: [i64; 4] = [11 * 30, 9 * 30, 5 * 30, 2 * 30];
    for (i, off) in a_only_offsets.iter().enumerate() {
        let body = build_lines("a", i + 1);
        write(repo, "core/a.rs", &body);
        commit_all(repo, &format!("a-only {i}"), now - off * DAY);
    }

    // 8 co-changes inside a 13-day burst centered 8 months ago. Days
    // 247..234 before HEAD — span 13 days, comfortably <= the
    // 14-day assertion in the oracle.
    let burst_anchor = 240 * DAY;
    for i in 0..8u32 {
        let body_a = build_lines("a", (i + 5) as usize);
        let body_b = build_lines("b", (i + 1) as usize);
        write(repo, "core/a.rs", &body_a);
        write(repo, "core/b.rs", &body_b);
        let off = burst_anchor - i64::from(i) * DAY;
        commit_all(repo, &format!("burst {i}"), now - off);
    }
}

/// Build a deterministic `count`-line body of the form
/// `<prefix>0\n<prefix>1\n…<prefix>{count-1}\n`. Inlined helper so the
/// test assembles bodies through the std `String::push_str` path
/// clippy prefers over `format!(..).collect()`.
fn build_lines(prefix: &str, count: usize) -> String {
    let mut s = String::new();
    for n in 0..count {
        s.push_str(prefix);
        s.push_str(&n.to_string());
        s.push('\n');
    }
    s
}

#[serial(cwd)]
#[test]
fn explain_surfaces_clustered_temporal_concentration() {
    // The fact `explain` adds: the 8 co-changes are concentrated in
    // a narrow window, not spread across the year. The K-of-N
    // summary in the COUPLING message cannot tell that story —
    // verifying it requires the per-commit evidence this subcommand
    // returns.
    let dir = TempDir::new().unwrap();
    let now: i64 = 1_700_000_000;
    build_clustered_coupling_fixture(dir.path(), now);

    let (res, stdout, stderr) =
        run_explain(dir.path(), explain_args("coupling:core/a.rs:core/b.rs"));
    res.unwrap_or_else(|e| {
        panic!(
            "explain failed: {e:#}\nstderr: {}",
            String::from_utf8_lossy(&stderr)
        )
    });

    let v: Value = serde_json::from_slice(&stdout).expect("explain emits valid JSON");
    assert_eq!(
        v["finding"], "coupling:core/a.rs:core/b.rs",
        "echoes the requested fingerprint"
    );
    assert_eq!(v["co_change_count"], 8, "8 commits touched both files");
    assert_eq!(
        v["commits_touching_either"], 12,
        "8 burst + 4 a-only = 12 commits touched at least one of the pair"
    );
    let span = v["co_change_span_days"]
        .as_u64()
        .expect("co_change_span_days is an integer");
    assert!(
        span <= 14,
        "the 8 co-changes must surface as concentrated in <=14 days; got {span}"
    );

    let evidence = v["evidence"].as_array().expect("evidence is an array");
    assert_eq!(
        evidence.len(),
        8,
        "evidence should list exactly the 8 co-change commits, not the a-only ones; got {}",
        evidence.len()
    );

    // Newest-first ordering: the agent reads the most recent burst
    // commits first when scrolling.
    let timestamps: Vec<i64> = evidence
        .iter()
        .map(|e| e["ts"].as_i64().expect("ts is a number"))
        .collect();
    let mut sorted = timestamps.clone();
    sorted.sort_by(|a, b| b.cmp(a));
    assert_eq!(timestamps, sorted, "evidence must be sorted newest-first");

    // Each evidence row carries the deltas for both pair members.
    for (i, entry) in evidence.iter().enumerate() {
        let deltas = entry["deltas"]
            .as_array()
            .unwrap_or_else(|| panic!("evidence[{i}].deltas missing"));
        let paths: Vec<&str> = deltas.iter().filter_map(|d| d["path"].as_str()).collect();
        assert!(
            paths.contains(&"core/a.rs") && paths.contains(&"core/b.rs"),
            "evidence[{i}] should carry both pair members; got paths {paths:?}"
        );
    }
}

#[serial(cwd)]
#[test]
fn explain_unknown_layer_errors_with_clear_message() {
    // Future layers (drift, hotspot) will land in v0.12+; today the
    // fingerprint must reject anything other than `coupling:` so the
    // agent doesn't get back silent empty evidence.
    let dir = TempDir::new().unwrap();
    let now: i64 = 1_700_000_000;
    build_clustered_coupling_fixture(dir.path(), now);

    let (res, _stdout, _stderr) = run_explain(dir.path(), explain_args("hotspot:core/a.rs"));
    let err = res.expect_err("unknown layer must error, not return empty evidence");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("unknown finding layer"),
        "error must explain the layer rejection; got: {msg}"
    );
}

#[serial(cwd)]
#[test]
fn explain_returns_empty_evidence_when_pair_absent_from_window() {
    // The finding could have come from a deeper window than the
    // current `--since` slice. Honest "no commits in this window
    // touched the pair" beats erroring — the agent then knows to
    // widen the window or accept that the evidence has aged out.
    let dir = TempDir::new().unwrap();
    let now: i64 = 1_700_000_000;
    build_clustered_coupling_fixture(dir.path(), now);

    let mut args = explain_args("coupling:nonexistent/x.rs:nonexistent/y.rs");
    args.since = "30days".into();
    let (res, stdout, stderr) = run_explain(dir.path(), args);
    res.unwrap_or_else(|e| {
        panic!(
            "absent-pair must succeed with empty evidence, not error: {e:#}\nstderr: {}",
            String::from_utf8_lossy(&stderr)
        )
    });

    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");
    assert_eq!(v["co_change_count"], 0);
    assert_eq!(v["commits_touching_either"], 0);
    let evidence = v["evidence"].as_array().expect("evidence array");
    assert!(
        evidence.is_empty(),
        "absent pair must yield empty evidence; got {} entries",
        evidence.len()
    );
}

#[test]
fn explain_missing_finding_arg_produces_clap_error() {
    // Sanity that clap enforces the required `--finding` flag —
    // without it the binary mustn't try to parse an empty fingerprint.
    use clap::Parser;
    let result = mokumokuren::args::Cli::try_parse_from(["mmk", "explain"]);
    let err = result.expect_err("missing --finding must trip clap");
    let msg = err.to_string();
    assert!(
        msg.contains("--finding") || msg.contains("required"),
        "clap error should name the missing argument; got: {msg}"
    );
}
