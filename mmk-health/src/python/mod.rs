//! Python adapter — stub.
//!
//! See `rust_lang/mod.rs` for the rationale; v0.5 ships the
//! adapter trait but only TypeScript has a full walker.

use crate::adapter::LanguageAdapter;
use crate::facts::StructuredFacts;
use std::path::Path;

#[derive(Debug, Default)]
pub struct PythonAdapter;

impl LanguageAdapter for PythonAdapter {
    fn extensions(&self) -> &'static [&'static str] {
        &["py"]
    }

    fn extract(&self, _path: &Path, _body: &str) -> Option<StructuredFacts> {
        None
    }
}
