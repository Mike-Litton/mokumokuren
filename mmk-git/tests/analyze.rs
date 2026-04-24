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
    assert!(out.loc.contains_key(&PathBuf::from("c.rs")));

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

    // b.rs was added in commit A but doesn't survive to HEAD (it was
    // renamed to c.rs in commit C). Its Addition event is counted as a
    // non-HEAD skip.
    assert!(out.counts.non_head_events >= 1);
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

/// A pure rename (100% identical content, file moves to a new path) carries
/// no content churn. It should not show up as a `commits_touching` hit on
/// the destination path — that would double-count when the next commit
/// actually edits the file. Gix emits this as a `Rewrite` with
/// `blob_diff == None`; we should suppress the zero-churn delta.
#[test]
fn pure_rename_is_not_counted_as_a_touch() {
    let dir = TempDir::new().unwrap();
    let now: i64 = 1_700_000_000;
    init_repo(dir.path());

    // A: add x.rs (3 lines)
    write(dir.path(), "x.rs", "alpha\nbeta\ngamma\n");
    commit_all(dir.path(), "A: add x.rs", now - 3 * common::DAY);

    // B: pure rename x.rs -> y.rs, byte-identical content
    std::fs::remove_file(dir.path().join("x.rs")).unwrap();
    write(dir.path(), "y.rs", "alpha\nbeta\ngamma\n");
    commit_all(dir.path(), "B: rename x->y", now - 2 * common::DAY);

    // C: modify y.rs (one line changed)
    write(dir.path(), "y.rs", "alpha\nBETA\ngamma\n");
    commit_all(dir.path(), "C: modify y.rs", now - common::DAY);

    let mut cfg = Config::default();
    cfg.window.days = 30;
    cfg.bulk.max_files = 1000;
    cfg.bulk.max_lines = 10_000;
    let out = mmk_git::analyze(dir.path(), &cfg).unwrap();
    assert_eq!(out.counts.commits_analyzed, 3);

    // y.rs should appear exactly ONCE — in commit C (the real edit).
    // The rename commit B carries zero content churn.
    let y_touches = out
        .commits
        .iter()
        .filter(|c| {
            c.deltas
                .iter()
                .any(|d| d.path.as_path() == Path::new("y.rs"))
        })
        .count();
    assert_eq!(
        y_touches, 1,
        "pure rename should not emit a (0,0) delta on the destination"
    );
}

/// `non_head_events` should only count skipped events from
/// commits that actually contribute to metrics. If a commit is
/// bulk-filtered (its partial deltas are discarded downstream), the
/// partial skips from that commit shouldn't bleed into the non-HEAD
/// event count.
///
/// Construction: a single commit with both non-HEAD Additions (paths
/// `apple*` — deleted before HEAD, processed first by tree-diff order)
/// AND at-HEAD Modifications (paths `zebra*`, processed later and
/// numerous enough to trip the bulk filter). The commit aborts mid-walk;
/// the `apple*` skips already tallied must not leak.
#[test]
fn bulk_filtered_commit_does_not_leak_skip_events() {
    let dir = TempDir::new().unwrap();
    let now: i64 = 1_700_000_000;
    init_repo(dir.path());

    // A: add 10 zebra files (under the bulk limit of 15).
    for i in 0..10 {
        write(dir.path(), &format!("zebra{i:02}.rs"), "a\n");
    }
    commit_all(dir.path(), "A: zebras", now - 4 * common::DAY);

    // B: add 10 more zebras, bringing HEAD total to 20 (also under 15
    // per-commit but doubles the HEAD set).
    for i in 10..20 {
        write(dir.path(), &format!("zebra{i:02}.rs"), "a\n");
    }
    commit_all(dir.path(), "B: more zebras", now - 3 * common::DAY);

    // C: 10 apple* Additions (non-HEAD, processed first alphabetically —
    // all will be skipped) + 20 zebra* Modifications (at HEAD). Tree-diff
    // is alphabetical so apple* events come first, generating 10 skip
    // tallies before any zebra* push increments running_files. Then
    // zebra* pushes eventually trip the bulk filter.
    for i in 0..10 {
        write(dir.path(), &format!("apple{i:02}.rs"), "x\n");
    }
    for i in 0..20 {
        write(dir.path(), &format!("zebra{i:02}.rs"), "a\nb\n");
    }
    commit_all(dir.path(), "C: apples + zebras", now - 2 * common::DAY);

    // D: delete the apples so they're not at HEAD.
    for i in 0..10 {
        std::fs::remove_file(dir.path().join(format!("apple{i:02}.rs"))).unwrap();
    }
    commit_all(dir.path(), "D: drop apples", now - common::DAY);

    let mut cfg = Config::default();
    cfg.window.days = 30;
    cfg.bulk.max_files = 15;
    cfg.bulk.max_lines = 1000;
    let out = mmk_git::analyze(dir.path(), &cfg).unwrap();

    // Commit C is bulk-filtered (20 zebra modifications > 15).
    assert_eq!(
        out.counts.commits_filtered_bulk, 1,
        "commit C should trigger bulk filter"
    );

    // Apple skips from commit C were tallied before early-abort. They
    // should NOT leak into the counter, because the commit that produced
    // them is itself discarded. Commit D's deletions DO count (commit D
    // is analyzed, emits 10 Deletion events for non-HEAD apples).
    assert_eq!(
        out.counts.non_head_events, 10,
        "only the analyzed delete-commit's skips should count; \
         the bulk-filtered commit's partial skips must not leak"
    );
}

