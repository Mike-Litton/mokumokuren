//! Per-key monotonic-worsening dedup for finding emissions.
//!
//! Complements the whole-set dedup in [`crate::dedup`]: where that
//! file says "same findings + same HEAD + within TTL → silent," this
//! one says "same key + neither axis worsened + within TTL → drop
//! this individual finding."
//!
//! The motivating case is COMPLEXITY: an agent test run reported
//! seeing the same `parse: nesting 8, 361 LOC` finding three times
//! across edits where neither nesting nor LOC actually changed. The
//! whole-set dedup couldn't help — the *findings array* matched the
//! prior fire, but other axes of the run (HOTSPOT, BUDGET ramp) had
//! moved, so the whole-set hash differed and the COMPLEXITY repeat
//! came along for the ride. The fix is a separate per-finding gate
//! keyed on `(path, function)` that drops the finding unless an axis
//! strictly worsened.
//!
//! Generic shape — `Key = String`, `Axes = Vec<u32>` — so HOTSPOT's
//! v0.7 rank-ratchet (key = path, axis = `[hotspot_rank]`) drops in
//! without rework.
//!
//! Storage: one bincode file per repo at
//! `<cache-root>/<repo-id>/monotonic.bincode.v1`. Honours
//! `MMK_CACHE_DIR` indirectly via `mmk_git::cache::repo_cache_dir`.
//! Best-effort load/save: corruption / decode error → treat as no
//! prior. Failed writes log to stderr; they never block the hook.
//!
//! On load, the store is swept of entries with `last_emitted_at`
//! older than `30 × MMK_DEDUP_TTL_SECONDS` so a long-lived repo
//! cache doesn't accumulate forever.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Per-repo file format version. Bumped on shape changes.
pub const MONOTONIC_VERSION: u32 = 1;

/// Sweep horizon — entries older than this many TTLs are dropped on
/// load. 30 × TTL keeps the hot-path lookups bounded even for repos
/// the agent uses daily for months.
pub const MONOTONIC_SWEEP_MULTIPLIER: i64 = 30;

/// Hard cap on cache entries.
///
/// The TTL sweep is necessary but not sufficient: a long-lived
/// repo with many distinct (subject, partner) COUPLING pairs can
/// grow well past 30 × TTL of fresh entries before any of them age
/// out. The cap drops the least-recently-emitted entries on save
/// so the file size stays bounded for any usage pattern.
pub const MONOTONIC_MAX_ENTRIES: usize = 10_000;

/// One entry per `(layer, identity)` key. The layer is encoded into
/// the key string so HOTSPOT and COMPLEXITY can share the same store
/// without colliding.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonotonicEntry {
    /// The axis values from the *last emitted* fire. The next fire
    /// is suppressed iff every axis is `≤` corresponding stored
    /// value AND the prior is within TTL.
    pub axes: Vec<u32>,
    pub last_emitted_at: i64,
    pub head_sha: String,
}

/// Persisted per-repo store.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MonotonicStore {
    pub entries: BTreeMap<String, MonotonicEntry>,
}

#[derive(Debug, Serialize, Deserialize)]
struct PersistedStore {
    version: u32,
    entries: BTreeMap<String, MonotonicEntry>,
}

/// Pure decision for one (key, axes) pair: should the new fire be
/// suppressed?
///
/// Suppressed iff a prior entry exists, every axis is `≤` the prior
/// stored value (no strict worsening), AND the prior fire is within
/// `ttl_seconds`. Any axis strictly increasing — or the TTL having
/// elapsed — re-fires.
#[must_use]
pub fn should_suppress(
    prior: Option<&MonotonicEntry>,
    axes: &[u32],
    now_unix: i64,
    ttl_seconds: i64,
) -> bool {
    let Some(prior) = prior else { return false };
    if prior.axes.len() != axes.len() {
        // Schema change for this key — re-fire conservatively.
        return false;
    }
    let elapsed = now_unix.saturating_sub(prior.last_emitted_at);
    if elapsed >= ttl_seconds {
        return false;
    }
    // Strict-worsening rule: if any axis is now larger than before,
    // re-fire. Equal or improving on every axis → suppress.
    let any_worsened = axes
        .iter()
        .zip(prior.axes.iter())
        .any(|(now_v, prior_v)| now_v > prior_v);
    !any_worsened
}

