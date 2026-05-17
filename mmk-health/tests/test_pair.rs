//! Pattern C — `analyze_ts` dispatch contract for `test_pair`.
//!
//! Naming-convention and partner-resolution logic is covered by the
//! unit tests in `src/ts/test_pair.rs`. This integration file only
//! verifies the public `analyze_ts` entry point routes through to
//! the TestPair detector when enabled and stays silent when not.

mod common;

use common::p;
use mmk_health::ts::analyze_ts;
use mmk_health::HealthPattern;

#[test]
fn analyze_ts_routes_to_test_pair_when_partner_exists() {
    let subject = p("src/widgets/foo.ts");
    let peers = vec![p("src/widgets/foo.test.ts")];
    let findings = analyze_ts(&subject, "", None, &peers, &[HealthPattern::TestPair], &[]);
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].pattern, HealthPattern::TestPair);
    assert_eq!(findings[0].related, vec![p("src/widgets/foo.test.ts")]);
}

#[test]
fn analyze_ts_skips_test_pair_when_not_enabled() {
    let subject = p("src/widgets/foo.ts");
    let peers = vec![p("src/widgets/foo.test.ts")];
    let findings = analyze_ts(
        &subject,
        "",
        None,
        &peers,
        &[HealthPattern::BroadException],
        &[],
    );
    assert!(
        findings.is_empty(),
        "test_pair must not fire when not requested; got {findings:?}",
    );
}
