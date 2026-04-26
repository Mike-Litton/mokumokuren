//! Configuration for Mokumokuren. Pure data: in-memory defaults, plus a
//! TOML loader for repo-local `mokumokuren.toml` files.

use serde::Serialize;

pub mod file;

pub use file::{BlastRadiusFile, ConfigFile, CouplingFile};

pub const SECONDS_PER_DAY: i64 = 86_400;

/// Default Jaccard threshold for the `--blast-radius` 1-hop neighborhood.
///
/// Loose enough to surface real coupling on young repos; tight enough
/// to filter merge-commit storms. Override via `[blast_radius]
/// threshold = N` in `mokumokuren.toml` or via
/// `--blast-radius-threshold <FLOAT>` on the CLI.
pub const DEFAULT_BLAST_RADIUS_THRESHOLD: f64 = 0.10;

/// Default Jaccard threshold for COUPLING findings.
///
/// Used by `mmk review` and `mmk pre-edit`. Higher than the exploratory
/// blast-radius default — the eval data showed sub-0.30 partners are
/// noise on real JS/TS repos and produce wrong-work demands when an
/// agent acts on them.
pub const DEFAULT_COUPLING_THRESHOLD: f64 = 0.30;

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
    /// Minimum Jaccard a partner must reach for `mmk review` /
    /// `mmk pre-edit` to emit a COUPLING finding. Defaults to
    /// [`DEFAULT_COUPLING_THRESHOLD`].
    pub threshold: f64,
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
    /// Rename-similarity threshold (0.0–1.0) passed to the diff engine.
    pub rename_similarity: f32,
    /// Final ignore globs after merging file + CLI sources. The git layer
    /// reads only this field; how it got populated isn't its concern.
    pub ignores: Vec<String>,
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
