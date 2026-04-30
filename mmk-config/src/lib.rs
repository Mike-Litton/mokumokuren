//! Configuration for Mokumokuren. Pure data: in-memory defaults, plus a
//! TOML loader for repo-local `mokumokuren.toml` files.

use serde::Serialize;

pub mod file;

pub use file::{
    BlastRadiusFile, BudgetRampFile, BulkFile, CohesionFile, ComplexityFile, ConfigFile,
    CouplingFile, HealthFile, HealthTsFile, SensorFile, StructureFile,
};

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
/// about partners with at least 30 % conditional probability of
/// co-edit, with 95 % statistical confidence." Frequency-invariant —
/// hot files (54 / 203 ≈ 0.27) and quiet files (1 / 1 = 1.0) are
/// scored on the same scale. v0.6 bumped this from 0.20: at 0.20 the
/// n=1 Wilson floor (`wilson_lower(1, 1) ≈ 0.206`) just barely cleared,
/// surfacing single-observation co-edits that agent test runs flagged
/// as load-bearing-feeling false positives. v0.7 will retune from
/// `mmk eval --replay` data across multiple repos.
pub const DEFAULT_COUPLING_CONFIDENCE_THRESHOLD: f64 = 0.30;

/// Default `[bulk] greenfield_threshold`.
///
/// When more than this fraction of `mmk review`'s diff is files the
/// historical analyzer has never seen, the review emits an explicit
/// "history priors don't apply" finding instead of letting the agent
/// guess why HOTSPOT/COUPLING are silent. 0.5 reads as "more new than
/// modified."
pub const DEFAULT_GREENFIELD_THRESHOLD: f64 = 0.5;

/// `[sensor.structure]` defaults.
///
/// 3 sibling files is the floor for declaring a directory has a
/// convention worth surfacing: two siblings is *too* easy to pattern
/// match on (every `mod.rs`/`lib.rs` pair would fire). 0.85 majority
/// is the calibrated reading after the first agent-eval session: at
/// 0.66, sibling directories with heterogeneous file roles
/// (e.g. routers where 4 of 5 import zod for input validation and
/// the 5th is a CRUD wrapper that legitimately doesn't) produced
/// false positives the agent had to mentally filter on every write.
/// 0.85 reads as "near-unanimous consensus," which is the right bar
/// for declaring a convention worth flagging divergence from.
/// `mmk eval --learn` reports per-repo fire rate so adopters can
/// see whether the defaults match their codebase.
pub const DEFAULT_STRUCTURE_MIN_SIBLINGS: u32 = 3;
pub const DEFAULT_STRUCTURE_IMPORT_MAJORITY: f64 = 0.85;
pub const DEFAULT_STRUCTURE_EXPORT_TEMPLATE_MAJORITY: f64 = 0.85;
pub const DEFAULT_STRUCTURE_TOP_IMPORTS_TO_SHOW: usize = 6;
pub const DEFAULT_STRUCTURE_DIVERGENCE_MIN_MISSING: u32 = 1;

/// `[sensor.structure] role_patterns` defaults.
///
/// Files matching any of these stem-suffix patterns are treated as
/// architectural-role files: STRUCTURE demotes divergence findings
/// from `Warn` to `Info` because the divergence is expected (factory,
/// registration, contribution files legitimately diverge from
/// sibling shape conventions). Patterns are stem-suffix matches —
/// `*<suffix>` checks whether the file stem (before final extension)
/// ends with `<suffix>`. Override or extend per-repo via
/// `[structure] role_patterns = [...]` in `mokumokuren.toml`.
///
/// The shipped patterns each accumulated dismissable Warn fires that
/// read as "role file diverges from directory shape" — exactly the
/// role / shape conflation this suppression separates. v0.9 adds
/// re-exporter / platform-entry-point patterns (`*index`,
/// `*extension`, `*.barrel`, `*Barrel`) — same divergence pattern,
/// same exemption.
pub const DEFAULT_STRUCTURE_ROLE_PATTERNS: &[&str] = &[
    "*.contribution",
    "*Factory",
    "*.action",
    "*.actions",
    "*Registry",
    "*.module",
    "*Module",
    "*.routes",
    "*.config",
    "*extension",
    "*index",
    "*.barrel",
    "*Barrel",
];

