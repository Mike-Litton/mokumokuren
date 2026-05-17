//! Structural-pattern adapter (Health layer) for Mokumokuren.
//!
//! Ships a TypeScript adapter today — test-pair, EVASION
//! (broad-exception delta), and test-weakening (the v0.13 anti-
//! evasion-on-tests sensor). Future implementations will add
//! Rust / Python / Go adapters under `src/<lang>/`.
//!
//! ## Cost
//!
//! Tree-sitter parses each touched file at most once per `mmk
//! review` / `mmk pre-edit` invocation; detectors that take a
//! HEAD body parse it once too.

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
    /// TEST_WEAKENING: net erosion of an existing test file's
    /// strength in the working tree relative to HEAD. Detects skip
    /// decorators added (`.skip` / `.only` / `xit` / `xtest` /
    /// `xdescribe`), assertion / test-case counts decreased, mocks
    /// added (`jest.mock` / `vi.mock`), or `@ts-expect-error` /
    /// `@ts-ignore` markers added. Targets the agent-self-validation
    /// failure mode documented in arXiv:2503.15223 *"Are 'Solved
    /// Issues' in SWE-bench Really Solved Correctly?"* — agents
    /// passing CI by weakening tests rather than fixing the
    /// implementation.
    TestWeakening,
}

impl HealthPattern {
    /// Stable token used in TOML config (`patterns = [...]`) and
    /// JSON output (`health.patterns_evaluated[]`). Kept terse and
    /// lowercase so users can type it.
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::TestPair => "test_pair",
            Self::BroadException => "broad_exception",
            Self::TestWeakening => "test_weakening",
        }
    }

    /// Parse a `token()` string back into a pattern. Returns `None`
    /// for unknown tokens — the caller decides whether to error or
    /// silently drop.
    #[must_use]
    pub fn from_token(s: &str) -> Option<Self> {
        match s {
            "test_pair" => Some(Self::TestPair),
            "broad_exception" => Some(Self::BroadException),
            "test_weakening" => Some(Self::TestWeakening),
            _ => None,
        }
    }

    /// True for patterns that compare working-tree vs HEAD.
    ///
    /// Audit (no HEAD baseline) and pre-edit (working tree *is*
    /// HEAD for the subject) must filter these out — otherwise the
    /// detector parses but always yields zero findings, wasting
    /// the parse.
    #[must_use]
    pub const fn is_delta_mode(self) -> bool {
        matches!(self, Self::BroadException | Self::TestWeakening)
    }
}

/// Optional numeric payload attached to a `HealthFinding`.
///
/// Only `TestWeakening` populates this today; other patterns leave
/// it `None` and the formatter renders from `subject` / `related`
/// alone. Carrying the payload here (instead of on a sibling type)
/// lets `analyze_ts` keep its single return-type signature while
/// renderers still get the per-axis erosion counts without
/// re-parsing the file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum HealthFindingDetail {
    /// Per-axis erosion counts comparing working tree to HEAD for a
    /// test file. Each field is the net working-vs-HEAD addition;
    /// `assertions_lost` is `head_count - working_count` when
    /// strictly positive. Zero fields are still present so consumers
    /// can `jq '.detail.skips_added'` without branching.
    TestWeakening {
        skips_added: u32,
        assertions_lost: u32,
        mocks_added: u32,
        ts_suppressions_added: u32,
        tests_removed: u32,
    },
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
    /// Optional numeric payload. `Some` for `TestWeakening` (carries
    /// per-axis erosion counts); `None` for every other pattern.
    /// Skipped from JSON when absent so the existing JSON shape is
    /// unchanged for old patterns.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<HealthFindingDetail>,
}
