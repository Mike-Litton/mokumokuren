//! `mmk audit` — static codebase snapshot integration tests.
//!
//! Asserts the audit command's contract:
//! - walks every health-eligible TS/TSX/JS/JSX file at HEAD,
//! - emits per-file STRUCTURE / COMPLEXITY / non-delta HEALTH findings,
//! - never emits HOTSPOT / COUPLING / DRIFT / BUDGET (those are
//!   diff- / history-dependent and intentionally skipped).
//!
//! All assertions go through `serde_json::Value` against
//! `--format json` so the test is robust to text-format reflow.

mod common;

use common::{commit_all, init_repo, write, DAY};
use mokumokuren::args::{AuditArgs, Format, Gate};
use serde_json::Value;
use serial_test::serial;
use tempfile::TempDir;

const fn audit_args() -> AuditArgs {
    AuditArgs {
        format: Format::Json,
        ignores: Vec::new(),
        config: None,
        verbose: false,
        gate: Gate::None,
    }
}

fn run_audit(repo: &std::path::Path, args: AuditArgs) -> Vec<u8> {
    let (res, stdout, _) = common::with_cwd(repo, |so, se| {
        mokumokuren::commands::audit::run(&args, so, se)
    });
    res.expect("audit run");
    stdout
}

fn write_audit_config(repo: &std::path::Path) {
    write(
        repo,
        "mokumokuren.toml",
        r#"[health.ts]
enabled = true
patterns = ["registration", "service", "test_pair", "broad_exception", "broad_catch_debt"]
"#,
    );
}

#[serial(cwd)]
#[test]
fn audit_emits_broad_catch_debt_and_no_history_layers() {
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    init_repo(dir.path());
    write_audit_config(dir.path());

    // File 1: clean — no broad handlers, no structural divergence.
    write(
        dir.path(),
        "src/clean.ts",
        "export function f() { return 1; }\n",
    );
    // File 2: two empty-body broad catches at HEAD. Audit reports
    // them; review never would (delta = 0).
    write(
        dir.path(),
        "src/debt.ts",
        "export function f() { try { g(); } catch {} }\n\
         export function h() { try { g(); } catch (e) {} }\n",
    );
    commit_all(dir.path(), "init", now - 5 * DAY);

    let stdout = run_audit(dir.path(), audit_args());
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");

    let findings = v["findings"].as_array().cloned().unwrap_or_default();

    // BroadCatchDebt fires on debt.ts.
    let debt_findings: Vec<&Value> = findings
        .iter()
        .filter(|f| {
            f["layer"] == "health" && f["message"].as_str().is_some_and(|m| m.contains("debt.ts"))
        })
        .collect();
    assert!(
        debt_findings.iter().any(|f| f["message"]
            .as_str()
            .is_some_and(|m| m.contains("broad catch handler"))),
        "expected BroadCatchDebt finding on debt.ts; got: {}",
        String::from_utf8_lossy(&stdout)
    );

    // Structured `health.matches[]` block must carry the same finding
    // with the queryable detail payload — the `findings[]` shape only
    // surfaces the rendered message string. Two empty-body catches in
    // debt.ts mean count = 2.
    let matches = v["health"]["matches"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let debt_match = matches
        .iter()
        .find(|m| {
            m["subject"]
                .as_str()
                .is_some_and(|s| s.ends_with("debt.ts"))
        })
        .unwrap_or_else(|| {
            panic!(
                "expected structured health.matches[] entry for debt.ts; got: {}",
                String::from_utf8_lossy(&stdout)
            )
        });
    assert_eq!(debt_match["pattern"], "broad_catch_debt");
    assert_eq!(debt_match["detail"]["count"].as_u64(), Some(2));
    assert!(v["health"]["patterns_evaluated"].is_array());

    // No HOTSPOT / COUPLING / DRIFT / BUDGET — audit skips them.
    for f in &findings {
        let layer = f["layer"].as_str().unwrap_or("");
        assert!(
            !matches!(layer, "hotspot" | "coupling" | "drift" | "budget"),
            "audit must not emit history-dependent layer `{layer}`; got: {}",
            String::from_utf8_lossy(&stdout)
        );
    }
}

#[serial(cwd)]
#[test]
fn audit_clean_repo_emits_no_signal_line_in_text_mode() {
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    init_repo(dir.path());
    write_audit_config(dir.path());
    write(
        dir.path(),
        "src/clean.ts",
        "export function f() { return 1; }\n",
    );
    commit_all(dir.path(), "init", now - 5 * DAY);

    let mut text_args = audit_args();
    text_args.format = Format::Text;
    let stdout = run_audit(dir.path(), text_args);
    let s = String::from_utf8_lossy(&stdout);
    assert!(
        s.contains("[no actionable signal]") && s.contains("audited"),
        "expected NO_SIGNAL_PREFIX line on clean repo; got: {s}"
    );
}

#[serial(cwd)]
#[test]
fn audit_skips_evasion_delta_pattern() {
    // Repo accumulates broad handlers at HEAD; review-mode EVASION
    // would not fire (delta = 0), but BroadCatchDebt should. Critically,
    // no BroadException finding should appear (audit strips the
    // delta-only pattern).
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    init_repo(dir.path());
    write_audit_config(dir.path());
    write(
        dir.path(),
        "src/foo.ts",
        "export function f() { try { g(); } catch (e) {} }\n",
    );
    commit_all(dir.path(), "init", now - 5 * DAY);

    let stdout = run_audit(dir.path(), audit_args());
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");
    let findings = v["findings"].as_array().cloned().unwrap_or_default();

    // BroadException's prose mentions "adds N broad exception handler[s]
    // not in HEAD"; BroadCatchDebt's mentions "broad catch handler".
    // Audit must surface only the latter.
    for f in &findings {
        let msg = f["message"].as_str().unwrap_or("");
        assert!(
            !msg.contains("adds ") || !msg.contains("not in HEAD"),
            "audit must not emit EVASION (delta-mode); got finding: {msg}"
        );
    }
}
