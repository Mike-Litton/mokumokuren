//! COHESION sensor — tangled-diff fingerprint detection.
//!
//! Locks the contract: a working-tree diff that decomposes into ≥2
//! qualifying connected components on the historical co-change
//! graph fires Severity::Info. The sensor is a structural-fingerprint
//! proxy for the failure mode Herzig & Zeller (2013) identified;
//! the Wilson-symmetric edge metric inherits COUPLING's
//! small-sample treatment so single-commit fixtures don't fire
//! spurious cluster boundaries.

mod common;

use common::{commit_all, init_repo, write, DAY};
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
        format: Format::Json,
        ignores: Vec::new(),
        config: None,
        verbose: false,
        coupling_threshold: None,
        blast_radius_threshold: None,
        gate: Gate::None,
        // Per-key gate state would otherwise carry across two
        // tests in the same binary; --no-dedup keeps each fixture
        // hermetic.
        no_dedup: true,
    }
}

fn run_review(repo: &std::path::Path, args: ReviewArgs) -> Vec<u8> {
    let (res, stdout, _) = common::with_cwd(repo, |so, se| {
        mokumokuren::commands::review::run(&args, None, so, se)
    });
    res.expect("review run");
    stdout
}

fn cohesion_count(stdout: &[u8]) -> usize {
    let v: Value = serde_json::from_slice(stdout).expect("valid JSON");
    v["findings"]
        .as_array()
        .expect("findings array")
        .iter()
        .filter(|f| f["layer"] == "cohesion")
        .count()
}

/// Build a fixture with two clearly disjoint historical clusters:
///
/// - Cluster A: `auth/{login,session,token}.ts` — co-change 5 of
///   5 commits. Strong intra-cluster Wilson on each pair.
/// - Cluster B: `billing/{invoice,plan}.ts` — co-change 4 of 4
///   commits. Strong intra-cluster Wilson.
///
/// No A↔B co-edits, so the historical co-change graph has two
/// disjoint components on the changed_set when both clusters are
/// touched in one diff.
fn build_two_cluster_fixture(repo: &std::path::Path, now: i64) {
    init_repo(repo);

    for round in 0..5_i64 {
        write(
            repo,
            "auth/login.ts",
            &format!("export const a = {round};\n"),
        );
        write(
            repo,
            "auth/session.ts",
            &format!("export const b = {round};\n"),
        );
        write(
            repo,
            "auth/token.ts",
            &format!("export const c = {round};\n"),
        );
        commit_all(
            repo,
            &format!("auth round {round}"),
            now - (40 - round) * DAY,
        );
    }

    for round in 0..4_i64 {
        write(
            repo,
            "billing/invoice.ts",
            &format!("export const d = {round};\n"),
        );
        write(
            repo,
            "billing/plan.ts",
            &format!("export const e = {round};\n"),
        );
        commit_all(
            repo,
            &format!("billing round {round}"),
            now - (20 - round) * DAY,
        );
    }
}

#[serial(cwd)]
#[test]
fn two_cluster_diff_fires_cohesion() {
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    build_two_cluster_fixture(dir.path(), now);

    // Tangle-style diff: touch ≥2 files of each historical cluster
    // in one working-tree change. Expectation: COHESION fires.
    write(dir.path(), "auth/login.ts", "export const a = 99;\n");
    write(dir.path(), "auth/session.ts", "export const b = 99;\n");
    write(dir.path(), "billing/invoice.ts", "export const d = 99;\n");
    write(dir.path(), "billing/plan.ts", "export const e = 99;\n");

    let stdout = run_review(dir.path(), review_args());
    assert!(
        cohesion_count(&stdout) >= 1,
        "diff spanning two distinct co-change clusters must fire \
         COHESION; got: {}",
        String::from_utf8_lossy(&stdout)
    );
}

