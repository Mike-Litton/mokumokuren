//! Configuration for Mokumokuren. v0.1 ships only in-memory defaults and
//! CLI-driven overrides — no file loading yet (that lands in v0.2 when it has
//! to contend with the agent/ci profiles).

use serde::Serialize;

pub const SECONDS_PER_DAY: i64 = 86_400;

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
pub struct Config {
    pub window: WindowCfg,
    pub hotspot: HotspotCfg,
    pub bulk: BulkCfg,
    /// Rename-similarity threshold (0.0–1.0) passed to the diff engine.
    pub rename_similarity: f32,
    /// User-supplied ignore globs (e.g. `vendor/**`).
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

impl Default for Config {
    fn default() -> Self {
        Self {
            window: WindowCfg::default(),
            hotspot: HotspotCfg::default(),
            bulk: BulkCfg::default(),
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
