//! Repo-local `mokumokuren.toml` loader.
//!
//! The schema is deliberately minimal: `ignore = [...]` plus an
//! optional `[blast_radius]` block. Everything else (window, top-N,
//! output format) stays on the CLI.

use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::Path;

/// Parsed contents of a `mokumokuren.toml`. Unknown keys are rejected
/// (`#[serde(deny_unknown_fields)]`) so a typo like `ignores = ...`
/// surfaces immediately instead of silently no-opping.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigFile {
    /// Glob patterns to exclude from analysis. Unioned with any
    /// `--ignore` flags on the command line.
    #[serde(default)]
    pub ignore: Vec<String>,
    /// Optional `[blast_radius]` block. Absent → use the in-code
    /// default ([`crate::DEFAULT_BLAST_RADIUS_THRESHOLD`]).
    #[serde(default)]
    pub blast_radius: Option<BlastRadiusFile>,
    /// Optional `[coupling]` block. Absent → use the in-code
    /// default ([`crate::DEFAULT_COUPLING_THRESHOLD`], no ignored partners).
    #[serde(default)]
    pub coupling: Option<CouplingFile>,
    /// Optional `[health]` block. Absent → adapter stays disabled.
    /// The `js-ts` profile flips it on with all three patterns.
    #[serde(default)]
    pub health: Option<HealthFile>,
    /// Optional `[bulk]` block. Absent → in-code defaults
    /// ([`crate::DEFAULT_GREENFIELD_THRESHOLD`]). Today exposes only
    /// the greenfield trigger; max_files / max_lines stay on the
    /// in-code defaults.
    #[serde(default)]
    pub bulk: Option<BulkFile>,
    /// Optional `[sensor]` block — STRUCTURE and COMPLEXITY sensors.
    /// Each subblock is itself optional. Absent → all sensors run on
    /// their in-code defaults.
    #[serde(default)]
    pub sensor: Option<SensorFile>,
}

/// `[blast_radius]` block in `mokumokuren.toml`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlastRadiusFile {
    pub threshold: f64,
}

/// `[bulk]` block in `mokumokuren.toml`.
///
/// Exposes both the per-diff guardrails and the per-commit
/// historical filter so codebases with naturally wider commit
/// grain (workspace projects, infrastructure repos, scaffold-heavy
/// histories) can tune them. The defaults are calibrated for
/// "typical" agent edits; on a repo where real feature work
/// routinely runs ≥30 files, leaving the cap at the default
/// renders most history invisible to the analyzer (the bulk filter
/// drops every wide-grain commit) and reads as "no analyzable
/// history" on files the agent edits — the wording for which is
/// in `messages::quiet_file`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BulkFile {
    #[serde(default)]
    pub greenfield_threshold: Option<f64>,
    /// Glob patterns whose paths are excluded from diff-time BUDGET
    /// accounting (only — they still appear in `review.diff.files`
    /// and in the historical analyzer). Defaults to the in-code
    /// `DEFAULT_BULK_IGNORE_FOR_BUDGET`.
    #[serde(default)]
    pub ignore_for_budget: Vec<String>,
    /// Override the per-commit / per-diff file cap. Affects both
    /// the historical-baseline filter (commits with > this many
    /// files don't contribute to coupling priors) and the
    /// working-tree bulk-self-filter (diffs over the cap silence
    /// HOTSPOT/COUPLING). Default 15. Bump for repos whose natural
    /// commit grain is wider; the LOC cap stays the dominant
    /// guardrail under Cohen's review-effectiveness threshold
    /// either way.
    #[serde(default)]
    pub max_files: Option<u32>,
    /// Override the per-commit / per-diff line cap. Default 1000.
    /// The line cap is the literature-backed half of BUDGET
    /// (Cohen 2006 SmartBear/Cisco data); the file cap above is
    /// an engineering heuristic.
    #[serde(default)]
    pub max_lines: Option<u32>,
}

/// `[coupling]` block in `mokumokuren.toml`.
///
/// Every field is optional so users can pin only what they care
/// about. `threshold` is a deprecated alias retained for back-compat
/// (callers map it to a `--verbose` warning); the active gate is
/// `confidence_threshold` + `min_sample_size`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CouplingFile {
    #[serde(default)]
    pub threshold: Option<f64>,
    #[serde(default)]
    pub confidence_threshold: Option<f64>,
    #[serde(default)]
    pub min_sample_size: Option<u32>,
    #[serde(default)]
    pub ignore_partners: Vec<String>,
}

/// `[health]` block in `mokumokuren.toml`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HealthFile {
    #[serde(default)]
    pub ts: Option<HealthTsFile>,
}

/// `[health.ts]` block in `mokumokuren.toml`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HealthTsFile {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub patterns: Option<Vec<String>>,
}

/// `[sensor]` block — wrapper around the per-sensor sub-blocks. Each
/// subblock is itself fully optional so `[sensor.complexity]
/// enabled = false` works without having to re-declare the
/// structure block.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SensorFile {
    #[serde(default)]
    pub structure: Option<StructureFile>,
    #[serde(default)]
    pub complexity: Option<ComplexityFile>,
    #[serde(default)]
    pub budget_ramp: Option<BudgetRampFile>,
    #[serde(default)]
    pub cohesion: Option<CohesionFile>,
}

/// `[sensor.budget_ramp]` block. Opt-in: under-cap BUDGET ramp
/// findings only fire when `enabled = true`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BudgetRampFile {
    #[serde(default)]
    pub enabled: Option<bool>,
}

/// `[sensor.structure]` block. Every field optional; unset fields
/// fall through to the in-code defaults.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructureFile {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub min_siblings: Option<u32>,
    #[serde(default)]
    pub import_majority: Option<f64>,
    #[serde(default)]
    pub export_template_majority: Option<f64>,
    #[serde(default)]
    pub top_imports_to_show: Option<usize>,
    #[serde(default)]
    pub divergence_min_missing: Option<u32>,
    #[serde(default)]
    pub report_conformance: Option<bool>,
    #[serde(default)]
    pub linescan_fallback: Option<bool>,
}

/// `[sensor.complexity]` block.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComplexityFile {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub nesting_ratio_threshold: Option<f64>,
    #[serde(default)]
    pub nesting_absolute_max: Option<u32>,
    #[serde(default)]
    pub loc_ratio_threshold: Option<f64>,
    #[serde(default)]
    pub loc_absolute_max: Option<u32>,
    #[serde(default)]
    pub min_directory_siblings: Option<u32>,
}

/// `[sensor.cohesion]` block.
///
/// Every field is optional so adopters can pin only the knob they
/// want to tune. Missing fields fall through to the in-code
/// defaults; the rationale for each default lives next to the
/// constant in `lib.rs`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CohesionFile {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub confidence_threshold: Option<f64>,
    #[serde(default)]
    pub min_sample_size: Option<u32>,
    #[serde(default)]
    pub min_files_per_cluster: Option<u32>,
}

impl ConfigFile {
    /// Read and parse a config file at `path`. The error context
    /// includes the path so users can find the source of any parse
    /// failure without further sleuthing.
    pub fn load_from_path(path: &Path) -> Result<Self> {
        let body = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        toml::from_str::<Self>(&body).with_context(|| format!("failed to parse {}", path.display()))
    }
}
