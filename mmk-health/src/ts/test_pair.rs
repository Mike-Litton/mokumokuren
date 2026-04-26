//! Pattern C — test-pair naming convention.
//!
//! Pure path-shape inference: `<base>.ts` is paired with one of
//! `<base>.test.ts`, `<base>.spec.ts`, or `test/<base>.test.ts` if
//! such a sibling exists in the candidate path list. Doesn't need a
//! parse — naming convention does the work.

use crate::{HealthFinding, HealthPattern};
use std::path::{Path, PathBuf};

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

/// "Implementation file" means a `.ts` (or `.tsx`) that isn't
/// itself a test. Test files are the partners we want to *find*,
/// not the subjects we analyze.
fn is_implementation_file(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    let ts = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("ts") || e.eq_ignore_ascii_case("tsx"));
    if !ts {
        return false;
    }
    !is_test_file_name(name)
}

fn is_test_file_name(name: &str) -> bool {
    name.ends_with(".test.ts")
        || name.ends_with(".test.tsx")
        || name.ends_with(".spec.ts")
        || name.ends_with(".spec.tsx")
}

/// Build the candidate partner paths for `subject`:
/// - sibling `<base>.test.ts`
/// - sibling `<base>.spec.ts`
/// - co-located `test/<base>.test.ts`
fn candidate_partner_paths(subject: &Path) -> Vec<PathBuf> {
    let Some(stem) = subject.file_stem().and_then(|s| s.to_str()) else {
        return Vec::new();
    };
    let parent = subject.parent().unwrap_or_else(|| Path::new(""));
    let mut out = Vec::new();
    out.push(parent.join(format!("{stem}.test.ts")));
    out.push(parent.join(format!("{stem}.spec.ts")));
    out.push(parent.join("test").join(format!("{stem}.test.ts")));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_files_are_not_subjects() {
        assert!(!is_implementation_file(Path::new("src/foo.test.ts")));
        assert!(!is_implementation_file(Path::new("src/foo.spec.ts")));
        assert!(is_implementation_file(Path::new("src/foo.ts")));
    }

    #[test]
    fn non_ts_files_are_not_subjects() {
        assert!(!is_implementation_file(Path::new("src/foo.rs")));
        assert!(!is_implementation_file(Path::new("README.md")));
    }
}