/// `[bulk] ignore_for_budget` default — empty.
///
/// Globs in this list are excluded from the *diff-time* BUDGET
/// accounting (bulk-self-filter and the over-cap / under-cap ramp
/// triggers) so a generated-file regeneration (e.g. a router
/// codegen output) doesn't trip BUDGET on every edit and silence
/// HOTSPOT/COUPLING for the rest of the session. Distinct from
/// `ignores` (which excludes paths from history analysis entirely)
/// and from the per-commit historical bulk filter in `mmk-git`,
/// which stays conservative. v0.6 ships an empty default; v0.7
/// retunes from `mmk eval --replay` data.
pub const DEFAULT_BULK_IGNORE_FOR_BUDGET: &[&str] = &[];

/// `[sensor.complexity]` defaults.
///
/// The relative thresholds catch outliers within a permissive
/// directory; the absolute thresholds catch files in directories
/// that are uniformly bad. Code Red's biomarker bundle lists nesting
/// and function size as the two strongest per-function defect
/// signals; the absolute caps are a conservative reading meant to be
/// lowered by `mmk eval --learn` once per-repo distribution data
/// exists.
pub const DEFAULT_COMPLEXITY_NESTING_RATIO: f64 = 3.0;
pub const DEFAULT_COMPLEXITY_NESTING_ABS_MAX: u32 = 6;
pub const DEFAULT_COMPLEXITY_LOC_RATIO: f64 = 3.0;
pub const DEFAULT_COMPLEXITY_LOC_ABS_MAX: u32 = 80;
pub const DEFAULT_COMPLEXITY_MIN_DIRECTORY_SIBLINGS: u32 = 3;

/// `[sensor.complexity]` delta-weighted severity defaults.
///
/// COMPLEXITY findings on pre-existing functions get severity from
/// the agent's actual contribution (Δ vs HEAD), not from the
/// absolute metric. New files / new functions stay `Warn`. For an
/// existing function whose metric ticked up by 1 above an absolute
/// cap, demote to `Info` — the agent's edit is small even if the
/// function was already over. Only when the delta clears either of
/// these thresholds does the finding earn `Warn`. Defaults derived
/// from the v0.8 N=20 calibration cohort, where 51 of 51 COMPLEXITY
/// fires landed on pre-existing oversized functions and only ~6 %
/// converted to refactors.
pub const DEFAULT_COMPLEXITY_DELTA_WARN_PCT: f64 = 0.50;
pub const DEFAULT_COMPLEXITY_DELTA_WARN_ABS: u32 = 20;

/// `[sensor.cohesion]` defaults.
///
/// Cohesion gates a graph-connectivity question: are these N
/// changed files in the diff plausibly part of one historical
/// cluster, or do they decompose into multiple disjoint clusters?
/// The bar is looser than COUPLING's because connectivity is a
/// softer claim than "you missed an edit." Calibration knobs only;
/// the math behind them lives in
/// `mmk_core::coupling::connected_components_by_wilson`.
///
/// `confidence_threshold = 0.20`: the Wilson lower bound on the
/// directional conditional co-change probability needed for an edge.
/// Below COUPLING's 0.30 because cohesion edges that don't merge
/// distinct historical clusters are silently fine; missing an edge
/// that *should* connect a cluster fragments the graph and produces
/// a false "tangled diff" finding. The looser bar is the right
/// failure-mode trade.
///
/// `min_sample_size = 3`: at least one endpoint of the edge needs
/// three commits of history. Same floor as COUPLING — single-commit
/// pairs would otherwise reach Wilson's small-sample lower bound
/// (≈0.21) and admit edges with no real evidence.
///
/// `min_files_per_cluster = 2`: a "cluster" with one file is just
/// an isolated change, not a cluster. Singleton greenfield files
/// (no commit history) are dropped before the count to avoid
/// flagging "added one new file alongside two coupled ones" as a
/// tangled diff.
pub const DEFAULT_COHESION_CONFIDENCE_THRESHOLD: f64 = 0.20;
pub const DEFAULT_COHESION_MIN_SAMPLE_SIZE: u32 = 3;
pub const DEFAULT_COHESION_MIN_FILES_PER_CLUSTER: u32 = 2;