/// Update the store with a fresh emission. Caller is expected to
/// only call this for findings that *actually* emit (i.e. weren't
/// suppressed by [`should_suppress`]).
pub fn record(
    store: &mut MonotonicStore,
    key: String,
    axes: Vec<u32>,
    now_unix: i64,
    head_sha: &str,
) {
    store.entries.insert(
        key,
        MonotonicEntry {
            axes,
            last_emitted_at: now_unix,
            head_sha: head_sha.to_string(),
        },
    );
}

/// Sweep entries older than `MONOTONIC_SWEEP_MULTIPLIER × ttl_seconds`.
/// Pure: takes the store by `&mut`, returns nothing.
pub fn sweep(store: &mut MonotonicStore, now_unix: i64, ttl_seconds: i64) {
    let horizon = now_unix.saturating_sub(MONOTONIC_SWEEP_MULTIPLIER.saturating_mul(ttl_seconds));
    store
        .entries
        .retain(|_, entry| entry.last_emitted_at >= horizon);
}

/// Drop entries until the store fits inside `MONOTONIC_MAX_ENTRIES`.
/// LRU policy: the oldest `last_emitted_at` goes first.
///
/// Called by the persistence path immediately before save. Reading the
/// state on a per-fire hot path doesn't pay the cap cost; the cap only
/// kicks in when something would actually be written.
pub fn cap_lru(store: &mut MonotonicStore) {
    if store.entries.len() <= MONOTONIC_MAX_ENTRIES {
        return;
    }
    let mut by_age: Vec<(String, i64)> = store
        .entries
        .iter()
        .map(|(k, v)| (k.clone(), v.last_emitted_at))
        .collect();
    // Ascending: oldest first.
    by_age.sort_by_key(|(_, ts)| *ts);
    let to_drop = store.entries.len() - MONOTONIC_MAX_ENTRIES;
    for (k, _) in by_age.into_iter().take(to_drop) {
        store.entries.remove(&k);
    }
}

/// Resolve the per-repo store path. Sits alongside the existing
/// dedup record so a single cache directory holds both.
#[must_use]
pub fn store_path(git_dir: &Path) -> Option<PathBuf> {
    mmk_git::cache::repo_cache_dir(git_dir)
        .ok()
        .map(|d| d.join(format!("monotonic.bincode.v{MONOTONIC_VERSION}")))
}

/// Best-effort load. Missing / corrupt / wrong-version → empty store.
/// Sweeps stale entries before returning.
#[must_use]
pub fn load(path: &Path, now_unix: i64, ttl_seconds: i64) -> MonotonicStore {
    let mut store = std::fs::read(path).map_or_else(
        |_| MonotonicStore::default(),
        |bytes| {
            bincode::serde::decode_from_slice::<PersistedStore, _>(
                &bytes,
                bincode::config::standard(),
            )
            .ok()
            .filter(|(p, _)| p.version == MONOTONIC_VERSION)
            .map_or_else(MonotonicStore::default, |(p, _)| MonotonicStore {
                entries: p.entries,
            })
        },
    );
    sweep(&mut store, now_unix, ttl_seconds);
    store
}

/// Best-effort save. I/O errors logged to stderr and swallowed —
/// dedup is an optimisation, not a safety invariant.
pub fn save(path: &Path, store: &MonotonicStore) {
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            eprintln!("[mmk] monotonic: mkdir {} failed: {e}", parent.display());
            return;
        }
    }
    let payload = PersistedStore {
        version: MONOTONIC_VERSION,
        entries: store.entries.clone(),
    };
    let Ok(bytes) = bincode::serde::encode_to_vec(&payload, bincode::config::standard()) else {
        eprintln!("[mmk] monotonic: encode failed");
        return;
    };
    let tmp = path.with_extension("tmp");
    if let Err(e) = std::fs::write(&tmp, &bytes) {
        eprintln!("[mmk] monotonic: write {} failed: {e}", tmp.display());
        return;
    }
    if let Err(e) = std::fs::rename(&tmp, path) {
        eprintln!("[mmk] monotonic: rename failed: {e}");
    }
}

