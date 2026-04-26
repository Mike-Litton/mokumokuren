use ahash::AHashSet;
use mmk_core::coupling::{
    compute_conditional_couples_for, neighborhood, top_couples_for, CouplingEntry,
};
use mmk_core::types::{Commit, CommitInfo, FileDelta};
use std::path::{Path, PathBuf};

fn commit(ts: i64, files: &[&str]) -> Commit {
    Commit {
        info: CommitInfo {
            sha: format!("{ts:040x}"),
            parent_sha: None,
            timestamp: ts,
            author_email: "t@example.com".into(),
        },
        deltas: files
            .iter()
            .map(|p| FileDelta {
                path: PathBuf::from(p),
                added: 1,
                deleted: 0,
            })
            .collect(),
    }
}

fn p(s: &str) -> PathBuf {
    PathBuf::from(s)
}

fn entry_for<'a>(entries: &'a [CouplingEntry], partner: &str) -> Option<&'a CouplingEntry> {
    entries.iter().find(|e| e.partner == p(partner))
}

#[test]
fn jaccard_three_quarters_on_hand_built_fixture() {
    // Four commits touching A or B:
    //   1: A, B   (co-change)
    //   2: A      (A only)
    //   3: A, B   (co-change)
    //   4: A, B   (co-change)
    // touches_A = 4, touches_B = 3, co_change = 3.
    // jaccard = 3 / (4 + 3 - 3) = 3/4 = 0.75.
    let commits = vec![
        commit(100, &["A", "B"]),
        commit(200, &["A"]),
        commit(300, &["A", "B"]),
        commit(400, &["A", "B"]),
    ];

    let mut targets = AHashSet::new();
    targets.insert(p("A"));

    let out = top_couples_for(&commits, &targets, 5);
    let a_couples = out.get(&p("A")).expect("A should be a key in the map");
    let b = entry_for(a_couples, "B").expect("A should be coupled to B");
    assert!(
        (b.jaccard - 0.75).abs() < 1e-9,
        "expected jaccard 0.75, got {}",
        b.jaccard
    );
    assert_eq!(b.co_change_count, 3);
}

#[test]
fn targets_pruning_returns_only_targeted_keys() {
    // Symmetric pair (A,B) co-changing in every commit. If we only ask
    // for A, the result map must not have a B key — the metric is
    // O(|targets| × |files-per-commit|), not O(|all-pairs|).
    let commits = vec![
        commit(100, &["A", "B"]),
        commit(200, &["A", "B"]),
        commit(300, &["A", "B"]),
    ];

    let mut targets = AHashSet::new();
    targets.insert(p("A"));

    let out = top_couples_for(&commits, &targets, 5);
    assert!(
        out.contains_key(&p("A")),
        "A should appear (it was targeted)"
    );
    assert!(
        !out.contains_key(&p("B")),
        "B should NOT appear (it was not in targets) — got keys {:?}",
        out.keys().collect::<Vec<_>>()
    );
}

#[test]
fn empty_targets_returns_empty_map() {
    let commits = vec![commit(100, &["A", "B"]), commit(200, &["A", "B"])];
    let targets: AHashSet<PathBuf> = AHashSet::new();
    let out = top_couples_for(&commits, &targets, 5);
    assert!(
        out.is_empty(),
        "empty targets should yield empty map, not 'all pairs' — got {} keys",
        out.len()
    );
}

#[test]
fn top_k_limits_returned_partners() {
    // A co-changes with B, C, D, E across distinct commits; ask for k=2.
    let commits = vec![
        commit(100, &["A", "B"]),
        commit(150, &["A", "B"]),
        commit(200, &["A", "C"]),
        commit(300, &["A", "D"]),
        commit(400, &["A", "E"]),
    ];
    let mut targets = AHashSet::new();
    targets.insert(p("A"));

    let out = top_couples_for(&commits, &targets, 2);
    let a_couples = out.get(&p("A")).expect("A should be present");
    assert_eq!(a_couples.len(), 2, "k=2 should return exactly 2 partners");
    // B has the highest co_change (2); it must be in the top 2.
    assert!(
        entry_for(a_couples, "B").is_some(),
        "highest-coupled partner B must be among the top-k"
    );
}

