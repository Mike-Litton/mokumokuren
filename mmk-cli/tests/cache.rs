//! `mmk cache info` / `mmk cache clear` integration tests.
//!
//! Until this file landed the cache subcommand had zero coverage —
//! the only way it surfaced regressions was downstream test failure
//! on whatever caller happened to hit a stale cache. The tests pin
//! the contract:
//!
//!   - `info` against a fresh repo prints "no cache yet" for each of
//!     the three caches.
//!   - After an `analyze` run the per-commit deltas / revwalk /
//!     head-tree caches all populate; `info` reports their entry
//!     counts.
//!   - `clear --scope all` (the default) removes every cache file.
//!   - `clear --scope deltas` / `revwalk` / `loc` targets one cache
//!     and leaves the others untouched.

mod common;

use common::{build_canonical_fixture, with_cwd};
use mokumokuren::args::{AnalyzeArgs, CacheArgs, CacheClearArgs, CacheCommand, CacheScope, Format};
use serial_test::serial;
use std::path::Path;
use tempfile::TempDir;

fn analyze_args() -> AnalyzeArgs {
    AnalyzeArgs {
        since: "30days".into(),
        top: 20,
        format: Format::Json,
        ignores: Vec::new(),
        config: None,
        verbose: false,
        couples_of: None,
        couples: false,
        blast_radius: None,
        blast_radius_threshold: None,
    }
}

fn run_cache(repo: &Path, args: CacheArgs) -> String {
    let (res, stdout, _) = with_cwd(repo, |so, se| {
        mokumokuren::commands::cache::run(&args, so, se)
    });
    res.expect("cache should succeed");
    String::from_utf8(stdout).expect("cache emits UTF-8")
}

fn run_analyze(repo: &Path) {
    let (res, _, _) = with_cwd(repo, |so, se| {
        mokumokuren::commands::analyze::run(&analyze_args(), so, se)
    });
    res.expect("analyze should succeed");
}

fn cache_files(repo: &Path) -> [std::path::PathBuf; 4] {
    let git_dir = repo.join(".git");
    [
        mmk_git::cache::cache_path(&git_dir).expect("deltas path"),
        mmk_git::cache::revwalk_cache_path(&git_dir).expect("revwalk path"),
        mmk_git::cache::head_tree_cache_path(&git_dir).expect("head_tree path"),
        mmk_git::cache::loc_cache_path(&git_dir).expect("loc path"),
    ]
}

