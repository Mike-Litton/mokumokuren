mod common;

use common::{build_canonical_fixture, commit_all, init_repo, write};
use mmk_config::Config;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

#[test]
fn analyze_canonical_fixture() {
    let dir = TempDir::new().unwrap();
    let now: i64 = 1_700_000_000;
    build_canonical_fixture(dir.path(), now);

    let mut cfg = Config::default();
    cfg.window.days = 30;
    cfg.bulk.max_files = 1000;
    cfg.bulk.max_lines = 10_000;

    let out = mmk_git::analyze(dir.path(), &cfg).expect("analyze");

    assert_eq!(out.counts.commits_seen, 4);
    assert_eq!(out.counts.commits_analyzed, 4);
    assert_eq!(out.counts.commits_filtered_bulk, 0);

    assert!(out.loc.contains_key(&PathBuf::from("a.rs")));
    assert_eq!(out.loc[&PathBuf::from("a.rs")], 6);
    assert!(!out.loc.contains_key(&PathBuf::from("b.rs")));
    assert!(!out.loc.contains_key(&PathBuf::from("c.rs")));

    let a_touched = out
        .commits
        .iter()
        .filter(|c| {
            c.deltas
                .iter()
                .any(|d| d.path.as_path() == Path::new("a.rs"))
        })
        .count();
    assert_eq!(a_touched, 2, "a.rs should appear in A and B");

    // Commit C is a pure rename b.rs -> c.rs; gix should detect it as a
    // Rewrite on the new path.
    let rename_seen = out.commits.iter().any(|c| {
        c.deltas
            .iter()
            .any(|d| d.path.as_path() == Path::new("c.rs"))
    });
    assert!(rename_seen, "rename to c.rs should be observed");

    // c.rs is deleted at HEAD; it should not survive into the LOC map but
    // should be counted via deleted_from_head.
    assert!(out.counts.files_deleted_from_head >= 1);
}

#[test]
fn analyze_on_non_git_path_errors() {
    let dir = TempDir::new().unwrap();
    let err = mmk_git::analyze(dir.path(), &Config::default()).unwrap_err();
    let msg = format!("{err:#}");
    assert!(
        msg.to_lowercase().contains("git"),
        "error should mention git: {msg}"
    );
}

#[test]
fn analyze_empty_repo() {
    let dir = TempDir::new().unwrap();
    init_repo(dir.path());
    let out = mmk_git::analyze(dir.path(), &Config::default()).expect("analyze empty");
    assert_eq!(out.counts.commits_seen, 0);
    assert_eq!(out.counts.commits_analyzed, 0);
    assert!(out.loc.is_empty());
    assert!(out.head_sha.is_none());
}

#[test]
fn analyze_respects_bulk_filter() {
    let dir = TempDir::new().unwrap();
    let now: i64 = 1_700_000_000;
    init_repo(dir.path());

    // Normal commit — one file, one line.
    write(dir.path(), "x.rs", "one\n");
    commit_all(dir.path(), "small", now - 2 * common::DAY);

    // Bulky commit — add many files in one go.
    for i in 0..40 {
        write(dir.path(), &format!("pkg/f{i}.rs"), "a\nb\nc\n");
    }
    commit_all(dir.path(), "bulky", now - common::DAY);

    let mut cfg = Config::default();
    cfg.window.days = 30;
    cfg.bulk.max_files = 15;
    cfg.bulk.max_lines = 1000;
    let out = mmk_git::analyze(dir.path(), &cfg).unwrap();
    assert_eq!(out.counts.commits_seen, 2);
    assert_eq!(out.counts.commits_filtered_bulk, 1);
    assert_eq!(out.counts.commits_analyzed, 1);
}
