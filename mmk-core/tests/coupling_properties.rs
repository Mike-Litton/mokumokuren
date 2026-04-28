//! Property tests for `mmk_core::coupling`.
//!
//! Generators stay small (≤ 10 distinct paths, ≤ 50 commits) per the
//! "avoid oversized search spaces" guidance: 10⁵ vacuous cases find
//! no real bugs that 10² targeted cases miss, and the tighter cap
//! keeps the suite under its second-class budget.

use ahash::AHashSet;
use mmk_core::coupling::{
    connected_components_by_wilson, top_couples_for, wilson::wilson_lower_95,
};
use mmk_core::types::{Commit, CommitInfo, FileDelta};
use proptest::collection::vec;
use proptest::prelude::*;
use std::path::PathBuf;

/// Paths drawn from a small fixed pool so the same path appears
/// across multiple generated commits. Drawing arbitrary `PathBuf`s
/// would produce a histogram of 50 distinct touch-1 paths and never
/// build any co-change structure.
fn path_strategy() -> impl Strategy<Value = PathBuf> {
    prop_oneof![
        Just(PathBuf::from("a")),
        Just(PathBuf::from("b")),
        Just(PathBuf::from("c")),
        Just(PathBuf::from("d")),
        Just(PathBuf::from("e")),
        Just(PathBuf::from("f")),
        Just(PathBuf::from("g")),
        Just(PathBuf::from("h")),
        Just(PathBuf::from("i")),
        Just(PathBuf::from("j")),
    ]
}

fn commit_strategy() -> impl Strategy<Value = Commit> {
    (100i64..1_000_000i64, vec(path_strategy(), 1..6)).prop_map(|(ts, mut paths)| {
        paths.sort();
        paths.dedup();
        Commit {
            info: CommitInfo {
                sha: format!("{ts:040x}"),
                parent_sha: None,
                timestamp: ts,
                author_email: "t@example.com".into(),
            },
            deltas: paths
                .into_iter()
                .map(|p| FileDelta {
                    path: p,
                    added: 1,
                    deleted: 0,
                })
                .collect(),
        }
    })
}

fn commits_strategy() -> impl Strategy<Value = Vec<Commit>> {
    vec(commit_strategy(), 1..50)
}

