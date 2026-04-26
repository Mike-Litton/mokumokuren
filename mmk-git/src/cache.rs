//! Persistent caches for the analyze pipeline.
//!
//! Three caches share the same `<cache-root>/<repo-id>/` folder and the
//! same atomic-rename save protocol. They cover different deterministic
//! phases of `analyze()`:
//!
//! - `cache.bincode.v<N>` — per-commit deltas. A commit's
//!   `(added, deleted)` deltas are immutable once the commit exists.
//!   Skipping recomputation is the round-1 win.
//! - `revwalk.bincode.v<N>` — revwalk results keyed by
//!   `(anchor_sha, since_ts)`. The set of commits reachable from a
//!   given anchor with committer-time ≥ a fixed cutoff is immutable
//!   once those commits exist. Saves ~210 ms warm on a ~140k-commit repo.
//! - `head_tree.bincode.v<N>` — HEAD/anchor tree enumeration keyed by
//!   `(commit_sha, ignores_hash)`. The set of blob entries in a tree
//!   is immutable once the tree exists; ignore globs filter
//!   deterministically. Saves the head-path-enum phase (~12 ms) +
//!   feeds the LOC-at-HEAD count without re-walking.
//!
//! Layout: `<cache-root>/<repo-id>/<filename>` where
//! - `<cache-root>` is the OS user cache dir (`~/Library/Caches/mmk` on
//!   macOS, `~/.cache/mmk` on Linux, `%LOCALAPPDATA%\mmk\cache` on
//!   Windows); override with `MMK_CACHE_DIR`.
//! - `<repo-id>` is the SHA-256 of the canonical `.git` path, so
//!   worktrees of the same repo share, and `cd` doesn't strand entries.
//! - The filename's `v<N>` suffix is bumped on shape changes; old
//!   caches at a different version are silently ignored.
//!
//! Concurrency: writes are atomic via tmp-file rename. Two concurrent
//! `mmk` invocations might race; last writer wins. Any lost entries are
//! recomputed next run — no correctness impact.
//!
//! Bounded growth: the revwalk and head-tree caches keep at most
//! `*_CACHE_MAX_ENTRIES` keys. On insert, the entry with the smallest
//! `last_used` (least-recently-inserted) is evicted. Touching a hit
//! does *not* update `last_used` — that would force a save on every
//! warm call, defeating the L2c "skip rewrite when nothing changed"
//! optimisation.

use anyhow::{Context, Result};
use mmk_core::types::{CommitInfo, FileDelta};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Per-commit delta cache version. Bumped on diff-implementation changes.
pub const CACHE_VERSION: u32 = 1;

/// Revwalk cache version. Bumped if `RevwalkCache`'s on-disk shape changes.
pub const REVWALK_CACHE_VERSION: u32 = 1;

/// Head-tree enumeration cache version. Bumped if `HeadTreeCache`'s
/// on-disk shape changes.
pub const HEAD_TREE_CACHE_VERSION: u32 = 1;

/// Soft cap on revwalk cache entries; least-recently-inserted evicted on overflow.
pub const REVWALK_CACHE_MAX_ENTRIES: usize = 32;

/// Soft cap on head-tree cache entries; same eviction rule as revwalk.
pub const HEAD_TREE_CACHE_MAX_ENTRIES: usize = 32;

/// Cached deltas + side-data for a single commit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitDeltas {
    pub deltas: Vec<FileDelta>,
    pub skipped: u64,
    pub bulk_filtered: bool,
}

/// On-disk per-commit cache. Keyed by commit SHA (full hex).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cache {
    pub version: u32,
    pub entries: HashMap<String, CommitDeltas>,
}

impl Cache {
    #[must_use]
    pub fn empty() -> Self {
        Self {
            version: CACHE_VERSION,
            entries: HashMap::new(),
        }
    }