/// Assert exact `(added, deleted)` counts for specific modifications.
/// This is the unit-level regression test: if anyone swaps the diff
/// algorithm (as I did, swapping LCS→multiset and systematically
/// undercounting churn), this test pins down the numeric answers for
/// several canonical edit patterns. The expected numbers below match
/// what `git diff --numstat` produces — the canonical ecosystem answer
/// — so deviation either means a bug or means we've consciously diverged.
#[test]
fn delta_counts_match_git_numstat_on_canonical_edits() {
    let dir = TempDir::new().unwrap();
    let now: i64 = 1_700_000_000;
    init_repo(dir.path());

    // Commit 0: baseline file with 5 lines.
    write(dir.path(), "f.rs", "a\nb\nc\nd\ne\n");
    commit_all(dir.path(), "0: seed", now - 6 * common::DAY);

    // Commit 1: pure addition of 2 lines (insert in middle).
    //   git numstat: 2 added, 0 deleted
    write(dir.path(), "f.rs", "a\nb\nX\nY\nc\nd\ne\n");
    commit_all(dir.path(), "1: insert", now - 5 * common::DAY);

    // Commit 2: pure deletion of 2 lines.
    //   git numstat: 0 added, 2 deleted
    write(dir.path(), "f.rs", "a\nb\nc\nd\ne\n");
    commit_all(dir.path(), "2: delete", now - 4 * common::DAY);

    // Commit 3: replace one line (1 del + 1 add).
    //   git numstat: 1 added, 1 deleted
    write(dir.path(), "f.rs", "a\nB\nc\nd\ne\n");
    commit_all(dir.path(), "3: replace", now - 3 * common::DAY);

    // Commit 4: mixed — keep [a, B, c], drop [d, e], add [P, Q, R].
    //   git numstat: 3 added, 2 deleted
    write(dir.path(), "f.rs", "a\nB\nP\nc\nQ\nR\n");
    commit_all(dir.path(), "4: mixed", now - 2 * common::DAY);

    let mut cfg = Config::default();
    cfg.window.days = 30;
    cfg.bulk.max_files = 1000;
    cfg.bulk.max_lines = 10_000;
    let out = mmk_git::analyze(dir.path(), &cfg).unwrap();

    // Commits come back newest-first. The seed commit (commit 0) is the
    // initial add — 5 added, 0 deleted. The remaining four commits
    // exercise each canonical edit pattern.
    let expected: &[(&str, u32, u32)] = &[
        ("4: mixed", 3, 2),
        ("3: replace", 1, 1),
        ("2: delete", 0, 2),
        ("1: insert", 2, 0),
        ("0: seed", 5, 0),
    ];

    assert_eq!(out.commits.len(), expected.len(), "unexpected commit count");

    for (commit, (msg, want_a, want_d)) in out.commits.iter().zip(expected) {
        let delta = commit
            .deltas
            .iter()
            .find(|d| d.path.as_path() == Path::new("f.rs"))
            .unwrap_or_else(|| panic!("no delta for f.rs in commit {msg:?}"));
        assert_eq!(
            (delta.added, delta.deleted),
            (*want_a, *want_d),
            "commit {msg:?}: expected ({want_a},{want_d}), got ({},{})",
            delta.added,
            delta.deleted
        );
    }
}