/// Hand-built graph: A↔B (jaccard 0.6), B↔C (jaccard 0.5), A↔D
/// (jaccard 0.05). 1-hop neighborhood of A at threshold 0.1 keeps B
/// (above threshold), drops D (below threshold), and *must* drop C
/// (it's two hops away). Locks the 1-hop invariant so a future
/// promotion to multi-hop is a contained change.
#[test]
fn blast_radius_one_hop_includes_above_threshold_only() {
    // 10 commits total. Construct touch counts/co-changes to land
    // jaccard exactly at the targets:
    //
    //   touches_A = 10, touches_B = 6, co(A,B) = 6
    //     -> jaccard = 6 / (10 + 6 - 6) = 0.6
    //
    //   touches_C = 5, co(B,C) = 4
    //     -> jaccard(B,C) = 4 / (6 + 5 - 4) = 0.571… ≥ 0.5 (close enough,
    //     but we only need C *not* in A's 1-hop neighborhood)
    //
    //   touches_D = 11, co(A,D) = 1
    //     -> jaccard(A,D) = 1 / (10 + 11 - 1) = 0.05
    //
    // Construct so C is reachable from A only *through* B (i.e.
    // co(A,C) = 0) — that's what makes "C is 2 hops from A" the
    // load-bearing invariant the test locks down.
    //
    //   6 commits A+B          → tA=6,  tB=6,  co(A,B)=6
    //   4 commits A only       → tA=10, tB=6,  co(A,B)=6
    //   4 commits B+C          → tA=10, tB=10, tC=4, co(B,C)=4, co(A,C)=0
    //   1 commit A+D           → tA=11, tD=1,  co(A,D)=1
    //   10 commits D only      → tD=11
    //
    // jaccard(A,B) = 6 / (11+10-6) = 6/15 = 0.40
    // jaccard(A,C) = 0  (no direct edge — C is 2 hops away)
    // jaccard(A,D) = 1 / (11+11-1) = 1/21 ≈ 0.048
    let mut commits = Vec::new();
    let mut t = 100;
    let mut next = || {
        t += 1;
        t
    };

    for _ in 0..6 {
        commits.push(commit(next(), &["A", "B"]));
    }
    for _ in 0..4 {
        commits.push(commit(next(), &["A"]));
    }
    for _ in 0..4 {
        commits.push(commit(next(), &["B", "C"]));
    }
    commits.push(commit(next(), &["A", "D"]));
    for _ in 0..10 {
        commits.push(commit(next(), &["D"]));
    }

    let nodes = neighborhood(&commits, Path::new("A"), 1, 0.1).expect("1-hop is supported");
    let names: Vec<&str> = nodes.iter().map(|n| n.path.to_str().unwrap()).collect();

    assert!(
        names.contains(&"B"),
        "B (jaccard 0.6) must be in A's 1-hop neighborhood; got {names:?}"
    );
    assert!(
        !names.contains(&"D"),
        "D (jaccard ~0.05) is below threshold 0.1 and must be excluded; got {names:?}"
    );
    assert!(
        !names.contains(&"C"),
        "C is two hops from A (only co-changes through B for the high-jaccard edge); 1-hop \
         must exclude it. got {names:?}"
    );
    assert!(
        nodes.iter().all(|n| n.hops == 1),
        "every node should report hops=1; got {:?}",
        nodes.iter().map(|n| n.hops).collect::<Vec<_>>()
    );
}

#[test]
fn blast_radius_rejects_multi_hop_with_err() {
    let commits = vec![commit(100, &["A", "B"])];
    let result = neighborhood(&commits, Path::new("A"), 2, 0.1);
    let err = result.expect_err("hops != 1 must surface as Err, not panic");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("1-hop"),
        "error should name the 1-hop limit; got: {msg}"
    );
}

