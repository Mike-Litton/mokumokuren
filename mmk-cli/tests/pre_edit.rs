//! `mmk pre-edit <PATH>` — emit findings *before* editing a file.
//! The PreToolUse:Edit hook target. Composes hotspot rank +
//! coupling lookup into the unified findings format.
//!
//! DRIFT findings are wired in once Step 4 lands `compute_drift`.

mod common;

use common::{build_coupling_fixture, commit_all, init_repo, write, CWD_LOCK, DAY};
use mokumokuren::args::{Format, Gate, PreEditArgs};
use serde_json::Value;
use std::path::PathBuf;
use tempfile::TempDir;

fn pre_edit_args(path: &str) -> PreEditArgs {
    PreEditArgs {
        path: PathBuf::from(path),
        since: "60days".into(),
        top: 20,
        format: Format::Json,
        ignores: Vec::new(),
        config: None,
        verbose: false,
        coupling_threshold: None,
        blast_radius_threshold: None,
        drift_sessions: 0,
        gate: Gate::None,
        // See review.rs::review_args for why dedup is disabled in
        // these tests.
        no_dedup: true,
    }
}

fn run_in(repo: &std::path::Path, args: PreEditArgs) -> Vec<u8> {
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

#[test]
fn pre_edit_emits_hotspot_when_path_is_top_n() {
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    build_coupling_fixture(dir.path(), now);

    let stdout = run_in(dir.path(), pre_edit_args("core/a.rs"));
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");

    let hotspot: Vec<&Value> = v["findings"]
        .as_array()
        .expect("findings array")
        .iter()
        .filter(|f| f["layer"] == "hotspot")
        .collect();
    assert!(
        !hotspot.is_empty(),
        "core/a.rs is the canonical fixture hotspot — must fire HOTSPOT; got: {}",
        v["findings"]
    );
    assert!(hotspot
        .iter()
        .any(|f| f["message"].as_str().unwrap_or("").contains("core/a.rs")));
}

#[test]
fn pre_edit_emits_coupling_for_partners_above_threshold() {
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    build_coupling_fixture(dir.path(), now);

    let stdout = run_in(dir.path(), pre_edit_args("core/a.rs"));
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");

    let coupling: Vec<&Value> = v["findings"]
        .as_array()
        .expect("findings array")
        .iter()
        .filter(|f| f["layer"] == "coupling")
        .collect();
    assert!(
        !coupling.is_empty(),
        "core/a.rs has Wilson 95% lower ≈ 0.23 with core/b.rs — must fire COUPLING informational; got: {}",
        v["findings"]
    );
    let mentions_b = coupling
        .iter()
        .any(|f| f["message"].as_str().unwrap_or("").contains("core/b.rs"));
    assert!(
        mentions_b,
        "COUPLING finding should list core/b.rs as the historical partner; got: {coupling:?}"
    );
}

#[test]
fn pre_edit_emits_ok_finding_when_no_signal_fires() {
    // When no layer (HOTSPOT/COUPLING/HEALTH/DRIFT) fires, pre-edit
    // emits a single Severity::Ok finding so the agent can tell
    // "mmk had nothing to say" from "mmk wasn't consulted."
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    init_repo(dir.path());
    // quiet.rs lives in its own seed commit so it never co-changes
    // with anything — zero couples. noisy.rs gets all the churn,
    // which makes it the top hotspot, leaving quiet.rs well outside
    // the rank-1 floor.
    write(dir.path(), "quiet.rs", "q\n");
    commit_all(dir.path(), "seed quiet", now - 31 * DAY);
    write(dir.path(), "noisy.rs", "n\n");
    commit_all(dir.path(), "seed noisy", now - 30 * DAY);
    for i in 0..6 {
        write(dir.path(), "noisy.rs", &format!("n{i}\n"));
        commit_all(dir.path(), &format!("noisy {i}"), now - (29 - i) * DAY);
    }

    // Look up quiet.rs with a tight top — well below the rank it'd
    // claim against noisy.rs.
    let mut args = pre_edit_args("quiet.rs");
    args.top = 1;
    let stdout = run_in(dir.path(), args);
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");

    let findings = v["findings"].as_array().expect("findings array");
    assert_eq!(
        findings.len(),
        1,
        "no-signal file must surface exactly one OK finding; got: {findings:?}"
    );
    let f = &findings[0];
    assert_eq!(
        f["severity"], "ok",
        "the no-signal fall-through has severity ok; got: {f}"
    );
    assert_eq!(
        f["layer"], "coupling",
        "absence-of-signal lives under the coupling layer; got: {f}"
    );
    let msg = f["message"].as_str().unwrap_or("");
    assert!(
        msg.contains("quiet.rs"),
        "OK finding must name the queried path; got: {msg}"
    );
    assert!(
        msg.contains("no signal"),
        "message should call out the absence of signal; got: {msg}"
    );
}

#[test]
fn pre_edit_no_ok_finding_when_coupling_already_fires() {
    // The OK finding is a fall-through, not an addition. If the
    // file is rich enough to fire COUPLING (or any other layer),
    // the OK finding must not appear.
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    common::build_coupling_fixture(dir.path(), now);

    let stdout = run_in(dir.path(), pre_edit_args("core/a.rs"));
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");

    let findings = v["findings"].as_array().expect("findings array");
    let any_ok = findings.iter().any(|f| f["severity"] == "ok");
    assert!(
        !any_ok,
        "OK fall-through must not fire when other findings exist; got: {findings:?}"
    );
}

#[test]
fn pre_edit_emits_ok_finding_for_lone_file_with_no_partners() {
    // A file with history but no co-edited partners and no other
    // signal also gets the OK fall-through — the agent should know
    // mmk ran and found nothing rather than infer "no output = pass."
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    init_repo(dir.path());
    // Seven solo commits to lonely.rs — no partner ever.
    for i in 0..7 {
        write(dir.path(), "lonely.rs", &format!("l{i}\n"));
        commit_all(dir.path(), &format!("lonely {i}"), now - (30 - i) * DAY);
    }
    // Heavy unrelated churn on noisy.rs so lonely.rs falls below
    // the top-1 hotspot bar — we want the no-partners path, not
    // the hotspot-rank path.
    write(dir.path(), "noisy.rs", "n\n");
    commit_all(dir.path(), "seed noisy", now - 25 * DAY);
    for i in 0..20 {
        write(dir.path(), "noisy.rs", &format!("n{i}\nx{i}\ny{i}\nz{i}\n"));
        commit_all(dir.path(), &format!("noisy {i}"), now - (24 - i) * DAY);
    }

    let mut args = pre_edit_args("lonely.rs");
    args.top = 1;
    let stdout = run_in(dir.path(), args);
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");

    let findings = v["findings"].as_array().expect("findings array");
    assert_eq!(
        findings.len(),
        1,
        "lonely file must surface exactly one OK finding; got: {findings:?}"
    );
    let f = &findings[0];
    assert_eq!(f["severity"], "ok");
    assert_eq!(f["layer"], "coupling");
}

#[test]
fn pre_edit_with_drift_sessions_runs_and_shapes_findings() {
    use std::fmt::Write as _;
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    init_repo(dir.path());
    // Reasonable churn fixture so find_session_boundaries can pick K
    // distinct linear-chunk boundaries. We assert the wiring works
    // (no error, drift findings — when present — are filtered to
    // args.path); pure-function climb correctness is locked in
    // mmk-core/tests/drift.rs.
    write(dir.path(), "target.rs", "t\n");
    write(dir.path(), "other.rs", "o\n");
    commit_all(dir.path(), "seed", now - 30 * DAY);
    for i in 0..10 {
        let mut body = String::new();
        for n in 0..(5 + i) {
            writeln!(body, "target{n}-r{i}").unwrap();
        }
        write(dir.path(), "target.rs", &body);
        commit_all(
            dir.path(),
            &format!("c{i}"),
            now - (28 - i64::from(i)) * DAY,
        );
    }

    let mut args = pre_edit_args("target.rs");
    args.drift_sessions = 5;
    args.top = 5;
    let stdout = run_in(dir.path(), args);
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");

    let findings = v["findings"].as_array().expect("findings array");
    // Any drift finding emitted must mention target.rs (pre-edit
    // filters DRIFT to the queried path; other files' drift signals
    // are out of scope for this view).
    for f in findings.iter().filter(|f| f["layer"] == "drift") {
        assert!(
            f["message"].as_str().unwrap_or("").contains("target.rs"),
            "DRIFT findings in pre-edit must concern the queried path; got: {f}"
        );
    }
}

#[test]
fn pre_edit_emits_health_test_pair_finding_when_partner_exists() {
    // With [health.ts] enabled, pre-edit on a TS impl file whose
    // `.test.ts` partner exists must surface a HEALTH info finding
    // pointing at the test partner.
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    init_repo(dir.path());
    write(dir.path(), "src/foo.ts", "export const foo = 1;\n");
    write(
        dir.path(),
        "src/foo.test.ts",
        "import {foo} from './foo';\n",
    );
    commit_all(dir.path(), "seed", now - 5 * common::DAY);

    std::fs::write(
        dir.path().join("mokumokuren.toml"),
        "[health.ts]\nenabled = true\npatterns = [\"test_pair\"]\n",
    )
    .unwrap();

    let stdout = run_in(dir.path(), pre_edit_args("src/foo.ts"));
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");

    let findings = v["findings"].as_array().expect("findings array");
    let health: Vec<&Value> = findings.iter().filter(|f| f["layer"] == "health").collect();
    assert!(
        !health.is_empty(),
        "test-pair pattern must fire when foo.test.ts exists; got: {findings:?}"
    );
    let mentions_test = health
        .iter()
        .any(|f| f["message"].as_str().unwrap_or("").contains("foo.test.ts"));
    assert!(
        mentions_test,
        "HEALTH finding must surface the test partner; got: {health:?}"
    );

    // The top-level health block must also be present, with the
    // pattern + related list mirrored structurally.
    let block = v["health"]
        .as_object()
        .expect("top-level health block must accompany Health findings");
    assert_eq!(
        block["patterns_evaluated"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["test_pair"]
    );
    let matches = block["matches"].as_array().unwrap();
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0]["pattern"], "test_pair");
    assert_eq!(matches[0]["subject"], "src/foo.ts");
}