    /// Load from `path`. Missing or version-mismatched files yield an
    /// empty cache (next save overwrites). Decode errors propagate so
    /// the caller can decide whether to nuke and continue.
    pub fn load(path: &Path) -> Result<Self> {
        match read_bincode::<Self>(path)? {
            Some(cache) if cache.version == CACHE_VERSION => Ok(cache),
            _ => Ok(Self::empty()),
        }
    }

    /// Atomic save: write to `<path>.tmp`, then rename. The rename is
    /// atomic on POSIX and on Windows (since 10 1607). If two writers
    /// race, last-rename wins; any entries the loser added are lost
    /// and will be recomputed on the next run.
    pub fn save(&self, path: &Path) -> Result<()> {
        write_bincode(self, path)
    }
}

/// Revwalk cache key: anchor commit + window cutoff. The walk output
/// is determined by these two values plus the immutable commit graph
/// reachable from the anchor.
#[derive(Debug, Clone, Hash, Eq, PartialEq, Serialize, Deserialize)]
pub struct RevwalkKey {
    pub anchor_sha: String,
    pub since_ts: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RevwalkEntry {
    pub commits: Vec<CommitInfo>,
    /// Wall-clock seconds-since-epoch when this entry was inserted.
    /// Used for least-recently-inserted eviction; not updated on hit
    /// (touching on hit would force a save every warm call).
    pub last_used: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RevwalkCache {
    pub version: u32,
    pub entries: HashMap<RevwalkKey, RevwalkEntry>,
}

impl RevwalkCache {
    #[must_use]
    pub fn empty() -> Self {
        Self {
            version: REVWALK_CACHE_VERSION,
            entries: HashMap::new(),
        }
    }

    pub fn load(path: &Path) -> Result<Self> {
        match read_bincode::<Self>(path)? {
            Some(cache) if cache.version == REVWALK_CACHE_VERSION => Ok(cache),
            _ => Ok(Self::empty()),
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        write_bincode(self, path)
    }

    /// Insert and prune to `REVWALK_CACHE_MAX_ENTRIES` by evicting the
    /// smallest `last_used` first. Stamps the new entry with the
    /// current wall-clock second.
    pub fn insert(&mut self, key: RevwalkKey, commits: Vec<CommitInfo>) {
        self.entries.insert(
            key,
            RevwalkEntry {
                commits,
                last_used: now_secs(),
            },
        );
        prune_lri(&mut self.entries, REVWALK_CACHE_MAX_ENTRIES);
    }
}

/// Head-tree cache key.
///
/// Combines the commit OID with a hash of the sorted ignore globs.
/// Different ignore configs partition into distinct entries, so a
/// worktree-scoped `mokumokuren.toml` doesn't collide with a global one.
#[derive(Debug, Clone, Hash, Eq, PartialEq, Serialize, Deserialize)]
pub struct HeadTreeKey {
    pub commit_sha: String,
    pub ignores_hash: String,
}

/// One blob entry from a tree walk. The oid is stored as a hex string
/// (the same form `gix::ObjectId::to_string` and `from_hex` already use
/// elsewhere in this crate); avoids enabling gix's `serde` feature.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedTreeEntry {
    pub path: PathBuf,
    pub oid_hex: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeadTreeEntry {
    pub entries: Vec<CachedTreeEntry>,
    pub head_paths_ignored: u64,
    pub last_used: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeadTreeCache {
    pub version: u32,
    pub entries: HashMap<HeadTreeKey, HeadTreeEntry>,
}

impl HeadTreeCache {
    #[must_use]
    pub fn empty() -> Self {
        Self {
            version: HEAD_TREE_CACHE_VERSION,
            entries: HashMap::new(),
        }
    }

    pub fn load(path: &Path) -> Result<Self> {
        match read_bincode::<Self>(path)? {
            Some(cache) if cache.version == HEAD_TREE_CACHE_VERSION => Ok(cache),
            _ => Ok(Self::empty()),
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        write_bincode(self, path)
    }

    pub fn insert(
        &mut self,
        key: HeadTreeKey,
        entries: Vec<CachedTreeEntry>,
        head_paths_ignored: u64,
    ) {
        self.entries.insert(
            key,
            HeadTreeEntry {
                entries,
                head_paths_ignored,
                last_used: now_secs(),
            },
        );
        prune_lri(&mut self.entries, HEAD_TREE_CACHE_MAX_ENTRIES);
    }
}

/// Per-repo cache directory. Worktrees of the same repo (same canonical
/// `.git` path) share this directory.
pub fn repo_cache_dir(git_dir: &Path) -> Result<PathBuf> {
    let root = cache_root()?;
    let canon = git_dir
        .canonicalize()
        .unwrap_or_else(|_| git_dir.to_path_buf());
    let mut hasher = Sha256::new();
    hasher.update(canon.as_os_str().as_encoded_bytes());
    let repo_id = format!("{:x}", hasher.finalize());
    Ok(root.join(repo_id))
}

/// Resolve the per-commit cache file path.
pub fn cache_path(git_dir: &Path) -> Result<PathBuf> {
    Ok(repo_cache_dir(git_dir)?.join(format!("cache.bincode.v{CACHE_VERSION}")))
}

/// Resolve the revwalk cache file path.
pub fn revwalk_cache_path(git_dir: &Path) -> Result<PathBuf> {
    Ok(repo_cache_dir(git_dir)?.join(format!("revwalk.bincode.v{REVWALK_CACHE_VERSION}")))
}

/// Resolve the head-tree cache file path.
pub fn head_tree_cache_path(git_dir: &Path) -> Result<PathBuf> {
    Ok(repo_cache_dir(git_dir)?.join(format!("head_tree.bincode.v{HEAD_TREE_CACHE_VERSION}")))
}

/// Stable hash of the ignore-glob set. Sorting first means
/// `["a", "b"]` and `["b", "a"]` produce the same key.
#[must_use]
pub fn ignores_hash(ignores: &[String]) -> String {
    let mut sorted: Vec<&str> = ignores.iter().map(String::as_str).collect();
    sorted.sort_unstable();
    let mut hasher = Sha256::new();
    for s in &sorted {
        hasher.update(s.as_bytes());
        hasher.update(b"\0");
    }
    format!("{:x}", hasher.finalize())
}

fn cache_root() -> Result<PathBuf> {
    if let Some(dir) = std::env::var_os("MMK_CACHE_DIR") {
        return Ok(PathBuf::from(dir));
    }
    let dirs = directories::ProjectDirs::from("", "", "mmk")
        .context("OS reports no cache directory; set MMK_CACHE_DIR")?;
    Ok(dirs.cache_dir().to_path_buf())
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

fn prune_lri<K, V>(map: &mut HashMap<K, V>, max: usize)
where
    K: std::hash::Hash + Eq + Clone,
    V: HasLastUsed,
{
    while map.len() > max {
        let Some(victim) = map
            .iter()
            .min_by_key(|(_, v)| v.last_used())
            .map(|(k, _)| k.clone())
        else {
            break;
        };
        map.remove(&victim);
    }
}

trait HasLastUsed {
    fn last_used(&self) -> i64;
}

impl HasLastUsed for RevwalkEntry {
    fn last_used(&self) -> i64 {
        self.last_used
    }
}

impl HasLastUsed for HeadTreeEntry {
    fn last_used(&self) -> i64 {
        self.last_used
    }
}

fn read_bincode<T: serde::de::DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    if !path.exists() {
        return Ok(None);
    }
    let bytes = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
    let (value, _) = bincode::serde::decode_from_slice::<T, _>(&bytes, bincode::config::standard())
        .context("decode cache")?;
    Ok(Some(value))
}

fn write_bincode<T: serde::Serialize>(value: &T, path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("mkdir {}", parent.display()))?;
    }
    let bytes = bincode::serde::encode_to_vec(value, bincode::config::standard())
        .context("encode cache")?;
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, &bytes).with_context(|| format!("write {}", tmp.display()))?;
    std::fs::rename(&tmp, path)
        .with_context(|| format!("rename {} -> {}", tmp.display(), path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn delta(p: &str, a: u32, d: u32) -> FileDelta {
        FileDelta {
            path: PathBuf::from(p),
            added: a,
            deleted: d,
        }
    }

    #[test]
    fn save_then_load_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("cache.bincode.v1");
        let mut cache = Cache::empty();
        cache.entries.insert(
            "abc".into(),
            CommitDeltas {
                deltas: vec![delta("foo.rs", 10, 2)],
                skipped: 0,
                bulk_filtered: false,
            },
        );
        cache.save(&path).unwrap();
        let loaded = Cache::load(&path).unwrap();
        assert_eq!(loaded.version, CACHE_VERSION);
        assert_eq!(loaded.entries.len(), 1);
        assert_eq!(loaded.entries["abc"].deltas[0].added, 10);
    }

    #[test]
    fn missing_file_is_empty_cache() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("missing.bin");
        let loaded = Cache::load(&path).unwrap();
        assert!(loaded.entries.is_empty());
    }

    #[test]
    fn version_mismatch_yields_empty() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("cache.bincode.v1");
        let bad = Cache {
            version: CACHE_VERSION + 1,
            entries: std::iter::once((
                "x".to_string(),
                CommitDeltas {
                    deltas: vec![],
                    skipped: 0,
                    bulk_filtered: false,
                },
            ))
            .collect(),
        };
        bad.save(&path).unwrap();
        let loaded = Cache::load(&path).unwrap();
        assert!(loaded.entries.is_empty());
    }

    #[test]
    fn revwalk_roundtrip_and_lri() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("revwalk.bincode.v1");
        let mut cache = RevwalkCache::empty();
        for i in 0..(REVWALK_CACHE_MAX_ENTRIES + 5) {
            cache.insert(
                RevwalkKey {
                    anchor_sha: format!("{i:040x}"),
                    since_ts: 0,
                },
                vec![CommitInfo {
                    sha: format!("{i:040x}"),
                    parent_sha: None,
                    timestamp: i as i64,
                    author_email: String::new(),
                }],
            );
        }
        assert_eq!(cache.entries.len(), REVWALK_CACHE_MAX_ENTRIES);
        cache.save(&path).unwrap();
        let loaded = RevwalkCache::load(&path).unwrap();
        assert_eq!(loaded.entries.len(), REVWALK_CACHE_MAX_ENTRIES);
    }

