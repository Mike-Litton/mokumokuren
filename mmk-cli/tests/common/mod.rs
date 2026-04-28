//! Fixture-repo helpers. Creation uses the git CLI (`git` in $PATH) —
//! rolling our own commit synthesis via gix's write APIs would be
//! considerably more code for no test-value gain.
//!
//! `init_repo` / `commit_all` / `write` / `git` / `DAY` are
//! intentionally duplicated in `mmk-git/tests/common/mod.rs`: a
//! workspace `mmk-test-fixtures` crate would add cross-crate path
//! dev-deps that complicate `cargo install`, and the helpers haven't
//! drifted across three releases. Re-evaluate if a third crate needs
//! the same surface.

use std::path::Path;
use std::process::Command;

/// Tests that flip `current_dir()` (cwd-flipping helpers like
/// `run_in`) carry `#[serial_test::serial(cwd)]` to serialize
/// against the process-wide CWD hazard. Tests that don't touch cwd
/// keep running in parallel.
pub const DAY: i64 = 86_400;

/// Run `f` with `current_dir = repo`, returning `(result, stdout,
/// stderr)`. Centralises the cwd-flip + restore boilerplate that
/// every per-command integration helper used to inline.
///
/// Callers pass a closure that takes `&mut Vec<u8>` writers for
/// stdout / stderr and returns the command's `Result`. The helper
/// owns lifetime of the buffers so callers can pull either back
/// without re-flipping cwd.
#[allow(dead_code)]
pub fn with_cwd<R, F>(repo: &Path, f: F) -> (anyhow::Result<R>, Vec<u8>, Vec<u8>)
where
    F: FnOnce(&mut Vec<u8>, &mut Vec<u8>) -> anyhow::Result<R>,
{
    let orig = std::env::current_dir().expect("read cwd");
    std::env::set_current_dir(repo).expect("set cwd");
    let mut stdout: Vec<u8> = Vec::new();
    let mut stderr: Vec<u8> = Vec::new();
    let res = f(&mut stdout, &mut stderr);
    std::env::set_current_dir(orig).expect("restore cwd");
    (res, stdout, stderr)
}

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
#[allow(dead_code)] // not all integration test files use this fixture
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

/// Coupling fixture: six commits over `core/a.rs` and `core/b.rs`,
/// designed to land both metrics in the same range so a single
/// fixture exercises blast-radius (jaccard) and COUPLING (Wilson
/// lower bound on conditional probability). Calibrated for the v0.6
/// gate (`confidence_threshold = 0.30`, `min_sample_size = 3`).
///
/// Derived metrics:
/// - co(A,B) = 4, touches_A = 5, touches_B = 4.
/// - jaccard(A,B) = 4 / (5+4-4) = 0.80.
/// - P(B|A) = 4/5 = 0.80.
/// - Wilson 95 % lower for 4/5, n=5 ≈ 0.376 (above the 0.30 default).
///
/// A sidecar `core/c.rs` touches once alone, providing a low-jaccard
/// non-partner for the "popular files" sanity check. Mirrors the
/// hand-built scenario in
/// `mmk-core/tests/coupling.rs::jaccard_three_quarters_on_hand_built_fixture`
/// but goes through the full `gix` walk so the CLI integration is
/// exercised end-to-end.
#[allow(dead_code)] // not all integration test files use this fixture
pub fn build_coupling_fixture(repo: &Path, now: i64) {
    init_repo(repo);

    write(repo, "core/a.rs", "a1\n");
    write(repo, "core/b.rs", "b1\n");
    commit_all(repo, "1: a+b co-change", now - 6 * DAY);

    write(repo, "core/a.rs", "a1\na2\n");
    write(repo, "core/b.rs", "b1\nb2\n");
    commit_all(repo, "2: a+b co-change", now - 5 * DAY);

    write(repo, "core/a.rs", "a1\na2\na3\n");
    write(repo, "core/b.rs", "b1\nb2\nb3\n");
    commit_all(repo, "3: a+b co-change", now - 4 * DAY);

    write(repo, "core/a.rs", "a1\na2\na3\na4\n");
    write(repo, "core/b.rs", "b1\nb2\nb3\nb4\n");
    commit_all(repo, "4: a+b co-change", now - 3 * DAY);

    // Extra A-only commit lifts touches_A to 5 (vs touches_B = 4)
    // so jaccard diverges from P(B|A) and COUPLING fires under the
    // v0.6 gate (Wilson 4/5 ≈ 0.376 ≥ 0.30, n=5 ≥ 3).
    write(repo, "core/a.rs", "a1\na2\na3\na4\na5\n");
    commit_all(repo, "5: a only", now - 2 * DAY);

    // Sidecar file that touches once alone, providing a non-partner
    // sanity check (a popular-file edit that's unrelated to A or B).
    write(repo, "core/c.rs", "c1\n");
    commit_all(repo, "6: c only", now - DAY);
}

/// Session fixture: `main` carries heavy historical churn across
/// `core/a.rs`..`core/e.rs`, dominating any reasonable top-N. The
/// feature branch then introduces `feat/x.rs` — barely a blip in the
/// window ranking but #1 in the session ranking. Running session
/// against `main` with `top=3` puts `feat/x.rs` in `entered_top_n`
/// (session top-N) without it appearing in the window top-N.
#[allow(dead_code)] // not all integration test files use this fixture
pub fn build_session_fixture(repo: &Path, now: i64) {
    use std::fmt::Write as _;

    init_repo(repo);

    // Five main-branch files, each with multiple churn commits to
    // drive their hotspot scores well above feat/x.rs's two commits.
    let main_files = [
        "core/a.rs",
        "core/b.rs",
        "core/c.rs",
        "core/d.rs",
        "core/e.rs",
    ];
    for (i, f) in main_files.iter().enumerate() {
        write(repo, f, "init\n");
        commit_all(repo, &format!("main init {f}"), now - (40 - i as i64) * DAY);
    }
    for round in 0..3_i64 {
        for (i, f) in main_files.iter().enumerate() {
            let mut body = String::new();
            for n in 0..(20 + round * 5) {
                writeln!(body, "line{n}").unwrap();
            }
            write(repo, f, &body);
            commit_all(
                repo,
                &format!("main r{round} {f}"),
                now - (30 - round * 5 - i as i64) * DAY,
            );
        }
    }

    git(repo, &["checkout", "-q", "-b", "feature"]);
    write(repo, "feat/x.rs", "x1\n");
    commit_all(repo, "feat 1: introduce x", now - 2 * DAY);
    write(repo, "feat/x.rs", "x1\nx2\n");
    commit_all(repo, "feat 2: edit x", now - DAY);
}
