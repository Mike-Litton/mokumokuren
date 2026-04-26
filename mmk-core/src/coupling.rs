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
//! load-bearing signal for an LLM-agent edit decision (motivated by
//! CodeScene's *Pull Risk Forward* and the Hallucinated Coupling
//! failure mode in the LLM-architectures paper).
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

pub mod wilson;

pub use wilson::wilson_lower_95;

/// One co-changing partner for a target file.
///
/// `jaccard` remains the symmetric similarity — it answers "how
/// related are A and B overall" and drives `--blast-radius` (the
/// exploratory neighborhood). `conditional_probability` and
/// `wilson_lower_95` are asymmetric: they answer "given an edit to
/// A's history, how often is B also touched?" — the question the
/// COUPLING finding actually wants.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CouplingEntry {
    pub partner: PathBuf,
    pub jaccard: f64,
    pub co_change_count: u32,
    /// `co_change_count / commits_touching(target)`. Direct point
    /// estimate of `P(partner | target)`.
    pub conditional_probability: f64,
    /// Wilson 95 % lower bound for `conditional_probability`. Used by
    /// `mmk review` / `mmk pre-edit` to decide whether to surface the
    /// partner as a missed-edit warning.
    pub wilson_lower_95: f64,
}

/// Compute, for each path in `targets`, its top-`k` co-changing
/// partners ranked by symmetric jaccard descending.
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
    let mut by_target = collect_couples_for(commits, targets);
    rank_and_truncate(&mut by_target, jaccard_ordering, k);
    by_target
}

/// Same shape as [`top_couples_for`] but ranks each target's
/// partners by [`CouplingEntry::wilson_lower_95`] descending.
#[must_use]
pub fn compute_conditional_couples_for(
    commits: &[Commit],
    targets: &AHashSet<PathBuf>,
    k: usize,
) -> AHashMap<PathBuf, Vec<CouplingEntry>> {
    let mut by_target = collect_couples_for(commits, targets);
    rank_and_truncate(&mut by_target, wilson_ordering, k);
    by_target
}

/// Build the unsorted partner map for `targets`. Single commit walk;
/// caller picks the sort order via [`rank_and_truncate`]. Splitting
/// the collection step from ordering means callers asking for both
/// jaccard- and Wilson-ordered views (analyze + review on the same
/// repo) do the O(commits × files) work once each, not twice.
fn collect_couples_for(
    commits: &[Commit],
    targets: &AHashSet<PathBuf>,
) -> AHashMap<PathBuf, Vec<CouplingEntry>> {
    if targets.is_empty() {
        return AHashMap::new();
    }

    let mut pair_counts: AHashMap<(PathBuf, PathBuf), u32> = AHashMap::new();
    let mut touches: AHashMap<PathBuf, u32> = AHashMap::new();

    for commit in commits {
        // A `FileDelta` is keyed by path (renames fold into a single
        // delta on the new path), so duplicates aren't expected, but
        // be defensive.
        let mut paths: Vec<&PathBuf> = commit.deltas.iter().map(|d| &d.path).collect();
        paths.sort();
        paths.dedup();

        for p in &paths {
            *touches.entry((*p).clone()).or_insert(0) += 1;
        }

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
        let conditional_probability = if touches_t == 0 {
            0.0
        } else {
            f64::from(co) / f64::from(touches_t)
        };
        let wilson_lower_95 = wilson_lower_95(co, touches_t);
        let bucket = by_target.entry(t).or_default();
        bucket.push(CouplingEntry {
            partner,
            jaccard,
            co_change_count: co,
            conditional_probability,
            wilson_lower_95,
        });
    }

    // Drop targets that never appeared in any commit — they have no
    // couples and would otherwise emit empty entries downstream.
    // Keep targets that appeared but had no co-changes (rare, but
    // possible for files only ever touched alone) since "no couples"
    // is itself information.
    by_target.retain(|t, _| touches.contains_key(t));

    by_target
}

fn rank_and_truncate<F>(by_target: &mut AHashMap<PathBuf, Vec<CouplingEntry>>, cmp: F, k: usize)
where
    F: Fn(&CouplingEntry, &CouplingEntry) -> std::cmp::Ordering + Copy,
{
    for entries in by_target.values_mut() {
        entries.sort_by(cmp);
        if k > 0 && entries.len() > k {
            entries.truncate(k);
        }
    }
}

fn jaccard_ordering(a: &CouplingEntry, b: &CouplingEntry) -> std::cmp::Ordering {
    b.jaccard
        .partial_cmp(&a.jaccard)
        .unwrap_or(std::cmp::Ordering::Equal)
        .then_with(|| b.co_change_count.cmp(&a.co_change_count))
        .then_with(|| a.partner.cmp(&b.partner))
}

fn wilson_ordering(a: &CouplingEntry, b: &CouplingEntry) -> std::cmp::Ordering {
    b.wilson_lower_95
        .partial_cmp(&a.wilson_lower_95)
        .unwrap_or(std::cmp::Ordering::Equal)
        .then_with(|| {
            b.conditional_probability
                .partial_cmp(&a.conditional_probability)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .then_with(|| b.co_change_count.cmp(&a.co_change_count))
        .then_with(|| a.partner.cmp(&b.partner))
}

/// One node of a 1-hop blast-radius neighborhood. `hops` is reserved
/// for forward compatibility with multi-hop neighborhoods.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct NeighborhoodNode {
    pub path: PathBuf,
    pub jaccard: f64,
    pub co_change_count: u32,
    pub hops: u32,
}

/// 1-hop blast radius: every partner of `root` whose jaccard ≥ `threshold`.
///
/// `hops` is currently fixed at 1. Other values return `Err` so
/// callers can surface a clean diagnostic instead of silently getting
/// the wrong topology.
pub fn neighborhood(
    commits: &[Commit],
    root: &std::path::Path,
    hops: u32,
    threshold: f64,
) -> anyhow::Result<Vec<NeighborhoodNode>> {
    if hops != 1 {
        return Err(anyhow::anyhow!(
            "neighborhood currently supports only 1-hop blast radius (got {hops})"
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
