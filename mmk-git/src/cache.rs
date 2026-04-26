//! Persistent per-commit delta cache.
//!
//! A commit's `(added, deleted)` deltas are immutable once the commit
//! exists. Caching them eliminates the gix LCS work on calls 2-N for
//! the same repo + window, which is the hot-path for LLM-agent
//! workflows that call `mmk` repeatedly during a session.
//!
//! Layout: `<cache-root>/<repo-id>/cache.bincode.v<N>` where
//! - `<cache-root>` is the OS user cache dir (`~/Library/Caches/mmk` on
//!   macOS, `~/.cache/mmk` on Linux, `%LOCALAPPDATA%\mmk\cache` on
//!   Windows); override with `MMK_CACHE_DIR`.
//! - `<repo-id>` is the SHA-256 of the canonical `.git` path, so
//!   worktrees of the same repo share, and `cd` doesn't strand entries.
//! - `v<N>` is bumped on diff-implementation changes; old caches are
//!   silently ignored.
//!
//! Concurrency: writes are atomic via tmp-file rename. Two concurrent
//! `mmk` invocations might race; last writer wins. Any lost entries are
//! recomputed next run — no correctness impact.

use anyhow::{Context, Result};
use mmk_core::types::FileDelta;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Bumped on diff-implementation changes. Old caches with a different
/// version are read as empty and rewritten.
pub const CACHE_VERSION: u32 = 1;

/// Cached deltas + side-data for a single commit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitDeltas {
    pub deltas: Vec<FileDelta>,
    pub skipped: u64,
    pub bulk_filtered: bool,
}

/// On-disk cache. Keyed by commit SHA (full hex).
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
        if !path.exists() {
            return Ok(Self::empty());
        }
        let bytes = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
        let (cache, _) =
            bincode::serde::decode_from_slice::<Self, _>(&bytes, bincode::config::standard())
                .context("decode cache")?;
        if cache.version != CACHE_VERSION {
            return Ok(Self::empty());
        }
        Ok(cache)
    }

    /// Atomic save: write to `<path>.tmp`, then rename. The rename is
    /// atomic on POSIX and on Windows (since 10 1607). If two writers
    /// race, last-rename wins; any entries the loser added are lost
    /// and will be recomputed on the next run.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("mkdir {}", parent.display()))?;
        }
        let bytes = bincode::serde::encode_to_vec(self, bincode::config::standard())
            .context("encode cache")?;
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, &bytes).with_context(|| format!("write {}", tmp.display()))?;
        std::fs::rename(&tmp, path)
            .with_context(|| format!("rename {} -> {}", tmp.display(), path.display()))?;
        Ok(())
    }
}

/// Resolve the cache file path for the repo whose `.git` directory is
/// at `git_dir`. Honours `MMK_CACHE_DIR` env override.
pub fn cache_path(git_dir: &Path) -> Result<PathBuf> {
    let root = cache_root()?;
    let canon = git_dir
        .canonicalize()
        .unwrap_or_else(|_| git_dir.to_path_buf());
    let mut hasher = Sha256::new();
    hasher.update(canon.as_os_str().as_encoded_bytes());
    let repo_id = format!("{:x}", hasher.finalize());
    Ok(root
        .join(repo_id)
        .join(format!("cache.bincode.v{CACHE_VERSION}")))
}

fn cache_root() -> Result<PathBuf> {
    if let Some(dir) = std::env::var_os("MMK_CACHE_DIR") {
        return Ok(PathBuf::from(dir));
    }
    let dirs = directories::ProjectDirs::from("", "", "mmk")
        .context("OS reports no cache directory; set MMK_CACHE_DIR")?;
    Ok(dirs.cache_dir().to_path_buf())
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
        // Hand-write a cache with a wrong version.
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
}