#[test]
fn pre_edit_health_block_absent_when_disabled() {
    // Default pre-edit (no [health.ts]) emits no `health` block,
    // matching the schema's "optional, present only when fired"
    // rule.
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    init_repo(dir.path());
    write(dir.path(), "src/foo.ts", "export const foo = 1;\n");
    write(dir.path(), "src/foo.test.ts", "ok\n");
    commit_all(dir.path(), "seed", now - 5 * common::DAY);

    let stdout = run_in(dir.path(), pre_edit_args("src/foo.ts"));
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");
    assert!(
        v.get("health").is_none(),
        "health block must be absent when adapter is disabled; got: {v}"
    );
}

#[test]
fn pre_edit_says_new_file_for_untracked_subject() {
    // Pre-edit on a file that has never appeared in history must
    // distinguish "new file (no history)" from "no signal (N
    // commits…)" — the former is structural, the latter
    // statistical, and conflating them misleads agents working in
    // greenfield slices.
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    init_repo(dir.path());
    write(dir.path(), "seed.rs", "x\n");
    commit_all(dir.path(), "seed", now - 5 * DAY);

    // brand-new.rs has never been committed.
    let stdout = run_in(dir.path(), pre_edit_args("brand-new.rs"));
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");

    let findings = v["findings"].as_array().expect("findings array");
    assert_eq!(
        findings.len(),
        1,
        "untracked subject must surface exactly one OK finding; got: {findings:?}"
    );
    let msg = findings[0]["message"].as_str().unwrap_or("");
    assert!(
        msg.contains("new file (no history)"),
        "untracked subject's wording must say 'new file (no history)'; got: {msg}"
    );
    assert!(
        !msg.contains("no signal"),
        "untracked subject must not be reported as 'no signal'; got: {msg}"
    );
}

