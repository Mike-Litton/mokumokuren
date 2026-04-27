//! `mmk eval` — sample N recent commits, run review against each,
//! aggregate a noise-floor report. Adoption tool.

mod common;

use common::{build_coupling_fixture, commit_all, init_repo, write, CWD_LOCK, DAY};
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
        learn: false,
        replay: false,
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
    assert!(
        v["wilson_lower_buckets"].is_object(),
        "eval reports COUPLING distribution as Wilson 95% lower bound buckets; got: {v}"
    );
}

/// Fixture mirroring the lockstep-version-bump noise pattern:
/// three unrelated subject files (`svc-a/svc-b/svc-c`), each with
/// enough history to fire COUPLING. A common partner (`CHANGELOG.md`)
/// gets touched alongside each subject often, plus dozens of
/// solo-CHANGELOG commits. The result is high *partner breadth*
/// (CHANGELOG fires across all 3 subjects) AND low inverse
/// conditional probability (`P(subject | CHANGELOG)` is small for
/// every subject) — the exact signature `--learn` is built to spot.
fn build_learn_noise_fixture(repo: &std::path::Path, now: i64) {
    init_repo(repo);
    // Background CHANGELOG churn — 30 solo CHANGELOG commits push
    // commits_touching(CHANGELOG) high enough that
    // P(subject | CHANGELOG) lands well under the 0.10 ceiling.
    for i in 0..30 {
        write(repo, "CHANGELOG.md", &format!("entry {i}\n"));
        commit_all(repo, &format!("changelog {i}"), now - (60 - i) * DAY);
    }
    // Three subjects. Each gets 1 solo commit (the one --learn must
    // see fire) followed by 4 (subject + CHANGELOG) co-changes.
    for (subject, day_base) in [("svc-a.rs", 25), ("svc-b.rs", 18), ("svc-c.rs", 10)] {
        write(repo, subject, "v0\n");
        commit_all(repo, &format!("seed {subject}"), now - day_base * DAY);
        for i in 1..=4 {
            // Distinct body each iteration — without this `git add -A`
            // would skip identical writes and the co-change would
            // silently degenerate into a CHANGELOG-only commit.
            write(repo, subject, &format!("v0\nv{i}\n"));
            write(repo, "CHANGELOG.md", &format!("entry post-{subject}-{i}\n"));
            commit_all(
                repo,
                &format!("co-change {subject} {i}"),
                now - (day_base - i) * DAY,
            );
        }
    }
}

#[test]
fn eval_learn_suggests_partners_with_high_breadth() {
    // --learn must surface the cross-subject CHANGELOG pattern.
    // The fixture is shaped so CHANGELOG fires COUPLING for every
    // subject (via the solo-subject commits) but P(subject |
    // CHANGELOG) is low (because of the background CHANGELOG
    // churn). Locks both axes of the heuristic in one test.
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    build_learn_noise_fixture(dir.path(), now);

    let mut args = eval_args();
    args.learn = true;
    args.sample = 50;
    let stdout = run_in(dir.path(), args);
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON report");

    let suggestions = v["learn_suggestions"]
        .as_array()
        .expect("learn_suggestions array present when --learn is set");
    let partners: Vec<&str> = suggestions
        .iter()
        .map(|s| s["partner"].as_str().unwrap_or(""))
        .collect();
    assert!(
        partners.contains(&"CHANGELOG.md"),
        "CHANGELOG.md should be flagged: it fires across 3 subjects with low \
         P(subject | partner). got: {partners:?}"
    );
    let cl = suggestions
        .iter()
        .find(|s| s["partner"] == "CHANGELOG.md")
        .unwrap();
    assert!(
        cl["subject_count"].as_u64().unwrap_or(0) >= 3,
        "CHANGELOG should be supported by ≥3 subjects; got: {cl}"
    );
    let mean_inv = cl["mean_inverse_conditional_probability"]
        .as_f64()
        .unwrap_or(-1.0);
    assert!(
        (0.0..=1.0).contains(&mean_inv),
        "mean P(subject | CHANGELOG) must be a valid probability; got: {mean_inv}"
    );
}

#[test]
fn eval_without_learn_omits_suggestions_block() {
    // Negative: the suggestion block is opt-in. Default eval output
    // must not synthesize one even on a fixture where --learn would
    // fire — keeps the noise-floor report focused.
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    build_learn_noise_fixture(dir.path(), now);

    let stdout = run_in(dir.path(), eval_args());
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON report");
    assert!(
        v.get("learn_suggestions").is_none(),
        "default eval JSON must not include learn_suggestions; got: {v}"
    );
}

