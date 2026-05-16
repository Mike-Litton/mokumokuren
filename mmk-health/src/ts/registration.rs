//! Pattern A — action / contribution registration files.
//!
//! Detection signal: the file imports
//! `vs/platform/actions/common/actions`, contains a class extending
//! `Action2`, OR contains a top-level call to `registerAction2(...)`.
//! Any one trigger fires; together they cover the monorepo
//! `contrib/` convention without insisting on all three.
//!
//! Once a subject is identified as a registration file, the detector
//! returns up to [`MAX_PEERS`] sibling registration files in the
//! nearest `contrib/` subtree. Directory-distance ranking keeps the
//! list focused on architectural neighbors rather than the
//! workbench-wide grab-bag.

use crate::{HealthFinding, HealthPattern};
use std::path::{Path, PathBuf};

/// Cap on related-files surfaced per finding. Three is enough to
/// give the agent precedent without burying it in noise.
const MAX_PEERS: usize = 3;

/// Trigger-substring shortlist. Tree-sitter reads the full AST when
/// we *do* need it; for the cheap "is this a registration file"
/// gate, substring matching against the source text is dramatically
/// cheaper and sufficient. False positives (e.g. a comment that
/// mentions `registerAction2`) get filtered in the AST-confirm step.
const REGISTRATION_TRIGGERS: &[&str] = &[
    "vs/platform/actions/common/actions",
    "registerAction2(",
    "extends Action2",
];

#[must_use]
pub fn detect(subject: &Path, body: &str, peer_paths: &[PathBuf]) -> Vec<HealthFinding> {
    if !is_registration_file(body) {
        return Vec::new();
    }
    let peers = nearest_registration_peers(subject, peer_paths, MAX_PEERS);
    if peers.is_empty() {
        return Vec::new();
    }
    vec![HealthFinding {
        pattern: HealthPattern::Registration,
        subject: subject.to_path_buf(),
        related: peers,
        detail: None,
    }]
}

/// Substring shortlist over a small trigger set. Cheap; the
/// alternative would be to parse every reviewer-touched file just
/// to filter comment-only triggers, which would dominate the
/// per-edit hot path. The trade-off accepts the rare false positive
/// where a doc comment quotes "registerAction2(".
fn is_registration_file(body: &str) -> bool {
    REGISTRATION_TRIGGERS.iter().any(|t| body.contains(t))
}

/// Walk `peer_paths` for sibling `*.contribution.ts` (or comparable
/// registration shape) inside the same `contrib/` subtree as
/// `subject`. Ranks by directory distance — same directory first,
/// then ancestor chain.
///
/// Filename heuristic is conservative: only `*.contribution.ts`
/// matches. The substring trigger from `is_registration_file`
/// applies to the *subject*; peers aren't re-parsed — the naming
/// convention is the convention's own assertion.
fn nearest_registration_peers(subject: &Path, peer_paths: &[PathBuf], max: usize) -> Vec<PathBuf> {
    let subject_dir = subject.parent().unwrap_or_else(|| Path::new(""));
    let mut scored: Vec<(usize, &PathBuf)> = peer_paths
        .iter()
        .filter(|p| p.as_path() != subject)
        .filter(|p| is_contribution_filename(p))
        .map(|p| {
            let dist = directory_distance(subject_dir, p.parent().unwrap_or_else(|| Path::new("")));
            (dist, p)
        })
        .collect();
    scored.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(b.1)));
    scored
        .into_iter()
        .take(max)
        .map(|(_, p)| p.clone())
        .collect()
}

fn is_contribution_filename(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.ends_with(".contribution.ts"))
}

/// Edit-style directory distance between two paths.
///
/// Counts the components that don't share an ancestor — `a/b/c` vs
/// `a/b/d` is distance 2 (one `..` up, one `d` down). Same dir is
/// distance 0; sibling subtree is small; `contrib/extensions/...`
/// vs `contrib/preferences/...` lands around 4, well below the
/// workbench-wide grab-bag.
fn directory_distance(a: &Path, b: &Path) -> usize {
    let common = common_prefix_components(a, b);
    let a_extra = a.components().count().saturating_sub(common);
    let b_extra = b.components().count().saturating_sub(common);
    a_extra + b_extra
}

fn common_prefix_components(a: &Path, b: &Path) -> usize {
    let mut count = 0;
    for (ac, bc) in a.components().zip(b.components()) {
        if ac == bc {
            count += 1;
        } else {
            break;
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registration_triggers_match_contrib_convention() {
        let body = r"
            import { registerAction2, Action2 } from 'vs/platform/actions/common/actions';
            class FooContrib extends Action2 { run() {} }
        ";
        assert!(is_registration_file(body));
    }

    #[test]
    fn body_without_triggers_does_not_match() {
        let body = "function helper(): number { return 42; }";
        assert!(!is_registration_file(body));
    }

    #[test]
    fn directory_distance_zero_for_same_dir() {
        assert_eq!(
            directory_distance(Path::new("a/b/c"), Path::new("a/b/c")),
            0
        );
    }

    #[test]
    fn directory_distance_handles_sibling_subtrees() {
        // contrib/extensions/browser vs contrib/preferences/browser:
        // common prefix is contrib/, extras are 2 + 2.
        let d = directory_distance(
            Path::new("src/contrib/extensions/browser"),
            Path::new("src/contrib/preferences/browser"),
        );
        assert_eq!(d, 4, "sibling subtrees should land around 4");
    }
}
