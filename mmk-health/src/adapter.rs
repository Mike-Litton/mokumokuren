//! Per-language adapter trait and dispatch.
//!
//! Each adapter knows how to turn a source-file body into
//! [`StructuredFacts`]. Sensors call [`extract`] (or
//! [`extract_with_imports`]); the dispatch handles "no adapter for
//! this extension" and "adapter declined to parse" by returning
//! `None`, so callers don't need to enumerate languages.
//!
//! Today only TypeScript ships a full adapter. Rust and Python land
//! as stubs that return `None` — STRUCTURE's import-aggregation
//! slice still works on those languages via [`crate::linescan`], and
//! COMPLEXITY refuses to fire (better silent than wrong on a
//! language without a real AST).

use crate::facts::{ImportFact, StructuredFacts};
use std::path::Path;

/// One adapter per source language.
pub trait LanguageAdapter: Send + Sync {
    /// File extensions this adapter claims (lowercase, no leading
    /// dot). The dispatch in [`extract`] picks the first adapter
    /// that claims the path's extension.
    fn extensions(&self) -> &'static [&'static str];

    /// Parse `body` and return facts, or `None` if the adapter has
    /// no AST to work from. Returning `None` is a positive signal:
    /// it tells callers "fall back to line-scan / refuse to fire,"
    /// not "something failed."
    fn extract(&self, path: &Path, body: &str) -> Option<StructuredFacts>;
}

/// Dispatch by extension to the right adapter and return its facts.
///
/// `None` here means *either* "no adapter claims this extension"
/// *or* "the adapter declined to produce facts." Callers that need
/// imports specifically should use [`extract_with_imports`], which
/// substitutes line-scan output when both axes fail.
#[must_use]
pub fn extract(path: &Path, body: &str) -> Option<StructuredFacts> {
    let ext = path.extension().and_then(|e| e.to_str())?.to_lowercase();
    for adapter in adapters() {
        if adapter.extensions().contains(&ext.as_str()) {
            if let Some(facts) = adapter.extract(path, body) {
                return Some(facts);
            }
        }
    }
    None
}

/// Like [`extract`], but synthesises an imports-only fact bundle.
///
/// Uses [`crate::linescan`] when the per-language adapter declined.
/// STRUCTURE uses this so its directory-aggregation slice still
/// works on languages without a full AST adapter.
#[must_use]
pub fn extract_with_imports(path: &Path, body: &str) -> Option<StructuredFacts> {
    if let Some(facts) = extract(path, body) {
        return Some(facts);
    }
    let imports = crate::linescan::extract_imports(path, body);
    if imports.is_empty() {
        return None;
    }
    Some(StructuredFacts {
        imports: imports.into_iter().map(line_scan_to_fact).collect(),
        ..StructuredFacts::default()
    })
}

const fn line_scan_to_fact(source: String) -> ImportFact {
    ImportFact {
        source,
        symbols: Vec::new(),
    }
}

fn adapters() -> &'static [&'static dyn LanguageAdapter] {
    // Static slice rather than a registry: the set of languages is
    // small and known at compile time, and sensor hot paths run this
    // dispatch per-file. A registry would force a heap allocation
    // per call for no benefit.
    &[
        &crate::ts::TsAdapter,
        &crate::rust_lang::RustAdapter,
        &crate::python::PythonAdapter,
    ]
}
