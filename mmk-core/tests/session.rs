//! Edge-case lock for `mmk_core::session::commit_entropy`.
//!
//! Each test name is the property it locks. If a future maintainer
//! breaks one, the failing-test name says which property fell.
//!
//! Orthogonality tag: protects **CI/CD mode** primarily — the
//! human reviewer needs `commit_entropy = 0.7` to mean something
//! reproducible. Agents will read the number too, but the human-
//! readable "what does this even measure" bar is higher.

use mmk_core::session::compute_delta;
use mmk_core::types::{Commit, CommitInfo, FileDelta};
use std::path::PathBuf;

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

#[test]
#[allow(clippy::float_cmp)] // exact-zero is the early-exit's literal return value, not a tolerance check
fn entropy_zero_commits_is_zero() {
    let delta = compute_delta(&[], &[], &[]);
    assert_eq!(
        delta.commit_entropy, 0.0,
        "zero-commit session must report entropy 0 — there's nothing to be \
         distributed over"
    );
}

#[test]
#[allow(clippy::float_cmp)] // exact-zero is the early-exit's literal return value, not a tolerance check
fn entropy_one_commit_is_zero() {
    // The early-exit at `commits.len() < 2` is intentional: with one
    // commit, there's no distribution. Lock this so a future
    // refactor doesn't accidentally return ln(0)/ln(1) = NaN.
    let commits = vec![commit(100, &["a", "b", "c"])];
    let delta = compute_delta(&[], &[], &commits);
    assert_eq!(delta.commit_entropy, 0.0);
}

#[test]
fn entropy_uniform_distribution_is_one() {
    // 4 commits, 1 file each → p_i = 1/4 → H = ln(4),
    // normalized by ln(4) = 1.0. This is the *maximum* normalized
    // entropy and the canonical "even spread" case.
    let commits = vec![
        commit(100, &["a"]),
        commit(200, &["b"]),
        commit(300, &["c"]),
        commit(400, &["d"]),
    ];
    let delta = compute_delta(&[], &[], &commits);
    assert!(
        (delta.commit_entropy - 1.0).abs() < 1e-9,
        "expected 1.0, got {}",
        delta.commit_entropy
    );
}

#[test]
fn entropy_all_commits_touch_one_identical_file_is_one() {
    // Locks the **definition**: entropy is over *file-counts-per-commit*,
    // not over *which file*. Four commits each touching the same file
    // (one file each) → p_i = 1/4 → entropy = 1.0.
    //
    // If this ever fails reading something other than 1.0, someone
    // changed the metric's meaning. That should be a new metric, not a
    // silent semantic shift on this one.
    let commits = vec![
        commit(100, &["x"]),
        commit(200, &["x"]),
        commit(300, &["x"]),
        commit(400, &["x"]),
    ];
    let delta = compute_delta(&[], &[], &commits);
    assert!(
        (delta.commit_entropy - 1.0).abs() < 1e-9,
        "expected 1.0 (uniform across commits), got {}",
        delta.commit_entropy
    );
}

#[test]
fn entropy_concentrated_distribution_is_low() {
    // 9 commits with 1 file each + 1 commit with 100 files. Mass
    // concentrated in one bucket → entropy << 1.0. Locks ordering
    // relative to the uniform case so a CI report can flag "this
    // session has one bulk-edit commit" by comparing entropy
    // against a baseline.
    let mut commits: Vec<Commit> = (0..9_i64).map(|i| commit(100 + i, &["a"])).collect();
    let many: Vec<String> = (0..100).map(|i| format!("f{i}")).collect();
    let many_refs: Vec<&str> = many.iter().map(String::as_str).collect();
    commits.push(commit(200, &many_refs));

    let delta = compute_delta(&[], &[], &commits);
    assert!(
        delta.commit_entropy < 0.5,
        "concentrated distribution should produce low normalized entropy; \
         got {} — if this rises above 0.5, either the metric drifted or \
         the bulk filter changed and this fixture is no longer concentrated",
        delta.commit_entropy
    );
}

#[test]
fn entropy_handles_empty_deltas_without_panic() {
    // Pathological: commits with no deltas. `analyze()` shouldn't
    // produce these in practice (the bulk filter rejects empty-diff
    // commits), but the metric must still be defined. The current
    // implementation treats an empty commit as count.max(1) — count
    // 1, which is sane.
    let mut c1 = commit(100, &["a"]);
    let mut c2 = commit(200, &["b"]);
    c1.deltas.clear();
    c2.deltas.clear();
    let commits = vec![c1, c2];
    let delta = compute_delta(&[], &[], &commits);
    assert!(
        delta.commit_entropy.is_finite(),
        "empty-deltas case must not produce NaN or inf"
    );
    assert!(
        (0.0..=1.0).contains(&delta.commit_entropy),
        "entropy must stay normalized; got {}",
        delta.commit_entropy
    );
}
