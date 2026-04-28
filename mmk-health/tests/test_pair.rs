//! Pattern C — naming-convention test pair.

mod common;

use common::p;
use mmk_health::ts::analyze_ts;
use mmk_health::HealthPattern;

#[test]
fn test_pair_finds_sibling_test_file() {
    let subject = p("src/widgets/foo.ts");
    let peers = vec![p("src/widgets/foo.ts"), p("src/widgets/foo.test.ts")];
    let findings = analyze_ts(&subject, "", &peers, &[HealthPattern::TestPair]);
    assert_eq!(findings.len(), 1);
    let f = &findings[0];
    assert_eq!(f.pattern, HealthPattern::TestPair);
    assert_eq!(f.related, vec![p("src/widgets/foo.test.ts")]);
}

#[test]
fn test_pair_finds_spec_variant() {
    let subject = p("src/widgets/foo.ts");
    let peers = vec![p("src/widgets/foo.spec.ts")];
    let findings = analyze_ts(&subject, "", &peers, &[HealthPattern::TestPair]);
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].related, vec![p("src/widgets/foo.spec.ts")]);
}

#[test]
fn test_pair_finds_subdirectory_test_layout() {
    let subject = p("src/widgets/foo.ts");
    let peers = vec![p("src/widgets/test/foo.test.ts")];
    let findings = analyze_ts(&subject, "", &peers, &[HealthPattern::TestPair]);
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].related, vec![p("src/widgets/test/foo.test.ts")]);
}

#[test]
fn test_pair_silent_when_no_partner_exists() {
    let subject = p("src/widgets/foo.ts");
    let peers = vec![p("src/widgets/foo.ts"), p("src/widgets/bar.ts")];
    let findings = analyze_ts(&subject, "", &peers, &[HealthPattern::TestPair]);
    assert!(findings.is_empty(), "no test sibling → no finding");
}

#[test]
fn test_pair_does_not_treat_test_file_as_subject() {
    // A `.test.ts` file is the partner, not the subject. Otherwise
    // the agent would get a self-pair finding pointing back at
    // itself when editing tests directly.
    let subject = p("src/widgets/foo.test.ts");
    let peers = vec![p("src/widgets/foo.ts"), p("src/widgets/foo.test.ts")];
    let findings = analyze_ts(&subject, "", &peers, &[HealthPattern::TestPair]);
    assert!(
        findings.is_empty(),
        "test file as subject must not fire test-pair; got {findings:?}"
    );
}

#[test]
fn test_pair_disabled_pattern_is_silent() {
    let subject = p("src/widgets/foo.ts");
    let peers = vec![p("src/widgets/foo.test.ts")];
    let findings = analyze_ts(&subject, "", &peers, &[HealthPattern::Registration]);
    assert!(
        findings.is_empty(),
        "test-pair not requested → must not fire even when partner exists"
    );
}
