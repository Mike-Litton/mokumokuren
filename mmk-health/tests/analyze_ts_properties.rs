//! Property tests for `mmk_health::ts::analyze_ts`. Pure dispatch
//! over (subject, body, peers, enabled patterns); the universal
//! invariants below should hold regardless of body / peer content.

use mmk_health::ts::analyze_ts;
use mmk_health::HealthPattern;
use proptest::collection::vec;
use proptest::prelude::*;
use std::path::PathBuf;

fn pattern_strategy() -> impl Strategy<Value = HealthPattern> {
    prop_oneof![
        Just(HealthPattern::Registration),
        Just(HealthPattern::Service),
        Just(HealthPattern::TestPair),
    ]
}

fn path_strategy() -> impl Strategy<Value = PathBuf> {
    "src/[a-z]{1,5}/[a-z]{1,5}\\.(ts|tsx)".prop_map(PathBuf::from)
}

fn peers_strategy() -> impl Strategy<Value = Vec<PathBuf>> {
    vec(path_strategy(), 0..15)
}

fn body_strategy() -> impl Strategy<Value = String> {
    // Bodies short enough to keep tree-sitter parses microseconds.
    // Mix of plausible TS-ish tokens and noise so detectors hit
    // varied paths.
    prop_oneof![
        Just(String::new()),
        Just("export const x = 1;\n".to_string()),
        Just("import { z } from 'zod';\nexport function Foo() {}\n".to_string()),
        Just("export interface IBar { x: number; }\n".to_string()),
        Just("registerAction2(Foo);\n".to_string()),
        Just("registerSingleton(IFoo, FooImpl);\n".to_string()),
    ]
}

proptest! {
    /// Empty pattern list ⇒ no findings, regardless of body or peer
    /// content. Locks the dispatch contract: every finding is
    /// pattern-gated.
    #[test]
    fn no_patterns_yields_no_findings(
        subject in path_strategy(),
        body in body_strategy(),
        peers in peers_strategy(),
    ) {
        let findings = analyze_ts(&subject, &body, None, &peers, &[]);
        prop_assert!(
            findings.is_empty(),
            "no patterns should produce no findings; got {findings:?}",
        );
    }

    /// `related[]` paths are bounded — Pattern A caps at 3, Pattern C
    /// caps at the test-partner count (typically ≤ 2). The universal
    /// claim: no detector emits a `related` set larger than 50,
    /// regardless of how many peers the caller passed. Catches a
    /// runaway list that would inflate hook-output context.
    #[test]
    fn related_paths_bounded(
        subject in path_strategy(),
        body in body_strategy(),
        peers in peers_strategy(),
        enabled in vec(pattern_strategy(), 0..4),
    ) {
        let findings = analyze_ts(&subject, &body, None, &peers, &enabled);
        for f in &findings {
            prop_assert!(
                f.related.len() <= 50,
                "related[] {} entries — bound is 50 to keep hook output tight",
                f.related.len(),
            );
        }
    }

    /// Subject is never reported as its own `related` partner. A
    /// self-pair would loop the agent back to the file it's already
    /// editing.
    #[test]
    fn subject_not_in_own_related(
        subject in path_strategy(),
        body in body_strategy(),
        peers in peers_strategy(),
        enabled in vec(pattern_strategy(), 0..4),
    ) {
        let findings = analyze_ts(&subject, &body, None, &peers, &enabled);
        for f in &findings {
            prop_assert!(
                !f.related.contains(&subject),
                "subject {subject:?} appeared in its own related list",
            );
        }
    }

}
