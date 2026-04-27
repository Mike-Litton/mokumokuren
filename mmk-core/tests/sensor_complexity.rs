//! Integration tests for the COMPLEXITY sensor — nesting/LOC,
//! ratio + absolute thresholds, min-siblings fallback.

use mmk_config::ComplexityCfg;
use mmk_core::sensors::{
    compute_complexity_findings, ComplexityFindingKind, ComplexityInput, FilesMap,
};
use std::path::{Path, PathBuf};

fn cfg() -> ComplexityCfg {
    ComplexityCfg::default()
}

fn bodies(entries: &[(&str, &str)]) -> (FilesMap, Vec<PathBuf>) {
    let mut m = FilesMap::default();
    let mut v = Vec::new();
    for (p, b) in entries {
        let path = PathBuf::from(p);
        m.insert(path.clone(), (*b).to_owned());
        v.push(path);
    }
    (m, v)
}

#[test]
fn deep_nesting_fires_absolute_threshold() {
    // Nesting depth 8 — well over default absolute_max=6.
    let body = r"
function deep() {
  if (a) { if (b) { if (c) { if (d) { if (e) { if (f) { if (g) { return 1; } } } } } } }
}
";
    let (b, _) = bodies(&[("a.ts", body)]);
    let cfg = cfg();
    let input = ComplexityInput {
        path: Path::new("a.ts"),
        siblings: &[],
        bodies: &b,
        cfg: &cfg,
    };
    let findings = compute_complexity_findings(&input);
    assert!(
        findings
            .iter()
            .any(|f| f.kind == ComplexityFindingKind::Nesting),
        "deep nesting must fire; got: {findings:?}"
    );
}

#[test]
fn long_function_fires_absolute_loc_threshold() {
    // 100-line function — over default loc_absolute_max=80.
    use std::fmt::Write as _;
    let mut body = String::from("function long() {\n");
    for i in 0..100 {
        writeln!(body, "  const x{i} = {i};").unwrap();
    }
    body.push_str("  return 0;\n}\n");
    let (b, _) = bodies(&[("a.ts", body.as_str())]);
    let cfg = cfg();
    let input = ComplexityInput {
        path: Path::new("a.ts"),
        siblings: &[],
        bodies: &b,
        cfg: &cfg,
    };
    let findings = compute_complexity_findings(&input);
    assert!(
        findings
            .iter()
            .any(|f| f.kind == ComplexityFindingKind::Size),
        "long function must fire size threshold; got: {findings:?}"
    );
}

#[test]
fn ratio_threshold_fires_with_directory_median() {
    // 3 sibling files with shallow functions (median nesting=1);
    // subject has nesting 4 → ratio 4 > default 3.0.
    let shallow = "function s(){ return 1; }\n";
    let medium = "function m(){ if (a){ if (b){ if (c){ return 1; } } } }\n";
    let (b, siblings) = bodies(&[
        ("dir/s1.ts", shallow),
        ("dir/s2.ts", shallow),
        ("dir/s3.ts", shallow),
        ("dir/subject.ts", medium),
    ]);
    let cfg = cfg();
    let input = ComplexityInput {
        path: Path::new("dir/subject.ts"),
        siblings: &siblings,
        bodies: &b,
        cfg: &cfg,
    };
    let findings = compute_complexity_findings(&input);
    assert!(
        findings
            .iter()
            .any(|f| f.kind == ComplexityFindingKind::Nesting && f.directory_median == Some(1)),
        "ratio-based nesting finding must fire; got: {findings:?}"
    );
}

#[test]
fn below_min_siblings_uses_only_absolute() {
    // 1 sibling — below min_directory_siblings=3.
    // Subject's nesting=5 — under absolute=6.
    // No ratio threshold should apply, so no finding.
    let (b, siblings) = bodies(&[
        ("dir/s1.ts", "function s(){ return 1; }\n"),
        (
            "dir/subject.ts",
            "function m(){ if (a){ if (b){ if (c){ if (d){ return 1; } } } } }\n",
        ),
    ]);
    let cfg = cfg();
    let input = ComplexityInput {
        path: Path::new("dir/subject.ts"),
        siblings: &siblings,
        bodies: &b,
        cfg: &cfg,
    };
    let findings = compute_complexity_findings(&input);
    assert!(
        findings.is_empty(),
        "ratio threshold must NOT fire below min_directory_siblings; got: {findings:?}"
    );
}

#[test]
fn refuses_on_language_without_real_adapter() {
    // .rs subject — adapter returns None; sensor stays silent.
    let (b, _) = bodies(&[(
        "a.rs",
        "fn deep() { if a { if b { if c { if d { if e { if f { return 1; } } } } } } }",
    )]);
    let cfg = cfg();
    let input = ComplexityInput {
        path: Path::new("a.rs"),
        siblings: &[],
        bodies: &b,
        cfg: &cfg,
    };
    assert!(compute_complexity_findings(&input).is_empty());
}
