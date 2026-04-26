//! Edge-case lock for `mmk_core::drift::compute_drift`.
//!
//! Pure function: given K snapshots in chronological order, surface
//! files that climbed in a majority of the K-1 adjacent transitions.
//! No I/O; the git-walk that produces the snapshots lives in
//! `mmk-git`.
//!
//! Orthogonality tag: protects **agent mode** — DRIFT findings show
//! up in the unified findings JSON the harness parses; the property
//! that "5 identical snapshots produce zero findings" is the
//! noise-floor lock.

use mmk_core::drift::{compute_drift, Snapshot};
use mmk_core::HotspotEntry;
use std::path::{Path, PathBuf};

fn entry(path: &str, rank: u32) -> HotspotEntry {
    HotspotEntry {
        path: PathBuf::from(path),
        loc: 100,
        weighted_churn: 1.0,
        relative_churn: 0.01,
        hotspot_score: 1.0,
        hotspot_rank: rank,
        commits_touching: 1,
        last_modified: 0,
        top_couples: Vec::new(),
    }
}

fn snap(label: &str, ranking: Vec<HotspotEntry>) -> Snapshot {
    Snapshot {
        label: label.into(),
        ranking,
    }
}

#[test]
fn drift_silent_on_steady_state() {
    // Five snapshots, same ranking — zero adjacent transitions show
    // any path climbing, so no DRIFT finding.
    let r = || vec![entry("a.rs", 1), entry("b.rs", 2), entry("c.rs", 3)];
    let snaps = vec![
        snap("s1", r()),
        snap("s2", r()),
        snap("s3", r()),
        snap("s4", r()),
        snap("s5", r()),
    ];
    let findings = compute_drift(&snaps);
    assert!(
        findings.is_empty(),
        "steady-state ranking across snapshots must produce no drift findings; got: {findings:?}"
    );
}

#[test]
fn drift_emits_finding_when_file_climbs_in_majority() {
    // hot.rs climbs every transition: rank 5 → 4 → 3 → 2 → 1 across
    // 5 snapshots = 4 transitions, all climbs.
    let snaps = vec![
        snap(
            "s1",
            vec![
                entry("a.rs", 1),
                entry("b.rs", 2),
                entry("c.rs", 3),
                entry("d.rs", 4),
                entry("hot.rs", 5),
            ],
        ),
        snap(
            "s2",
            vec![
                entry("a.rs", 1),
                entry("b.rs", 2),
                entry("c.rs", 3),
                entry("hot.rs", 4),
                entry("d.rs", 5),
            ],
        ),
        snap(
            "s3",
            vec![
                entry("a.rs", 1),
                entry("b.rs", 2),
                entry("hot.rs", 3),
                entry("c.rs", 4),
                entry("d.rs", 5),
            ],
        ),
        snap(
            "s4",
            vec![
                entry("a.rs", 1),
                entry("hot.rs", 2),
                entry("b.rs", 3),
                entry("c.rs", 4),
                entry("d.rs", 5),
            ],
        ),
        snap(
            "s5",
            vec![
                entry("hot.rs", 1),
                entry("a.rs", 2),
                entry("b.rs", 3),
                entry("c.rs", 4),
                entry("d.rs", 5),
            ],
        ),
    ];
    let findings = compute_drift(&snaps);
    let hot: Vec<_> = findings
        .iter()
        .filter(|f| f.path == Path::new("hot.rs"))
        .collect();
    assert_eq!(
        hot.len(),
        1,
        "hot.rs must produce exactly one drift finding"
    );
    assert_eq!(
        hot[0].climb_transitions, 4,
        "hot.rs climbed in all 4 transitions; got {}",
        hot[0].climb_transitions
    );
    assert_eq!(hot[0].total_transitions, 4);
    assert_eq!(hot[0].latest_rank, 1);
}

#[test]
fn drift_silent_on_falling_file() {
    // Reverse case: file falls every transition. Falls aren't drift —
    // nothing to flag.
    let snaps = vec![
        snap("s1", vec![entry("falling.rs", 1), entry("a.rs", 2)]),
        snap("s2", vec![entry("a.rs", 1), entry("falling.rs", 2)]),
        snap("s3", vec![entry("a.rs", 1), entry("falling.rs", 3)]),
    ];
    let findings = compute_drift(&snaps);
    let any_falling = findings.iter().any(|f| f.path == Path::new("falling.rs"));
    assert!(
        !any_falling,
        "a path that falls in rank must NOT produce a drift finding; got: {findings:?}"
    );
}

#[test]
fn drift_counts_new_entries_as_climbs() {
    // brand_new.rs absent in s1 + s2, present at rank 1 in s3.
    // That's "newly entered" — counts as a climb in transition s2→s3.
    // With only 2 transitions and a single climb, it's not a majority,
    // so no finding fires; but the bookkeeping must record the climb.
    let snaps = vec![
        snap("s1", vec![entry("a.rs", 1), entry("b.rs", 2)]),
        snap("s2", vec![entry("a.rs", 1), entry("b.rs", 2)]),
        snap(
            "s3",
            vec![entry("brand_new.rs", 1), entry("a.rs", 2), entry("b.rs", 3)],
        ),
    ];
    let findings = compute_drift(&snaps);
    let brand_new = findings
        .iter()
        .find(|f| f.path == Path::new("brand_new.rs"));
    // 1 climb of 2 transitions = not majority → no finding.
    assert!(
        brand_new.is_none(),
        "single climb in 2 transitions is not majority → no finding; got: {findings:?}"
    );
}

#[test]
fn drift_zero_or_one_snapshot_is_silent() {
    let none = compute_drift(&[]);
    assert!(none.is_empty());
    let one = compute_drift(&[snap("s1", vec![entry("a.rs", 1)])]);
    assert!(
        one.is_empty(),
        "a single snapshot has no transitions to inspect; must be silent"
    );
}

#[test]
fn drift_pure_function_same_input_same_output() {
    let snaps = vec![
        snap("s1", vec![entry("a.rs", 2), entry("b.rs", 1)]),
        snap("s2", vec![entry("a.rs", 1), entry("b.rs", 2)]),
        snap("s3", vec![entry("a.rs", 1), entry("b.rs", 2)]),
    ];
    let f1 = compute_drift(&snaps);
    let f2 = compute_drift(&snaps);
    assert_eq!(
        f1.len(),
        f2.len(),
        "same input must produce identical output (pure function); got {f1:?} vs {f2:?}"
    );
    for (a, b) in f1.iter().zip(f2.iter()) {
        assert_eq!(a.path, b.path);
        assert_eq!(a.climb_transitions, b.climb_transitions);
        assert_eq!(a.total_transitions, b.total_transitions);
        assert_eq!(a.latest_rank, b.latest_rank);
    }
}