#[serial(cwd)]
#[test]
fn cohesion_finding_is_warn_severity() {
    // v0.7: COHESION is Warn (was Info pre-v0.7). Empirical
    // grounding: MSR 2026 LGTM — auto-merged PRs are smaller and
    // more focused than non-auto-merged ones across the AIDev
    // corpus.
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    build_two_cluster_fixture(dir.path(), now);

    write(dir.path(), "auth/login.ts", "export const a = 99;\n");
    write(dir.path(), "auth/session.ts", "export const b = 99;\n");
    write(dir.path(), "billing/invoice.ts", "export const d = 99;\n");
    write(dir.path(), "billing/plan.ts", "export const e = 99;\n");

    let stdout = run_review(dir.path(), review_args());
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");
    let cohesion: Vec<&Value> = v["findings"]
        .as_array()
        .expect("findings array")
        .iter()
        .filter(|f| f["layer"] == "cohesion")
        .collect();
    assert!(!cohesion.is_empty(), "no COHESION fired: {v:#}");
    for f in &cohesion {
        assert_eq!(
            f["severity"], "warn",
            "COHESION must be Warn in v0.7; got {f:#}"
        );
    }
}

#[serial(cwd)]
#[test]
fn cohesion_warn_triggers_gate_warn_exit_code() {
    use mokumokuren::Verdict;

    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    build_two_cluster_fixture(dir.path(), now);

    write(dir.path(), "auth/login.ts", "export const a = 99;\n");
    write(dir.path(), "auth/session.ts", "export const b = 99;\n");
    write(dir.path(), "billing/invoice.ts", "export const d = 99;\n");
    write(dir.path(), "billing/plan.ts", "export const e = 99;\n");

    let mut args = review_args();
    args.gate = Gate::Warn;
    let (verdict, _stdout, _stderr) = common::with_cwd(dir.path(), |so, se| {
        mokumokuren::commands::review::run(&args, None, so, se)
    });
    assert_eq!(
        verdict.expect("review run"),
        Verdict::GateTriggered,
        "Gate::Warn must trip on a COHESION Warn",
    );
}

