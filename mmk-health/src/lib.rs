//! Structural-pattern adapter (Health layer) for Mokumokuren.
//!
//! Currently ships a TypeScript adapter — three pattern detectors
//! that surface architectural neighbors the empirical co-change cone
//! cannot see (because the patterns live in non-overlapping commit
//! cones across the workbench). Future implementations will add
//! Rust / Python / Go adapters under `src/<lang>/`.
//!
//! ## Cost
//!
//! Tree-sitter parses each touched file at most once per `mmk
//! review` / `mmk pre-edit` invocation. Pattern A's peer search and
//! Pattern B's import sweep iterate the candidate path list and
//! parse files lazily — both bounded (Pattern B by an explicit
//! peer-scan cap; Pattern A by the directory tree size).

pub mod adapter;
pub mod facts;
pub mod linescan;
pub mod python;
#[path = "rust_lang/mod.rs"]
pub mod rust_lang;
pub mod ts;

pub use adapter::{extract, extract_with_imports, LanguageAdapter};
pub use facts::{
    template_for, ExportFact, ExportKind, FunctionFact, ImportFact, StructuredFacts, TypeDensity,
};

use serde::Serialize;
use std::path::PathBuf;

/// Which structural pattern surfaced a finding. Keep this enum
/// closed so callers can exhaustively switch on it (the `Severity`
/// mapping in review/pre-edit depends on knowing every variant).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthPattern {
    /// Pattern A: file matches the action / contribution registration
    /// shape (e.g. monorepo-style `*.contribution.ts`). Surfaces
    /// nearby peer files as architectural precedent.
    Registration,
    /// Pattern B: file declares an `interface IFoo` plus
    /// `registerSingleton(IFoo, FooImpl)`. Surfaces top consumers
    /// importing the interface.
    Service,
    /// Pattern C: file paired with `<name>.test.ts` / `<name>.spec.ts`
    /// by naming convention. Surfaces the test partner so an edit to
    /// the implementation can re-touch its tests.
    TestPair,
}

impl HealthPattern {
    /// Stable token used in TOML config (`patterns = [...]`) and
    /// JSON output (`health.patterns_evaluated[]`). Kept terse and
    /// lowercase so users can type it.
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::Registration => "registration",
            Self::Service => "service",
            Self::TestPair => "test_pair",
        }
    }

    /// Parse a `token()` string back into a pattern. Returns `None`
    /// for unknown tokens — the caller decides whether to error or
    /// silently drop.
    #[must_use]
    pub fn from_token(s: &str) -> Option<Self> {
        match s {
            "registration" => Some(Self::Registration),
            "service" => Some(Self::Service),
            "test_pair" => Some(Self::TestPair),
            _ => None,
        }
    }
}

/// One Health-layer finding. Self-contained: the caller renders
/// the unified `Finding` shape in mmk-cli without re-querying us.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HealthFinding {
    pub pattern: HealthPattern,
    /// The file the analysis was *about* — typically the path the
    /// agent is editing or just edited.
    pub subject: PathBuf,
    /// Architectural neighbors / partners surfaced by this pattern,
    /// in deterministic order (closest-first for Pattern A's
    /// directory-distance ranking; lexicographic otherwise).
    pub related: Vec<PathBuf>,
}
