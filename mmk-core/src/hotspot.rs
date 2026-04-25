//! Hotspot scoring: `log(1 + weighted_churn) * log(1 + loc)`.

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

/// Compute hotspot scores and return the top `top_n` entries, ranked
/// descending. Files missing from `loc` (deleted from HEAD) are excluded.
///
/// `top_n == 0` returns every ranked entry.
#[must_use]
pub fn rank(
    weighted: &AHashMap<PathBuf, f64>,
    relative: &AHashMap<PathBuf, f64>,
    loc: &AHashMap<PathBuf, u32>,
    commits_touching: &AHashMap<PathBuf, u32>,
    last_modified: &AHashMap<PathBuf, i64>,
    top_n: usize,
) -> Vec<HotspotEntry> {
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
        entry.hotspot_rank = u32::try_from(idx + 1).unwrap_or(u32::MAX);
    }
    entries
}
