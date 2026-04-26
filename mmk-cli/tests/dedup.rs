//! Per-fire dedup for `mmk review` and `mmk pre-edit`.
//!
//! Unit truth-table for `should_suppress`, plus integration tests
//! that drive the real subcommands twice in succession and verify
//! the second invocation goes silent under the documented rules:
//!   1. same findings hash AND
//!   2. same HEAD SHA AND
//!   3. emitted within `MMK_DEDUP_TTL_SECONDS` of the first fire.
//!
//! Any one of those changing flushes the suppression — the agent's
//! mental model is "if the picture changed since last time, show me."

mod common;

use common::{commit_all, init_repo, write, CWD_LOCK, DAY};
use mokumokuren::args::{Format, Gate, ReviewArgs};
use mokumokuren::dedup::{should_suppress, DedupRecord};
use std::path::PathBuf;
use tempfile::TempDir;

fn rec(hash: u64, sha: &str, ts: i64) -> DedupRecord {
    DedupRecord {
        findings_hash: hash,
        head_sha: sha.to_string(),
        emitted_at: ts,
    }
}

#[test]
fn should_suppress_same_hash_same_sha_within_ttl_returns_true() {
    let prior = rec(7, "abc", 1_000);
    assert!(should_suppress(7, "abc", Some(&prior), 1_500, 600));
}

#[test]
fn should_suppress_different_hash_returns_false() {
    let prior = rec(7, "abc", 1_000);
    assert!(!should_suppress(8, "abc", Some(&prior), 1_500, 600));
}

#[test]
fn should_suppress_different_sha_returns_false() {
    let prior = rec(7, "abc", 1_000);
    assert!(!should_suppress(7, "def", Some(&prior), 1_500, 600));
}

#[test]
fn should_suppress_over_ttl_returns_false() {
    let prior = rec(7, "abc", 1_000);
    assert!(!should_suppress(7, "abc", Some(&prior), 5_000, 600));
}

#[test]
fn should_suppress_no_prior_record_returns_false() {
    assert!(!should_suppress(7, "abc", None, 1_500, 600));
}

#[test]
fn should_suppress_exactly_at_ttl_boundary_returns_false() {
    // The TTL boundary is exclusive: at exactly ttl seconds elapsed
    // we re-emit, matching the "if the picture changed since last
    // time" mental model rather than playing games with off-by-one.
    let prior = rec(7, "abc", 1_000);
    assert!(!should_suppress(7, "abc", Some(&prior), 1_600, 600));
}

#[test]
fn dedup_record_corruption_recovers_to_none() {
    use mokumokuren::dedup::load_record;
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("dedup.bincode.v1");
    std::fs::write(&path, b"this is not a valid bincode payload").unwrap();
    assert!(load_record(&path).is_none());
}

#[test]
fn dedup_record_missing_returns_none() {
    use mokumokuren::dedup::load_record;
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("does-not-exist.bin");
    assert!(load_record(&path).is_none());
}

#[test]
fn dedup_record_save_then_load_roundtrip() {
    use mokumokuren::dedup::{load_record, record_emission};
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("dedup.bincode.v1");
    let r = rec(42, "deadbeef", 12_345);
    record_emission(&path, &r);
    let loaded = load_record(&path).expect("must load back");
    assert_eq!(loaded.findings_hash, 42);
    assert_eq!(loaded.head_sha, "deadbeef");
    assert_eq!(loaded.emitted_at, 12_345);
}

// --- Integration tests against the real `mmk review` invocation ---

fn review_args() -> ReviewArgs {
    ReviewArgs {
        staged: false,
        range: None,
        commit: None,
        since: "60days".into(),
        top: 20,
        format: Format::Text,
        ignores: Vec::new(),
        config: None,
        verbose: false,
        coupling_threshold: None,
        blast_radius_threshold: None,
        gate: Gate::None,
        no_dedup: false,
    }
}

fn run_in(repo: &std::path::Path, args: ReviewArgs) -> Vec<u8> {
    let _g = CWD_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let orig = std::env::current_dir().unwrap();
    std::env::set_current_dir(repo).unwrap();
    let mut stdout: Vec<u8> = Vec::new();
    let mut stderr: Vec<u8> = Vec::new();
    let res = mokumokuren::commands::review::run(&args, &mut stdout, &mut stderr);
    std::env::set_current_dir(orig).unwrap();
    res.expect("review");
    stdout
}

