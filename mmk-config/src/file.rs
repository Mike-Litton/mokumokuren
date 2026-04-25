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
}

/// `[blast_radius]` block in `mokumokuren.toml`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlastRadiusFile {
    pub threshold: f64,
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
