//! TypeScript pattern detectors.
//!
//! The grammar is loaded once per process via
//! [`tree_sitter_typescript::LANGUAGE_TYPESCRIPT`]; each `analyze_*`
//! call gets a fresh `Parser` (parsers aren't `Send`, and the cost
//! is microseconds — pooling would be premature).

pub mod facts;
pub mod registration;
pub mod service;
pub mod test_pair;

pub use facts::TsAdapter;

use crate::{HealthFinding, HealthPattern};
use std::path::Path;

/// Run every requested detector against `subject`'s body and any
/// candidate peer paths the caller wants to consider for cross-file
/// patterns.
///
/// `peer_paths` is the set the cross-file detectors consult:
/// - Pattern A scans paths in the same `contrib/` subtree to find
///   sibling registration files.
/// - Pattern B scans paths workbench-wide to find consumers
///   importing the declared interface.
/// - Pattern C uses the filesystem (`peer_paths` lookup) to confirm
///   the test sibling exists.
///
/// Detectors that don't apply silently return no findings — callers
/// concatenate results without per-detector branching.
#[must_use]
pub fn analyze_ts(
    subject: &Path,
    body: &str,
    peer_paths: &[std::path::PathBuf],
    enabled: &[HealthPattern],
) -> Vec<HealthFinding> {
    let mut out = Vec::new();
    if enabled.contains(&HealthPattern::Registration) {
        out.extend(registration::detect(subject, body, peer_paths));
    }
    if enabled.contains(&HealthPattern::Service) {
        out.extend(service::detect(subject, body, peer_paths));
    }
    if enabled.contains(&HealthPattern::TestPair) {
        out.extend(test_pair::detect(subject, peer_paths));
    }
    out
}

/// Parse `body` as TypeScript.
///
/// Returns `None` if the language fails to load (shouldn't happen
/// post-build) or the parse fails (rare — tree-sitter is
/// error-tolerant). Callers skip the file silently rather than
/// emit a noisy diagnostic.
#[must_use]
pub fn parse(body: &str) -> Option<tree_sitter::Tree> {
    let language = tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into();
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&language).ok()?;
    parser.parse(body, None)
}