/// Minimum `commits_touching(target)` required before COUPLING fires.
///
/// v0.4 set this to 5 (defensive floor over Wilson). v0.5 dropped to
/// 1 (Wilson alone, since the lower bound handles small-n honestly).
/// v0.6 calibrated to 3: with `confidence_threshold = 0.30`, the n=1
/// case (Wilson 0.21) already fails confidence, but n=2/2 still has
/// Wilson ≈ 0.34 and would fire on a single coincidental co-edit.
/// `min_sample_size = 3` enforces "at least three commits of subject
/// history" before COUPLING infers anything, which agent test runs
/// validated as the right bar for the gate without measurably
/// suppressing real coupling. v0.7 will retune from
/// `mmk eval --replay` data across 5+ reference repos.
pub const DEFAULT_COUPLING_MIN_SAMPLE_SIZE: u32 = 3;

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
    /// Fraction in `[0.0, 1.0]`. When the working-tree diff's
    /// new-file fraction exceeds this, `mmk review` emits a single
    /// explicit greenfield acknowledgement so the agent doesn't have
    /// to guess why history-based layers are silent.
    pub greenfield_threshold: f64,
    /// Glob patterns whose paths are excluded from diff-time BUDGET
    /// accounting (bulk-self-filter, over-cap trigger, under-cap
    /// ramp). The full diff still appears in `review.diff.files[]`
    /// — this only affects the BUDGET layer's gross-vs-net counts.
    /// Defaults to [`DEFAULT_BULK_IGNORE_FOR_BUDGET`] (empty).
    pub ignore_for_budget: Vec<String>,
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
    pub sensor: SensorCfg,
    /// Rename-similarity threshold (0.0–1.0) passed to the diff engine.
    pub rename_similarity: f32,
    /// Final ignore globs after merging file + CLI sources. The git layer
    /// reads only this field; how it got populated isn't its concern.
    pub ignores: Vec<String>,
}

/// `[sensor]` block — directory-aggregated and per-function
/// architecture-fitness sensors that don't depend on git history.
/// Each subblock can be flipped independently.
#[derive(Debug, Clone, Default, Serialize)]
pub struct SensorCfg {
    pub structure: StructureCfg,
    pub complexity: ComplexityCfg,
    pub budget_ramp: BudgetRampCfg,
    pub cohesion: CohesionCfg,
}

/// `[sensor.budget_ramp]` — under-cap continuous BUDGET feedback.
///
/// On by default. `mmk review` and `mmk pre-edit` emit a progressive
/// Info @ ≥50% of cap and Warn @ ≥75% of cap so the agent sees the
/// meter climbing before it snaps over the line. The over-cap BUDGET
/// finding is unaffected — it always fires when the diff exceeds
/// `bulk.max_files` or `bulk.max_lines`. Set `enabled = false` to
/// silence the under-cap ramp.
#[derive(Debug, Clone, Serialize)]
pub struct BudgetRampCfg {
    pub enabled: bool,
}

impl Default for BudgetRampCfg {
    fn default() -> Self {
        Self { enabled: true }
    }
}

/// `[sensor.structure]` — directory-convention sensor.
#[derive(Debug, Clone, Serialize)]
pub struct StructureCfg {
    pub enabled: bool,
    /// Floor on sibling count before declaring a convention exists.
    pub min_siblings: u32,
    /// Fraction of siblings that must share an import for it to be
    /// declared "common" to the directory.
    pub import_majority: f64,
    /// Same fraction for export templates (e.g. `Create*Dialog`).
    pub export_template_majority: f64,
    /// Cap on imports listed in a single finding so the message
    /// stays scannable.
    pub top_imports_to_show: usize,
    /// Min missing common-imports for review-mode divergence to fire.
    pub divergence_min_missing: u32,
    /// When true, also emit Severity::Ok for new files that *match*
    /// the convention. Off by default — keeps review terse; on lets
    /// calibration runs verify the sensor fires correctly.
    pub report_conformance: bool,
    /// When true, fall back to the line-scan import extractor for
    /// languages without a real AST adapter (currently anything but
    /// TS). Imports-only signal — exports / templates won't be
    /// considered for those files.
    pub linescan_fallback: bool,
    /// Role-file stem-suffix patterns. Files matching any pattern
    /// have their STRUCTURE Warn findings demoted to Info — role
    /// divergence is expected. See [`DEFAULT_STRUCTURE_ROLE_PATTERNS`]
    /// for the rationale and shipped defaults.
    pub role_patterns: Vec<String>,
}

impl Default for StructureCfg {
    fn default() -> Self {
        Self {
            enabled: true,
            min_siblings: DEFAULT_STRUCTURE_MIN_SIBLINGS,
            import_majority: DEFAULT_STRUCTURE_IMPORT_MAJORITY,
            export_template_majority: DEFAULT_STRUCTURE_EXPORT_TEMPLATE_MAJORITY,
            top_imports_to_show: DEFAULT_STRUCTURE_TOP_IMPORTS_TO_SHOW,
            divergence_min_missing: DEFAULT_STRUCTURE_DIVERGENCE_MIN_MISSING,
            report_conformance: false,
            linescan_fallback: true,
            role_patterns: DEFAULT_STRUCTURE_ROLE_PATTERNS
                .iter()
                .map(|s| (*s).to_string())
                .collect(),
        }
    }
}

