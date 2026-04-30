//! `mmk review` — compare a diff (working-tree by default, or
//! `--staged`/`--range`/`--commit`) against the historical baseline
//! and emit layer-labeled findings.
//!
//! Orthogonality tag: protects **agent mode** (the
//! `PostToolUse:Edit` hook reads JSON findings) and **human mode**
//! (line-by-line text the reviewer scans before commit).

mod common;

use common::{build_coupling_fixture, commit_all, init_repo, write, DAY};
use mokumokuren::args::{Format, Gate, ReviewArgs};
use serde_json::Value;
use serial_test::serial;
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
        coupling_threshold: None,
        blast_radius_threshold: None,
        gate: Gate::None,
        // Tests in this file run pairs of invocations against the
        // same fixture; without dedup off they'd silently shadow
        // each other. The dedup integration tests live in
        // mmk-cli/tests/dedup.rs.
        no_dedup: true,
    }
}

fn run_in(repo: &std::path::Path, args: ReviewArgs) -> (Vec<u8>, Vec<u8>) {
    let (res, stdout, stderr) = common::with_cwd(repo, |so, se| {
        mokumokuren::commands::review::run(&args, None, so, se)
    });
    res.expect("review should succeed on fixture");
    (stdout, stderr)
}

fn run_in_with_verdict(
    repo: &std::path::Path,
    args: ReviewArgs,
) -> (Vec<u8>, mokumokuren::Verdict) {
    let (res, stdout, _) = common::with_cwd(repo, |so, se| {
        mokumokuren::commands::review::run(&args, None, so, se)
    });
    let verdict = res.expect("review");
    (stdout, verdict)
}