/// Build a tiny fixture that produces deterministic findings on
/// every `mmk review` call.
fn fixture(now: i64) -> TempDir {
    let dir = TempDir::new().unwrap();
    init_repo(dir.path());
    write(dir.path(), "core/a.rs", "a1\n");
    write(dir.path(), "core/b.rs", "b1\n");
    commit_all(dir.path(), "1", now - 5 * DAY);
    write(dir.path(), "core/a.rs", "a1\na2\n");
    write(dir.path(), "core/b.rs", "b1\nb2\n");
    commit_all(dir.path(), "2", now - 4 * DAY);
    write(dir.path(), "core/a.rs", "a1\na2\na3\n");
    write(dir.path(), "core/b.rs", "b1\nb2\nb3\n");
    commit_all(dir.path(), "3", now - 3 * DAY);
    write(dir.path(), "core/a.rs", "a1\na2\na3\na4\n");
    commit_all(dir.path(), "4", now - 2 * DAY);
    write(dir.path(), "core/a.rs", "a1\na2\na3\na4\na5\n");
    commit_all(dir.path(), "5", now - DAY);
    // Dirty working tree so review actually does work.
    write(dir.path(), "core/a.rs", "a1\na2\na3\na4\na5\nNEW\n");
    dir
}

fn unique_cache_dir() -> TempDir {
    TempDir::new().expect("tmpdir")
}

#[test]
fn review_twice_in_succession_suppresses_second() {
    // Same findings hash + same HEAD + within TTL → second invocation
    // emits empty stdout. The agent sees the finding once.
    let cache = unique_cache_dir();
    std::env::set_var("MMK_CACHE_DIR", cache.path());
    let _restore_cache = scopeguard(|| std::env::remove_var("MMK_CACHE_DIR"));

    let now = 1_700_000_000_i64;
    let dir = fixture(now);
    let first = run_in(dir.path(), review_args());
    assert!(
        !first.is_empty(),
        "first invocation must emit findings; got empty"
    );
    let second = run_in(dir.path(), review_args());
    assert!(
        second.is_empty(),
        "second invocation with same state must be suppressed; got: {}",
        String::from_utf8_lossy(&second)
    );
}

#[test]
fn review_re_emits_after_head_changes() {
    // A fresh commit changes HEAD SHA → re-emit even within TTL.
    let cache = unique_cache_dir();
    std::env::set_var("MMK_CACHE_DIR", cache.path());
    let _restore_cache = scopeguard(|| std::env::remove_var("MMK_CACHE_DIR"));

    let now = 1_700_000_000_i64;
    let dir = fixture(now);
    let _first = run_in(dir.path(), review_args());

    common::git(
        dir.path(),
        &["commit", "--allow-empty", "-m", "tick", "--no-gpg-sign"],
    );

    let after = run_in(dir.path(), review_args());
    assert!(
        !after.is_empty(),
        "fresh commit must flush the dedup cache and re-emit; got empty"
    );
}

#[test]
fn review_re_emits_after_ttl_elapsed() {
    // TTL=1s + sleep 2 → second invocation re-emits.
    let cache = unique_cache_dir();
    std::env::set_var("MMK_CACHE_DIR", cache.path());
    std::env::set_var("MMK_DEDUP_TTL_SECONDS", "1");
    let _restore_cache = scopeguard(|| {
        std::env::remove_var("MMK_CACHE_DIR");
        std::env::remove_var("MMK_DEDUP_TTL_SECONDS");
    });

    let now = 1_700_000_000_i64;
    let dir = fixture(now);
    let _first = run_in(dir.path(), review_args());

    std::thread::sleep(std::time::Duration::from_secs(2));

    let after = run_in(dir.path(), review_args());
    assert!(
        !after.is_empty(),
        "TTL elapsed must allow re-emit; got empty"
    );
}

#[test]
fn review_no_dedup_flag_always_emits() {
    // --no-dedup must bypass the suppression entirely.
    let cache = unique_cache_dir();
    std::env::set_var("MMK_CACHE_DIR", cache.path());
    let _restore_cache = scopeguard(|| std::env::remove_var("MMK_CACHE_DIR"));

    let now = 1_700_000_000_i64;
    let dir = fixture(now);
    let mut a1 = review_args();
    a1.no_dedup = true;
    let mut a2 = review_args();
    a2.no_dedup = true;
    let first = run_in(dir.path(), a1);
    let second = run_in(dir.path(), a2);
    assert_eq!(
        first,
        second,
        "--no-dedup must produce identical output on repeat; first={:?} second={:?}",
        String::from_utf8_lossy(&first),
        String::from_utf8_lossy(&second)
    );
    assert!(!first.is_empty());
}

// Tiny scopeguard so the env vars get restored even on panic.
struct ScopeGuard<F: FnMut()>(F);
impl<F: FnMut()> Drop for ScopeGuard<F> {
    fn drop(&mut self) {
        (self.0)();
    }
}
const fn scopeguard<F: FnMut()>(f: F) -> ScopeGuard<F> {
    ScopeGuard(f)
}

// Suppress unused warning when only some integration tests reference PathBuf.
#[allow(dead_code)]
fn _path_keepalive(_: PathBuf) {}
