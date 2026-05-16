//! `BroadCatchDebt` HEALTH sensor — end-to-end via `mmk audit`.
//!
//! Asserts that a single TS file with a mix of broad-shape catches
//! produces exactly one BroadCatchDebt finding whose count matches
//! the number of broad handlers (excluding the rethrow that's
//! intentionally not broad).

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
patterns = ["broad_catch_debt"]
"#,
    );
}

#[serial(cwd)]
#[test]
fn broad_catch_debt_counts_four_broad_handlers_and_skips_rethrow() {
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    init_repo(dir.path());
    write_audit_config(dir.path());

    // Five catches: empty body, no-param, typed unknown, log-and-swallow,
    // and a non-broad rethrow. Expected count = 4.
    let body = "\
export function a() { try { f(); } catch {} }\n\
export function b() { try { f(); } catch (e) {} }\n\
export function c() { try { f(); } catch (e: unknown) { handle(e); } }\n\
export function d() { try { f(); } catch (e) { logger.warn(e); } }\n\
export function e() { try { f(); } catch (e) { logger.warn(e); throw e; } }\n";
    write(dir.path(), "src/foo.ts", body);
    commit_all(dir.path(), "init", now - 5 * DAY);

    let stdout = run_audit(dir.path(), audit_args());
    let v: Value = serde_json::from_slice(&stdout).expect("valid JSON");
    let findings = v["findings"].as_array().cloned().unwrap_or_default();

    let bcd: Vec<&Value> = findings
        .iter()
        .filter(|f| {
            f["layer"] == "health"
                && f["message"]
                    .as_str()
                    .is_some_and(|m| m.contains("broad catch handler") && m.contains("foo.ts"))
        })
        .collect();
    assert_eq!(
        bcd.len(),
        1,
        "expected exactly one BroadCatchDebt finding on foo.ts; got: {}",
        String::from_utf8_lossy(&stdout)
    );
    let msg = bcd[0]["message"].as_str().unwrap();
    assert!(
        msg.contains("4 broad catch handlers"),
        "expected count 4 in message; got: {msg}"
    );
    // Lines should reference the four broad-catch source positions
    // (lines 1, 2, 3, 4 — function `e` on line 5 is the rethrow).
    assert!(
        msg.contains("lines 1, 2, 3, 4"),
        "expected line numbers 1, 2, 3, 4 in message; got: {msg}"
    );

    // The structured `health.matches[]` block carries the same data
    // in queryable form: `detail.count` + `detail.lines` instead of
    // having to regex-parse `findings[].message`. This is the surface
    // an agent harness consumes (`jq '.health.matches[] | select(...)'`).
    let matches = v["health"]["matches"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let bcd_match = matches
        .iter()
        .find(|m| {
            m["pattern"] == "broad_catch_debt"
                && m["subject"].as_str().is_some_and(|s| s.ends_with("foo.ts"))
        })
        .unwrap_or_else(|| {
            panic!(
                "expected health.matches[] entry for broad_catch_debt on foo.ts; got: {}",
                String::from_utf8_lossy(&stdout)
            )
        });
    assert_eq!(
        bcd_match["detail"]["count"].as_u64(),
        Some(4),
        "structured detail.count must equal 4; got: {bcd_match}"
    );
    let lines: Vec<u64> = bcd_match["detail"]["lines"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|v| v.as_u64())
        .collect();
    assert_eq!(
        lines,
        vec![1, 2, 3, 4],
        "structured detail.lines must list each broad handler's line; got: {bcd_match}"
    );
}