/// Cross-check against the external `git` CLI — the canonical reference.
/// For each modification commit in a fixture, compare our delta's
/// `(added, deleted)` with `git diff --numstat` for the same file pair.
/// This is the most robust regression guard: if we ever silently diverge
/// from git's numbers (e.g. by changing the diff algorithm, normalization
/// pipeline, or binary detection), this test fails.
#[test]
fn delta_counts_match_git_numstat_externally() {
    use std::process::Command;

    let dir = TempDir::new().unwrap();
    let now: i64 = 1_700_000_000;
    init_repo(dir.path());

    // A multi-commit history exercising several edit shapes.
    write(dir.path(), "lib.py", "def f():\n    return 1\n");
    commit_all(dir.path(), "seed lib", now - 5 * common::DAY);

    write(
        dir.path(),
        "lib.py",
        "def f():\n    # docstring\n    return 1\n",
    );
    commit_all(dir.path(), "doc comment", now - 4 * common::DAY);

    write(
        dir.path(),
        "lib.py",
        "def f():\n    # docstring\n    return 2\n\ndef g():\n    return 3\n",
    );
    commit_all(dir.path(), "add g", now - 3 * common::DAY);

    write(dir.path(), "lib.py", "def h():\n    return 42\n");
    commit_all(dir.path(), "rewrite", now - 2 * common::DAY);

    let mut cfg = Config::default();
    cfg.window.days = 30;
    cfg.bulk.max_files = 1000;
    cfg.bulk.max_lines = 10_000;
    let out = mmk_git::analyze(dir.path(), &cfg).unwrap();

    // For each non-root commit, ask git what the numstat says and assert
    // our delta for `lib.py` matches exactly.
    for commit in &out.commits {
        let Some(parent) = &commit.info.parent_sha else {
            continue; // seed commit — no parent to diff against
        };
        let output = Command::new("git")
            .args([
                "diff",
                "--numstat",
                "-M",
                "--no-renames",
                parent,
                &commit.info.sha,
                "--",
                "lib.py",
            ])
            .current_dir(dir.path())
            .output()
            .expect("git diff");
        assert!(output.status.success(), "git diff failed");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let line = stdout.trim();
        if line.is_empty() {
            // git says no change to this file in this commit — assert
            // we emitted no delta for it either.
            let ours = commit
                .deltas
                .iter()
                .find(|d| d.path.as_path() == Path::new("lib.py"));
            assert!(
                ours.is_none(),
                "commit {}: git says no change but we emitted {ours:?}",
                commit.info.sha
            );
            continue;
        }
        let mut parts = line.split_whitespace();
        let git_added: u32 = parts.next().unwrap().parse().unwrap();
        let git_deleted: u32 = parts.next().unwrap().parse().unwrap();

        let ours = commit
            .deltas
            .iter()
            .find(|d| d.path.as_path() == Path::new("lib.py"))
            .unwrap_or_else(|| {
                panic!(
                    "commit {}: git reports ({git_added},{git_deleted}) for lib.py \
                     but we emitted no delta",
                    commit.info.sha
                )
            });
        assert_eq!(
            (ours.added, ours.deleted),
            (git_added, git_deleted),
            "commit {}: mmk=({},{}), git=({},{})",
            commit.info.sha,
            ours.added,
            ours.deleted,
            git_added,
            git_deleted
        );
    }
}

/// HEAD-path filter must match gix's raw path bytes. If we lossy-convert
/// before building the filter set, paths that round-trip lossily would
/// silently fall out of metrics.
///
/// This test uses only ASCII so it doesn't actually exercise the
/// round-trip risk, but it locks in the expected behavior for the
/// common case: every HEAD path is probed successfully.
#[test]
fn head_path_filter_matches_all_head_files_in_common_case() {
    let dir = TempDir::new().unwrap();
    let now: i64 = 1_700_000_000;
    init_repo(dir.path());

    // A variety of path shapes — dotfiles, nested dirs, punctuation.
    write(dir.path(), "README.md", "hello\n");
    write(dir.path(), "src/main.rs", "fn main() {}\n");
    write(dir.path(), "pkg/sub-dir/file.txt", "data\n");
    write(dir.path(), ".config/nested.toml", "key = 1\n");
    commit_all(dir.path(), "A: initial", now - common::DAY);

    // Modify each.
    write(dir.path(), "README.md", "hello world\n");
    write(dir.path(), "src/main.rs", "fn main() { println!(); }\n");
    write(dir.path(), "pkg/sub-dir/file.txt", "data2\n");
    write(dir.path(), ".config/nested.toml", "key = 2\n");
    commit_all(dir.path(), "B: modify all", now - 60);

    let mut cfg = Config::default();
    cfg.window.days = 30;
    cfg.bulk.max_files = 1000;
    cfg.bulk.max_lines = 10_000;
    let out = mmk_git::analyze(dir.path(), &cfg).unwrap();

    for expected in [
        "README.md",
        "src/main.rs",
        "pkg/sub-dir/file.txt",
        ".config/nested.toml",
    ] {
        assert!(
            out.loc.contains_key(&PathBuf::from(expected)),
            "HEAD LOC should contain {expected}"
        );
        let touches = out
            .commits
            .iter()
            .filter(|c| {
                c.deltas
                    .iter()
                    .any(|d| d.path.as_path() == Path::new(expected))
            })
            .count();
        assert_eq!(
            touches, 2,
            "{expected} should be touched by both commits (add + modify)"
        );
    }

    // Zero non-HEAD events — everything added is still at HEAD.
    assert_eq!(out.counts.non_head_events, 0);
}
