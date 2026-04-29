//! Pattern C — test-pair naming convention.
//!
//! Pure path-shape inference: `<base>.ts` (or `.tsx` / `.js` /
//! `.jsx`) is paired with one of `<base>.test.<ext>`,
//! `<base>.spec.<ext>`, or `test/<base>.test.<ext>` if such a
//! sibling exists in the candidate path list. Doesn't need a
//! parse — naming convention does the work.

use crate::{HealthFinding, HealthPattern};
use std::path::{Path, PathBuf};

const SUPPORTED_EXTS: &[&str] = &["ts", "tsx", "js", "jsx"];

#[must_use]
pub fn detect(subject: &Path, peer_paths: &[PathBuf]) -> Vec<HealthFinding> {
    if !is_implementation_file(subject) {
        return Vec::new();
    }
    let candidates = candidate_partner_paths(subject);
    let mut related: Vec<PathBuf> = candidates
        .into_iter()
        .filter(|c| peer_paths.iter().any(|p| p == c))
        .collect();
    if related.is_empty() {
        return Vec::new();
    }
    related.sort();
    vec![HealthFinding {
        pattern: HealthPattern::TestPair,
        subject: subject.to_path_buf(),
        related,
    }]
}

/// "Implementation file" means a `.ts` / `.tsx` / `.js` / `.jsx`
/// that isn't itself a test. Test files are the partners we want
/// to *find*, not the subjects we analyze.
fn is_implementation_file(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    let supported = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .is_some_and(|e| SUPPORTED_EXTS.contains(&e.as_str()));
    if !supported {
        return false;
    }
    !is_test_file_name(name)
}

fn is_test_file_name(name: &str) -> bool {
    SUPPORTED_EXTS.iter().any(|ext| {
        name.ends_with(&format!(".test.{ext}")) || name.ends_with(&format!(".spec.{ext}"))
    })
}

