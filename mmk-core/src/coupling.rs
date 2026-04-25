//! Change coupling: empirical co-change frequency between files.
//!
//! For two files `A` and `B`, define
//!
//! ```text
//! co_change(A, B) = | commits touching both A and B |
//! jaccard(A, B)   = co_change(A, B) / | commits touching A or B |
//! ```
//!
//! Coupling answers "if I touch X, what historically co-changes?" — the
//! load-bearing signal for an LLM-agent edit decision (per the v0.2.0
//! plan, derived from CodeScene's *Pull Risk Forward* and the Hallucinated
//! Coupling failure mode in the LLM-architectures paper).
//!
//! ## Cost
//!
//! `top_couples_for` is targeted: pair counts are only updated where at
//! least one side is in `targets`. With the default top-N = 50 and the
//! existing `bulk.max_files = 15` filter, godot at 3187 commits is ~1.6M
//! pair-updates — single-digit ms.

use ahash::{AHashMap, AHashSet};
use serde::Serialize;
use std::path::PathBuf;

use crate::types::Commit;

/// One co-changing partner for a target file.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CouplingEntry {
    pub partner: PathBuf,
    pub jaccard: f64,
    pub co_change_count: u32,
}

/// Compute, for each path in `targets`, its top-`k` co-changing
/// partners.
///
/// Walks `commits` once. For every commit, only updates pair counts
/// `(t, p)` where `t ∈ targets`; this bounds work to
/// `O(|targets| × files_per_commit)` per commit instead of the full
/// `O(files_per_commit²)`.
///
/// `k == 0` returns every partner, ranked.
#[must_use]
pub fn top_couples_for(
    commits: &[Commit],
    targets: &AHashSet<PathBuf>,
    k: usize,
) -> AHashMap<PathBuf, Vec<CouplingEntry>> {
    if targets.is_empty() {
        return AHashMap::new();
    }

    // (target_path, partner_path) -> co_change_count.
    let mut pair_counts: AHashMap<(PathBuf, PathBuf), u32> = AHashMap::new();
    // Touch counts across the whole window (needed for the jaccard
    // denominator, including for non-target partners). Walking commits
    // once gives us this for free; we'd need it anyway.
    let mut touches: AHashMap<PathBuf, u32> = AHashMap::new();

    for commit in commits {
        // Distinct paths in the commit. A `FileDelta` is keyed by path
        // already (renames fold into a single delta on the new path),
        // so duplicates are not expected, but be defensive.
        let mut paths: Vec<&PathBuf> = commit.deltas.iter().map(|d| &d.path).collect();
        paths.sort();
        paths.dedup();

        for p in &paths {
            *touches.entry((*p).clone()).or_insert(0) += 1;
        }

        // Pair updates: only where at least one side is a target.
        // Iterate target-first so we can short-circuit on commits that
        // touch no targeted files.
        let touched_targets: Vec<&PathBuf> = paths
            .iter()
            .copied()
            .filter(|p| targets.contains(*p))
            .collect();
        if touched_targets.is_empty() {
            continue;
        }

        for t in &touched_targets {
            for other in &paths {
                if other == t {
                    continue;
                }
                *pair_counts
                    .entry(((*t).clone(), (*other).clone()))
                    .or_insert(0) += 1;
            }
        }
    }

    // Bucket pair_counts by target.
    let mut by_target: AHashMap<PathBuf, Vec<CouplingEntry>> =
        targets.iter().map(|t| (t.clone(), Vec::new())).collect();

    for ((t, partner), co) in pair_counts {
        let touches_t = touches.get(&t).copied().unwrap_or(0);
        let touches_p = touches.get(&partner).copied().unwrap_or(0);
        let union = touches_t.saturating_add(touches_p).saturating_sub(co);
        let jaccard = if union == 0 {
            0.0
        } else {
            f64::from(co) / f64::from(union)
        };
        let bucket = by_target.entry(t).or_default();
        bucket.push(CouplingEntry {
            partner,
            jaccard,
            co_change_count: co,
        });
    }

    // Sort each bucket by jaccard desc, then co_change desc, then path asc.
    for entries in by_target.values_mut() {
        entries.sort_by(|a, b| {
            b.jaccard
                .partial_cmp(&a.jaccard)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.co_change_count.cmp(&a.co_change_count))
                .then_with(|| a.partner.cmp(&b.partner))
        });
        if k > 0 && entries.len() > k {
            entries.truncate(k);
        }
    }

    // Drop targets that appeared in `targets` but never showed up in any
    // commit — they have no couples and would otherwise emit empty
    // entries downstream. Keep targets that appeared but had no
    // co-changes (rare, but possible for files only ever touched alone)
    // since "no couples" is itself information.
    by_target.retain(|t, _| touches.contains_key(t));

    by_target
}

/// One node of a 1-hop blast-radius neighborhood. Hop is implicit (1)
/// in v0.2.0; the field exists so that promoting to multi-hop in v0.3
/// is a contained schema change.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct NeighborhoodNode {
    pub path: PathBuf,
    pub jaccard: f64,
    pub co_change_count: u32,
    pub hops: u32,
}

/// 1-hop blast radius: every partner of `root` whose jaccard ≥ `threshold`.
///
/// `hops` is reserved for forward compatibility; v0.2.x rejects
/// anything other than `1` with an error so callers can surface a
/// clean diagnostic. Multi-hop arrives in v0.3.
pub fn neighborhood(
    commits: &[Commit],
    root: &std::path::Path,
    hops: u32,
    threshold: f64,
) -> anyhow::Result<Vec<NeighborhoodNode>> {
    if hops != 1 {
        return Err(anyhow::anyhow!(
            "v0.2.x supports only 1-hop blast radius (got {hops})"
        ));
    }

    let mut targets: AHashSet<PathBuf> = AHashSet::new();
    targets.insert(root.to_path_buf());
    let map = top_couples_for(commits, &targets, 0);
    let Some(couples) = map.get(root) else {
        return Ok(Vec::new());
    };
    Ok(couples
        .iter()
        .filter(|c| c.jaccard >= threshold)
        .map(|c| NeighborhoodNode {
            path: c.partner.clone(),
            jaccard: c.jaccard,
            co_change_count: c.co_change_count,
            hops: 1,
        })
        .collect())
}
