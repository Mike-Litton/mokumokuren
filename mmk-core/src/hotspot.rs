//! Hotspot scoring: `log(1 + weighted_churn) * log(1 + loc)`.
//!
//! ## Research lineage
//!
//! The "files that are both big and frequently changed predict
//! defects" finding goes back to Nagappan & Ball *Use of Relative
//! Code Churn Measures to Predict System Defect Density* (ICSE
//! 2005), which established churn-times-size as the dominant
//! per-file defect predictor in industrial codebases. The
//! recency-weighted variant mmk ships matches the "code-as-a-crime-
//! scene" formulation Tornhill popularized in *Your Code as a Crime
//! Scene* (2nd ed., 2024) — exponential time decay applied to the
//! churn axis so old churn fades while still contributing.
//!
//! This is mmk's most directly research-grounded sensor: every
//! component (the two-axis product, the log-damping, the
//! `exp(-age / tau)` weighting) traces to a published finding the
//! detector reproduces.

use ahash::AHashMap;
use serde::Serialize;
use std::path::PathBuf;

use crate::coupling::CouplingEntry;

#[derive(Debug, Clone, Serialize)]
pub struct HotspotEntry {
    pub path: PathBuf,
    pub loc: u32,
    pub weighted_churn: f64,
    pub relative_churn: f64,
    pub hotspot_score: f64,
    pub hotspot_rank: u32,
    pub commits_touching: u32,
    /// Most recent commit timestamp (seconds since Unix epoch) that touched
    /// this file within the analysis window.
    pub last_modified: i64,
    /// Top co-changing partners, populated by a separate step after
    /// `rank()` returns. `rank()` itself leaves this empty.
    #[serde(default)]
    pub top_couples: Vec<CouplingEntry>,
}

/// Inputs to [`rank`]. Bundled in a struct so call sites construct
/// named fields and don't accidentally swap the two `AHashMap<PathBuf,
/// f64>` parameters (`weighted` vs. `relative`).
#[derive(Debug, Clone, Copy)]
pub struct RankInputs<'a> {
    pub weighted: &'a AHashMap<PathBuf, f64>,
    pub relative: &'a AHashMap<PathBuf, f64>,
    pub loc: &'a AHashMap<PathBuf, u32>,
    pub commits_touching: &'a AHashMap<PathBuf, u32>,
    pub last_modified: &'a AHashMap<PathBuf, i64>,
}

/// Compute hotspot scores and return the top `top_n` entries, ranked
/// descending. Files missing from `loc` (deleted from HEAD) are excluded.
///
/// `top_n == 0` returns every ranked entry.
#[must_use]
pub fn rank(inputs: RankInputs<'_>, top_n: usize) -> Vec<HotspotEntry> {
    let RankInputs {
        weighted,
        relative,
        loc,
        commits_touching,
        last_modified,
    } = inputs;
    let mut entries: Vec<HotspotEntry> = weighted
        .iter()
        .filter_map(|(path, &w)| {
            let lines = *loc.get(path)?;
            let score = w.ln_1p() * f64::from(lines).ln_1p();
            Some(HotspotEntry {
                path: path.clone(),
                loc: lines,
                weighted_churn: w,
                relative_churn: relative.get(path).copied().unwrap_or(0.0),
                hotspot_score: score,
                hotspot_rank: 0,
                commits_touching: commits_touching.get(path).copied().unwrap_or(0),
                last_modified: last_modified.get(path).copied().unwrap_or(0),
                top_couples: Vec::new(),
            })
        })
        .collect();

    entries.sort_by(|a, b| {
        b.hotspot_score
            .partial_cmp(&a.hotspot_score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.path.cmp(&b.path))
    });

    if top_n > 0 && entries.len() > top_n {
        entries.truncate(top_n);
    }
    for (idx, entry) in entries.iter_mut().enumerate() {
        // idx is bounded by the file count of the repo; saturation not needed.
        entry.hotspot_rank = (idx + 1) as u32;
    }
    entries
}