    #[test]
    fn head_tree_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("head_tree.bincode.v1");
        let mut cache = HeadTreeCache::empty();
        cache.insert(
            HeadTreeKey {
                commit_sha: "deadbeef".into(),
                ignores_hash: "abc".into(),
            },
            vec![CachedTreeEntry {
                path: PathBuf::from("src/main.rs"),
                oid_hex: "0123456789abcdef0123456789abcdef01234567".into(),
            }],
            7,
        );
        cache.save(&path).unwrap();
        let loaded = HeadTreeCache::load(&path).unwrap();
        let key = HeadTreeKey {
            commit_sha: "deadbeef".into(),
            ignores_hash: "abc".into(),
        };
        let entry = &loaded.entries[&key];
        assert_eq!(entry.head_paths_ignored, 7);
        assert_eq!(entry.entries.len(), 1);
        assert_eq!(
            entry.entries[0].oid_hex,
            "0123456789abcdef0123456789abcdef01234567"
        );
    }

    #[test]
    fn ignores_hash_is_order_insensitive() {
        let a = ignores_hash(&["**/*.lock".into(), "node_modules/**".into()]);
        let b = ignores_hash(&["node_modules/**".into(), "**/*.lock".into()]);
        assert_eq!(a, b);
        let c = ignores_hash(&["node_modules/**".into()]);
        assert_ne!(a, c);
    }
}
