//! Fixture-repo helpers. Creation uses the git CLI (`git` in $PATH) —
//! rolling our own commit synthesis via gix's write APIs would be
//! considerably more code for no test-value gain.
//!
//! `init_repo` / `commit_all` / `write` / `git` / `DAY` are
//! intentionally duplicated in `mmk-cli/tests/common/mod.rs`: a
//! workspace `mmk-test-fixtures` crate would add cross-crate path
//! dev-deps that complicate `cargo install`, and the helpers haven't
//! drifted across three releases. Re-evaluate if a third crate needs
//! the same surface.

use std::path::Path;
use std::process::Command;

pub const DAY: i64 = 86_400;

pub fn git(cwd: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("failed to invoke git");
    assert!(
        out.status.success(),
        "git {args:?} failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
}

pub fn write(repo: &Path, rel: &str, body: &str) {
    let p = repo.join(rel);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).expect("mkdir");
    }
    std::fs::write(&p, body).expect("write fixture file");
}

pub fn init_repo(repo: &Path) {
    git(repo, &["init", "-q", "-b", "main"]);
    git(repo, &["config", "user.email", "test@example.com"]);
    git(repo, &["config", "user.name", "Test"]);
    git(repo, &["config", "commit.gpgsign", "false"]);
}

pub fn commit_all(repo: &Path, msg: &str, when: i64) {
    git(repo, &["add", "-A"]);
    let when_str = format!("{when} +0000");
    let status = Command::new("git")
        .args(["commit", "-q", "-m", msg])
        .env("GIT_AUTHOR_DATE", &when_str)
        .env("GIT_COMMITTER_DATE", &when_str)
        .current_dir(repo)
        .status()
        .expect("git commit");
    assert!(status.success(), "git commit failed");
}

/// Canonical fixture: commit A adds `a.rs` + `b.rs`, commit B heavily
/// modifies `a.rs`, commit C renames `b.rs` → `c.rs`, commit D modifies
/// `c.rs`. `b.rs` exists in-window but is not at HEAD (it was renamed
/// away), exercising the "deleted from HEAD" counter. `c.rs` survives
/// to HEAD so its rename-detection event is observable.
pub fn build_canonical_fixture(repo: &Path, now: i64) {
    init_repo(repo);

    write(repo, "a.rs", "line1\nline2\nline3\n");
    write(repo, "b.rs", "hello\nworld\n");
    commit_all(repo, "A: initial", now - 4 * DAY);

    write(
        repo,
        "a.rs",
        "line1\nline2_modified\nline3_modified\nline4\nline5\nline6\n",
    );
    commit_all(repo, "B: modify a.rs", now - 3 * DAY);

    std::fs::remove_file(repo.join("b.rs")).expect("rm b.rs");
    write(repo, "c.rs", "hello\nworld\n");
    commit_all(repo, "C: rename b->c", now - 2 * DAY);

    write(repo, "c.rs", "hello\nworld\nplus more\n");
    commit_all(repo, "D: modify c.rs", now - DAY);
}
