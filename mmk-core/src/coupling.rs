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
//! existing `bulk.max_files = 15` filter, a ~3.2k-commit reference repo
//! is ~1.6M pair-updates — single-digit ms.

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
///
/// # Examples
///
/// Two-file fixture: A and B co-change in two commits, A appears
/// alone in a third. `jaccard(A,B) = 2 / (3 + 2 - 2) = 2/3`.
///
/// ```
/// use ahash::AHashSet;
/// use mmk_core::coupling::top_couples_for;
/// use mmk_core::types::{Commit, CommitInfo, FileDelta};
/// use std::path::PathBuf;
///
/// fn commit(ts: i64, files: &[&str]) -> Commit {
///     Commit {
///         info: CommitInfo {
///             sha: format!("{ts:040x}"),
///             parent_sha: None,
///             timestamp: ts,
///             author_email: "t@example.com".into(),
///         },
///         deltas: files
///             .iter()
///             .map(|p| FileDelta {
///                 path: PathBuf::from(p),
///                 added: 1,
///                 deleted: 0,
///             })
///             .collect(),
///     }
/// }
///
/// let commits = vec![
///     commit(100, &["A", "B"]),
///     commit(200, &["A", "B"]),
///     commit(300, &["A"]),
/// ];
/// let mut targets = AHashSet::new();
/// targets.insert(PathBuf::from("A"));
/// let out = top_couples_for(&commits, &targets, 1);
/// let entries = out.get(&PathBuf::from("A")).unwrap();
/// assert_eq!(entries.len(), 1);
/// assert_eq!(entries[0].partner, PathBuf::from("B"));
/// assert!((entries[0].jaccard - 2.0 / 3.0).abs() < 1e-9);
/// ```
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

