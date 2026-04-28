//! Per-key monotonic-worsening dedup for sensor findings.
//!
//! Locks the v0.6 contract: a COMPLEXITY finding for `(path,
//! function, kind)` only re-fires if its axis strictly worsened
//! since the last emission (within TTL). Equal or improving axes
//! within TTL → suppressed.

mod common;

use common::{commit_all, init_repo, write, CWD_LOCK, DAY};
use mokumokuren::args::{Format, Gate, ReviewArgs};
use mokumokuren::monotonic::{
    self, should_suppress, MonotonicEntry, MonotonicStore, MONOTONIC_SWEEP_MULTIPLIER,
};
use serde_json::Value;
use tempfile::TempDir;

fn review_args() -> ReviewArgs {
    ReviewArgs {
        staged: false,
        range: None,
        commit: None,
        since: "60days".into(),
        top: 20,
        format: Format::Json,
        ignores: Vec::new(),
        config: None,
        verbose: false,
        coupling_threshold: None,
        blast_radius_threshold: None,
        gate: Gate::None,
        // Leave the whole-set dedup ENABLED so the per-key gate is
        // exercised (it sits in front of the whole-set dedup). The
        // per-key gate's own state lives in the cache directory and
        // is per-key, so two consecutive review runs in the same
        // tempdir share state.
        no_dedup: false,
    }
}

/// Like the per-test-binary `run_in` helper used elsewhere, but
/// expects the caller to already hold `CWD_LOCK`. The monotonic
/// integration tests need that property because they manipulate
/// `MMK_CACHE_DIR` (a process-wide env var) and `cwd` together —
/// holding `CWD_LOCK` across the whole test scope keeps both
/// modifications atomic from the perspective of any concurrent
/// run from another test in this binary.
fn run_review_held_lock(repo: &std::path::Path, args: ReviewArgs) -> Vec<u8> {
    let orig = std::env::current_dir().unwrap();
    std::env::set_current_dir(repo).unwrap();
    let mut stdout: Vec<u8> = Vec::new();
    let mut stderr: Vec<u8> = Vec::new();
    let res = mokumokuren::commands::review::run(&args, None, &mut stdout, &mut stderr);
    std::env::set_current_dir(orig).unwrap();
    res.expect("review should succeed");
    stdout
}

const DEEP_BODY: &str = "function deep() {\n\
    if (a) { if (b) { if (c) { if (d) { if (e) { if (f) { if (g) { return 1; } } } } } } }\n\
}\n";
const DEEPER_BODY: &str = "function deep() {\n\
    if (a) { if (b) { if (c) { if (d) { if (e) { if (f) { if (g) { if (h) { return 1; } } } } } } } }\n\
}\n";

fn complexity_count(stdout: &[u8]) -> usize {
    let v: Value = serde_json::from_slice(stdout).expect("valid JSON");
    v["findings"]
        .as_array()
        .expect("findings array")
        .iter()
        .filter(|f| f["layer"] == "complexity")
        .count()
}

#[test]
fn complexity_re_emit_with_unchanged_axes_is_suppressed() {
    // The motivating failure: same `parse: nesting 8` finding
    // re-emitted across edits where neither nesting nor LOC actually
    // changed. Pre-v0.6 the agent saw multiple copies; the monotonic
    // gate must drop the second and later repeats.
    let _g = CWD_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);

    let dir = TempDir::new().unwrap();
    // Scope MMK_CACHE_DIR to this tempdir. Because we hold the
    // `CWD_LOCK` for the whole test, no concurrent test in this
    // binary can clobber the env var between our two runs.
    let cache = dir.path().join(".mmk-cache");
    std::env::set_var("MMK_CACHE_DIR", &cache);

    let now = 1_700_000_000_i64;
    init_repo(dir.path());
    write(dir.path(), "src/seed.ts", "export const x = 1;\n");
    commit_all(dir.path(), "seed", now - 5 * DAY);

    // Edit #1: deep nesting (8) appears in src/deep.ts.
    write(dir.path(), "src/deep.ts", DEEP_BODY);
    let first = run_review_held_lock(dir.path(), review_args());
    assert!(
        complexity_count(&first) >= 1,
        "first edit must surface COMPLEXITY; got: {}",
        String::from_utf8_lossy(&first)
    );

    // Edit #2: identical complexity content. Simultaneously change
    // an unrelated seed file so the whole-set dedup *would* re-emit
    // — that isolates the per-key gate as the only thing that can
    // suppress the COMPLEXITY repeat.
    write(dir.path(), "src/seed.ts", "export const x = 2;\n");
    let second = run_review_held_lock(dir.path(), review_args());
    assert_eq!(
        complexity_count(&second),
        0,
        "second edit with unchanged complexity axes must suppress \
         the COMPLEXITY finding; got: {}",
        String::from_utf8_lossy(&second)
    );

    std::env::remove_var("MMK_CACHE_DIR");
}

