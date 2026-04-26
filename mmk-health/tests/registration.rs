//! Pattern A — monorepo-style registration files
//! (`*.contribution.ts` paired with `registerAction2` / `Action2`).

use mmk_health::ts::analyze_ts;
use mmk_health::HealthPattern;
use std::path::PathBuf;

fn p(s: &str) -> PathBuf {
    PathBuf::from(s)
}

const REGISTRATION_BODY: &str = r"
import { registerAction2, Action2 } from 'vs/platform/actions/common/actions';

class FooContrib extends Action2 {
    constructor() { super({ id: 'foo' }); }
    run() {}
}

registerAction2(FooContrib);
";

#[test]
fn registration_surfaces_nearby_contribution_peers() {
    let subject = p("src/contrib/extensions/extensions.contribution.ts");
    let peers = vec![
        p("src/contrib/extensions/extensions.contribution.ts"),
        p("src/contrib/preferences/preferences.contribution.ts"),
        p("src/contrib/search/search.contribution.ts"),
        // Non-contribution sibling — must not surface.
        p("src/contrib/extensions/util.ts"),
    ];
    let findings = analyze_ts(
        &subject,
        REGISTRATION_BODY,
        &peers,
        &[HealthPattern::Registration],
    );
    assert_eq!(findings.len(), 1);
    let f = &findings[0];
    assert_eq!(f.pattern, HealthPattern::Registration);
    let names: Vec<&str> = f.related.iter().map(|p| p.to_str().unwrap_or("")).collect();
    assert!(
        names.contains(&"src/contrib/preferences/preferences.contribution.ts"),
        "related must include the sibling contribution file as architectural precedent; got {names:?}"
    );
    assert!(
        !names.iter().any(|n| n.ends_with("util.ts")),
        "non-contribution siblings must not be returned; got {names:?}"
    );
}

#[test]
fn registration_silent_when_body_lacks_triggers() {
    let subject = p("src/contrib/extensions/extensions.contribution.ts");
    let peers = vec![p("src/contrib/preferences/preferences.contribution.ts")];
    let plain = "function helper(): number { return 42; }";
    let findings = analyze_ts(&subject, plain, &peers, &[HealthPattern::Registration]);
    assert!(
        findings.is_empty(),
        "no registration trigger → no finding; got {findings:?}"
    );
}

#[test]
fn registration_silent_when_no_peers_exist() {
    let subject = p("src/standalone/foo.contribution.ts");
    let peers = vec![p("src/standalone/foo.contribution.ts")];
    let findings = analyze_ts(
        &subject,
        REGISTRATION_BODY,
        &peers,
        &[HealthPattern::Registration],
    );
    assert!(
        findings.is_empty(),
        "no peer contribution files → no finding; got {findings:?}"
    );
}

#[test]
fn registration_caps_related_at_three() {
    let subject = p("src/contrib/extensions/extensions.contribution.ts");
    let mut peers = vec![p("src/contrib/extensions/extensions.contribution.ts")];
    for i in 0..10 {
        peers.push(PathBuf::from(format!(
            "src/contrib/feature{i}/feature{i}.contribution.ts"
        )));
    }
    let findings = analyze_ts(
        &subject,
        REGISTRATION_BODY,
        &peers,
        &[HealthPattern::Registration],
    );
    assert_eq!(findings.len(), 1);
    assert!(
        findings[0].related.len() <= 3,
        "MAX_PEERS = 3; got {} related",
        findings[0].related.len()
    );
}
