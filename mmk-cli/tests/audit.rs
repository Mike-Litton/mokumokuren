//! `mmk audit` — static codebase snapshot integration tests.
//!
//! Asserts the audit command's contract:
//! - walks every health-eligible TS/TSX/JS/JSX file at HEAD,
//! - emits per-file STRUCTURE / COMPLEXITY / non-delta HEALTH findings,
//! - never emits HOTSPOT / COUPLING / DRIFT / BUDGET (those are
//!   diff- / history-dependent and intentionally skipped),
//! - strips delta-mode HEALTH patterns (`broad_exception`,
//!   `test_weakening`) from the requested set.
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
patterns = ["test_pair", "broad_exception", "test_weakening"]
"#,
    );
}

#[serial(cwd)]
#[test]
fn audit_emits_test_pair_and_no_history_layers() {
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    init_repo(dir.path());
    write_audit_config(dir.path());

    // Implementation file with a test partner — TestPair always fires
    // because partner isn't in a "diff" (audit has no diff).
    write(
        dir.path(),
        "src/widget.ts",
        "export function f() { return 1; }\n",
    );
    write(
        dir.path(),
        "src/widget.test.ts",
        "test('f', () => { expect(1).toBe(1); });\n",
    );
    commit_all(dir.path(), "init", now - 5 * DAY);

    let stdout = run_audit(dir.path(), audit_args());
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");

    let findings = v["findings"].as_array().cloned().unwrap_or_default();

    let test_pair_fired = findings.iter().any(|f| {
        f["layer"] == "health"
            && f["message"]
                .as_str()
                .is_some_and(|m| m.contains("widget.ts") && m.contains("test partner"))
    });
    assert!(
        test_pair_fired,
        "expected TestPair finding on widget.ts; got: {}",
        String::from_utf8_lossy(&stdout)
    );

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
fn audit_strips_delta_mode_patterns() {
    // Repo accumulates broad handlers at HEAD; audit-mode must never
    // fire EVASION (delta = 0 by construction in audit mode), and
    // must never fire TEST_WEAKENING either.
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    init_repo(dir.path());
    write_audit_config(dir.path());
    write(
        dir.path(),
        "src/foo.ts",
        "export function f() { try { g(); } catch (e) {} }\n",
    );
    write(
        dir.path(),
        "src/foo.test.ts",
        "test('f', () => { expect(1).toBe(1); });\n",
    );
    commit_all(dir.path(), "init", now - 5 * DAY);

    let stdout = run_audit(dir.path(), audit_args());
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");
    let findings = v["findings"].as_array().cloned().unwrap_or_default();

    for f in &findings {
        let msg = f["message"].as_str().unwrap_or("");
        assert!(
            !msg.contains("not in HEAD"),
            "audit must not emit EVASION (delta-mode); got finding: {msg}"
        );
        assert!(
            !msg.contains("test weakened"),
            "audit must not emit TEST_WEAKENING (delta-mode); got finding: {msg}"
        );
    }
}