#[serial(cwd)]
#[test]
fn review_emits_clean_state_line_on_clean_working_tree() {
    // v0.9: text mode prints exactly one canonical line on a clean
    // tree (was silent in v0.8). The 7-char HEAD sha disambiguates
    // *which* baseline the all-clear was computed against.
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    build_coupling_fixture(dir.path(), now);

    let (stdout, _) = run_in(dir.path(), review_args());
    let text = String::from_utf8(stdout).unwrap();
    let trimmed = text.trim_end_matches('\n');
    assert_eq!(
        trimmed.lines().count(),
        1,
        "clean tree must produce exactly one stdout line; got: {text:?}"
    );
    assert!(
        trimmed.starts_with("[no actionable signal] no findings (HEAD "),
        "clean tree text must carry canonical prefix + HEAD sha; got: {text:?}"
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

#[serial(cwd)]
#[test]
fn review_emits_clean_state_line_with_diff_size_when_no_findings() {
    // Cohort feedback (3 agents across 2 runs): a real diff that
    // cleared every sensor was visually identical to "mmk silently
    // failed" — text mode rendered nothing, the agent re-ran the
    // command. The diff-bearing clean-state line names file count,
    // total LOC churn, and HEAD baseline so the silence is read as
    // a positive verdict, not a missing-data state.
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    build_coupling_fixture(dir.path(), now);

    // core/c.rs has 1 prior commit (sidecar in the fixture), no
    // COUPLING partner, and falls outside top-1. With `top = 1` only
    // core/a.rs is the hotspot — editing core/c.rs produces zero
    // findings while still being a non-empty diff.
    write(dir.path(), "core/c.rs", "c1\nc2\n");

    let mut args = review_args();
    args.top = 1;
    let (stdout, _) = run_in(dir.path(), args);
    let text = String::from_utf8(stdout).unwrap();
    let trimmed = text.trim_end_matches('\n');
    assert_eq!(
        trimmed.lines().count(),
        1,
        "diff-with-no-findings must produce exactly one stdout line; got: {text:?}"
    );
    assert!(
        trimmed.starts_with("[no actionable signal] no findings ("),
        "expected canonical prefix; got: {text:?}"
    );
    assert!(
        trimmed.contains("1 file, +"),
        "expected diff size to surface (file count + LOC); got: {text:?}"
    );
    assert!(
        trimmed.contains("vs HEAD "),
        "expected `vs HEAD <sha>` clause; got: {text:?}"
    );
}

#[serial(cwd)]
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

#[serial(cwd)]
#[test]
fn review_emits_coupling_miss_on_uncommitted_diff() {
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    build_coupling_fixture(dir.path(), now);

    // Edit core/a.rs but leave its historical partner core/b.rs
    // untouched. The fixture lands P(B|A) = 0.80 with Wilson 95 %
    // lower ≈ 0.38 — above the default 0.30 confidence floor — so
    // the COUPLING finding fires for the missed partner.
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

#[serial(cwd)]
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

#[serial(cwd)]
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

#[serial(cwd)]
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

#[serial(cwd)]
#[test]
fn review_self_throttles_on_bulk_diff() {
    // When the input diff itself trips bulk thresholds, review
    // emits exactly one BUDGET finding and skips HOTSPOT/COUPLING.
    // The alternative — running coupling against a vendored snapshot
    // — produces a finding storm on bulk imports.
    use std::fmt::Write as _;
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    build_coupling_fixture(dir.path(), now);

    // Edit core/a.rs (which has a known partner core/b.rs) AND drop a
    // huge unrelated commit-sized blob. If self-filter works, the
    // would-be COUPLING finding for core/b.rs is suppressed because
    // the diff itself blew past bulk thresholds.
    let mut huge = String::with_capacity(6000 * 8);
    for i in 0..6000 {
        writeln!(huge, "line{i}").unwrap();
    }
    write(dir.path(), "core/a.rs", &huge);

    let mut args = review_args();
    args.format = Format::Json;
    let (stdout, _) = run_in(dir.path(), args);
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");

    let findings = v["findings"].as_array().expect("findings array");
    let layers: Vec<&str> = findings
        .iter()
        .map(|f| f["layer"].as_str().unwrap_or(""))
        .collect();
    assert!(
        layers.iter().all(|l| *l == "budget"),
        "bulk-self-filter must suppress non-budget findings; got layers: {layers:?}"
    );
    assert!(
        !layers.is_empty(),
        "must still emit at least one BUDGET finding; got: {findings:?}"
    );
}

#[serial(cwd)]
#[test]
fn review_respects_coupling_threshold() {
    // --coupling-threshold above the partner's Wilson lower bound
    // (~0.38) must suppress the would-be COUPLING finding for
    // core/b.rs. The flag routes to confidence_threshold.
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    build_coupling_fixture(dir.path(), now);
    write(dir.path(), "core/a.rs", "a1\na2\na3\na4\na5\nNEW\n");

    let mut args = review_args();
    args.format = Format::Json;
    args.coupling_threshold = Some(0.99);
    let (stdout, _) = run_in(dir.path(), args);
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");

    let coupling: Vec<&Value> = v["findings"]
        .as_array()
        .expect("findings array")
        .iter()
        .filter(|f| f["layer"] == "coupling")
        .collect();
    assert!(
        coupling.is_empty(),
        "threshold 0.99 must suppress coupling whose Wilson lower is ≈0.38; got: {coupling:?}"
    );
}

#[serial(cwd)]
#[test]
fn review_respects_coupling_ignore_partners() {
    // A glob in [coupling] ignore_partners must drop the matching
    // partner from COUPLING findings even when it's above threshold.
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    build_coupling_fixture(dir.path(), now);
    write(dir.path(), "core/a.rs", "a1\na2\na3\na4\na5\nNEW\n");

    std::fs::write(
        dir.path().join("mokumokuren.toml"),
        "[coupling]\nthreshold = 0.10\nignore_partners = [\"core/b.rs\"]\n",
    )
    .unwrap();

    let mut args = review_args();
    args.format = Format::Json;
    let (stdout, _) = run_in(dir.path(), args);
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");

    let coupling_b: Vec<&Value> = v["findings"]
        .as_array()
        .expect("findings array")
        .iter()
        .filter(|f| {
            f["layer"] == "coupling" && f["message"].as_str().unwrap_or("").contains("core/b.rs")
        })
        .collect();
    assert!(
        coupling_b.is_empty(),
        "core/b.rs glob in ignore_partners must suppress its coupling finding; got: {coupling_b:?}"
    );
}

#[serial(cwd)]
#[test]
fn review_gate_warn_returns_nonzero_on_warn_finding() {
    // --gate warn must surface a non-zero verdict when any
    // warn-severity finding fires. With the default
    // confidence_threshold (0.30) and min_sample_size (3), editing
    // core/a.rs alone surfaces COUPLING for core/b.rs (Wilson
    // lower ≈ 0.38) — above the floor.
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    build_coupling_fixture(dir.path(), now);
    write(dir.path(), "core/a.rs", "a1\na2\na3\na4\na5\nNEW\n");

    let mut args = review_args();
    args.format = Format::Json;
    args.gate = Gate::Warn;
    let (_, verdict) = run_in_with_verdict(dir.path(), args);
    assert_eq!(verdict, mokumokuren::Verdict::GateTriggered);
}

#[serial(cwd)]
#[test]
fn review_gate_none_returns_ok_even_with_findings() {
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    build_coupling_fixture(dir.path(), now);
    write(dir.path(), "core/a.rs", "a1\na2\na3\na4\na5\nNEW\n");

    let mut args = review_args();
    args.format = Format::Json;
    let (_, verdict) = run_in_with_verdict(dir.path(), args);
    assert_eq!(verdict, mokumokuren::Verdict::Ok);
}

#[serial(cwd)]
#[test]
fn review_legacy_coupling_threshold_emits_deprecation_warning_in_verbose() {
    // Pinning [coupling] threshold = X must surface a one-line
    // deprecation note on stderr when the user passes -v. Locks the
    // back-compat behavior so a future refactor that silently drops
    // the alias gets caught.
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    build_coupling_fixture(dir.path(), now);
    write(dir.path(), "core/a.rs", "a1\na2\na3\na4\na5\nNEW\n");

    std::fs::write(
        dir.path().join("mokumokuren.toml"),
        "[coupling]\nthreshold = 0.30\n",
    )
    .unwrap();

    let mut args = review_args();
    args.format = Format::Json;
    args.verbose = true;

    let (res, _, stderr) = common::with_cwd(dir.path(), |so, se| {
        mokumokuren::commands::review::run(&args, None, so, se).map(|_| ())
    });
    res.expect("review");
    let err = String::from_utf8(stderr).unwrap();
    assert!(
        err.contains("[coupling] threshold is deprecated"),
        "verbose stderr should call out the legacy [coupling] threshold; got: {err}"
    );
}

#[serial(cwd)]
#[test]
fn review_legacy_coupling_threshold_silent_without_verbose() {
    // Same config, no -v → no deprecation noise. The warning is
    // opt-in so production hooks aren't spammed.
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    build_coupling_fixture(dir.path(), now);
    write(dir.path(), "core/a.rs", "a1\na2\na3\na4\na5\nNEW\n");

    std::fs::write(
        dir.path().join("mokumokuren.toml"),
        "[coupling]\nthreshold = 0.30\n",
    )
    .unwrap();

    let mut args = review_args();
    args.format = Format::Json;

    let (res, _, stderr) = common::with_cwd(dir.path(), |so, se| {
        mokumokuren::commands::review::run(&args, None, so, se).map(|_| ())
    });
    res.expect("review");
    let err = String::from_utf8(stderr).unwrap();
    assert!(
        !err.contains("deprecated"),
        "non-verbose stderr must not warn about the legacy field; got: {err}"
    );
}

#[serial(cwd)]
#[test]
fn review_emits_health_warn_when_test_partner_not_touched() {
    // Editing src/foo.ts without touching src/foo.test.ts must
    // surface a HEALTH Warn finding via Pattern C (test-pair).
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    init_repo(dir.path());
    write(dir.path(), "src/foo.ts", "export const foo = 1;\n");
    write(
        dir.path(),
        "src/foo.test.ts",
        "import {foo} from './foo';\n",
    );
    commit_all(dir.path(), "seed", now - 5 * DAY);

    std::fs::write(
        dir.path().join("mokumokuren.toml"),
        "[health.ts]\nenabled = true\npatterns = [\"test_pair\"]\n",
    )
    .unwrap();

    // Edit foo.ts without touching foo.test.ts.
    write(dir.path(), "src/foo.ts", "export const foo = 2;\n");

    let mut args = review_args();
    args.format = Format::Json;
    let (stdout, _) = run_in(dir.path(), args);
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");

    let findings = v["findings"].as_array().expect("findings array");
    let health: Vec<&Value> = findings.iter().filter(|f| f["layer"] == "health").collect();
    assert!(
        !health.is_empty(),
        "Pattern C must fire when impl moves but test partner doesn't; got: {findings:?}"
    );
    let warn = health.iter().any(|f| {
        f["severity"] == "warn" && f["message"].as_str().unwrap_or("").contains("foo.test.ts")
    });
    assert!(
        warn,
        "Pattern C in review mode must be Warn (not Info); got: {health:?}"
    );
}

#[serial(cwd)]
#[test]
fn review_health_warn_fires_when_tsx_impl_pairs_with_test_ts_partner() {
    // Real-world TS pattern: a `.tsx` impl (needs JSX) paired with a
    // `.test.ts` (doesn't render JSX). Pre-fix v0.7 missed this
    // because candidate generation required the partner extension to
    // exactly match the subject's. Reactive-Resume's whole importer
    // family has this shape.
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    init_repo(dir.path());
    write(
        dir.path(),
        "src/integrations/import/json-resume.tsx",
        "export class JSONResumeImporter { parse() {} }\n",
    );
    write(
        dir.path(),
        "src/integrations/import/json-resume.test.ts",
        "import { JSONResumeImporter } from './json-resume';\n",
    );
    commit_all(dir.path(), "seed", now - 5 * DAY);

    std::fs::write(
        dir.path().join("mokumokuren.toml"),
        "[health.ts]\nenabled = true\npatterns = [\"test_pair\"]\n",
    )
    .unwrap();

    // Edit the .tsx impl without touching its .test.ts partner.
    write(
        dir.path(),
        "src/integrations/import/json-resume.tsx",
        "export class JSONResumeImporter { parse(json: string) { return JSON.parse(json); } }\n",
    );

    let mut args = review_args();
    args.format = Format::Json;
    let (stdout, _) = run_in(dir.path(), args);
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");

    let findings = v["findings"].as_array().expect("findings array");
    let warn_on_test_ts = findings.iter().any(|f| {
        f["layer"] == "health"
            && f["severity"] == "warn"
            && f["message"]
                .as_str()
                .unwrap_or("")
                .contains("json-resume.test.ts")
    });
    assert!(
        warn_on_test_ts,
        "Pattern C must fire on .tsx impl with .test.ts partner not in diff; got: {findings:?}"
    );
}

#[serial(cwd)]
#[test]
fn review_health_warn_fires_when_test_partner_has_no_recent_churn() {
    // Pre-fix v0.7: TestPair sourced peer paths from
    // `analysis.loc.keys()`, which only contains files that churned
    // in the window. A stable, untouched test partner was invisible —
    // the agent could edit the impl and never get the "test partner
    // not in diff" signal because the analyzer pipeline never knew
    // the test file existed. Fix: augment peer paths with the
    // working-tree directory listing before running TestPair.
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    init_repo(dir.path());
    write(dir.path(), "src/util.ts", "export const helper = 1;\n");
    // Test partner committed once, far outside the window.
    write(
        dir.path(),
        "src/util.test.ts",
        "import { helper } from './util';\n",
    );
    commit_all(dir.path(), "seed util + test (low churn)", now - 200 * DAY);

    // A second commit a year later that DOESN'T touch util.test.ts —
    // keeps the test partner stable / out of the analyzer's churn set.
    write(dir.path(), "src/other.ts", "export const x = 1;\n");
    commit_all(dir.path(), "unrelated", now - 5 * DAY);

    std::fs::write(
        dir.path().join("mokumokuren.toml"),
        "[health.ts]\nenabled = true\npatterns = [\"test_pair\"]\n",
    )
    .unwrap();

    // Edit util.ts without touching its (stable) test partner.
    write(dir.path(), "src/util.ts", "export const helper = 2;\n");

    let mut args = review_args();
    args.format = Format::Json;
    let (stdout, _) = run_in(dir.path(), args);
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");

    let findings = v["findings"].as_array().expect("findings array");
    let warn = findings.iter().any(|f| {
        f["layer"] == "health"
            && f["severity"] == "warn"
            && f["message"].as_str().unwrap_or("").contains("util.test.ts")
    });
    assert!(
        warn,
        "TestPair must surface stable test partners not in the analyzer's churn set; got: {findings:?}"
    );
}

#[serial(cwd)]
#[test]
fn review_health_warn_suppressed_when_test_partner_also_touched() {
    // The Warn for Pattern C is the "you forgot the test" signal.
    // If the agent *did* touch the test in this diff, the Warn must
    // not fire — same shape as COUPLING's "partner also touched"
    // suppression.
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    init_repo(dir.path());
    write(dir.path(), "src/foo.ts", "export const foo = 1;\n");
    write(
        dir.path(),
        "src/foo.test.ts",
        "import {foo} from './foo';\n",
    );
    commit_all(dir.path(), "seed", now - 5 * DAY);

    std::fs::write(
        dir.path().join("mokumokuren.toml"),
        "[health.ts]\nenabled = true\npatterns = [\"test_pair\"]\n",
    )
    .unwrap();

    // Edit BOTH the impl and its test.
    write(dir.path(), "src/foo.ts", "export const foo = 2;\n");
    write(
        dir.path(),
        "src/foo.test.ts",
        "import {foo} from './foo'; // updated\n",
    );

    let mut args = review_args();
    args.format = Format::Json;
    let (stdout, _) = run_in(dir.path(), args);
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");

    let findings = v["findings"].as_array().expect("findings array");
    let health_warn: Vec<&Value> = findings
        .iter()
        .filter(|f| f["layer"] == "health" && f["severity"] == "warn")
        .collect();
    assert!(
        health_warn.is_empty(),
        "test partner WAS touched — Pattern C Warn must be suppressed; got: {health_warn:?}"
    );
}

#[serial(cwd)]
#[test]
fn review_includes_untracked_in_changed_files() {
    // An untracked file (created on disk, not `git add`-ed) should be
    // visible in `review.diff.files` for working-tree mode, with
    // `added > 0` reflecting its line count. Without this, an agent's
    // brand-new file is invisible to coupling-suppression and BUDGET
    // accounting.
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    init_repo(dir.path());
    write(dir.path(), "seed.rs", "x\n");
    commit_all(dir.path(), "seed", now - DAY);

    write(dir.path(), "new.rs", "alpha\nbeta\ngamma\n");

    let mut args = review_args();
    args.format = Format::Json;
    let (stdout, _) = run_in(dir.path(), args);
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");

    let files = v["review"]["diff"]["files"]
        .as_array()
        .expect("review.diff.files array");
    let new_entry = files
        .iter()
        .find(|f| f["path"] == "new.rs")
        .unwrap_or_else(|| panic!("untracked new.rs missing from diff.files; got: {files:?}"));
    let added = new_entry["added"].as_u64().unwrap_or(0);
    assert!(
        added >= 3,
        "untracked file's `added` should be >= line count; got {added} for {new_entry:?}"
    );
}

#[serial(cwd)]
#[test]
fn review_suppresses_coupling_when_partner_is_now_untracked() {
    // The historical partner `core/b.rs` is a co-changer of
    // `core/a.rs`. If the agent edits `core/a.rs` AND creates an
    // untracked file at `core/b.rs`, the COUPLING "partner not in
    // diff" finding must be suppressed — the partner IS in the diff,
    // just not yet `git add`-ed.
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    build_coupling_fixture(dir.path(), now);

    // Sanity: in the fixture, core/b.rs already exists and was
    // committed. To exercise the untracked-as-partner path, recreate
    // the partner's path as an untracked sibling. We delete the
    // tracked file from the worktree (its deletion will appear as a
    // separate diff event) and then write it as untracked.
    //
    // Simpler: just edit core/a.rs and create a NEW untracked file
    // with a partner-style name that's actually a known partner from
    // the fixture. core/b.rs is already tracked so editing it would
    // show up as tracked. To prove the suppression-via-untracked
    // path, we rely on the fact that any modification of core/b.rs
    // (including a create-after-delete) will surface via either the
    // tracked-diff path OR the untracked path.
    //
    // Concretely: remove core/b.rs first (it'll show up as a tracked
    // deletion), then re-create it untracked (it'll show up via
    // list_untracked). The combined effect must still suppress the
    // COUPLING miss for core/b.rs.
    write(dir.path(), "core/a.rs", "a1\na2\na3\na4\na5\nNEW\n");
    // Stage the deletion; that takes core/b.rs out of the worktree
    // entirely, leaving room for an untracked replacement.
    common::git(dir.path(), &["rm", "-q", "core/b.rs"]);
    write(dir.path(), "core/b.rs", "fresh body\n");

    let mut args = review_args();
    args.format = Format::Json;
    let (stdout, _) = run_in(dir.path(), args);
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");

    let files = v["review"]["diff"]["files"]
        .as_array()
        .expect("review.diff.files array");
    let touches_b = files.iter().any(|f| f["path"] == "core/b.rs");
    assert!(
        touches_b,
        "core/b.rs (recreated untracked) must appear in diff.files; got: {files:?}"
    );
    let coupling_b: Vec<&Value> = v["findings"]
        .as_array()
        .expect("findings array")
        .iter()
        .filter(|f| {
            f["layer"] == "coupling" && f["message"].as_str().unwrap_or("").contains("core/b.rs")
        })
        .collect();
    assert!(
        coupling_b.is_empty(),
        "untracked partner must suppress COUPLING miss for core/b.rs; got: {coupling_b:?}"
    );
}

#[serial(cwd)]
#[test]
fn review_respects_ignores_for_untracked() {
    // An untracked file matching an ignore glob must not appear in
    // diff.files. Mirrors how head-tree enumeration filters by the
    // same globset.
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    init_repo(dir.path());
    write(dir.path(), "seed.rs", "x\n");
    commit_all(dir.path(), "seed", now - DAY);

    std::fs::write(
        dir.path().join("mokumokuren.toml"),
        "ignore = [\"dist/**\"]\n",
    )
    .unwrap();
    write(dir.path(), "dist/foo.js", "console.log('x')\n");
    write(dir.path(), "src/keep.rs", "kept\n");

    let mut args = review_args();
    args.format = Format::Json;
    let (stdout, _) = run_in(dir.path(), args);
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");

    let files = v["review"]["diff"]["files"]
        .as_array()
        .expect("review.diff.files array");
    let paths: Vec<&str> = files
        .iter()
        .map(|f| f["path"].as_str().unwrap_or(""))
        .collect();
    assert!(
        !paths.iter().any(|p| p.starts_with("dist/")),
        "ignore glob must filter untracked files; got: {paths:?}"
    );
    assert!(
        paths.contains(&"src/keep.rs"),
        "non-ignored untracked must still appear; got: {paths:?}"
    );
}

#[serial(cwd)]
#[test]
fn review_skips_binary_untracked() {
    // Untracked files whose first 8 KiB contains a NUL byte must be
    // dropped — same shape as `git diff --numstat` skipping binary
    // files via the `- -` columns.
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    init_repo(dir.path());
    write(dir.path(), "seed.rs", "x\n");
    commit_all(dir.path(), "seed", now - DAY);

    let bin_path = dir.path().join("blob.bin");
    std::fs::write(&bin_path, b"some text\0and a NUL").unwrap();
    write(dir.path(), "text.rs", "hello\n");

    let mut args = review_args();
    args.format = Format::Json;
    let (stdout, _) = run_in(dir.path(), args);
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");

    let files = v["review"]["diff"]["files"]
        .as_array()
        .expect("review.diff.files array");
    let paths: Vec<&str> = files
        .iter()
        .map(|f| f["path"].as_str().unwrap_or(""))
        .collect();
    assert!(
        !paths.contains(&"blob.bin"),
        "binary untracked must be skipped; got: {paths:?}"
    );
    assert!(
        paths.contains(&"text.rs"),
        "text untracked must still appear; got: {paths:?}"
    );
}

#[serial(cwd)]
#[test]
fn review_signals_greenfield_when_diff_is_mostly_new() {
    // When most of the diff is paths the analyzer hasn't seen, the
    // history-based layers (HOTSPOT/COUPLING/DRIFT) are silent. The
    // greenfield short-circuit must surface a single Info finding so
    // the agent reads the silence as expected — not as "mmk decided
    // to be quiet."
    use std::fmt::Write as _;
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    init_repo(dir.path());
    // A loud unrelated hotspot dominates rank, so the file we touch
    // (quiet.rs) doesn't make top-1.
    write(dir.path(), "noisy.rs", "n\n");
    commit_all(dir.path(), "seed noisy", now - 30 * DAY);
    for i in 0..10_i64 {
        let mut body = String::new();
        for n in 0..(20 + i) {
            writeln!(body, "line{n}-r{i}").unwrap();
        }
        write(dir.path(), "noisy.rs", &body);
        commit_all(dir.path(), &format!("noisy r{i}"), now - (28 - i) * DAY);
    }
    write(dir.path(), "quiet.rs", "q\n");
    commit_all(dir.path(), "seed quiet", now - DAY);

    // One modified-with-history (quiet.rs, not in top-1) plus
    // several untracked-new files → new-file fraction above 0.5.
    write(dir.path(), "quiet.rs", "q\nnew\n");
    write(dir.path(), "new1.rs", "n1\n");
    write(dir.path(), "new2.rs", "n2\n");
    write(dir.path(), "new3.rs", "n3\n");
    write(dir.path(), "new4.rs", "n4\n");

    let mut args = review_args();
    args.format = Format::Json;
    args.top = 1;
    let (stdout, _) = run_in(dir.path(), args);
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");

    let frac = v["review"]["diff"]["new_file_fraction"]
        .as_f64()
        .expect("new_file_fraction must be present on a greenfield diff");
    assert!(
        frac > 0.5,
        "new-file fraction must exceed 0.5 on this fixture; got {frac}"
    );

    let greenfield: Vec<&Value> = v["findings"]
        .as_array()
        .expect("findings array")
        .iter()
        .filter(|f| {
            f["message"]
                .as_str()
                .unwrap_or("")
                .contains("new files; history priors don't apply")
        })
        .collect();
    assert_eq!(
        greenfield.len(),
        1,
        "exactly one greenfield Info finding expected; got: {:?}",
        v["findings"]
    );
    assert_eq!(greenfield[0]["severity"], "info");
}

#[serial(cwd)]
#[test]
fn review_does_not_signal_greenfield_for_cold_existing_file_with_additive_edit() {
    // Files present at HEAD are not greenfield, regardless of
    // whether they churned in the analysis window. The fixture
    // anchors HEAD with a recent commit so the window walker has a
    // start point; seed.ts is committed far enough in the past that
    // `commits_touching` (window-only) won't see it, exposing any
    // predicate that conflates "no in-window churn" with "new file".
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    init_repo(dir.path());
    write(dir.path(), "src/seed.ts", "export const x = 1;\n");
    commit_all(dir.path(), "seed", now - 365 * DAY);
    write(dir.path(), "other/recent.ts", "export const r = 1;\n");
    commit_all(dir.path(), "recent", now - 30 * DAY);

    // Working-tree edit of the cold file — additive, still at HEAD.
    write(
        dir.path(),
        "src/seed.ts",
        "export const x = 1;\nexport const y = 2;\n",
    );

    let mut args = review_args();
    args.format = Format::Json;
    let (stdout, _) = run_in(dir.path(), args);
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");

    let greenfield: Vec<&Value> = v["findings"]
        .as_array()
        .expect("findings array")
        .iter()
        .filter(|f| {
            f["message"]
                .as_str()
                .unwrap_or("")
                .contains("history priors don't apply")
        })
        .collect();
    assert!(
        greenfield.is_empty(),
        "edited file present at HEAD must not be classified as greenfield; got: {greenfield:?}"
    );
}

#[serial(cwd)]
#[test]
fn review_no_greenfield_signal_when_history_layer_fired() {
    // The greenfield finding is a fall-through, not an addition.
    // When HOTSPOT/COUPLING/DRIFT fired, the agent already has
    // history-based signal — no need to add the "priors don't apply"
    // line. Mirrors pre-edit's no-OK-when-other-layers-fired rule.
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    build_coupling_fixture(dir.path(), now);

    // Edit core/a.rs (HOTSPOT + COUPLING fire) and add several new
    // untracked files (which would push new_file_fraction above 0.5
    // and might tempt the short-circuit).
    write(dir.path(), "core/a.rs", "a1\na2\na3\na4\na5\nNEW\n");
    write(dir.path(), "feat/x.rs", "x\n");
    write(dir.path(), "feat/y.rs", "y\n");
    write(dir.path(), "feat/z.rs", "z\n");

    let mut args = review_args();
    args.format = Format::Json;
    let (stdout, _) = run_in(dir.path(), args);
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");

    let greenfield: Vec<&Value> = v["findings"]
        .as_array()
        .expect("findings array")
        .iter()
        .filter(|f| {
            f["message"]
                .as_str()
                .unwrap_or("")
                .contains("history priors don't apply")
        })
        .collect();
    assert!(
        greenfield.is_empty(),
        "greenfield signal must not co-fire with HOTSPOT/COUPLING/DRIFT; got: {greenfield:?}"
    );
}

#[serial(cwd)]
#[test]
fn review_budget_ramp_fires_by_default() {
    // The under-cap continuous ramp is on by default: a 60 %-of-cap
    // diff surfaces an Approaching Info finding without any toml
    // flipping it on. Useful signal defaults on; users who don't
    // want it set [sensor.budget_ramp] enabled = false.
    use std::fmt::Write as _;
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    init_repo(dir.path());
    write(dir.path(), "seed.rs", "x\n");
    commit_all(dir.path(), "seed", now - DAY);

    let mut body = String::with_capacity(600 * 8);
    for i in 0..600 {
        writeln!(body, "line{i}").unwrap();
    }
    write(dir.path(), "seed.rs", &body);

    let mut args = review_args();
    args.format = Format::Json;
    let (stdout, _) = run_in(dir.path(), args);
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");

    let ramp_count = v["findings"]
        .as_array()
        .expect("findings array")
        .iter()
        .filter(|f| {
            f["layer"] == "budget" && f["message"].as_str().unwrap_or("").contains("of cap")
        })
        .count();
    assert_eq!(
        ramp_count, 1,
        "ramp must fire exactly once at 60 % of cap by default; got: {:?}",
        v["findings"]
    );
}

#[serial(cwd)]
#[test]
fn review_budget_ramp_silent_when_disabled() {
    // [sensor.budget_ramp] enabled = false silences the under-cap
    // ramp. The over-cap BUDGET finding is unaffected (covered by
    // review_emits_budget_when_diff_exceeds).
    use std::fmt::Write as _;
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    init_repo(dir.path());
    write(dir.path(), "seed.rs", "x\n");
    commit_all(dir.path(), "seed", now - DAY);

    std::fs::write(
        dir.path().join("mokumokuren.toml"),
        "[sensor.budget_ramp]\nenabled = false\n",
    )
    .unwrap();

    let mut body = String::with_capacity(600 * 8);
    for i in 0..600 {
        writeln!(body, "line{i}").unwrap();
    }
    write(dir.path(), "seed.rs", &body);

    let mut args = review_args();
    args.format = Format::Json;
    let (stdout, _) = run_in(dir.path(), args);
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");

    let budget: Vec<&Value> = v["findings"]
        .as_array()
        .expect("findings array")
        .iter()
        .filter(|f| f["layer"] == "budget")
        .collect();
    assert!(
        budget.is_empty(),
        "ramp must be silent when explicitly disabled; got: {budget:?}"
    );
}

#[serial(cwd)]
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

#[serial(cwd)]
#[test]
fn complexity_suppressed_for_unchanged_pre_existing_function() {
    // Real-world failure mode (data point #2): an over-cap function
    // that already existed at HEAD fires COMPLEXITY on every fresh
    // agent's first review, even when the agent's diff didn't
    // change the function's shape. Fix: filter out findings whose
    // (path, function-name) pair has the same-or-better metric at
    // HEAD. Only fire when the agent worsens it or introduces a
    // brand-new over-cap function.
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    init_repo(dir.path());

    // Seed a 100-LOC function at HEAD (already over the 80-LOC
    // default cap — pre-existing complexity the agent inherited).
    let mut body = String::from("export function bigFn() {\n");
    for i in 0..98 {
        use std::fmt::Write as _;
        writeln!(body, "  const v{i} = {i};").unwrap();
    }
    body.push_str("}\n");
    write(dir.path(), "src/foo.ts", &body);
    write(
        dir.path(),
        "src/sibling.ts",
        "export function smallSibling() { return 1; }\n",
    );
    commit_all(dir.path(), "seed", now - 5 * DAY);

    // Working-tree edit: agent adds an unrelated comment to bigFn,
    // doesn't change its shape. The function is *still* over the
    // cap, but the agent didn't make it worse.
    let mut working = body.clone();
    working = working.replace(
        "export function bigFn() {\n",
        "// docs: business logic\nexport function bigFn() {\n",
    );
    write(dir.path(), "src/foo.ts", &working);

    let mut args = review_args();
    args.format = Format::Json;
    let (stdout, _) = run_in(dir.path(), args);
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");

    let complexity_on_bigfn: Vec<&Value> = v["findings"]
        .as_array()
        .expect("findings array")
        .iter()
        .filter(|f| {
            f["layer"] == "complexity" && f["message"].as_str().unwrap_or("").contains("bigFn")
        })
        .collect();
    assert!(
        complexity_on_bigfn.is_empty(),
        "COMPLEXITY must not fire on a pre-existing over-cap function whose shape \
         the agent didn't change; got: {complexity_on_bigfn:?}"
    );
}

#[serial(cwd)]
#[test]
fn complexity_prose_includes_delta_vs_head_when_agent_worsens() {
    // Data point #3 motivation: agent grew an over-cap function by
    // a few LOC. The fire is correct, but the prose alone ("366 LOC
    // exceeds cap 80") doesn't tell the agent how much of that
    // they actually contributed. With the delta clause they can
    // judge their contribution at a glance: "+3" reads differently
    // from "+60".
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    init_repo(dir.path());
    let mut head_body = String::from("export function bigFn() {\n");
    for i in 0..98 {
        use std::fmt::Write as _;
        writeln!(head_body, "  const v{i} = {i};").unwrap();
    }
    head_body.push_str("}\n");
    write(dir.path(), "src/foo.ts", &head_body);
    write(
        dir.path(),
        "src/sibling.ts",
        "export function smallSibling() { return 1; }\n",
    );
    commit_all(dir.path(), "seed", now - 5 * DAY);

    // Working-tree: extend bigFn from 100 LOC to 110 LOC (+10).
    let mut working = String::from("export function bigFn() {\n");
    for i in 0..108 {
        use std::fmt::Write as _;
        writeln!(working, "  const v{i} = {i};").unwrap();
    }
    working.push_str("}\n");
    write(dir.path(), "src/foo.ts", &working);

    let mut args = review_args();
    args.format = Format::Json;
    let (stdout, _) = run_in(dir.path(), args);
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");

    let bigfn_msg: Vec<String> = v["findings"]
        .as_array()
        .expect("findings array")
        .iter()
        .filter(|f| {
            f["layer"] == "complexity" && f["message"].as_str().unwrap_or("").contains("bigFn")
        })
        .map(|f| f["message"].as_str().unwrap_or("").to_string())
        .collect();
    assert_eq!(
        bigfn_msg.len(),
        1,
        "exactly one COMPLEXITY finding on bigFn; got: {bigfn_msg:?}"
    );
    assert!(
        bigfn_msg[0].contains("vs HEAD"),
        "delta clause must surface end-to-end; got: {}",
        bigfn_msg[0]
    );
}

#[serial(cwd)]
#[test]
fn complexity_fires_when_agent_worsens_pre_existing_function() {
    // Counterpart to the suppression test: if the agent makes an
    // already-over-cap function worse, the finding must fire so the
    // signal isn't lost.
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    init_repo(dir.path());

    let mut head_body = String::from("export function bigFn() {\n");
    for i in 0..98 {
        use std::fmt::Write as _;
        writeln!(head_body, "  const v{i} = {i};").unwrap();
    }
    head_body.push_str("}\n");
    write(dir.path(), "src/foo.ts", &head_body);
    write(
        dir.path(),
        "src/sibling.ts",
        "export function smallSibling() { return 1; }\n",
    );
    commit_all(dir.path(), "seed", now - 5 * DAY);

    // Working-tree: extend bigFn with 50 more LOC.
    let mut working = String::from("export function bigFn() {\n");
    for i in 0..148 {
        use std::fmt::Write as _;
        writeln!(working, "  const v{i} = {i};").unwrap();
    }
    working.push_str("}\n");
    write(dir.path(), "src/foo.ts", &working);

    let mut args = review_args();
    args.format = Format::Json;
    let (stdout, _) = run_in(dir.path(), args);
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");

    let complexity_on_bigfn = v["findings"]
        .as_array()
        .expect("findings array")
        .iter()
        .any(|f| {
            f["layer"] == "complexity" && f["message"].as_str().unwrap_or("").contains("bigFn")
        });
    assert!(
        complexity_on_bigfn,
        "COMPLEXITY must fire when the agent worsens a pre-existing \
         over-cap function; got: {:?}",
        v["findings"]
    );
}

#[serial(cwd)]
#[test]
fn complexity_fires_for_newly_added_over_cap_function() {
    // A brand-new function that's over the cap is genuinely new
    // signal — fire.
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    init_repo(dir.path());

    write(
        dir.path(),
        "src/foo.ts",
        "export function tiny() { return 1; }\n",
    );
    write(
        dir.path(),
        "src/sibling.ts",
        "export function smallSibling() { return 1; }\n",
    );
    commit_all(dir.path(), "seed", now - 5 * DAY);

    // Working-tree: add a brand-new over-cap function.
    let mut working = String::from("export function tiny() { return 1; }\n");
    working.push_str("export function newBig() {\n");
    for i in 0..120 {
        use std::fmt::Write as _;
        writeln!(working, "  const v{i} = {i};").unwrap();
    }
    working.push_str("}\n");
    write(dir.path(), "src/foo.ts", &working);

    let mut args = review_args();
    args.format = Format::Json;
    let (stdout, _) = run_in(dir.path(), args);
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");

    let complexity_on_newbig = v["findings"]
        .as_array()
        .expect("findings array")
        .iter()
        .any(|f| {
            f["layer"] == "complexity" && f["message"].as_str().unwrap_or("").contains("newBig")
        });
    assert!(
        complexity_on_newbig,
        "COMPLEXITY must fire on a newly-added over-cap function; got: {:?}",
        v["findings"]
    );
}

#[serial(cwd)]
#[test]
fn complexity_head_baseline_qualifies_function_by_class() {
    // Regression: pre-v0.9 the HEAD-baseline filter matched
    // FunctionFact entries by bare `name`, so a file with two classes
    // each containing a `constructor` collided. The first match in
    // AST order won — concretely, an agent that *shrank* the second
    // class's constructor still saw COMPLEXITY fire on the *first*
    // class's constructor with a "+N vs HEAD" delta computed against
    // the wrong baseline. The fix qualifies FunctionFact identity by
    // enclosing class (`Inner::constructor` vs `Outer::constructor`).
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    init_repo(dir.path());

    // HEAD: Outer::constructor 10 LOC (under cap); Inner::constructor
    // 100 LOC (well over default loc_absolute_max=80).
    let mut head = String::from("export class Outer {\n  constructor() {\n");
    for i in 0..6 {
        use std::fmt::Write as _;
        writeln!(head, "    const o{i} = {i};").unwrap();
    }
    head.push_str("  }\n}\n\nexport class Inner {\n  constructor() {\n");
    for i in 0..96 {
        use std::fmt::Write as _;
        writeln!(head, "    const i{i} = {i};").unwrap();
    }
    head.push_str("  }\n}\n");
    write(dir.path(), "src/two-class.ts", &head);
    write(
        dir.path(),
        "src/sibling.ts",
        "export function smallSibling() { return 1; }\n",
    );
    commit_all(dir.path(), "seed", now - 5 * DAY);

    // Working tree: shrink Inner::constructor from 100 → 90 LOC.
    // Still over the 80-LOC cap (so the absolute gate would fire),
    // but strictly *smaller* than HEAD.
    let mut working = String::from("export class Outer {\n  constructor() {\n");
    for i in 0..6 {
        use std::fmt::Write as _;
        writeln!(working, "    const o{i} = {i};").unwrap();
    }
    working.push_str("  }\n}\n\nexport class Inner {\n  constructor() {\n");
    for i in 0..86 {
        use std::fmt::Write as _;
        writeln!(working, "    const i{i} = {i};").unwrap();
    }
    working.push_str("  }\n}\n");
    write(dir.path(), "src/two-class.ts", &working);

    let mut args = review_args();
    args.format = Format::Json;
    let (stdout, _) = run_in(dir.path(), args);
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");

    // Pre-fix: bare-name find() returned Outer::constructor (10 LOC).
    // The filter saw 90 > 10 → kept the finding with `head_actual=10`,
    // and the prose rendered "+80 vs HEAD" against Outer's baseline —
    // wrong function attribution.
    //
    // Post-fix: qualified match on Inner::constructor (100). Working
    // tree's 90 < 100 → strict-worsening filter suppresses the
    // finding. The "+N vs HEAD" misattribution is *structurally*
    // unreachable.
    let constructor_findings: Vec<String> = v["findings"]
        .as_array()
        .expect("findings array")
        .iter()
        .filter(|f| {
            f["layer"] == "complexity"
                && f["message"].as_str().unwrap_or("").contains("constructor")
        })
        .map(|f| f["message"].as_str().unwrap_or("").to_string())
        .collect();
    assert!(
        constructor_findings.is_empty(),
        "Inner::constructor shrank vs HEAD; HEAD-baseline filter must \
         match by qualified function identity and suppress the finding. \
         Got: {constructor_findings:?}"
    );
}

#[serial(cwd)]
#[test]
fn complexity_head_baseline_delta_uses_qualified_baseline() {
    // Sibling assertion to the suppression test: when the agent
    // grows the second class's constructor, the rendered
    // "+N vs HEAD" delta must be computed against *that* class's
    // HEAD baseline — not the first class's collision-named partner.
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    init_repo(dir.path());

    // HEAD: Outer::constructor 10 LOC, Inner::constructor 100 LOC.
    let mut head = String::from("export class Outer {\n  constructor() {\n");
    for i in 0..6 {
        use std::fmt::Write as _;
        writeln!(head, "    const o{i} = {i};").unwrap();
    }
    head.push_str("  }\n}\n\nexport class Inner {\n  constructor() {\n");
    for i in 0..96 {
        use std::fmt::Write as _;
        writeln!(head, "    const i{i} = {i};").unwrap();
    }
    head.push_str("  }\n}\n");
    write(dir.path(), "src/two-class.ts", &head);
    write(
        dir.path(),
        "src/sibling.ts",
        "export function smallSibling() { return 1; }\n",
    );
    commit_all(dir.path(), "seed", now - 5 * DAY);

    // Working tree: grow Inner::constructor from 100 → 115 LOC (+15).
    let mut working = String::from("export class Outer {\n  constructor() {\n");
    for i in 0..6 {
        use std::fmt::Write as _;
        writeln!(working, "    const o{i} = {i};").unwrap();
    }
    working.push_str("  }\n}\n\nexport class Inner {\n  constructor() {\n");
    for i in 0..111 {
        use std::fmt::Write as _;
        writeln!(working, "    const i{i} = {i};").unwrap();
    }
    working.push_str("  }\n}\n");
    write(dir.path(), "src/two-class.ts", &working);

    let mut args = review_args();
    args.format = Format::Json;
    let (stdout, _) = run_in(dir.path(), args);
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");

    let inner_msgs: Vec<String> = v["findings"]
        .as_array()
        .expect("findings array")
        .iter()
        .filter(|f| {
            f["layer"] == "complexity"
                && f["message"]
                    .as_str()
                    .unwrap_or("")
                    .contains("Inner::constructor")
        })
        .map(|f| f["message"].as_str().unwrap_or("").to_string())
        .collect();
    assert_eq!(
        inner_msgs.len(),
        1,
        "exactly one COMPLEXITY finding on Inner::constructor; got: {inner_msgs:?}"
    );
    assert!(
        inner_msgs[0].contains("(+15 vs HEAD)"),
        "delta must be computed against Inner::constructor's HEAD \
         baseline (100), not Outer::constructor's (10). Got: {}",
        inner_msgs[0]
    );
}

#[serial(cwd)]
#[test]
fn health_test_pair_re_fire_suppressed_when_state_unchanged() {
    // DP#4 failure mode: hooks fire on every Edit, so an agent
    // doing 6 Edits sees the same TestPair warning 6 times. The
    // existing whole-set dedup keys on findings hash, which
    // changes whenever the diff grows even by one line — never
    // suppresses. Per-finding MonotonicSignal keyed on the
    // (pattern, subject) pair fixes it: fire once, then stay
    // silent until TTL expires or the partner enters the diff.
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    init_repo(dir.path());
    write(dir.path(), "src/foo.ts", "export const foo = 1;\n");
    write(
        dir.path(),
        "src/foo.test.ts",
        "import {foo} from './foo';\n",
    );
    commit_all(dir.path(), "seed", now - 5 * DAY);

    std::fs::write(
        dir.path().join("mokumokuren.toml"),
        "[health.ts]\nenabled = true\npatterns = [\"test_pair\"]\n",
    )
    .unwrap();

    let count_test_pair_warns = |stdout: &[u8]| -> usize {
        let v: Value = serde_json::from_slice(stdout).expect("valid JSON");
        v["findings"].as_array().map_or(0, |a| {
            a.iter()
                .filter(|f| {
                    f["layer"] == "health"
                        && f["severity"] == "warn"
                        && f["message"]
                            .as_str()
                            .is_some_and(|m| m.contains("test partner"))
                })
                .count()
        })
    };

    let dedup_args = || -> ReviewArgs {
        let mut a = review_args();
        a.no_dedup = false;
        a.format = Format::Json;
        a
    };

    // Edit 1: foo.ts gets a 600-line dump (BUDGET ramp at 60% =
    // Approaching). TestPair fires; whole findings set is
    // {TestPair, BUDGET ramp 60%}.
    let mut edit1 = String::new();
    for i in 0..600 {
        use std::fmt::Write as _;
        writeln!(edit1, "export const v{i} = {i};").unwrap();
    }
    write(dir.path(), "src/foo.ts", &edit1);
    let (stdout1, _) = run_in(dir.path(), dedup_args());
    assert!(
        count_test_pair_warns(&stdout1) >= 1,
        "first edit must fire TestPair Warn; got: {}",
        String::from_utf8_lossy(&stdout1)
    );

    // Edit 2: extend foo.ts to 750 lines (BUDGET ramp at 75% =
    // Near tier, severity Warn). The BUDGET finding's text and
    // severity change → whole findings hash differs from Edit 1
    // → envelope-level dedup misses. TestPair on foo.ts is still
    // the same finding (subject under-test, partner untouched).
    // Per-key MonotonicSignal must suppress the TestPair re-fire.
    let mut edit2 = String::new();
    for i in 0..750 {
        use std::fmt::Write as _;
        writeln!(edit2, "export const v{i} = {i};").unwrap();
    }
    write(dir.path(), "src/foo.ts", &edit2);
    let (stdout2, _) = run_in(dir.path(), dedup_args());
    assert_eq!(
        count_test_pair_warns(&stdout2),
        0,
        "TestPair on foo.ts must not re-fire when its state \
         (subject under-test, partner untouched) is unchanged, \
         even though the broader findings set changed (BUDGET ramp \
         tier escalated); got: {}",
        String::from_utf8_lossy(&stdout2)
    );
}

#[serial(cwd)]
#[test]
fn budget_re_fire_suppressed_when_counts_unchanged() {
    // Real-world failure mode (data point #1): drizzle generated
    // a 3.5k-line snapshot.json once; subsequent Edit/Write hooks
    // re-fired the BUDGET warning ~12 times with the same numbers.
    // Fix: per-key MonotonicSignal on BUDGET. Re-fire only when
    // files_net or lines_net strictly worsens past the prior fire.
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    init_repo(dir.path());
    write(dir.path(), "seed.rs", "x\n");
    commit_all(dir.path(), "seed", now - DAY);

    // Working-tree blast that trips BUDGET (6000 lines > default 1000).
    let mut huge = String::with_capacity(6000 * 8);
    for i in 0..6000 {
        use std::fmt::Write as _;
        writeln!(huge, "line{i}").unwrap();
    }
    write(dir.path(), "seed.rs", &huge);

    let count_budget = |stdout: &[u8]| -> usize {
        // Bulk-self path emits text by default; this test uses JSON
        // so we can count BUDGET findings precisely.
        let v: Value = serde_json::from_slice(stdout).expect("valid JSON");
        v["findings"]
            .as_array()
            .map_or(0, |a| a.iter().filter(|f| f["layer"] == "budget").count())
    };

    let dedup_args = || -> ReviewArgs {
        let mut a = review_args();
        a.no_dedup = false;
        a.format = Format::Json;
        a
    };

    // First fire: BUDGET should appear.
    let (stdout1, _) = run_in(dir.path(), dedup_args());
    assert!(
        count_budget(&stdout1) >= 1,
        "first fire on a 6000-line edit must emit BUDGET; got: {}",
        String::from_utf8_lossy(&stdout1)
    );

    // Second fire against the same working tree: same numbers,
    // MonotonicSignal must suppress.
    let (stdout2, _) = run_in(dir.path(), dedup_args());
    assert_eq!(
        count_budget(&stdout2),
        0,
        "identical BUDGET fire must be suppressed by MonotonicSignal; got: {}",
        String::from_utf8_lossy(&stdout2)
    );

    // Push lines strictly higher → must re-fire (axis worsened).
    let mut huger = huge;
    for i in 6000..7000 {
        use std::fmt::Write as _;
        writeln!(huger, "line{i}").unwrap();
    }
    write(dir.path(), "seed.rs", &huger);
    let (stdout3, _) = run_in(dir.path(), dedup_args());
    assert!(
        count_budget(&stdout3) >= 1,
        "BUDGET must re-fire when lines_net worsens; got: {}",
        String::from_utf8_lossy(&stdout3)
    );
}

#[serial(cwd)]
#[test]
fn review_emits_structure_divergence_on_new_file_not_matching_convention() {
    // 3 sibling .tsx files share zod + Create*Dialog template.
    // Adding a new untracked sibling that imports neither must
    // fire a STRUCTURE divergence.
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    init_repo(dir.path());
    let body = "import { z } from 'zod';\nexport function CreateAwardDialog(){}\n";
    let body_g = "import { z } from 'zod';\nexport function CreateGoalDialog(){}\n";
    let body_j = "import { z } from 'zod';\nexport function CreateJobDialog(){}\n";
    write(dir.path(), "dlg/award.tsx", body);
    write(dir.path(), "dlg/goal.tsx", body_g);
    write(dir.path(), "dlg/job.tsx", body_j);
    commit_all(dir.path(), "seed", now - 5 * DAY);
    write(dir.path(), "dlg/divergent.tsx", "export const x = 1;\n");

    let mut args = review_args();
    args.format = Format::Json;
    let (stdout, _) = run_in(dir.path(), args);
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");

    let findings = v["findings"].as_array().expect("findings array");
    let structure: Vec<&Value> = findings
        .iter()
        .filter(|f| f["layer"] == "structure")
        .collect();
    assert!(
        !structure.is_empty(),
        "STRUCTURE divergence must fire on dlg/divergent.tsx; got: {findings:?}"
    );
    let msg = structure[0]["message"].as_str().unwrap_or("");
    assert!(
        msg.contains("missing") || msg.contains("not exporting"),
        "STRUCTURE divergence message should mention the missing piece; got: {msg}"
    );
}

#[serial(cwd)]
#[test]
fn review_bulk_does_not_suppress_structure_or_complexity() {
    // The bulk-self-filter exists to skip the *expensive* analyze
    // path (HOTSPOT/COUPLING). STRUCTURE and COMPLEXITY are per-file
    // and cheap, so they must still surface alongside BUDGET when the
    // diff exceeds bulk thresholds — otherwise the v0.5 sensors are
    // invisible in exactly the session shape (sweep, generated drop)
    // where they have signal.
    use std::fmt::Write as _;
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    init_repo(dir.path());
    // Three sibling .tsx files share `zod` + Create*Dialog template,
    // committed so they're known siblings on disk.
    let body_a = "import { z } from 'zod';\nexport function CreateAwardDialog(){}\n";
    let body_g = "import { z } from 'zod';\nexport function CreateGoalDialog(){}\n";
    let body_j = "import { z } from 'zod';\nexport function CreateJobDialog(){}\n";
    write(dir.path(), "dlg/award.tsx", body_a);
    write(dir.path(), "dlg/goal.tsx", body_g);
    write(dir.path(), "dlg/job.tsx", body_j);
    // A separate seed file we can balloon to push the diff over cap.
    write(dir.path(), "seed.rs", "x\n");
    commit_all(dir.path(), "seed", now - 5 * DAY);

    // Untracked sibling that diverges from the convention — STRUCTURE
    // should fire on it.
    write(dir.path(), "dlg/divergent.tsx", "export const x = 1;\n");
    // Untracked deeply-nested .ts function — COMPLEXITY should fire.
    let deep = "function deep() {\n\
        if (a) { if (b) { if (c) { if (d) { if (e) { if (f) { if (g) { return 1; } } } } } } }\n\
        }\n";
    write(dir.path(), "src/deep.ts", deep);
    // And blow past bulk.max_lines so the bulk-self-filter trips.
    let mut huge = String::with_capacity(2000 * 8);
    for i in 0..2000 {
        writeln!(huge, "line{i}").unwrap();
    }
    write(dir.path(), "seed.rs", &huge);

    let mut args = review_args();
    args.format = Format::Json;
    let (stdout, _) = run_in(dir.path(), args);
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");

    let findings = v["findings"].as_array().expect("findings array");
    let layers: Vec<&str> = findings
        .iter()
        .map(|f| f["layer"].as_str().unwrap_or(""))
        .collect();
    assert!(
        layers.contains(&"budget"),
        "bulk diff must still emit BUDGET; got: {layers:?}"
    );
    assert!(
        layers.contains(&"structure"),
        "STRUCTURE must surface even when bulk fires; got: {layers:?}"
    );
    assert!(
        layers.contains(&"complexity"),
        "COMPLEXITY must surface even when bulk fires; got: {layers:?}"
    );
    // History-based layers stay suppressed.
    assert!(
        !layers.contains(&"hotspot") && !layers.contains(&"coupling"),
        "bulk path must still suppress HOTSPOT/COUPLING; got: {layers:?}"
    );
}

#[serial(cwd)]
#[test]
fn review_emits_complexity_for_deeply_nested_function() {
    // Untracked subject with nesting depth 8 — over default cap=6.
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    init_repo(dir.path());
    write(dir.path(), "src/seed.ts", "export const x = 1;\n");
    commit_all(dir.path(), "seed", now - 5 * DAY);

    let deep = "function deep() {\n\
        if (a) { if (b) { if (c) { if (d) { if (e) { if (f) { if (g) { return 1; } } } } } } }\n\
        }\n";
    write(dir.path(), "src/deep.ts", deep);

    let mut args = review_args();
    args.format = Format::Json;
    let (stdout, _) = run_in(dir.path(), args);
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");

    let complexity: Vec<&Value> = v["findings"]
        .as_array()
        .expect("findings array")
        .iter()
        .filter(|f| f["layer"] == "complexity")
        .collect();
    assert!(
        !complexity.is_empty(),
        "deep nesting must fire COMPLEXITY; got: {}",
        v["findings"]
    );
    let msg = complexity[0]["message"].as_str().unwrap_or("");
    assert!(
        msg.contains("nesting"),
        "COMPLEXITY message must name the metric; got: {msg}"
    );
    assert!(
        msg.contains("deep"),
        "COMPLEXITY message must name the function; got: {msg}"
    );
    assert!(
        msg.contains("correlates with"),
        "COMPLEXITY message must state the empirical implication; got: {msg}"
    );
}

#[serial(cwd)]
#[test]
fn review_ignore_for_budget_keeps_generated_file_below_cap() {
    // v0.6: a 1500-line generated file regeneration would otherwise
    // trip the 1000-line over-cap BUDGET trigger and self-DoS the
    // analyzer pass — silencing HOTSPOT/COUPLING for the rest of the
    // session. With `bulk.ignore_for_budget = ["**/routeTree.gen.ts"]`
    // the generated file is excluded from BUDGET accounting, the diff
    // sits comfortably under cap, and HOTSPOT can fire on the small
    // hand-edit. The full diff is still surfaced in
    // `review.diff.files[]` and a `review.diff.budget` sub-block
    // reports both gross and net counts.
    use std::fmt::Write as _;
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    init_repo(dir.path());
    write(dir.path(), "src/router.ts", "export const router = 1;\n");
    write(
        dir.path(),
        "src/routeTree.gen.ts",
        "export const tree = 0;\n",
    );
    commit_all(dir.path(), "seed", now - DAY);

    std::fs::write(
        dir.path().join("mokumokuren.toml"),
        "[bulk]\nignore_for_budget = [\"**/routeTree.gen.ts\"]\n",
    )
    .unwrap();

    // Hand edit: 5 lines added in router.ts.
    write(
        dir.path(),
        "src/router.ts",
        "export const router = 1;\nA\nB\nC\nD\nE\n",
    );
    // Generated file rewrite: 1500 lines.
    let mut huge = String::with_capacity(1500 * 8);
    for i in 0..1500 {
        writeln!(huge, "tree{i}").unwrap();
    }
    write(dir.path(), "src/routeTree.gen.ts", &huge);

    let mut args = review_args();
    args.format = Format::Json;
    let (stdout, _) = run_in(dir.path(), args);
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");

    // BUDGET must NOT fire — net is well under cap.
    assert!(
        v["findings"]
            .as_array()
            .expect("findings array")
            .iter()
            .all(|f| !(f["layer"] == "budget" && f["severity"] != "info")),
        "ignore_for_budget must suppress the over-cap BUDGET trigger; got: {:?}",
        v["findings"]
    );

    // The generated file IS still present in diff.files[].
    let diff_files = v["review"]["diff"]["files"]
        .as_array()
        .expect("diff.files array");
    let paths: Vec<&str> = diff_files
        .iter()
        .map(|f| f["path"].as_str().unwrap_or(""))
        .collect();
    assert!(
        paths.contains(&"src/routeTree.gen.ts"),
        "generated file must still appear in diff.files even when excluded from BUDGET; got: {paths:?}"
    );

    // The new budget sub-block reports both totals.
    let budget = v["review"]["diff"]["budget"]
        .as_object()
        .expect("budget sub-block must be present when ignore_for_budget excluded a file");
    let files_gross = budget["files_gross"].as_u64().unwrap();
    let files_net = budget["files_net"].as_u64().unwrap();
    let lines_gross = budget["lines_gross"].as_u64().unwrap();
    let lines_net = budget["lines_net"].as_u64().unwrap();
    // Diff also includes the untracked `mokumokuren.toml` we wrote
    // above, so gross is 3 (router + gen + toml). Net excludes only
    // the generated file.
    assert_eq!(
        files_gross - files_net,
        1,
        "one file excluded by ignore_for_budget"
    );
    assert!(
        files_net >= 1,
        "the hand-edited router.ts must remain in net"
    );
    assert!(
        lines_gross >= 1500,
        "lines_gross must include the generated file; got {lines_gross}"
    );
    assert!(
        lines_gross - lines_net >= 1500,
        "the generated file's lines must be excluded from net; got gross={lines_gross} net={lines_net}"
    );
    let ignored = budget["ignored_for_budget"].as_array().unwrap();
    assert_eq!(ignored.len(), 1);
    assert_eq!(ignored[0].as_str().unwrap(), "**/routeTree.gen.ts");
}

#[serial(cwd)]
#[test]
fn review_default_gate_suppresses_wilson_one_of_one() {
    // n=1 / k=1 has Wilson 95% lower ≈ 0.206. Pre-v0.6 this scraped
    // past `confidence_threshold = 0.20` and fired COUPLING — the
    // false-positive class agent test runs flagged. v0.6 calibration
    // (`confidence_threshold = 0.30`, `min_sample_size = 3`) must
    // suppress it. The opt-back-in path (set both knobs to the v0.5
    // values via TOML) restores the firing — so users with established
    // calibration aren't surprised by the bump.
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    init_repo(dir.path());

    // One commit touching A AND B. touches_A = 1, co(A,B) = 1.
    write(dir.path(), "core/a.rs", "a1\n");
    write(dir.path(), "core/b.rs", "b1\n");
    commit_all(dir.path(), "seed: a+b co-change", now - DAY);

    // Edit A, leave B untouched — this is the case Wilson(1,1) would
    // fire on if the gate were loose enough.
    write(dir.path(), "core/a.rs", "a1\nNEW\n");

    // Default gate: COUPLING for B must be suppressed.
    let mut args = review_args();
    args.format = Format::Json;
    let (stdout, _) = run_in(dir.path(), args);
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");
    let coupling_b: Vec<&Value> = v["findings"]
        .as_array()
        .expect("findings array")
        .iter()
        .filter(|f| {
            f["layer"] == "coupling" && f["message"].as_str().unwrap_or("").contains("core/b.rs")
        })
        .collect();
    assert!(
        coupling_b.is_empty(),
        "v0.6 default gate must suppress Wilson(1,1) ≈ 0.21 case; got: {coupling_b:?}"
    );

    // Opt back in to v0.5 behavior via TOML — both knobs together.
    std::fs::write(
        dir.path().join("mokumokuren.toml"),
        "[coupling]\nconfidence_threshold = 0.20\nmin_sample_size = 1\n",
    )
    .unwrap();

    let mut args = review_args();
    args.format = Format::Json;
    let (stdout, _) = run_in(dir.path(), args);
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");
    let any_coupling_b = v["findings"]
        .as_array()
        .expect("findings array")
        .iter()
        .any(|f| {
            f["layer"] == "coupling" && f["message"].as_str().unwrap_or("").contains("core/b.rs")
        });
    assert!(
        any_coupling_b,
        "explicit confidence_threshold=0.20 + min_sample_size=1 must restore the v0.5 firing; got: {:?}",
        v["findings"]
    );
}