#[serial(cwd)]
#[test]
fn cohesion_json_block_carries_full_cluster_split() {
    // v0.7: structured `cohesion.tangles[].clusters[]` block with
    // the full per-cluster decomposition. Order is deterministic
    // (within-cluster: lex; cluster list: by smallest path).
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    build_two_cluster_fixture(dir.path(), now);

    write(dir.path(), "auth/login.ts", "export const a = 99;\n");
    write(dir.path(), "auth/session.ts", "export const b = 99;\n");
    write(dir.path(), "billing/invoice.ts", "export const d = 99;\n");
    write(dir.path(), "billing/plan.ts", "export const e = 99;\n");

    let stdout = run_review(dir.path(), review_args());
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");
    let block = v["cohesion"]
        .as_object()
        .expect("cohesion block must be present when COHESION fires");
    let tangles = block["tangles"].as_array().expect("tangles array");
    assert_eq!(tangles.len(), 1, "one cohesion fire → one tangle entry");
    let clusters = tangles[0]["clusters"]
        .as_array()
        .expect("clusters array")
        .iter()
        .map(|c| {
            c.as_array()
                .expect("cluster")
                .iter()
                .map(|p| p.as_str().expect("path string").to_owned())
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    assert_eq!(clusters.len(), 2, "two clusters expected; got {clusters:?}");
    // Sort-by-smallest-path: auth/* < billing/*.
    assert!(
        clusters[0][0].starts_with("auth/"),
        "first cluster should start with auth/; got {clusters:?}"
    );
    assert!(
        clusters[1][0].starts_with("billing/"),
        "second cluster should start with billing/; got {clusters:?}"
    );
}

#[serial(cwd)]
#[test]
fn cohesion_block_absent_when_no_tangle() {
    // Single-cluster diff: structured `cohesion` block must be
    // entirely absent (additive shape — Option::is_none skips
    // serialization).
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    build_two_cluster_fixture(dir.path(), now);

    write(dir.path(), "auth/login.ts", "export const a = 99;\n");
    write(dir.path(), "auth/session.ts", "export const b = 99;\n");

    let stdout = run_review(dir.path(), review_args());
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");
    assert!(
        v.get("cohesion").is_none(),
        "cohesion block must be absent when no tangle qualifies; got: {v:#}"
    );
}

#[serial(cwd)]
#[test]
fn cohesion_repeat_fire_is_deduped_by_monotonic_signal() {
    // v0.7: COHESION carries a MonotonicSignal keyed by canonical
    // cluster signature with axes [cluster_count, total_files]. A
    // second run against the *same* tangled diff with dedup ON
    // must not re-fire (no axis worsened, key unchanged).
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    build_two_cluster_fixture(dir.path(), now);

    write(dir.path(), "auth/login.ts", "export const a = 99;\n");
    write(dir.path(), "auth/session.ts", "export const b = 99;\n");
    write(dir.path(), "billing/invoice.ts", "export const d = 99;\n");
    write(dir.path(), "billing/plan.ts", "export const e = 99;\n");

    let mut a1 = review_args();
    a1.no_dedup = false;
    let mut a2 = review_args();
    a2.no_dedup = false;
    let stdout1 = run_review(dir.path(), a1);
    assert!(
        cohesion_count(&stdout1) >= 1,
        "first run must fire COHESION; got: {}",
        String::from_utf8_lossy(&stdout1)
    );
    let stdout2 = run_review(dir.path(), a2);
    assert_eq!(
        cohesion_count(&stdout2),
        0,
        "repeat fire of identical cluster signature must be \
         suppressed by MonotonicSignal; got: {}",
        String::from_utf8_lossy(&stdout2)
    );
}

#[serial(cwd)]
#[test]
fn one_cluster_diff_does_not_fire_cohesion() {
    // The cluster-A files are *all* in the diff but no cluster-B
    // file is touched. The graph has one component on the
    // changed_set; COHESION must stay quiet.
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    build_two_cluster_fixture(dir.path(), now);

    write(dir.path(), "auth/login.ts", "export const a = 99;\n");
    write(dir.path(), "auth/session.ts", "export const b = 99;\n");
    write(dir.path(), "auth/token.ts", "export const c = 99;\n");

    let stdout = run_review(dir.path(), review_args());
    assert_eq!(
        cohesion_count(&stdout),
        0,
        "single-cluster diff must not fire COHESION; got: {}",
        String::from_utf8_lossy(&stdout)
    );
}

#[serial(cwd)]
#[test]
fn greenfield_only_diff_does_not_fire_cohesion() {
    // Brand-new files with no history: every "component" the
    // graph produces is a singleton greenfield path. The fire
    // condition explicitly drops greenfield singletons before the
    // cluster count, so COHESION must stay quiet — the agent
    // gets the GREENFIELD signal instead.
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    init_repo(dir.path());
    write(dir.path(), "src/seed.ts", "export const x = 1;\n");
    commit_all(dir.path(), "seed", now - 5 * DAY);

    write(dir.path(), "feature/new1.ts", "export const a = 1;\n");
    write(dir.path(), "feature/new2.ts", "export const b = 1;\n");
    write(dir.path(), "feature/new3.ts", "export const c = 1;\n");

    let stdout = run_review(dir.path(), review_args());
    assert_eq!(
        cohesion_count(&stdout),
        0,
        "all-greenfield diff must not fire COHESION; got: {}",
        String::from_utf8_lossy(&stdout)
    );
}

#[serial(cwd)]
#[test]
fn min_files_per_cluster_blocks_singleton_clusters() {
    // One clear historical cluster (auth/{a,b,c}) plus one
    // unrelated lone file. The lone file is a singleton component;
    // with default `min_files_per_cluster = 2` it shouldn't count
    // toward the ≥2-cluster fire condition.
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    init_repo(dir.path());

    for round in 0..5_i64 {
        write(
            dir.path(),
            "auth/login.ts",
            &format!("export const a = {round};\n"),
        );
        write(
            dir.path(),
            "auth/session.ts",
            &format!("export const b = {round};\n"),
        );
        write(
            dir.path(),
            "auth/token.ts",
            &format!("export const c = {round};\n"),
        );
        commit_all(
            dir.path(),
            &format!("auth round {round}"),
            now - (40 - round) * DAY,
        );
    }
    // A lone file with its own committed history but no coupling
    // to the auth cluster.
    for round in 0..3_i64 {
        write(
            dir.path(),
            "lone/util.ts",
            &format!("export const u = {round};\n"),
        );
        commit_all(
            dir.path(),
            &format!("lone util {round}"),
            now - (10 - round) * DAY,
        );
    }

    write(dir.path(), "auth/login.ts", "export const a = 99;\n");
    write(dir.path(), "auth/session.ts", "export const b = 99;\n");
    write(dir.path(), "lone/util.ts", "export const u = 99;\n");

    let stdout = run_review(dir.path(), review_args());
    assert_eq!(
        cohesion_count(&stdout),
        0,
        "singleton lone-file cluster must not satisfy \
         min_files_per_cluster=2; got: {}",
        String::from_utf8_lossy(&stdout)
    );
}
