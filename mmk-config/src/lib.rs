//! Configuration for Mokumokuren. Pure data: in-memory defaults, plus a
//! TOML loader for repo-local `mokumokuren.toml` files.

use serde::Serialize;

pub mod file;

pub use file::{BlastRadiusFile, ConfigFile, CouplingFile, HealthFile, HealthTsFile};

pub const SECONDS_PER_DAY: i64 = 86_400;

/// Default Jaccard threshold for the `--blast-radius` 1-hop neighborhood.
///
/// Loose enough to surface real coupling on young repos; tight enough
/// to filter merge-commit storms. Override via `[blast_radius]
/// threshold = N` in `mokumokuren.toml` or via
/// `--blast-radius-threshold <FLOAT>` on the CLI.
pub const DEFAULT_BLAST_RADIUS_THRESHOLD: f64 = 0.10;

/// Deprecated alias for the COUPLING threshold.
///
/// Retained so existing `--coupling-threshold` and `[coupling] threshold`
/// invocations parse; the value is silently mapped to
/// [`DEFAULT_COUPLING_CONFIDENCE_THRESHOLD`] (with a deprecation note in
/// `--verbose` mode). Surfaced in diagnostic echoes of `Config`.
pub const DEFAULT_COUPLING_THRESHOLD: f64 = 0.30;

/// Default Wilson 95 % lower-bound floor for `P(partner | target)`.
///
/// Used by `mmk review` and `mmk pre-edit`. Reads as "I want to know
/// about partners with at least 20 % conditional probability of
/// co-edit, with 95 % statistical confidence." Frequency-invariant —
/// hot files (54 / 203 ≈ 0.27) and quiet files (1 / 1 = 1.0) are
/// scored on the same scale.
pub const DEFAULT_COUPLING_CONFIDENCE_THRESHOLD: f64 = 0.20;

/// Minimum `commits_touching(target)` required before COUPLING fires.
///
/// Defaults to 1 — Wilson's lower bound already handles small-n
/// correctly (a single observation scores `wilson_lower(1, 1) ≈ 0.21`,
/// barely above the default `confidence_threshold = 0.20`). Earlier
/// versions of this constant defaulted to 5 as a defensive floor on
/// top of Wilson; that floor measurably suppressed real co-edits on
/// quiet subjects (most fix commits) without improving precision,
/// because Wilson alone was already gating the small-n cases.
pub const DEFAULT_COUPLING_MIN_SAMPLE_SIZE: u32 = 1;

#[derive(Debug, Clone, Serialize)]
pub struct WindowCfg {
    /// Upper bound on commit age to include in the walk, in days.
    pub days: u32,
    /// Decay half-life (strictly, 1/e point) for recency weighting, in days.
    pub tau_days: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct HotspotCfg {
    pub top_n: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct BulkCfg {
    pub max_files: u32,
    pub max_lines: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct BlastRadiusCfg {
    /// Minimum Jaccard a partner must reach to land in the 1-hop
    /// neighborhood. Defaults to [`DEFAULT_BLAST_RADIUS_THRESHOLD`].
    pub threshold: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct CouplingCfg {
    /// Deprecated alias for the COUPLING firing threshold; silently
    /// mapped to [`Self::confidence_threshold`] when set. Kept so
    /// older configs and CLI invocations keep working.
    pub threshold: f64,
    /// Wilson 95 % lower-bound floor for `P(partner | target)`. A
    /// partner fires COUPLING only if its lower-bound clears this and
    /// `commits_touching(target) ≥ min_sample_size`. Defaults to
    /// [`DEFAULT_COUPLING_CONFIDENCE_THRESHOLD`].
    pub confidence_threshold: f64,
    /// Floor on the binomial sample size (commits touching the
    /// target). Defaults to [`DEFAULT_COUPLING_MIN_SAMPLE_SIZE`].
    pub min_sample_size: u32,
    /// Glob patterns of paths that never trigger a COUPLING finding
    /// as the *missed partner*. Distinct from `ignores`: a workspace's
    /// `package.json` IS legit history; it just shouldn't be demanded
    /// when its sibling workspace's `package.json` was edited.
    pub ignore_partners: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Config {
    pub window: WindowCfg,
    pub hotspot: HotspotCfg,
    pub bulk: BulkCfg,
    pub blast_radius: BlastRadiusCfg,
    pub coupling: CouplingCfg,
    pub health: HealthCfg,
    /// Rename-similarity threshold (0.0–1.0) passed to the diff engine.
    pub rename_similarity: f32,
    /// Final ignore globs after merging file + CLI sources. The git layer
    /// reads only this field; how it got populated isn't its concern.
    pub ignores: Vec<String>,
}

/// `[health]` block — structural-pattern adapter (mmk-health).
///
/// Currently ships a TypeScript adapter only; future implementations
/// will add `rust`, `python`, `go` subblocks alongside `ts`. The
/// whole block defaults to disabled so non-TS users don't get
/// surprised; the `js-ts` profile flips it on with all three
/// patterns enabled.
#[derive(Debug, Clone, Default, Serialize)]
pub struct HealthCfg {
    pub ts: HealthTsCfg,
}

/// `[health.ts]` — TypeScript Health adapter knobs.
#[derive(Debug, Clone, Serialize)]
pub struct HealthTsCfg {
    pub enabled: bool,
    /// Pattern tokens (`registration`, `service`, `test_pair`).
    /// Keep as plain strings here; mmk-health resolves them to
    /// `HealthPattern` enums at the boundary so this crate doesn't
    /// pull in tree-sitter.
    pub patterns: Vec<String>,
}

impl Default for HealthTsCfg {
    fn default() -> Self {
        Self {
            enabled: false,
            patterns: vec!["registration".into(), "service".into(), "test_pair".into()],
        }
    }
}

impl Default for WindowCfg {
    fn default() -> Self {
        Self {
            days: 180,
            tau_days: 90,
        }
    }
}

impl Default for HotspotCfg {
    fn default() -> Self {
        Self { top_n: 20 }
    }
}

impl Default for BulkCfg {
    fn default() -> Self {
        Self {
            max_files: 15,
            max_lines: 1000,
        }
    }
}

impl Default for BlastRadiusCfg {
    fn default() -> Self {
        Self {
            threshold: DEFAULT_BLAST_RADIUS_THRESHOLD,
        }
    }
}

impl Default for CouplingCfg {
    fn default() -> Self {
        Self {
            threshold: DEFAULT_COUPLING_THRESHOLD,
            confidence_threshold: DEFAULT_COUPLING_CONFIDENCE_THRESHOLD,
            min_sample_size: DEFAULT_COUPLING_MIN_SAMPLE_SIZE,
            ignore_partners: Vec::new(),
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            window: WindowCfg::default(),
            hotspot: HotspotCfg::default(),
            bulk: BulkCfg::default(),
            blast_radius: BlastRadiusCfg::default(),
            coupling: CouplingCfg::default(),
            health: HealthCfg::default(),
            rename_similarity: 0.5,
            ignores: Vec::new(),
        }
    }
}

impl Config {
    #[must_use]
    pub fn tau_seconds(&self) -> f64 {
        f64::from(self.window.tau_days) * 86_400.0
    }

    #[must_use]
    pub fn window_seconds(&self) -> i64 {
        i64::from(self.window.days) * SECONDS_PER_DAY
    }
}
