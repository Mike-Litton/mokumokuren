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

/// `[bulk]` block in `mokumokuren.toml`. Currently exposes only
/// `greenfield_threshold`; the per-commit `max_files` / `max_lines`
/// stay on the in-code defaults to keep the surface stable.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BulkFile {
    #[serde(default)]
    pub greenfield_threshold: Option<f64>,
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
