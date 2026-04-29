//! `read_head_blob` round-trip: reads the HEAD-tree blob bytes for a
//! path even when the working tree has diverged. Pre-condition for
//! the v0.7 EVASION sensor — without HEAD vs working-tree
//! comparison, the "newly added broad handler" delta is
//! unobservable.

mod common;

use common::{commit_all, init_repo, write};
use mmk_git::read_head_blob;
use std::path::Path;
use tempfile::TempDir;

#[test]
fn returns_head_bytes_when_working_tree_diverges() {
    let dir = TempDir::new().unwrap();
    let repo_path = dir.path();
    init_repo(repo_path);

    write(repo_path, "file.ts", "head content v1\n");
    commit_all(repo_path, "init", 1_700_000_000);

    // Diverge the working tree without committing.
    std::fs::write(repo_path.join("file.ts"), "working content v2\n")
        .expect("rewrite working tree");

    let repo = gix::open(repo_path).expect("open repo");
    let bytes = read_head_blob(&repo, Path::new("file.ts"))
        .expect("read_head_blob ok")
        .expect("file.ts must exist at HEAD");
    assert_eq!(
        std::str::from_utf8(&bytes).unwrap(),
        "head content v1\n",
        "must return HEAD bytes, not working-tree bytes"
    );
}

#[test]
fn new_file_returns_none() {
    let dir = TempDir::new().unwrap();
    let repo_path = dir.path();
    init_repo(repo_path);
    write(repo_path, "first.ts", "x = 1;\n");
    commit_all(repo_path, "init", 1_700_000_000);

    // brand-new working-tree file — never committed
    std::fs::write(repo_path.join("brand_new.ts"), "fresh\n").unwrap();

    let repo = gix::open(repo_path).expect("open repo");
    let bytes = read_head_blob(&repo, Path::new("brand_new.ts")).expect("read_head_blob ok");
    assert!(bytes.is_none(), "new file must yield None; got {bytes:?}");
}

#[test]
fn missing_path_returns_none() {
    let dir = TempDir::new().unwrap();
    let repo_path = dir.path();
    init_repo(repo_path);
    write(repo_path, "exists.ts", "x = 1;\n");
    commit_all(repo_path, "init", 1_700_000_000);

    let repo = gix::open(repo_path).expect("open repo");
    let bytes = read_head_blob(&repo, Path::new("does/not/exist.ts")).expect("read_head_blob ok");
    assert!(
        bytes.is_none(),
        "missing path must yield None; got {bytes:?}"
    );
}

#[test]
fn unborn_head_returns_none() {
    let dir = TempDir::new().unwrap();
    let repo_path = dir.path();
    init_repo(repo_path);
    // No commits yet — HEAD is unborn.
    let repo = gix::open(repo_path).expect("open repo");
    let bytes =
        read_head_blob(&repo, Path::new("anything.ts")).expect("read_head_blob ok on unborn HEAD");
    assert!(
        bytes.is_none(),
        "unborn HEAD must yield None; got {bytes:?}"
    );
}