#[test]
fn complexity_re_emit_with_strict_worsening_re_fires() {
    let _g = CWD_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);

    let dir = TempDir::new().unwrap();
    let cache = dir.path().join(".mmk-cache");
    std::env::set_var("MMK_CACHE_DIR", &cache);

    let now = 1_700_000_000_i64;
    init_repo(dir.path());
    write(dir.path(), "src/seed.ts", "export const x = 1;\n");
    commit_all(dir.path(), "seed", now - 5 * DAY);

    // Edit #1: nesting depth 8 (over default cap=6).
    write(dir.path(), "src/deep.ts", DEEP_BODY);
    let first = run_review_held_lock(dir.path(), review_args());
    assert!(complexity_count(&first) >= 1, "first edit must fire");

    // Edit #2: nesting depth 9 — strict worsening on the nesting
    // axis. The gate must let this re-fire so the agent sees the
    // axis moved against them.
    write(dir.path(), "src/deep.ts", DEEPER_BODY);
    let second = run_review_held_lock(dir.path(), review_args());
    assert!(
        complexity_count(&second) >= 1,
        "strict worsening on nesting must re-fire COMPLEXITY; got: {}",
        String::from_utf8_lossy(&second)
    );

    std::env::remove_var("MMK_CACHE_DIR");
}

#[test]
fn sweep_drops_entries_older_than_30_ttl() {
    // Pure unit-style check: an entry older than 30×TTL is dropped on
    // load. Without the sweep, a long-lived agent session would
    // accumulate entries forever.
    let mut store = MonotonicStore::default();
    let ttl = 1800_i64;
    let now = 1_700_000_000_i64;
    store.entries.insert(
        "fresh".into(),
        MonotonicEntry {
            axes: vec![1],
            last_emitted_at: now - ttl, // 1 TTL ago — keep
            head_sha: "abc".into(),
        },
    );
    store.entries.insert(
        "stale".into(),
        MonotonicEntry {
            axes: vec![1],
            // (MULTIPLIER + 1) × TTL ago — definitely beyond horizon.
            last_emitted_at: now - (MONOTONIC_SWEEP_MULTIPLIER + 1) * ttl,
            head_sha: "abc".into(),
        },
    );

    monotonic::sweep(&mut store, now, ttl);
    assert!(
        store.entries.contains_key("fresh"),
        "fresh entry must survive the sweep"
    );
    assert!(
        !store.entries.contains_key("stale"),
        "entry older than 30×TTL must be dropped"
    );
}

#[test]
fn should_suppress_handles_axis_count_change() {
    // Forward-compat: a future schema change (e.g. adding a third
    // axis like cyclomatic complexity) shouldn't suppress through
    // a stale two-axis prior entry — the gate can't compare apples
    // to apples across axis counts.
    let prior = MonotonicEntry {
        axes: vec![8, 80],
        last_emitted_at: 1000,
        head_sha: "abc".into(),
    };
    assert!(!should_suppress(Some(&prior), &[8, 80, 0], 2000, 1800));
}

#[test]
fn coupling_re_emit_with_unchanged_axes_is_suppressed() {
    // Generalization of the COMPLEXITY case to COUPLING. The
    // monotonic key is `coupling::<subject>::<partner>` with axes
    // `[k, n]` (k = co_change_count, n = commits_touching(subject)).
    // A second review run against the same diff produces identical
    // axes; the per-key gate must drop the repeat.
    let _g = CWD_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);

    let dir = TempDir::new().unwrap();
    let cache = dir.path().join(".mmk-cache");
    std::env::set_var("MMK_CACHE_DIR", &cache);

    let now = 1_700_000_000_i64;
    common::build_coupling_fixture(dir.path(), now);
    write(dir.path(), "core/a.rs", "a1\na2\na3\na4\na5\nuncommitted\n");

    let first = run_review_held_lock(dir.path(), review_args());
    let coupling_first = coupling_count(&first);
    assert!(
        coupling_first >= 1,
        "first review should surface COUPLING; got: {}",
        String::from_utf8_lossy(&first)
    );

    // Touch an unrelated file so whole-set dedup *would* re-emit;
    // this isolates the per-key COUPLING gate.
    write(dir.path(), "core/c.rs", "c1\nc2\n");
    let second = run_review_held_lock(dir.path(), review_args());
    let coupling_second = coupling_count(&second);
    assert_eq!(
        coupling_second,
        0,
        "second review with unchanged COUPLING axes must drop the \
         re-fire; got: {}",
        String::from_utf8_lossy(&second)
    );

    std::env::remove_var("MMK_CACHE_DIR");
}

#[test]
fn cap_lru_drops_oldest_when_above_cap() {
    use mokumokuren::monotonic::{cap_lru, MONOTONIC_MAX_ENTRIES};

    let mut store = MonotonicStore::default();
    // Insert MAX + 1 entries with strictly ascending
    // last_emitted_at; the LRU drop must remove the single oldest.
    for i in 0..=MONOTONIC_MAX_ENTRIES {
        store.entries.insert(
            format!("k{i:08}"),
            MonotonicEntry {
                axes: vec![1],
                last_emitted_at: i as i64,
                head_sha: "abc".into(),
            },
        );
    }
    cap_lru(&mut store);
    assert_eq!(store.entries.len(), MONOTONIC_MAX_ENTRIES);
    assert!(
        !store.entries.contains_key("k00000000"),
        "oldest entry must be evicted"
    );
    assert!(
        store
            .entries
            .contains_key(&format!("k{MONOTONIC_MAX_ENTRIES:08}")),
        "newest entry must survive"
    );
}

fn coupling_count(stdout: &[u8]) -> usize {
    let v: Value = serde_json::from_slice(stdout).expect("valid JSON");
    v["findings"]
        .as_array()
        .expect("findings array")
        .iter()
        .filter(|f| f["layer"] == "coupling")
        .count()
}
