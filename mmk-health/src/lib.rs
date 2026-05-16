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
    /// EVASION: a non-top-level broad TS/JS catch handler was added
    /// in the working tree relative to HEAD. "Broad" means empty
    /// catch body, no parameter, a parameter typed as
    /// `any | unknown | Error`, or a body that is exclusively log
    /// calls on a configured log identifier (the dominant TS
    /// log-and-swallow shape, v0.12). Targets the *"evasive repairs
    /// with try-except blocks"* failure mode named in
    /// arXiv:2509.13941.
    BroadException,
    /// BroadCatchDebt: static count of non-top-level broad TS/JS
    /// catch handlers in the working tree. Audit-mode counterpart to
    /// `BroadException` — no HEAD comparison, fires on accumulated
    /// debt. Reuses the same `is_broad` predicate so the same
    /// shapes count in both modes. v0.12.
    BroadCatchDebt,
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
            Self::BroadException => "broad_exception",
            Self::BroadCatchDebt => "broad_catch_debt",
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
            "broad_exception" => Some(Self::BroadException),
            "broad_catch_debt" => Some(Self::BroadCatchDebt),
            _ => None,
        }
    }
}

/// Optional numeric payload attached to a `HealthFinding`.
///
/// Only `BroadCatchDebt` populates this today; other patterns leave
/// it `None` and the formatter renders from `subject` / `related`
/// alone. Carrying the payload here (instead of on a sibling type)
/// lets `analyze_ts` keep its single return-type signature while
/// audit-mode renderers still get the count + line numbers without
/// re-parsing the file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum HealthFindingDetail {
    /// Static count of broad non-top-level catch handlers in the
    /// working tree, plus the 1-based line numbers of each handler.
    BroadCatchDebt { count: u32, lines: Vec<usize> },
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
    /// Optional numeric payload. `Some` for `BroadCatchDebt`
    /// (carries count + line numbers); `None` for every other
    /// pattern. Skipped from JSON when absent so the existing JSON
    /// shape is unchanged for old patterns.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<HealthFindingDetail>,
}