/// Locks the `jaccard ∈ [0.0, 1.0]` invariant across a varied input.
/// `top_couples_for` constructs `touches` and `pair_counts` from the
/// same `commit.deltas`, so the invariant `co ≤ min(touches_t,
/// touches_p)` holds today. Future co-change accounting changes (rename
/// post-processing, binary filtering, dedup edge cases) could land a
/// pair in `pair_counts` whose path is missing from `touches`, where
/// plain `u32` subtraction would wrap.
#[test]
fn jaccard_stays_in_unit_interval_across_varied_inputs() {
    let commits = vec![
        commit(100, &["A", "B"]),
        commit(200, &["A"]),
        commit(300, &["A", "B", "C"]),
        commit(400, &["B", "C"]),
        commit(500, &["A", "C"]),
        commit(600, &["A"]),
        commit(700, &["A", "B"]),
    ];
    let mut targets = AHashSet::new();
    targets.insert(p("A"));
    targets.insert(p("B"));

    let out = top_couples_for(&commits, &targets, 0);
    for (target, entries) in &out {
        for e in entries {
            assert!(
                (0.0..=1.0).contains(&e.jaccard),
                "jaccard out of range for {target:?} -> {:?}: {}",
                e.partner,
                e.jaccard
            );
        }
    }
}

#[test]
fn self_pair_is_excluded() {
    let commits = vec![commit(100, &["A"]), commit(200, &["A", "B"])];
    let mut targets = AHashSet::new();
    targets.insert(p("A"));
    let out = top_couples_for(&commits, &targets, 5);
    let a_couples = out.get(&p("A")).expect("A present");
    assert!(
        entry_for(a_couples, "A").is_none(),
        "self-coupling must not be reported"
    );
}

/// Schema lock: every CouplingEntry carries
/// `conditional_probability` and a Wilson 95 % lower bound. The
/// `top_couples_for` walk feeds both fields directly.
#[test]
fn conditional_probability_and_wilson_lower_populate() {
    // touches_A = 4, touches_B = 3, co_change = 3 → P(B|A) = 3/4 = 0.75.
    let commits = vec![
        commit(100, &["A", "B"]),
        commit(200, &["A"]),
        commit(300, &["A", "B"]),
        commit(400, &["A", "B"]),
    ];
    let mut targets = AHashSet::new();
    targets.insert(p("A"));
    let out = top_couples_for(&commits, &targets, 5);
    let b = entry_for(out.get(&p("A")).unwrap(), "B").expect("A coupled to B");
    assert!(
        (b.conditional_probability - 0.75).abs() < 1e-9,
        "P(B|A) should be 3/4; got {}",
        b.conditional_probability
    );
    // Wilson 95 % lower for 3/4 ≈ 0.301 (scipy: 0.30067...).
    assert!(
        (b.wilson_lower_95 - 0.30).abs() < 0.05,
        "Wilson lower for 3/4 should be near 0.30; got {}",
        b.wilson_lower_95
    );
}

/// Hot-file regression mirroring vscode's
/// `runInTerminalTool.ts` ↔ `runInTerminalTool.test.ts` pair: 54
/// co-changes inside 203 commits touching A. P(B|A)=0.266; Wilson
/// 95 % lower ≈ 0.211 — clears the default 0.20 confidence floor.
/// The headline calibration target for the Wilson rule.
#[test]
fn hot_file_real_partner_clears_default_confidence_threshold() {
    let mut commits = Vec::new();
    let mut t = 100_i64;
    let mut next = || {
        t += 1;
        t
    };
    for _ in 0..54 {
        commits.push(commit(next(), &["A", "B"])); // co-changes
    }
    for _ in 0..(203 - 54) {
        commits.push(commit(next(), &["A"])); // A-only commits
    }
    let mut targets = AHashSet::new();
    targets.insert(p("A"));
    let out = compute_conditional_couples_for(&commits, &targets, 0);
    let b = entry_for(out.get(&p("A")).unwrap(), "B").expect("A coupled to B");
    assert!(
        (b.conditional_probability - 54.0 / 203.0).abs() < 1e-9,
        "P(B|A) for 54/203 should be ≈0.266; got {}",
        b.conditional_probability
    );
    assert!(
        b.wilson_lower_95 >= 0.20,
        "Wilson lower for 54/203 should clear 0.20 (scipy ≈ 0.211); got {}",
        b.wilson_lower_95
    );
}

