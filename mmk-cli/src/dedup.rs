//! Per-fire dedup for `mmk review` and `mmk pre-edit`.
//!
//! Both commands are wired as `Pre/PostToolUse:Edit` hooks. A real
//! agent session can fire them dozens of times — and most of those
//! fires re-emit the *same* findings against the *same* HEAD baseline
//! because the agent's local edits did not move the inputs the
//! analyzer reads. Re-injecting identical findings into the agent's
//! context is the §7.4 "success fires verbosely" anti-feature.
//!
//! Dedup suppresses the second fire iff three independent boundaries
//! all match the first:
//!   1. **same findings** — hash of the sorted finding set;
//!   2. **same HEAD** — a fresh commit means a new baseline;
//!   3. **same active session** — TTL window via wall clock.
//!
//! Any one of those changing flushes the suppression. The agent gets
//! the new picture, never a partial delta.
//!
//! Storage is a single-record bincode file alongside the existing
//! per-repo caches in `<cache-root>/<repo-id>/dedup.bincode.v1`.
//! Honours `MMK_CACHE_DIR`. Best-effort: corruption / missing /
//! decode error → treat as no prior, emit. Failed writes log to
//! stderr; they never block the hook.

use serde::{Deserialize, Serialize};
use std::hash::{BuildHasher, Hasher};
use std::path::{Path, PathBuf};

use crate::output::findings::Finding;

/// Per-commit cache version. Bumped on shape changes.
pub const DEDUP_VERSION: u32 = 1;

/// Default TTL — long enough for one active task, short enough that
/// idle gaps reset cleanly. Override via `MMK_DEDUP_TTL_SECONDS`.
pub const DEFAULT_TTL_SECONDS: i64 = 1_800;

/// Persisted dedup state.
///
/// Single record per repo, latest-write-wins. Storing `head_sha` as
/// `String` keeps the file format independent of gix's hash type.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DedupRecord {
    pub findings_hash: u64,
    pub head_sha: String,
    pub emitted_at: i64,
}

/// Pure decision: should this fire be suppressed?
///
/// Returns `true` only if all three boundaries match the prior
/// record:
///   - `current_hash == prior.findings_hash`
///   - `current_head_sha == prior.head_sha`
///   - `(now_unix - prior.emitted_at) < ttl_seconds`
///
/// The TTL boundary is exclusive: at exactly `ttl_seconds` elapsed
/// we re-emit, which avoids off-by-one ambiguity at the edge.
#[must_use]
pub fn should_suppress(
    current_hash: u64,
    current_head_sha: &str,
    prior: Option<&DedupRecord>,
    now_unix: i64,
    ttl_seconds: i64,
) -> bool {
    let Some(prior) = prior else { return false };
    if prior.findings_hash != current_hash {
        return false;
    }
    if prior.head_sha != current_head_sha {
        return false;
    }
    let elapsed = now_unix.saturating_sub(prior.emitted_at);
    elapsed < ttl_seconds
}

/// Hash a slice of findings for dedup comparison.
///
/// Sort by `(layer, severity, message)` first so the hash is
/// orientation-independent. Whole-set means a finding gone or a
/// finding added invalidates suppression — the agent gets the new
/// picture, not a delta.
#[must_use]
pub fn hash_findings(findings: &[Finding]) -> u64 {
    let mut keyed: Vec<(String, String, &str)> = findings
        .iter()
        .map(|f| {
            (
                format!("{:?}", f.layer),
                format!("{:?}", f.severity),
                f.message.as_str(),
            )
        })
        .collect();
    keyed.sort();
    let hasher_builder = ahash::RandomState::with_seeds(0, 0, 0, 0);
    let mut h = hasher_builder.build_hasher();
    for (layer, severity, message) in &keyed {
        h.write(layer.as_bytes());
        h.write_u8(0);
        h.write(severity.as_bytes());
        h.write_u8(0);
        h.write(message.as_bytes());
        h.write_u8(0);
    }
    h.finish()
}

/// Best-effort load of the prior record. Any error → `None`.
#[must_use]
pub fn load_record(path: &Path) -> Option<DedupRecord> {
    if !path.exists() {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    let (record, _) = bincode::serde::decode_from_slice::<PersistedRecord, _>(
        &bytes,
        bincode::config::standard(),
    )
    .ok()?;
    if record.version != DEDUP_VERSION {
        return None;
    }
    Some(DedupRecord {
        findings_hash: record.findings_hash,
        head_sha: record.head_sha,
        emitted_at: record.emitted_at,
    })
}

/// Best-effort write of the new record.
///
/// Any I/O error is logged to stderr and swallowed — dedup is an
/// optimisation, not a safety invariant; failing the hook on a disk
/// hiccup would be worse than re-emitting the same finding next time.
pub fn record_emission(path: &Path, record: &DedupRecord) {
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            eprintln!("[mmk] dedup: mkdir {} failed: {e}", parent.display());
            return;
        }
    }
    let payload = PersistedRecord {
        version: DEDUP_VERSION,
        findings_hash: record.findings_hash,
        head_sha: record.head_sha.clone(),
        emitted_at: record.emitted_at,
    };
    let Ok(bytes) = bincode::serde::encode_to_vec(&payload, bincode::config::standard()) else {
        eprintln!("[mmk] dedup: encode failed");
        return;
    };
    let tmp = path.with_extension("tmp");
    if let Err(e) = std::fs::write(&tmp, &bytes) {
        eprintln!("[mmk] dedup: write {} failed: {e}", tmp.display());
        return;
    }
    if let Err(e) = std::fs::rename(&tmp, path) {
        eprintln!("[mmk] dedup: rename failed: {e}");
    }
}

/// Resolve the dedup record path for `git_dir`. Sits alongside the
/// existing per-commit / revwalk / head-tree caches.
#[must_use]
pub fn dedup_path(git_dir: &Path) -> Option<PathBuf> {
    mmk_git::cache::repo_cache_dir(git_dir)
        .ok()
        .map(|d| d.join(format!("dedup.bincode.v{DEDUP_VERSION}")))
}

/// Read TTL from the environment, falling back to the default. A
/// non-parseable or non-positive value falls back to the default —
/// silent recovery beats failing the hook on a typo.
#[must_use]
pub fn ttl_seconds() -> i64 {
    std::env::var("MMK_DEDUP_TTL_SECONDS")
        .ok()
        .and_then(|s| s.parse::<i64>().ok())
        .filter(|t| *t > 0)
        .unwrap_or(DEFAULT_TTL_SECONDS)
}

/// Wall-clock seconds since UNIX epoch. Saturates to 0 on the
/// (impossible) case where the system clock predates the epoch.
#[must_use]
pub fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

#[derive(Debug, Serialize, Deserialize)]
struct PersistedRecord {
    version: u32,
    findings_hash: u64,
    head_sha: String,
    emitted_at: i64,
}