/// Per-finding monotonic key + axes annotation, captured at
/// finding-construction time so the dedup gate can act without
/// re-parsing the message string.
#[derive(Debug, Clone)]
pub struct MonotonicSignal {
    /// Stable string identity. Layer prefix prevents COMPLEXITY and
    /// HOTSPOT entries from colliding even if their natural
    /// identities (path) overlap.
    pub key: String,
    /// One or more axis values. The order matters — element `i` is
    /// compared to the prior entry's element `i`.
    pub axes: Vec<u32>,
}

/// Apply the gate to a list of findings paired with optional
/// monotonic signals.
///
/// Returns `(kept_findings, axes_recorded)`, where the second
/// value is the list of (key, axes) pairs that fired and should
/// be persisted by the caller.
///
/// Findings without a `MonotonicSignal` are passed through
/// unchanged. The function does not load or save the store —
/// that's the caller's concern, so the orchestration code can
/// scope I/O to one place.
#[must_use]
pub fn apply<F>(
    items: Vec<(F, Option<MonotonicSignal>)>,
    store: &MonotonicStore,
    now_unix: i64,
    ttl_seconds: i64,
) -> (Vec<F>, Vec<MonotonicSignal>) {
    let mut kept = Vec::with_capacity(items.len());
    let mut to_record = Vec::new();
    for (finding, signal) in items {
        let Some(sig) = signal else {
            kept.push(finding);
            continue;
        };
        let prior = store.entries.get(&sig.key);
        if should_suppress(prior, &sig.axes, now_unix, ttl_seconds) {
            continue;
        }
        kept.push(finding);
        to_record.push(sig);
    }
    (kept, to_record)
}

/// Convenience: the dedup TTL — read from `MMK_DEDUP_TTL_SECONDS`
/// (same env var the whole-set dedup uses) so both gates share a
/// session boundary.
#[must_use]
pub fn ttl_seconds() -> i64 {
    crate::dedup::ttl_seconds()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(axes: &[u32], at: i64) -> MonotonicEntry {
        MonotonicEntry {
            axes: axes.to_vec(),
            last_emitted_at: at,
            head_sha: "deadbeef".into(),
        }
    }

    #[test]
    fn no_prior_does_not_suppress() {
        assert!(!should_suppress(None, &[8, 80], 1000, 1800));
    }

    #[test]
    fn equal_axes_within_ttl_suppress() {
        let prior = entry(&[8, 80], 1000);
        assert!(should_suppress(Some(&prior), &[8, 80], 2000, 1800));
    }

    #[test]
    fn improving_on_every_axis_within_ttl_suppress() {
        let prior = entry(&[8, 80], 1000);
        assert!(should_suppress(Some(&prior), &[7, 70], 2000, 1800));
    }

    #[test]
    fn strict_worsening_on_one_axis_re_fires() {
        let prior = entry(&[8, 80], 1000);
        assert!(!should_suppress(Some(&prior), &[8, 81], 2000, 1800));
    }

    #[test]
    fn beyond_ttl_re_fires_even_if_unchanged() {
        let prior = entry(&[8, 80], 1000);
        assert!(!should_suppress(Some(&prior), &[8, 80], 5000, 1800));
    }

    #[test]
    fn axis_count_change_re_fires() {
        // Schema change on the axes (e.g. v0.7 adds a third metric)
        // → conservative re-fire so the user sees the new picture.
        let prior = entry(&[8, 80], 1000);
        assert!(!should_suppress(Some(&prior), &[8, 80, 0], 2000, 1800));
    }

    #[test]
    fn sweep_drops_stale_entries() {
        let mut store = MonotonicStore::default();
        store.entries.insert("fresh".into(), entry(&[1], 10_000));
        store.entries.insert("stale".into(), entry(&[1], 0));
        // ttl=100, multiplier=30 → horizon = now - 3000. now=10_000.
        sweep(&mut store, 10_000, 100);
        assert!(store.entries.contains_key("fresh"));
        assert!(!store.entries.contains_key("stale"));
    }

    #[test]
    fn cap_lru_is_noop_below_threshold() {
        let mut store = MonotonicStore::default();
        for i in 0..10 {
            store.entries.insert(format!("k{i}"), entry(&[1], i));
        }
        cap_lru(&mut store);
        assert_eq!(store.entries.len(), 10);
    }
}
