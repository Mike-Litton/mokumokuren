use ahash::AHashMap;
use mmk_core::churn::{commits_touching, relative_churn, weighted_churn};
use mmk_core::types::{Commit, CommitInfo, FileDelta};
use std::path::PathBuf;

const DAY: i64 = 86_400;

fn commit(ts: i64, files: &[(&str, u32, u32)]) -> Commit {
    Commit {
        info: CommitInfo {
            sha: format!("{ts:040x}"),
            parent_sha: None,
            timestamp: ts,
            author_email: "t@example.com".into(),
        },
        deltas: files
            .iter()
            .map(|(p, a, d)| FileDelta {
                path: PathBuf::from(p),
                added: *a,
                deleted: *d,
            })
            .collect(),
    }
}

#[test]
fn empty_input_yields_empty_map() {
    let out = weighted_churn(&[], 0, 90.0 * DAY as f64);
    assert!(out.is_empty());
}

#[test]
fn commit_at_now_weight_is_one() {
    let now = 1_700_000_000;
    let commits = vec![commit(now, &[("a.rs", 10, 5)])];
    let out = weighted_churn(&commits, now, 90.0 * DAY as f64);
    assert!((out[&PathBuf::from("a.rs")] - 15.0).abs() < 1e-9);
}

#[test]
fn commit_one_tau_old_decays_to_1_over_e() {
    let now = 1_700_000_000;
    let tau = 90.0 * DAY as f64;
    let one_tau_ago = now - (tau as i64);
    let commits = vec![commit(one_tau_ago, &[("a.rs", 10, 0)])];
    let out = weighted_churn(&commits, now, tau);
    let expected = 10.0 * (-1.0_f64).exp();
    assert!((out[&PathBuf::from("a.rs")] - expected).abs() < 1e-6);
}

#[test]
fn churn_sums_across_commits() {
    let now = 1_700_000_000;
    let tau = 90.0 * DAY as f64;
    let commits = vec![
        commit(now, &[("a.rs", 5, 0)]),
        commit(now, &[("a.rs", 3, 2)]),
    ];
    let out = weighted_churn(&commits, now, tau);
    assert!((out[&PathBuf::from("a.rs")] - 10.0).abs() < 1e-9);
}

#[test]
fn future_timestamp_clamped_to_zero_age() {
    let now = 1_700_000_000;
    let tau = 90.0 * DAY as f64;
    let commits = vec![commit(now + 3600, &[("a.rs", 10, 0)])];
    let out = weighted_churn(&commits, now, tau);
    assert!((out[&PathBuf::from("a.rs")] - 10.0).abs() < 1e-9);
}

#[test]
fn relative_churn_excludes_missing_and_zero_loc() {
    let mut weighted: AHashMap<PathBuf, f64> = AHashMap::new();
    weighted.insert(PathBuf::from("a.rs"), 100.0);
    weighted.insert(PathBuf::from("gone.rs"), 50.0);
    weighted.insert(PathBuf::from("empty.rs"), 20.0);

    let mut loc: AHashMap<PathBuf, u32> = AHashMap::new();
    loc.insert(PathBuf::from("a.rs"), 50);
    loc.insert(PathBuf::from("empty.rs"), 0);

    let rel = relative_churn(&weighted, &loc);
    assert!((rel[&PathBuf::from("a.rs")] - 2.0).abs() < 1e-9);
    assert!(!rel.contains_key(&PathBuf::from("gone.rs")));
    assert!(!rel.contains_key(&PathBuf::from("empty.rs")));
}

#[test]
fn commits_touching_counts_distinct_commits() {
    let now = 1_700_000_000;
    let commits = vec![
        commit(now, &[("a.rs", 1, 0), ("b.rs", 1, 0)]),
        commit(now - DAY, &[("a.rs", 2, 1)]),
        commit(now - 2 * DAY, &[("b.rs", 3, 0)]),
    ];
    let counts = commits_touching(&commits);
    assert_eq!(counts[&PathBuf::from("a.rs")], 2);
    assert_eq!(counts[&PathBuf::from("b.rs")], 2);
}
