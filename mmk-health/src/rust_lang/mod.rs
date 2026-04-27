//! Rust adapter — stub.
//!
//! v0.5 ships the per-language adapter pattern but only TypeScript
//! has a full tree-sitter walker. The Rust stub returns `None` from
//! [`LanguageAdapter::extract`] so STRUCTURE falls back to the
//! line-scan import slice and COMPLEXITY refuses to fire (better
//! silent than wrong without a real AST).
//!
//! v0.6+ replaces this with a `tree_sitter_rust` walker. The shape
//! of [`StructuredFacts`] won't change — sensors keep working.
//!
//! Module name is `rust_lang` (not `rust`) to avoid colliding with
//! the language reserved word; the `extensions()` claim is still
//! `rs`.

use crate::adapter::LanguageAdapter;
use crate::facts::StructuredFacts;
use std::path::Path;

#[derive(Debug, Default)]
pub struct RustAdapter;

impl LanguageAdapter for RustAdapter {
    fn extensions(&self) -> &'static [&'static str] {
        &["rs"]
    }

    fn extract(&self, _path: &Path, _body: &str) -> Option<StructuredFacts> {
        None
    }
}