#[test]
fn eval_learn_text_mode_emits_toml_block() {
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    build_learn_noise_fixture(dir.path(), now);

    let mut args = eval_args();
    args.learn = true;
    args.format = Format::Text;
    let stdout = run_in(dir.path(), args);
    let text = String::from_utf8(stdout).unwrap();
    assert!(
        text.contains("[coupling]"),
        "text --learn output must include the [coupling] block header; got: {text}"
    );
    assert!(
        text.contains("ignore_partners"),
        "text --learn output must include ignore_partners list; got: {text}"
    );
    assert!(
        text.contains("CHANGELOG.md"),
        "text --learn output must list CHANGELOG.md; got: {text}"
    );
}

#[test]
fn eval_learn_emits_sensor_suggestion_blocks() {
    // Build a small TS-flavoured fixture so eval --learn has
    // something to chew on for the sensor suggestion blocks. The
    // fixture mixes file edits the COMPLEXITY/STRUCTURE collectors
    // can sample from.
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    init_repo(dir.path());
    let dlg_body = "import { z } from 'zod';\nexport function CreateADialog(){ return 1; }\n";
    write(dir.path(), "dlg/a.tsx", dlg_body);
    write(dir.path(), "dlg/b.tsx", dlg_body);
    write(dir.path(), "dlg/c.tsx", dlg_body);
    commit_all(dir.path(), "seed dlg", now - 4 * DAY);
    write(
        dir.path(),
        "dlg/a.tsx",
        "import { z } from 'zod';\nexport function CreateADialog(){ return 2; }\n",
    );
    commit_all(dir.path(), "edit dlg/a", now - 3 * DAY);

    let mut args = eval_args();
    args.learn = true;
    args.format = Format::Text;
    let stdout = run_in(dir.path(), args);
    let text = String::from_utf8(stdout).unwrap();
    assert!(
        text.contains("[sensor.structure]"),
        "text --learn output must include [sensor.structure] block; got: {text}"
    );
    assert!(
        text.contains("[sensor.complexity]"),
        "text --learn output must include [sensor.complexity] block; got: {text}"
    );
}

#[test]
fn eval_replay_emits_per_layer_histogram() {
    // --replay populates replay_histogram with per-layer entries: at
    // least the layer name, fire_rate ∈ [0, 1], commits_with_fire,
    // distinct_paths, severity mix. The coupling fixture fires
    // COUPLING on the a-only commits, so COUPLING should land in the
    // histogram with a non-zero fire rate.
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    build_coupling_fixture(dir.path(), now);

    let mut args = eval_args();
    args.replay = true;
    let stdout = run_in(dir.path(), args);
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON report");

    let hist = v["replay_histogram"]
        .as_object()
        .expect("replay_histogram block must be present when --replay is set");
    let sampled = hist["commits_sampled"]
        .as_u64()
        .expect("commits_sampled must be a number");
    assert!(sampled > 0, "must have sampled at least one commit");

    let layers = hist["layers"]
        .as_array()
        .expect("layers array must be present");
    assert!(
        !layers.is_empty(),
        "fixture fires COUPLING — at least one layer entry expected; got: {layers:?}"
    );
    let coupling = layers
        .iter()
        .find(|l| l["layer"] == "coupling")
        .expect("COUPLING layer entry must be present");
    let fire_rate = coupling["fire_rate"]
        .as_f64()
        .expect("fire_rate is a number");
    assert!(
        (0.0..=1.0).contains(&fire_rate) && fire_rate > 0.0,
        "COUPLING fire_rate must be a positive fraction; got: {fire_rate}"
    );
    assert!(
        coupling["distinct_paths"].as_u64().unwrap_or(0) >= 1,
        "COUPLING must surface at least one distinct path; got: {coupling}"
    );
    assert!(
        coupling.get("severity").is_some(),
        "severity mix must be present per layer; got: {coupling}"
    );
}

#[test]
fn eval_replay_text_emits_histogram_block() {
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    build_coupling_fixture(dir.path(), now);

    let mut args = eval_args();
    args.replay = true;
    args.format = Format::Text;
    let stdout = run_in(dir.path(), args);
    let text = String::from_utf8(stdout).unwrap();
    assert!(
        text.contains("replay histogram"),
        "text --replay output must include the histogram heading; got: {text}"
    );
    assert!(
        text.contains("fire_rate"),
        "text --replay output must include the fire_rate column; got: {text}"
    );
}

#[test]
fn eval_replay_omitted_by_default() {
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    build_coupling_fixture(dir.path(), now);

    let stdout = run_in(dir.path(), eval_args());
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON report");
    assert!(
        v.get("replay_histogram").is_none(),
        "default eval JSON must not include replay_histogram; got: {v}"
    );
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