#[test]
fn pre_edit_emits_structure_for_directory_convention() {
    // 4 sibling .tsx files share `zod` and a Create*Dialog
    // template. Pre-edit on a brand-new sibling path must surface
    // a STRUCTURE Info finding listing the convention.
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    common::init_repo(dir.path());
    let body_a = "import { z } from 'zod';\nexport function CreateAwardDialog(){}\n";
    let body_g = "import { z } from 'zod';\nexport function CreateGoalDialog(){}\n";
    let body_j = "import { z } from 'zod';\nexport function CreateJobDialog(){}\n";
    let body_x = "import { z } from 'zod';\nexport function CreateXDialog(){}\n";
    common::write(dir.path(), "dlg/award.tsx", body_a);
    common::write(dir.path(), "dlg/goal.tsx", body_g);
    common::write(dir.path(), "dlg/job.tsx", body_j);
    common::write(dir.path(), "dlg/extra.tsx", body_x);
    common::commit_all(dir.path(), "seed", now - 5 * common::DAY);

    let stdout = run_in(dir.path(), pre_edit_args("dlg/new.tsx"));
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");

    let findings = v["findings"].as_array().expect("findings array");
    let structure: Vec<&Value> = findings
        .iter()
        .filter(|f| f["layer"] == "structure")
        .collect();
    assert!(
        !structure.is_empty(),
        "STRUCTURE must fire on a 4-sibling directory; got: {findings:?}"
    );
    let msg = structure[0]["message"].as_str().unwrap_or("");
    assert!(
        msg.contains("zod"),
        "STRUCTURE message must list the common import; got: {msg}"
    );
    assert!(
        msg.contains("Create*Dialog"),
        "STRUCTURE message must list the common export template; got: {msg}"
    );
}

#[test]
fn pre_edit_json_envelope_has_path_and_findings() {
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    build_coupling_fixture(dir.path(), now);

    let stdout = run_in(dir.path(), pre_edit_args("core/a.rs"));
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");

    assert_eq!(
        v["pre_edit"]["path"], "core/a.rs",
        "pre_edit.path must echo the queried path; got: {}",
        v["pre_edit"]
    );
    assert!(
        v["findings"].is_array(),
        "top-level findings array must be present; got: {v}"
    );
    assert!(v["schema_version"].is_string());
    assert!(v["crate_version"].is_string());
}