/// `[sensor.complexity]` — per-function structural budget.
#[derive(Debug, Clone, Serialize)]
pub struct ComplexityCfg {
    pub enabled: bool,
    pub nesting_ratio_threshold: f64,
    pub nesting_absolute_max: u32,
    pub loc_ratio_threshold: f64,
    pub loc_absolute_max: u32,
    /// Below this many siblings, only the absolute thresholds apply
    /// — the directory median has too little support to drive a
    /// ratio-based finding.
    pub min_directory_siblings: u32,
    /// Δ-percent threshold above which a pre-existing function's
    /// finding earns `Warn`. Below this AND below `delta_warn_abs`,
    /// the formatter demotes to `Info`. Defaults to
    /// [`DEFAULT_COMPLEXITY_DELTA_WARN_PCT`].
    pub delta_warn_pct: f64,
    /// Δ-absolute threshold above which a pre-existing function's
    /// finding earns `Warn`. Defaults to
    /// [`DEFAULT_COMPLEXITY_DELTA_WARN_ABS`].
    pub delta_warn_abs: u32,
}

impl Default for ComplexityCfg {
    fn default() -> Self {
        Self {
            enabled: true,
            nesting_ratio_threshold: DEFAULT_COMPLEXITY_NESTING_RATIO,
            nesting_absolute_max: DEFAULT_COMPLEXITY_NESTING_ABS_MAX,
            loc_ratio_threshold: DEFAULT_COMPLEXITY_LOC_RATIO,
            loc_absolute_max: DEFAULT_COMPLEXITY_LOC_ABS_MAX,
            min_directory_siblings: DEFAULT_COMPLEXITY_MIN_DIRECTORY_SIBLINGS,
            delta_warn_pct: DEFAULT_COMPLEXITY_DELTA_WARN_PCT,
            delta_warn_abs: DEFAULT_COMPLEXITY_DELTA_WARN_ABS,
        }
    }
}

/// `[sensor.cohesion]` — tangled-diff detection via co-change-graph
/// cohesion.
///
/// Fires when a working-tree diff partitions into multiple disjoint
/// connected components in the historical co-change graph. The
/// failure mode is well-evidenced (Herzig & Zeller 2013 on tangled
/// changes inflating revert / review cost); mmk's implementation is
/// a structural-fingerprint proxy at diff granularity rather than
/// the AST-level untangling those papers describe. Severity is
/// Info — the sensor names a pattern, it doesn't prescribe a fix.
#[derive(Debug, Clone, Serialize)]
pub struct CohesionCfg {
    pub enabled: bool,
    /// Wilson 95 % lower-bound floor for the symmetrized edge
    /// metric. See [`DEFAULT_COHESION_CONFIDENCE_THRESHOLD`] for
    /// the motivation behind the default.
    pub confidence_threshold: f64,
    /// Minimum value of `max(commits_touching(A),
    /// commits_touching(B))` for the pair to admit an edge. Filters
    /// the small-sample pairs Wilson alone wouldn't fully reject.
    pub min_sample_size: u32,
    /// Minimum cluster size for a component to count toward the
    /// fire decision. Singletons aren't clusters; cohesion needs
    /// ≥2 multi-file groups to claim "tangled."
    pub min_files_per_cluster: u32,
}

impl Default for CohesionCfg {
    fn default() -> Self {
        Self {
            enabled: true,
            confidence_threshold: DEFAULT_COHESION_CONFIDENCE_THRESHOLD,
            min_sample_size: DEFAULT_COHESION_MIN_SAMPLE_SIZE,
            min_files_per_cluster: DEFAULT_COHESION_MIN_FILES_PER_CLUSTER,
        }
    }
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
    /// Pattern tokens (`registration`, `service`, `test_pair`,
    /// `broad_exception`). Keep as plain strings here; mmk-health
    /// resolves them to `HealthPattern` enums at the boundary so this
    /// crate doesn't pull in tree-sitter.
    pub patterns: Vec<String>,
}

impl Default for HealthTsCfg {
    fn default() -> Self {
        Self {
            enabled: false,
            patterns: vec![
                "registration".into(),
                "service".into(),
                "test_pair".into(),
                "broad_exception".into(),
            ],
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
            greenfield_threshold: DEFAULT_GREENFIELD_THRESHOLD,
            ignore_for_budget: DEFAULT_BULK_IGNORE_FOR_BUDGET
                .iter()
                .map(|s| (*s).to_string())
                .collect(),
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
            sensor: SensorCfg::default(),
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