/// Quiet-file regression: n=3, k=3 produces a Wilson lower of 0.44
/// (high) but commits_touching=3 < min_sample_size=5. The pure
/// function emits the entry; downstream filtering by min_sample_size
/// must drop it. This test pins the entry's *values* so the
/// downstream gate has the right inputs to work with.
#[test]
fn quiet_file_high_proportion_low_count_keeps_high_wilson_but_low_n() {
    let commits = vec![
        commit(100, &["A", "B"]),
        commit(200, &["A", "B"]),
        commit(300, &["A", "B"]),
    ];
    let mut targets = AHashSet::new();
    targets.insert(p("A"));
    let out = top_couples_for(&commits, &targets, 0);
    let b = entry_for(out.get(&p("A")).unwrap(), "B").expect("A coupled to B");
    assert!(
        (b.conditional_probability - 1.0).abs() < 1e-9,
        "P(B|A) for 3/3 should be 1.0; got {}",
        b.conditional_probability
    );
    assert!(
        b.wilson_lower_95 > 0.30,
        "Wilson lower for 3/3 should be above 0.30 (scipy ≈ 0.44); got {}",
        b.wilson_lower_95
    );
    assert_eq!(b.co_change_count, 3, "co_change_count carries n_k");
}

/// Wilson discriminating-power lock: vscode's `chatWidget.ts` →
/// `chatInputPart.ts` pair (27 co-changes inside 133 commits, Wilson
/// 95 % lower ≈ 0.14) sits *below* the default
/// `confidence_threshold = 0.20`. The Wilson rule's value is its
/// discrimination — surfacing real partners on hot files (54/203)
/// while *not* over-firing on borderline ones. This test pins the
/// suppression alongside the firing test to lock both sides.
#[test]
fn chatwidget_borderline_partner_does_not_clear_default_confidence_threshold() {
    let mut commits = Vec::new();
    let mut t = 100_i64;
    let mut next = || {
        t += 1;
        t
    };
    for _ in 0..27 {
        commits.push(commit(next(), &["A", "B"])); // co-changes
    }
    for _ in 0..(133 - 27) {
        commits.push(commit(next(), &["A"])); // A-only commits
    }
    let mut targets = AHashSet::new();
    targets.insert(p("A"));
    let out = compute_conditional_couples_for(&commits, &targets, 0);
    let b = entry_for(out.get(&p("A")).unwrap(), "B").expect("A coupled to B");
    assert!(
        b.wilson_lower_95 < 0.20,
        "Wilson lower for 27/133 must NOT clear 0.20 (scipy ≈ 0.142); \
         got {} — the metric stopped being discriminating",
        b.wilson_lower_95
    );
    // Sanity: P(B|A) is still 0.20 — at the threshold on the point
    // estimate. The Wilson lower bound is what enforces the
    // statistical-confidence guard.
    assert!(
        (b.conditional_probability - 27.0 / 133.0).abs() < 1e-9,
        "P(B|A) should be 27/133 ≈ 0.203; got {}",
        b.conditional_probability
    );
}

/// `compute_conditional_couples_for` ranks by Wilson lower descending
/// — the order `mmk review` / `mmk pre-edit` consume so the strongest
/// partners surface first.
#[test]
fn conditional_couples_sort_by_wilson_lower_desc() {
    // A→strong: 30/40 (Wilson lower ≈ 0.60)
    // A→weak:    1/40 (Wilson lower ≈ 0.00, point estimate 0.025)
    let mut commits = Vec::new();
    let mut t = 100_i64;
    let mut next = || {
        t += 1;
        t
    };
    for _ in 0..30 {
        commits.push(commit(next(), &["A", "STRONG"]));
    }
    for _ in 0..9 {
        commits.push(commit(next(), &["A"]));
    }
    commits.push(commit(next(), &["A", "WEAK"]));

    let mut targets = AHashSet::new();
    targets.insert(p("A"));
    let out = compute_conditional_couples_for(&commits, &targets, 0);
    let entries = out.get(&p("A")).unwrap();
    assert_eq!(
        entries[0].partner.to_str().unwrap(),
        "STRONG",
        "Wilson-sorted output should put STRONG first; got {entries:?}"
    );
}
