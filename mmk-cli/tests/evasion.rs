//! EVASION sensor — newly-added broad TS/JS catch handlers.
//!
//! Locks the contract: a working-tree diff that adds a non-top-level
//! broad catch handler relative to HEAD fires `Severity::Warn` under
//! `Layer::Health` with `pattern = "broad_exception"`. Targets the
//! *"evasive repairs with try-except blocks"* failure mode named in
//! arXiv:2509.13941.

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

fn evasion_findings(stdout: &[u8]) -> Vec<Value> {
    let v: Value = serde_json::from_slice(stdout).expect("valid JSON");
    let matches = v["health"]["matches"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    matches
        .into_iter()
        .filter(|m| m["pattern"] == "broad_exception")
        .collect()
}

fn evasion_findings_count_in_findings(stdout: &[u8]) -> usize {
    let v: Value = serde_json::from_slice(stdout).expect("valid JSON");
    v["findings"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter(|f| {
            f["layer"] == "health"
                && f["severity"] == "warn"
                && f["message"]
                    .as_str()
                    .is_some_and(|m| m.contains("broad exception handler"))
        })
        .count()
}

/// Enable the broad_exception pattern via mokumokuren.toml. The
/// default mmk-config ships with it on, but `mokumokuren.toml` in
/// this fixture starts blank so we make the intent explicit.
fn write_health_config(repo: &std::path::Path) {
    write(
        repo,
        "mokumokuren.toml",
        r#"[health.ts]
enabled = true
patterns = ["registration", "service", "test_pair", "broad_exception"]
"#,
    );
}

#[serial(cwd)]
#[test]
fn newly_added_empty_catch_in_function_fires_evasion() {
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    init_repo(dir.path());
    write_health_config(dir.path());
    write(
        dir.path(),
        "src/foo.ts",
        "export function f() { try { g(); } catch (e) { throw e; } }\n",
    );
    commit_all(dir.path(), "init", now - 5 * DAY);

    // Working-tree edit: rethrow → empty catch. Net delta = +1 broad.
    write(
        dir.path(),
        "src/foo.ts",
        "export function f() { try { g(); } catch (e) {} }\n",
    );

    let stdout = run_review(dir.path(), review_args());
    let matches = evasion_findings(&stdout);
    assert_eq!(
        matches.len(),
        1,
        "broad-exception addition must fire EVASION; got: {}",
        String::from_utf8_lossy(&stdout)
    );
    assert!(
        evasion_findings_count_in_findings(&stdout) >= 1,
        "EVASION finding must surface in findings[] at severity=warn; got: {}",
        String::from_utf8_lossy(&stdout)
    );
}

#[serial(cwd)]
#[test]
fn unchanged_broad_catch_does_not_fire_evasion() {
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    init_repo(dir.path());
    write_health_config(dir.path());
    write(
        dir.path(),
        "src/foo.ts",
        "export function f() { try { g(); } catch (e) {} }\n",
    );
    commit_all(dir.path(), "init", now - 5 * DAY);

    // Working tree adds a comment but doesn't change the broad-handler count.
    write(
        dir.path(),
        "src/foo.ts",
        "// docs\nexport function f() { try { g(); } catch (e) {} }\n",
    );

    let stdout = run_review(dir.path(), review_args());
    let matches = evasion_findings(&stdout);
    assert!(
        matches.is_empty(),
        "no net-broad-handler delta must not fire EVASION; got: {}",
        String::from_utf8_lossy(&stdout)
    );
}

#[serial(cwd)]
#[test]
fn tsx_with_jsx_evasion_fires() {
    // .tsx must parse via the TSX grammar — the v0.7 prerequisite
    // bugfix. Without it the JSX would error and the broad handler
    // would be missed.
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    init_repo(dir.path());
    write_health_config(dir.path());
    write(
        dir.path(),
        "src/App.tsx",
        "export function App() { try { f(); } catch (e) { throw e; } return <div />; }\n",
    );
    commit_all(dir.path(), "init", now - 5 * DAY);

    write(
        dir.path(),
        "src/App.tsx",
        "export function App() { try { f(); } catch (e) {} return <div />; }\n",
    );

    let stdout = run_review(dir.path(), review_args());
    let matches = evasion_findings(&stdout);
    assert_eq!(
        matches.len(),
        1,
        ".tsx with JSX must be parsed correctly and EVASION must fire; got: {}",
        String::from_utf8_lossy(&stdout)
    );
}

#[serial(cwd)]
#[test]
fn js_subject_evasion_fires() {
    // EVASION on .js: the v0.7 cross-language coverage extension.
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    init_repo(dir.path());
    write_health_config(dir.path());
    write(
        dir.path(),
        "src/foo.js",
        "export function f() { try { g(); } catch (e) { throw e; } }\n",
    );
    commit_all(dir.path(), "init", now - 5 * DAY);

    write(
        dir.path(),
        "src/foo.js",
        "export function f() { try { g(); } catch (e) {} }\n",
    );

    let stdout = run_review(dir.path(), review_args());
    let matches = evasion_findings(&stdout);
    assert_eq!(
        matches.len(),
        1,
        ".js EVASION must fire; got: {}",
        String::from_utf8_lossy(&stdout)
    );
}
