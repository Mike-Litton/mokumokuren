//! `mmk session` — compares ranking on a session window (commits since
//! a base ref) against the full window, so an LLM agent can ask "what
//! shifted since I started?" Per the v0.2.0 plan, the base resolution
//! cascade is: explicit `--base` / `--since-commit` → merge-base with
//! `origin/main` → `main` → `origin/master` → `master` → `HEAD~1`,
//! warning whenever it falls back.

mod common;

use common::{build_session_fixture, commit_all, git, init_repo, write, CWD_LOCK, DAY};
use mokumokuren::args::{Format, SessionArgs};
use serde_json::Value;
use std::path::Path;
use tempfile::TempDir;

#[test]
fn session_summary_includes_window_and_session_blocks() {
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    build_session_fixture(dir.path(), now);

    let (stdout, _) = run_session(dir.path(), json_args());
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");

    assert!(v["files"].is_array(), "window ranking present");
    assert!(v["session_files"].is_array(), "session ranking present");
    assert!(v["session"].is_object(), "session delta block present");
    assert!(
        v["findings"].is_array(),
        "findings array present (possibly empty); got {:?}",
        v["findings"]
    );
}

#[test]
fn session_summary_diff_budget_finding_fires_on_oversize_session() {
    use std::fmt::Write as _;
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    init_repo(dir.path());
    write(dir.path(), "seed.rs", "x\n");
    commit_all(dir.path(), "seed", now - 30 * DAY);
    git(dir.path(), &["checkout", "-q", "-b", "feature"]);
    // One commit on the feature branch with line count well above
    // bulk.max_lines (1000) × 1 commit = 1000 budget.
    let mut huge = String::new();
    for n in 0..3000 {
        writeln!(huge, "line{n}").unwrap();
    }
    write(dir.path(), "seed.rs", &huge);
    commit_all(dir.path(), "blast", now - DAY);

    let mut args = json_args();
    args.base = Some("main".into());
    let (stdout, _) = run_session(dir.path(), args);
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");

    let any_budget = v["findings"]
        .as_array()
        .expect("findings array")
        .iter()
        .any(|f| f["layer"] == "budget");
    assert!(
        any_budget,
        "session aggregate exceeding bulk.max_lines × commits must emit BUDGET; got: {}",
        v["findings"]
    );
}

fn run_session(repo: &Path, args: SessionArgs) -> (Vec<u8>, Vec<u8>) {
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
    (stdout, stderr)
}

