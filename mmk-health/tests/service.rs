//! Pattern B — service / interface declaration pairs.
//!
//! The detector reads peer-file bodies from disk to confirm
//! consumers (vs. building a full import graph). The integration
//! tests therefore stage real files in a tempdir.

use mmk_health::ts::analyze_ts;
use mmk_health::HealthPattern;
use std::path::PathBuf;
use tempfile::TempDir;

const SERVICE_DECL_BODY: &str = r"
import { createDecorator } from 'vs/platform/instantiation/common/instantiation';
import { registerSingleton } from 'vs/platform/instantiation/common/extensions';

export interface IFooService {
    doFoo(): void;
}

export const IFooService = createDecorator<IFooService>('fooService');

class FooServiceImpl implements IFooService {
    doFoo() {}
}

registerSingleton(IFooService, FooServiceImpl, true);
";

#[test]
fn service_finds_consumers_importing_the_interface() {
    let dir = TempDir::new().unwrap();
    let subject_rel = PathBuf::from("src/services/foo.service.ts");
    let consumer_rel = PathBuf::from("src/widgets/widget.ts");
    let unrelated_rel = PathBuf::from("src/widgets/other.ts");

    let subject_abs = dir.path().join(&subject_rel);
    let consumer_abs = dir.path().join(&consumer_rel);
    let unrelated_abs = dir.path().join(&unrelated_rel);
    std::fs::create_dir_all(subject_abs.parent().unwrap()).unwrap();
    std::fs::create_dir_all(consumer_abs.parent().unwrap()).unwrap();
    std::fs::write(&subject_abs, SERVICE_DECL_BODY).unwrap();
    std::fs::write(
        &consumer_abs,
        "import { IFooService } from '../services/foo.service'; declare const x: IFooService;",
    )
    .unwrap();
    std::fs::write(&unrelated_abs, "function unrelated(): number { return 0; }").unwrap();

    // service.rs reads peers from disk via std::fs::read_to_string,
    // so peer paths must be absolute (or resolvable from CWD).
    let peers = vec![
        subject_abs.clone(),
        consumer_abs.clone(),
        unrelated_abs.clone(),
    ];
    let findings = analyze_ts(
        &subject_abs,
        SERVICE_DECL_BODY,
        &peers,
        &[HealthPattern::Service],
    );
    assert_eq!(findings.len(), 1, "got {findings:?}");
    let f = &findings[0];
    assert_eq!(f.pattern, HealthPattern::Service);
    assert!(
        f.related.contains(&consumer_abs),
        "consumer of IFooService must be surfaced; got {:?}",
        f.related
    );
    assert!(
        !f.related.contains(&unrelated_abs),
        "unrelated file must not be surfaced; got {:?}",
        f.related
    );
}

#[test]
fn service_silent_without_register_call() {
    let dir = TempDir::new().unwrap();
    let subject_abs = dir.path().join("foo.ts");
    // Interface but no registerSingleton/createDecorator — not a
    // service module by the convention this detector targets.
    let body = "export interface IFoo { x: number; }";
    std::fs::write(&subject_abs, body).unwrap();
    let findings = analyze_ts(
        &subject_abs,
        body,
        std::slice::from_ref(&subject_abs),
        &[HealthPattern::Service],
    );
    assert!(
        findings.is_empty(),
        "interface alone (no register) must not fire service; got {findings:?}"
    );
}

#[test]
fn service_silent_without_iprefixed_interface() {
    let dir = TempDir::new().unwrap();
    let subject_abs = dir.path().join("foo.ts");
    let body = r"
        interface Foo { x: number; }
        registerSingleton(Foo, FooImpl);
    ";
    std::fs::write(&subject_abs, body).unwrap();
    let findings = analyze_ts(
        &subject_abs,
        body,
        std::slice::from_ref(&subject_abs),
        &[HealthPattern::Service],
    );
    assert!(
        findings.is_empty(),
        "non-IPrefixed interface must not fire service; got {findings:?}"
    );
}