/// Build the candidate partner paths for `subject`. The convention
/// pairs an implementation with a test file in the *same family*:
/// TS family (`.ts` ↔ `.tsx`) or JS family (`.js` ↔ `.jsx`).
/// Cross-family (TS ↔ JS) is rejected — type-checking semantics
/// differ and a cross-family pair is almost always a typo.
///
/// Within-family extension variation is legitimate: a React-aware
/// importer often lives in `.tsx` (needs JSX support) while its
/// unit test sits in `.ts` (doesn't render JSX). v0.7's strict
/// same-extension rule made `.tsx` ↔ `.test.ts` invisible, missing
/// every `.tsx` impl whose tests don't render JSX.
fn candidate_partner_paths(subject: &Path) -> Vec<PathBuf> {
    let Some(stem) = subject.file_stem().and_then(|s| s.to_str()) else {
        return Vec::new();
    };
    let Some(ext) = subject
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
    else {
        return Vec::new();
    };
    let family_exts: &[&str] = if matches!(ext.as_str(), "ts" | "tsx") {
        &["ts", "tsx"]
    } else if matches!(ext.as_str(), "js" | "jsx") {
        &["js", "jsx"]
    } else {
        return Vec::new();
    };
    let parent = subject.parent().unwrap_or_else(|| Path::new(""));
    let mut out = Vec::new();
    for fext in family_exts {
        out.push(parent.join(format!("{stem}.test.{fext}")));
        out.push(parent.join(format!("{stem}.spec.{fext}")));
        out.push(parent.join("test").join(format!("{stem}.test.{fext}")));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_files_are_not_subjects() {
        assert!(!is_implementation_file(Path::new("src/foo.test.ts")));
        assert!(!is_implementation_file(Path::new("src/foo.spec.ts")));
        assert!(!is_implementation_file(Path::new("src/foo.test.js")));
        assert!(!is_implementation_file(Path::new("src/foo.spec.jsx")));
        assert!(is_implementation_file(Path::new("src/foo.ts")));
        assert!(is_implementation_file(Path::new("src/foo.js")));
        assert!(is_implementation_file(Path::new("src/foo.jsx")));
    }

    #[test]
    fn non_ts_files_are_not_subjects() {
        assert!(!is_implementation_file(Path::new("src/foo.rs")));
        assert!(!is_implementation_file(Path::new("README.md")));
    }

    #[test]
    fn js_impl_pairs_with_test_js() {
        let subject = PathBuf::from("src/foo.js");
        let peers = vec![
            PathBuf::from("src/foo.js"),
            PathBuf::from("src/foo.test.js"),
        ];
        let f = detect(&subject, &peers);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].related, vec![PathBuf::from("src/foo.test.js")]);
    }

    #[test]
    fn jsx_impl_pairs_with_test_jsx() {
        let subject = PathBuf::from("src/Widget.jsx");
        let peers = vec![PathBuf::from("src/Widget.spec.jsx")];
        let f = detect(&subject, &peers);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].related, vec![PathBuf::from("src/Widget.spec.jsx")]);
    }

    #[test]
    fn js_test_file_is_not_a_subject() {
        let subject = PathBuf::from("src/foo.test.js");
        let peers = vec![
            PathBuf::from("src/foo.js"),
            PathBuf::from("src/foo.test.js"),
        ];
        let f = detect(&subject, &peers);
        assert!(
            f.is_empty(),
            "test file as subject must not fire test-pair; got {f:?}"
        );
    }

    #[test]
    fn js_impl_does_not_pair_with_ts_test() {
        // A `.js` impl with only a `.test.ts` partner has no
        // language-matching sibling. The convention pairs
        // same-family (TS↔TS or JS↔JS); cross-family is rejected.
        let subject = PathBuf::from("src/foo.js");
        let peers = vec![PathBuf::from("src/foo.test.ts")];
        let f = detect(&subject, &peers);
        assert!(f.is_empty(), "cross-family pair must not fire; got {f:?}");
    }

    #[test]
    fn tsx_impl_pairs_with_test_ts_partner() {
        // Real-world TS pattern (Reactive-Resume, e.g.): a React-aware
        // module exposes a class via `.tsx` because it needs JSX
        // support, while the test is `.ts` because it doesn't render
        // JSX (just unit-tests parse/convert logic). v0.7 had this
        // wrong — same-extension matching made the partner invisible.
        let subject = PathBuf::from("src/integrations/import/json-resume.tsx");
        let peers = vec![PathBuf::from("src/integrations/import/json-resume.test.ts")];
        let f = detect(&subject, &peers);
        assert_eq!(f.len(), 1, "tsx ↔ test.ts must pair; got {f:?}");
        assert_eq!(
            f[0].related,
            vec![PathBuf::from("src/integrations/import/json-resume.test.ts")]
        );
    }

    #[test]
    fn tsx_impl_pairs_with_spec_ts_partner() {
        let subject = PathBuf::from("src/widget.tsx");
        let peers = vec![PathBuf::from("src/widget.spec.ts")];
        let f = detect(&subject, &peers);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].related, vec![PathBuf::from("src/widget.spec.ts")]);
    }

    #[test]
    fn ts_impl_pairs_with_test_tsx_partner() {
        // Symmetric case: pure-TS impl, JSX-rendering test.
        let subject = PathBuf::from("src/store.ts");
        let peers = vec![PathBuf::from("src/store.test.tsx")];
        let f = detect(&subject, &peers);
        assert_eq!(f.len(), 1, "ts ↔ test.tsx must pair; got {f:?}");
    }

    #[test]
    fn jsx_impl_pairs_with_test_js_partner() {
        let subject = PathBuf::from("src/App.jsx");
        let peers = vec![PathBuf::from("src/App.test.js")];
        let f = detect(&subject, &peers);
        assert_eq!(f.len(), 1, "jsx ↔ test.js must pair; got {f:?}");
    }

    #[test]
    fn cross_family_jsx_to_test_ts_does_not_pair() {
        // JS family ≠ TS family. Type checking semantics differ;
        // pairing a `.jsx` impl with a `.test.ts` test is almost
        // always a typo or wrong test, not a convention.
        let subject = PathBuf::from("src/Widget.jsx");
        let peers = vec![PathBuf::from("src/Widget.test.ts")];
        let f = detect(&subject, &peers);
        assert!(f.is_empty(), "jsx ↔ test.ts is cross-family; got {f:?}");
    }
}