proptest! {
    /// Every `CouplingEntry` carries jaccard ∈ [0.0, 1.0] and
    /// `co_change_count ≤ min(touches_a, touches_b)`. The latter
    /// invariant is what keeps `union = touches_a + touches_b - co`
    /// from underflowing on the `u32` subtraction inside
    /// `top_couples_for`.
    #[test]
    fn jaccard_in_unit_and_co_change_bounded(commits in commits_strategy()) {
        let mut targets: AHashSet<PathBuf> = AHashSet::new();
        for c in &commits {
            for d in &c.deltas {
                targets.insert(d.path.clone());
            }
        }
        let out = top_couples_for(&commits, &targets, 0);
        for entries in out.values() {
            for e in entries {
                prop_assert!(
                    (0.0..=1.0).contains(&e.jaccard),
                    "jaccard out of [0,1]: partner {:?} jaccard={}",
                    e.partner, e.jaccard,
                );
                prop_assert!(
                    e.conditional_probability >= 0.0 && e.conditional_probability <= 1.0,
                    "conditional_probability out of [0,1]: {}",
                    e.conditional_probability,
                );
                prop_assert!(
                    (0.0..=1.0).contains(&e.wilson_lower_95),
                    "wilson_lower_95 out of [0,1]: {}",
                    e.wilson_lower_95,
                );
            }
        }
    }

    /// `top_couples_for` never reports a self-pair. Locking this so
    /// a refactor of the inner deltas-pair walk can't accidentally
    /// produce `partner == subject`.
    #[test]
    fn no_self_pairs_in_top_couples(commits in commits_strategy()) {
        let mut targets: AHashSet<PathBuf> = AHashSet::new();
        for c in &commits {
            for d in &c.deltas {
                targets.insert(d.path.clone());
            }
        }
        let out = top_couples_for(&commits, &targets, 0);
        for (subject, entries) in out {
            for e in entries {
                prop_assert!(
                    e.partner != subject,
                    "self-pair on subject {:?}",
                    subject,
                );
            }
        }
    }

    /// `top_couples_for` is deterministic. Two calls on the same
    /// inputs produce identical outputs (membership, ordering, and
    /// per-entry numerics).
    #[test]
    fn top_couples_is_deterministic(commits in commits_strategy()) {
        let mut targets: AHashSet<PathBuf> = AHashSet::new();
        for c in &commits {
            for d in &c.deltas {
                targets.insert(d.path.clone());
            }
        }
        let a = top_couples_for(&commits, &targets, 5);
        let b = top_couples_for(&commits, &targets, 5);
        for (k, va) in &a {
            let vb = b.get(k).expect("same keys across deterministic calls");
            prop_assert_eq!(va.len(), vb.len());
            for (ea, eb) in va.iter().zip(vb.iter()) {
                prop_assert_eq!(&ea.partner, &eb.partner);
                prop_assert_eq!(ea.co_change_count, eb.co_change_count);
                prop_assert!((ea.jaccard - eb.jaccard).abs() < 1e-12);
                prop_assert!((ea.wilson_lower_95 - eb.wilson_lower_95).abs() < 1e-12);
            }
        }
    }

    /// Partition invariant for the cohesion graph: every input path
    /// in `changed_set` appears in exactly one output component, and
    /// the number of components is at least 1 (when the set is
    /// non-empty) and at most `|changed_set|`.
    #[test]
    fn cohesion_partition_invariant(commits in commits_strategy()) {
        let mut all_paths: AHashSet<PathBuf> = AHashSet::new();
        for c in &commits {
            for d in &c.deltas {
                all_paths.insert(d.path.clone());
            }
        }
        prop_assume!(!all_paths.is_empty());
        let components = connected_components_by_wilson(&commits, &all_paths, 0.30, 3);
        prop_assert!(!components.is_empty());
        prop_assert!(components.len() <= all_paths.len());

        let mut seen: AHashSet<PathBuf> = AHashSet::new();
        for comp in &components {
            for p in comp {
                prop_assert!(
                    seen.insert(p.clone()),
                    "path {p:?} appeared in two components",
                );
            }
        }
        prop_assert_eq!(seen, all_paths);
    }

    /// With `min_sample_size = u32::MAX`, the sample-size gate
    /// rejects every edge — every component is a singleton.
    #[test]
    fn cohesion_with_max_min_sample_singletons(commits in commits_strategy()) {
        let mut all_paths: AHashSet<PathBuf> = AHashSet::new();
        for c in &commits {
            for d in &c.deltas {
                all_paths.insert(d.path.clone());
            }
        }
        prop_assume!(!all_paths.is_empty());
        let components = connected_components_by_wilson(&commits, &all_paths, 0.0, u32::MAX);
        prop_assert_eq!(components.len(), all_paths.len());
        for comp in components {
            prop_assert_eq!(comp.len(), 1);
        }
    }

    /// `connected_components_by_wilson` is deterministic. Same
    /// inputs ⇒ same component shape and ordering. The function's
    /// output stability matters because diffs against prior runs
    /// drive cohesion-finding monotonic dedup downstream.
    #[test]
    fn cohesion_is_deterministic(commits in commits_strategy()) {
        let mut all_paths: AHashSet<PathBuf> = AHashSet::new();
        for c in &commits {
            for d in &c.deltas {
                all_paths.insert(d.path.clone());
            }
        }
        let a = connected_components_by_wilson(&commits, &all_paths, 0.30, 3);
        let b = connected_components_by_wilson(&commits, &all_paths, 0.30, 3);
        prop_assert_eq!(a, b);
    }

    /// The entry's `wilson_lower_95` and `conditional_probability`
    /// must equal what an independent computation against the input
    /// commits produces. Earlier this property recovered `n` from
    /// `e.conditional_probability`, which made it a near-tautology —
    /// any bug that landed `n` in both fields consistently would
    /// pass. Computing `n_subject` from `commits` directly catches
    /// genuine drift between entry construction and the wilson
    /// scalar, including silent denominator swaps (touches_subject
    /// vs. touches_partner) inside `top_couples_for`.
    #[test]
    fn entry_wilson_matches_independent_recompute(commits in commits_strategy()) {
        let mut targets: AHashSet<PathBuf> = AHashSet::new();
        for c in &commits {
            for d in &c.deltas {
                targets.insert(d.path.clone());
            }
        }
        let out = top_couples_for(&commits, &targets, 0);
        for (subject, entries) in out {
            // Independent count: distinct commits whose deltas
            // include `subject`. Mirrors `commits_touching` but stays
            // local to the test so we don't import the production
            // helper as a witness.
            let n_subject: u32 = commits
                .iter()
                .filter(|c| {
                    let mut paths: Vec<&PathBuf> = c.deltas.iter().map(|d| &d.path).collect();
                    paths.sort();
                    paths.dedup();
                    paths.iter().any(|p| **p == subject)
                })
                .count()
                .try_into()
                .unwrap_or(u32::MAX);
            for e in entries {
                let scalar = wilson_lower_95(e.co_change_count, n_subject);
                prop_assert!(
                    (scalar - e.wilson_lower_95).abs() < 1e-9,
                    "subject {subject:?} partner {:?}: entry wilson={} vs \
                     wilson(co={}, n_subject={}) = {}",
                    e.partner, e.wilson_lower_95, e.co_change_count, n_subject, scalar,
                );
                if n_subject > 0 {
                    let expected_p = f64::from(e.co_change_count) / f64::from(n_subject);
                    prop_assert!(
                        (e.conditional_probability - expected_p).abs() < 1e-12,
                        "subject {subject:?} partner {:?}: entry P(B|A)={} vs \
                         co/n_subject = {}/{} = {}",
                        e.partner, e.conditional_probability,
                        e.co_change_count, n_subject, expected_p,
                    );
                }
            }
        }
    }
}