fn json_args() -> SessionArgs {
    SessionArgs {
        since: "60days".into(),
        // Tight top so the window's main-branch churn occupies the
        // entire top list, leaving feat/x.rs as a session-only entry.
        top: 3,
        format: Format::Json,
        base: None,
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

#[test]
fn explicit_base_resolves_via_explicit() {
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    build_session_fixture(dir.path(), now);

    let mut args = json_args();
    args.base = Some("main".into());

    let (stdout, _stderr) = run_session(dir.path(), args);
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");
    let session = v["session"].as_object().expect("session block");
    assert_eq!(
        session["base_resolved_via"], "explicit",
        "with --base supplied, base_resolved_via should be 'explicit'"
    );
    let entered: Vec<&str> = session["entered_top_n"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(
        entered.contains(&"feat/x.rs"),
        "feat/x.rs should be a session-only top-N entry; got {entered:?}"
    );
    let entropy = session["commit_entropy"].as_f64().unwrap();
    assert!(
        (0.0..=1.0).contains(&entropy),
        "commit_entropy must be normalized to [0, 1], got {entropy}"
    );
}

#[test]
fn since_commit_equals_merge_base_yields_same_output() {
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    build_session_fixture(dir.path(), now);

    // Resolve the merge-base SHA outside `mmk` so the test is
    // independent of `mmk`'s own resolution code.
    let mb_sha = std::process::Command::new("git")
        .args(["merge-base", "HEAD", "main"])
        .current_dir(dir.path())
        .output()
        .expect("git merge-base");
    assert!(mb_sha.status.success(), "git merge-base failed");
    let mb_sha = String::from_utf8(mb_sha.stdout).unwrap().trim().to_string();

    let mut args_a = json_args();
    args_a.base = Some("main".into());
    let (out_a, _) = run_session(dir.path(), args_a);

    let mut args_b = json_args();
    args_b.since_commit = Some(mb_sha);
    let (out_b, _) = run_session(dir.path(), args_b);

    let va: Value = serde_json::from_slice(&out_a).unwrap();
    let vb: Value = serde_json::from_slice(&out_b).unwrap();

    // The set of session-only entries should match.
    let entered_a = &va["session"]["entered_top_n"];
    let entered_b = &vb["session"]["entered_top_n"];
    assert_eq!(
        entered_a, entered_b,
        "--since-commit at merge-base SHA should produce the same entered_top_n as --base main"
    );
    assert_eq!(
        vb["session"]["base_resolved_via"], "since_commit",
        "since_commit should report itself as the resolution method"
    );
}

#[test]
fn detached_head_falls_back_to_head_minus_one_with_warning() {
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    build_session_fixture(dir.path(), now);

    // Drop main, master, origin/* — force the cascade to hit HEAD~1.
    git(dir.path(), &["branch", "-D", "main"]);
    git(dir.path(), &["checkout", "-q", "--detach", "HEAD"]);

    let (stdout, _stderr) = run_session(dir.path(), json_args());
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");
    let session = v["session"].as_object().expect("session block");
    assert_eq!(
        session["base_resolved_via"], "head_minus_one",
        "with no main/master/origin refs, fallback should be head_minus_one"
    );
    let warnings: Vec<&str> = v["repo"]["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap())
        .collect();
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("base") || w.contains("fallback")),
        "synthetic-base fallback should fire a warning; got {warnings:?}"
    );
}

// --- Step 4 (issue 3): synthetic-base gating contract. ---
//
// The agent's review noted that the synthetic-base fallback informs
// (warning + base_resolved_via field) but doesn't gate. The design
// is "inform, don't gate" — the harness is the gate — but the
// machine-readable contract a CI/CD pipeline or LLM harness needs to
// detect synthetic results and refuse them must be locked here.
//
// Orthogonality:
// - First test protects **agent mode** + **CI/CD mode**: a parser
//   keying on `base_resolved_via == "head_minus_one"` AND/OR
//   grep'ing the warning text for "fallback" must succeed.
// - Second test (the negative one) protects **CI/CD mode** against
//   false-positive review noise on a healthy `--base main`.

#[test]
fn synthetic_base_emits_machine_readable_resolution_field() {
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    build_session_fixture(dir.path(), now);

    // Force the cascade all the way to head_minus_one.
    git(dir.path(), &["branch", "-D", "main"]);
    git(dir.path(), &["checkout", "-q", "--detach", "HEAD"]);

    let (stdout, _stderr) = run_session(dir.path(), json_args());
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");

    // Machine-readable contract #1: the resolution field is the
    // exact literal that BaseResolvedVia::is_synthetic() flags.
    let resolved = v["session"]["base_resolved_via"]
        .as_str()
        .expect("base_resolved_via is a string");
    assert_eq!(
        resolved, "head_minus_one",
        "harness gate keys on this string; if the value drifts, every \
         consumer must re-pin"
    );

    // Machine-readable contract #2: the warning text is greppable
    // for either of the two stable substrings a CI report can scan
    // for. Not asserting a full warning string — that's prose and
    // can change — but the substrings must hold.
    let warnings: Vec<String> = v["repo"]["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap().to_string())
        .collect();
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("fallback") || w.contains("synthetic")),
        "warning text must contain `fallback` or `synthetic` so a CI \
         grep step can detect the unhealthy state; got {warnings:?}"
    );
}

#[test]
fn explicit_base_does_not_emit_synthetic_warning() {
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    build_session_fixture(dir.path(), now);

    // Healthy explicit base: `main` exists in the fixture, no fallback
    // expected. Asserts the *negative* of the synthetic-base case so
    // a human-review CI report doesn't see false alarms on every run.
    let mut args = json_args();
    args.base = Some("main".into());
    let (stdout, _stderr) = run_session(dir.path(), args);
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");

    let resolved = v["session"]["base_resolved_via"].as_str().unwrap();
    assert_eq!(
        resolved, "explicit",
        "explicit --base must report `explicit`, not a fallback method"
    );

    let warnings: Vec<String> = v["repo"]["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap().to_string())
        .collect();
    for w in &warnings {
        assert!(
            !w.contains("fallback") && !w.contains("synthetic"),
            "no fallback/synthetic warning should fire on a healthy \
             --base main run; got warning {w:?}. Other warnings are fine \
             (e.g. shallow-clone notices), but synthetic-base alarms are not."
        );
    }
}

// --- Step 3 (issue 1): session ranking must use base-epoch LOC. ---
//
// Orthogonality tag: protects **agent mode** (the relative_churn
// signal becomes meaningful) and **CI/CD mode** (a human reviewer
// can trust that "this file is X% churn over its size during the
// session").
//
// Fixture: a file (`core/big.rs`) is large at session-base, gets
// churned during the session, then is *truncated* at HEAD. With a
// HEAD-LOC denominator the relative_churn would collapse to
// session_weighted_churn / 5; with the base-LOC denominator (the
// chosen semantic) it stays session_weighted_churn / ~100.

/// Builds a fixture with `core/big.rs` at 100 lines on `main`,
/// churned heavily on `feature`, then truncated to 5 lines at HEAD.
/// `core/stable.rs` is sized similarly on main and remains unchanged
/// through session — control file.
fn build_loc_drift_fixture(repo: &Path, now: i64) {
    use std::fmt::Write as _;

    init_repo(repo);

    // main: introduce big.rs at 100 lines, stable.rs at 50 lines.
    let mut big_initial = String::new();
    for i in 0..100 {
        writeln!(big_initial, "orig{i}").unwrap();
    }
    write(repo, "core/big.rs", &big_initial);
    let mut stable = String::new();
    for i in 0..50 {
        writeln!(stable, "stable{i}").unwrap();
    }
    write(repo, "core/stable.rs", &stable);
    commit_all(repo, "main: seed", now - 30 * DAY);

    // Branch off; this is the session base.
    git(repo, &["checkout", "-q", "-b", "feature"]);

    // Churn big.rs in the session: append a chunk of new lines.
    let mut big_churned = String::new();
    for i in 0..100 {
        writeln!(big_churned, "orig{i}").unwrap();
    }
    for i in 0..40 {
        writeln!(big_churned, "session_added{i}").unwrap();
    }
    write(repo, "core/big.rs", &big_churned);
    commit_all(repo, "feat: append to big", now - 5 * DAY);

    // Truncate big.rs at HEAD to 5 lines — this is the post-session
    // refactor that makes HEAD-LOC misleading.
    write(repo, "core/big.rs", "kept0\nkept1\nkept2\nkept3\nkept4\n");
    commit_all(repo, "feat: truncate big", now - DAY);
}

#[test]
fn session_relative_churn_uses_base_epoch_loc() {
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    build_loc_drift_fixture(dir.path(), now);

    let mut args = json_args();
    args.base = Some("main".into());
    args.top = 20;
    let (stdout, _stderr) = run_session(dir.path(), args);
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");

    let session_files = v["session_files"].as_array().expect("session_files array");
    let big = session_files
        .iter()
        .find(|f| f["path"] == "core/big.rs")
        .expect("core/big.rs should be in session_files");

    // base-LOC for big.rs is 100; HEAD-LOC is 5. The fix forces
    // session_files[].loc to reflect the BASE epoch.
    let loc = big["loc"].as_u64().unwrap();
    assert!(
        (90..=110).contains(&loc),
        "session_files[].loc must be base-epoch LOC (≈100), not HEAD-LOC (5); got {loc}. \
         If this fails reading 5: the v0.2.0 bug is back — session ranking is using HEAD-LOC."
    );

    // relative_churn = weighted_churn / loc; with base-LOC ~100, it
    // must be at least 20× smaller than what HEAD-LOC=5 would give.
    let weighted = big["weighted_churn"].as_f64().unwrap();
    let relative = big["relative_churn"].as_f64().unwrap();
    let head_loc_relative = weighted / 5.0;
    assert!(
        relative < head_loc_relative / 10.0,
        "session relative_churn ({relative}) is suspiciously close to the \
         HEAD-LOC denominator value ({head_loc_relative}); base-LOC fix must \
         produce a substantially smaller ratio"
    );
    // And: relative ≈ weighted / loc (within float tolerance).
    let expected = weighted / (loc as f64);
    assert!(
        (relative - expected).abs() < 1e-6,
        "relative_churn ({relative}) should equal weighted_churn / loc ({expected})"
    );
}

#[test]
fn window_files_still_use_head_loc() {
    // Sanity: the top-level `files[]` (window ranking) keeps using
    // HEAD-LOC. Only session_files[] uses the base-LOC denominator.
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    build_loc_drift_fixture(dir.path(), now);

    let mut args = json_args();
    args.base = Some("main".into());
    args.top = 20;
    let (stdout, _stderr) = run_session(dir.path(), args);
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");

    let files = v["files"].as_array().expect("files array");
    let big = files
        .iter()
        .find(|f| f["path"] == "core/big.rs")
        .expect("core/big.rs should be in window files too");
    let loc = big["loc"].as_u64().unwrap();
    assert!(
        loc <= 10,
        "window files[].loc should be HEAD-LOC (5); got {loc}. \
         If this fails reading ~100: the window ranking has been changed too — \
         only session_files[] is supposed to use base-LOC."
    );
}