/// Group `changed_set` into connected components by historical
/// co-change strength.
///
/// Edges connect two paths when their pairwise co-change clears the
/// confidence + sample-size gate. Components are returned sorted by
/// size descending; paths within each component are sorted
/// lexicographically so output is stable across runs.
///
/// ## Edge metric: symmetrized Wilson on conditional probability
///
/// Given two paths A and B, the edge weight is
///
/// ```text
/// w(A, B) = max(
///     wilson_lower_95(co_change, commits_touching(A)),
///     wilson_lower_95(co_change, commits_touching(B)),
/// )
/// ```
///
/// — the larger of the two directional Wilson lower bounds on
/// `P(other | this)`. An edge is admitted when `w(A, B) ≥
/// confidence_threshold` AND `max(commits_touching(A),
/// commits_touching(B)) ≥ min_sample_size`.
///
/// Why not raw Jaccard:
/// - Jaccard collapses small-sample cases: two files sharing a
///   single commit each yields jaccard = 1.0, which the obvious
///   "above threshold 0.10" gate would treat as load-bearing
///   coupling. This is the same failure mode that motivated
///   COUPLING's migration from raw jaccard to Wilson on the
///   conditional in v0.6 calibration.
///
/// Why not symmetric Wilson on jaccard (Wilson treating co-change as
/// binomial trials over the union):
/// - "Wilson-corrected jaccard" is an ad-hoc construction; the
///   binomial model on union counts misrepresents the underlying
///   process (commits aren't independent Bernoulli trials over the
///   union). No published precedent for the technique.
///
/// Why max-symmetrize the conditional (not min, not average):
/// - Graph clustering has to admit *satellite* membership: a path B
///   whose entire history sits inside A's history belongs in A's
///   cluster, even though jaccard is small (B's history is a small
///   slice of A∪B). The directional `wilson_lower(P(A|B))` captures
///   this — every commit touching B also touched A, so the lower
///   bound is high. The reverse direction `wilson_lower(P(B|A))` is
///   correctly low (most A-commits are solo). Taking `max` keeps
///   the edge; taking `min` or symmetric jaccard would drop it.
///   For COUPLING ("did the agent miss B?") the satellite case is
///   uninteresting; for cohesion clustering it's central.
///
/// Why not the same `confidence_threshold` as COUPLING:
/// - COUPLING's bar (default 0.30) is "actionable miss" — strong
///   enough to demand a re-edit. Cohesion's bar is graph
///   connectivity — strong enough to plausibly belong in the same
///   cluster. The looser bar produces fewer disconnected components
///   on legitimate refactors and tighter ones on tangled diffs;
///   adopters tune via `mmk eval --learn`.
///
/// Lineage: the symmetrized Wilson construction follows
/// Goutte & Gaussier (2005) "A Probabilistic Interpretation of
/// Precision, Recall and F-score" for the IR-side prior art. The
/// cohesion-graph proxy for tangled-change detection itself is
/// looser than Herzig & Zeller (2013) — they untangle at AST /
/// dependency granularity; mmk approximates with co-change-graph
/// cohesion at diff time.
///
/// # Examples
///
/// Three paths, four commits: `A↔B` co-change tightly, `C` is
/// untouched-with-them. The graph splits into two components.
///
/// ```
/// use ahash::AHashSet;
/// use mmk_core::coupling::connected_components_by_wilson;
/// use mmk_core::types::{Commit, CommitInfo, FileDelta};
/// use std::path::PathBuf;
///
/// fn c(ts: i64, files: &[&str]) -> Commit {
///     Commit {
///         info: CommitInfo {
///             sha: format!("{ts:040x}"),
///             parent_sha: None,
///             timestamp: ts,
///             author_email: "t@example.com".into(),
///         },
///         deltas: files.iter().map(|p| FileDelta {
///             path: PathBuf::from(p),
///             added: 1,
///             deleted: 0,
///         }).collect(),
///     }
/// }
///
/// // 3 co-change commits over (A,B), 1 solo touch on C.
/// let commits = vec![
///     c(100, &["A", "B"]),
///     c(200, &["A", "B"]),
///     c(300, &["A", "B"]),
///     c(400, &["C"]),
/// ];
/// let mut changed: AHashSet<PathBuf> = AHashSet::new();
/// changed.insert(PathBuf::from("A"));
/// changed.insert(PathBuf::from("B"));
/// changed.insert(PathBuf::from("C"));
///
/// // Default-ish gate: confidence 0.30, sample-size 3.
/// let comps = connected_components_by_wilson(&commits, &changed, 0.30, 3);
/// assert_eq!(comps.len(), 2);
/// // Largest component first; lex order within.
/// assert_eq!(comps[0], vec![PathBuf::from("A"), PathBuf::from("B")]);
/// assert_eq!(comps[1], vec![PathBuf::from("C")]);
/// ```
#[must_use]
pub fn connected_components_by_wilson(
    commits: &[Commit],
    changed_set: &AHashSet<PathBuf>,
    confidence_threshold: f64,
    min_sample_size: u32,
) -> Vec<Vec<PathBuf>> {
    if changed_set.len() < 2 {
        return changed_set.iter().cloned().map(|p| vec![p]).collect();
    }
    // `collect_couples_for` walks every commit once and counts pair
    // touches restricted to `changed_set`; reusing it keeps the
    // cohesion path on a single shared revwalk with the COUPLING
    // gate.
    let by_target = collect_couples_for(commits, changed_set);

    // Sorted vec for stable union-find indexing — output ordering
    // depends on root-of-component identity, which depends on union
    // order, which depends on iteration order over partners. With
    // unsorted indices the output would be HashMap-iteration-order
    // dependent and break across builds.
    let paths: Vec<PathBuf> = {
        let mut v: Vec<PathBuf> = changed_set.iter().cloned().collect();
        v.sort();
        v
    };
    let index: AHashMap<&PathBuf, usize> = paths.iter().enumerate().map(|(i, p)| (p, i)).collect();

    // commits_touching(p) for each p in changed_set, used as the
    // denominator of the directional conditional probability. Built
    // from the partner records in `by_target`: every entry shares
    // the same `commits_touching(subject)` count, so we can derive
    // it as `co_change_count + (1/conditional - 1) * co_change_count`
    // — but since collect_couples_for already publishes
    // conditional_probability, walk that.
    let touches = touches_from_couples(&by_target);

    let mut parent: Vec<usize> = (0..paths.len()).collect();
    for (subject, partners) in &by_target {
        let Some(&i) = index.get(subject) else {
            continue;
        };
        let n_subject = touches.get(subject).copied().unwrap_or(0);
        for entry in partners {
            if !changed_set.contains(&entry.partner) {
                continue;
            }
            let n_partner = touches.get(&entry.partner).copied().unwrap_or(0);
            if n_subject.max(n_partner) < min_sample_size {
                continue;
            }
            let w_forward = entry.wilson_lower_95;
            let w_reverse = wilson_lower_95(entry.co_change_count, n_partner);
            if w_forward.max(w_reverse) < confidence_threshold {
                continue;
            }
            if let Some(&j) = index.get(&entry.partner) {
                union_find_union(&mut parent, i, j);
            }
        }
    }

    let mut by_root: AHashMap<usize, Vec<PathBuf>> = AHashMap::new();
    for (i, p) in paths.iter().enumerate() {
        let r = union_find_root(&mut parent, i);
        by_root.entry(r).or_default().push(p.clone());
    }
    let mut out: Vec<Vec<PathBuf>> = by_root
        .into_values()
        .map(|mut v| {
            v.sort();
            v
        })
        .collect();
    out.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
    out
}

/// Recover `commits_touching(p)` for every `p` appearing as a
/// subject in the partner map. `collect_couples_for` doesn't expose
/// the touches table directly; this rebuild is fine because each
/// partner record carries `co_change_count` and
/// `conditional_probability = co / commits_touching(subject)`.
fn touches_from_couples(
    by_target: &AHashMap<PathBuf, Vec<CouplingEntry>>,
) -> AHashMap<PathBuf, u32> {
    let mut out: AHashMap<PathBuf, u32> = AHashMap::new();
    for (subject, partners) in by_target {
        // The first partner is enough to derive the denominator
        // (every entry under one subject shares `commits_touching
        // (subject)`). Empty partner lists mean the subject was
        // never co-touched; we infer a baseline of zero, which the
        // sample-floor will reject anyway.
        if let Some(entry) = partners.first() {
            let n = if entry.conditional_probability > 0.0 {
                (f64::from(entry.co_change_count) / entry.conditional_probability).round() as u32
            } else {
                entry.co_change_count
            };
            out.insert(subject.clone(), n);
        }
    }
    out
}

fn union_find_root(parent: &mut [usize], mut x: usize) -> usize {
    while parent[x] != x {
        parent[x] = parent[parent[x]];
        x = parent[x];
    }
    x
}

fn union_find_union(parent: &mut [usize], a: usize, b: usize) {
    let ra = union_find_root(parent, a);
    let rb = union_find_root(parent, b);
    if ra != rb {
        parent[rb] = ra;
    }
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