/// RAII guard for one env var. `MMK_CACHE_DIR` is process-wide;
/// `#[serial(cwd)]` prevents concurrent overlap across tests, but
/// each test still needs to clear the override on its own way out.
struct ScopedEnv(&'static str);

impl ScopedEnv {
    fn set(key: &'static str, value: &Path) -> Self {
        std::env::set_var(key, value);
        Self(key)
    }
}

impl Drop for ScopedEnv {
    fn drop(&mut self) {
        std::env::remove_var(self.0);
    }
}

#[serial(cwd)]
#[test]
fn cache_info_on_fresh_repo_reports_no_cache_yet() {
    let cache_root = TempDir::new().unwrap();
    let _env = ScopedEnv::set("MMK_CACHE_DIR", cache_root.path());
    let dir = TempDir::new().unwrap();
    build_canonical_fixture(dir.path(), 1_700_000_000);

    let info = run_cache(
        dir.path(),
        CacheArgs {
            command: CacheCommand::Info,
        },
    );
    // Expect the "no cache yet" status for each of the three caches —
    // populated by analyze in subsequent tests but not yet here.
    assert!(
        info.contains("deltas (per-commit):") && info.contains("no cache yet"),
        "fresh repo should report no deltas cache; got: {info}"
    );
    assert!(
        info.contains("revwalk:") && info.matches("no cache yet").count() >= 4,
        "fresh repo should report no cache yet on all four caches; got: {info}"
    );
    assert!(
        info.contains("head-tree:"),
        "info should list head-tree cache; got: {info}"
    );
    assert!(
        info.contains("loc (per-blob):"),
        "info should list loc cache; got: {info}"
    );
}

#[serial(cwd)]
#[test]
fn cache_info_after_analyze_reports_populated_entries() {
    let cache_root = TempDir::new().unwrap();
    let _env = ScopedEnv::set("MMK_CACHE_DIR", cache_root.path());
    let dir = TempDir::new().unwrap();
    build_canonical_fixture(dir.path(), 1_700_000_000);

    run_analyze(dir.path());

    let info = run_cache(
        dir.path(),
        CacheArgs {
            command: CacheCommand::Info,
        },
    );
    // After analyze: every cache reports an entry count + schema
    // version. None should still say "no cache yet".
    assert!(
        info.contains("entries:") && info.contains("schema:"),
        "post-analyze info must report entries + schema; got: {info}"
    );
    assert!(
        !info.contains("no cache yet"),
        "post-analyze info must not show 'no cache yet'; got: {info}"
    );
}

#[serial(cwd)]
#[test]
fn cache_clear_all_removes_every_cache_file() {
    let cache_root = TempDir::new().unwrap();
    let _env = ScopedEnv::set("MMK_CACHE_DIR", cache_root.path());
    let dir = TempDir::new().unwrap();
    build_canonical_fixture(dir.path(), 1_700_000_000);

    run_analyze(dir.path());
    let [deltas, revwalk, head_tree, loc] = cache_files(dir.path());
    assert!(deltas.exists() && revwalk.exists() && head_tree.exists() && loc.exists());

    let out = run_cache(
        dir.path(),
        CacheArgs {
            command: CacheCommand::Clear(CacheClearArgs {
                scope: CacheScope::All,
            }),
        },
    );
    assert!(
        out.contains("removed"),
        "clear should report removal; got: {out}"
    );
    assert!(!deltas.exists(), "deltas cache must be gone");
    assert!(!revwalk.exists(), "revwalk cache must be gone");
    assert!(!head_tree.exists(), "head-tree cache must be gone");
    assert!(!loc.exists(), "loc cache must be gone");
}

#[serial(cwd)]
#[test]
fn cache_clear_scoped_only_removes_targeted_cache() {
    // Targeted clear — `--scope deltas` removes the deltas cache but
    // leaves revwalk and head-tree intact. Locks the contract that
    // each scope name maps to exactly one cache file.
    let cache_root = TempDir::new().unwrap();
    let _env = ScopedEnv::set("MMK_CACHE_DIR", cache_root.path());
    let dir = TempDir::new().unwrap();
    build_canonical_fixture(dir.path(), 1_700_000_000);

    run_analyze(dir.path());
    let [deltas, revwalk, head_tree, loc] = cache_files(dir.path());

    let _ = run_cache(
        dir.path(),
        CacheArgs {
            command: CacheCommand::Clear(CacheClearArgs {
                scope: CacheScope::Deltas,
            }),
        },
    );
    assert!(!deltas.exists(), "deltas cache must be removed");
    assert!(revwalk.exists(), "revwalk must remain");
    assert!(head_tree.exists(), "head-tree must remain");
    assert!(loc.exists(), "loc must remain");

    let _ = run_cache(
        dir.path(),
        CacheArgs {
            command: CacheCommand::Clear(CacheClearArgs {
                scope: CacheScope::Revwalk,
            }),
        },
    );
    assert!(!revwalk.exists(), "revwalk must be removed");
    assert!(head_tree.exists(), "head-tree must remain");
    assert!(loc.exists(), "loc must remain");

    let _ = run_cache(
        dir.path(),
        CacheArgs {
            command: CacheCommand::Clear(CacheClearArgs {
                scope: CacheScope::HeadTree,
            }),
        },
    );
    assert!(!head_tree.exists(), "head-tree must be removed");
    assert!(loc.exists(), "loc must remain");

    let _ = run_cache(
        dir.path(),
        CacheArgs {
            command: CacheCommand::Clear(CacheClearArgs {
                scope: CacheScope::Loc,
            }),
        },
    );
    assert!(!loc.exists(), "loc cache must be removed");
}

#[serial(cwd)]
#[test]
fn cache_clear_on_empty_caches_reports_nothing_to_clear() {
    // Defensive: clear before analyze. Each cache file is missing —
    // the command must succeed with a "no cache to clear" line per
    // cache rather than erroring out.
    let cache_root = TempDir::new().unwrap();
    let _env = ScopedEnv::set("MMK_CACHE_DIR", cache_root.path());
    let dir = TempDir::new().unwrap();
    build_canonical_fixture(dir.path(), 1_700_000_000);

    let out = run_cache(
        dir.path(),
        CacheArgs {
            command: CacheCommand::Clear(CacheClearArgs {
                scope: CacheScope::All,
            }),
        },
    );
    assert!(
        out.contains("no cache to clear"),
        "clear on empty cache should be a no-op with status; got: {out}"
    );
}
