//! TypeScript pattern detectors.
//!
//! The grammar is loaded once per process via
//! [`tree_sitter_typescript::LANGUAGE_TYPESCRIPT`] for `.ts` /
//! `.js` and [`tree_sitter_typescript::LANGUAGE_TSX`] for `.tsx` /
//! `.jsx`; each `analyze_*` call gets a fresh `Parser` (parsers aren't
//! `Send`, and the cost is microseconds — pooling would be
//! premature).

pub mod broad_exception;
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
///
/// `head_body` is the file's content at HEAD (when available);
/// EVASION uses it to compute working-vs-HEAD broad-handler delta.
/// `None` means "no HEAD body available" — either a new file, or
/// the caller doesn't have a HEAD snapshot (e.g. pre-edit, where
/// the working tree *is* HEAD for this subject).
#[must_use]
pub fn analyze_ts(
    subject: &Path,
    body: &str,
    head_body: Option<&str>,
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
    if enabled.contains(&HealthPattern::BroadException) {
        out.extend(broad_exception::detect(subject, head_body, body));
    }
    out
}

/// Parse `body` as TypeScript using the default (non-TSX) grammar.
///
/// Prefer [`parse_for`] when you have a path — TSX files require
/// the TSX grammar to handle JSX nodes correctly. This thin wrapper
/// remains for callers that genuinely don't have a path (e.g.
/// snippet-based unit tests).
///
/// Returns `None` if the language fails to load (shouldn't happen
/// post-build) or the parse fails (rare — tree-sitter is
/// error-tolerant). Callers skip the file silently rather than
/// emit a noisy diagnostic.
#[must_use]
pub fn parse(body: &str) -> Option<tree_sitter::Tree> {
    parse_with_language(body, tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
}

/// Parse `body` choosing the grammar by `path`'s extension.
///
/// `.tsx` / `.jsx` use [`tree_sitter_typescript::LANGUAGE_TSX`];
/// every other extension (including `.ts` / `.js`) uses
/// [`tree_sitter_typescript::LANGUAGE_TYPESCRIPT`]. The TSX grammar
/// is a superset that handles JSX nodes correctly — running it on
/// non-JSX code is benign, but using the non-TSX grammar on TSX
/// silently degrades on JSX-bearing files.
#[must_use]
pub fn parse_for(path: &Path, body: &str) -> Option<tree_sitter::Tree> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    let lang = match ext.as_deref() {
        Some("tsx" | "jsx") => tree_sitter_typescript::LANGUAGE_TSX,
        _ => tree_sitter_typescript::LANGUAGE_TYPESCRIPT,
    };
    parse_with_language(body, lang.into())
}

fn parse_with_language(body: &str, language: tree_sitter::Language) -> Option<tree_sitter::Tree> {
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&language).ok()?;
    parser.parse(body, None)
}
