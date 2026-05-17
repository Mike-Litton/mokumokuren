//! TEST_WEAKENING — `analyze_ts` dispatch contract.
//!
//! The per-axis detection logic is covered by the unit tests in
//! `src/ts/test_weakening.rs`. This integration file only verifies
//! the bits a `detect()` call can't reach: that the public
//! `analyze_ts` entry point routes through to TEST_WEAKENING when
//! the pattern is enabled, skips it when not enabled, and surfaces
//! the per-axis `detail` payload through the `HealthFinding`
//! envelope.

mod common;

use common::p;
use mmk_health::ts::analyze_ts;
use mmk_health::{HealthFindingDetail, HealthPattern};

const PATTERNS: &[HealthPattern] = &[HealthPattern::TestWeakening];

#[test]
fn analyze_ts_routes_to_test_weakening_with_detail_payload() {
    let subject = p("src/foo.test.ts");
    let head = "test('f', () => { expect(a).toBe(1); expect(b).toBe(2); });";
    let working = "test.skip('f', () => { expect(a).toBe(1); });";
    let findings = analyze_ts(&subject, working, Some(head), &[], PATTERNS, &[]);
    assert_eq!(findings.len(), 1, "expected one fire; got {findings:?}");
    assert_eq!(findings[0].pattern, HealthPattern::TestWeakening);
    let Some(HealthFindingDetail::TestWeakening {
        skips_added,
        assertions_lost,
        ..
    }) = findings[0].detail
    else {
        panic!("expected TestWeakening detail; got {:?}", findings[0].detail);
    };
    assert!(skips_added >= 1, "skip axis flows through");
    assert_eq!(assertions_lost, 1, "assertion axis flows through");
}

#[test]
fn analyze_ts_skips_test_weakening_when_not_enabled() {
    // Same weakening shape; pattern set asks only for TestPair, so
    // TEST_WEAKENING must stay silent. Locks the dispatch contract:
    // every detector is pattern-gated.
    let subject = p("src/foo.test.ts");
    let head = "test('f', () => { expect(a).toBe(1); expect(b).toBe(2); });";
    let working = "test.skip('f', () => {});";
    let findings = analyze_ts(
        &subject,
        working,
        Some(head),
        &[],
        &[HealthPattern::TestPair],
        &[],
    );
    assert!(
        findings.is_empty(),
        "test_weakening must not fire when not enabled; got {findings:?}",
    );
}
