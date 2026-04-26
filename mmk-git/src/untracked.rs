//! List untracked files in the working tree.
//!
//! `git diff --numstat HEAD` — the numstat source for the working-tree
//! review path — reports tracked changes only. New files that the
//! agent has just created (not yet `git add`-ed) are invisible to it.
//! That makes them invisible to coupling-suppression and BUDGET
//! accounting, even though they're materially part of the diff the
//! agent is staging.
//!
//! This helper shells `git ls-files --others --exclude-standard` to
//! enumerate those files, applies the same ignore-glob filter as the
//! HEAD-tree walk, drops binaries (NUL-byte heuristic on the first
//! 8 KiB, matching `mmk-git::binary::is_binary`), and counts text
//! lines. Each entry slots into the review path as a `ChangedFile`
//! with `added = line_count` and `deleted = 0`.

use anyhow::{Context, Result};
use globset::GlobSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::binary::{count_lines, is_binary};

/// One untracked file plus its newline count. Binary files are
/// filtered out before they reach this struct (see [`list_untracked`]).
#[derive(Debug, Clone)]
pub struct UntrackedFile {
    pub path: PathBuf,
    pub line_count: u64,
}

/// Enumerate untracked-but-not-ignored files under `repo`, dropping
/// any path that matches `ignores` or whose body looks binary.
///
/// Uses `git ls-files --others --exclude-standard` so the .gitignore
/// rules already in effect for the repo are honoured natively. We
/// then layer mmk's own ignore-glob set on top so a worktree-scoped
/// `mokumokuren.toml` filter applies consistently between the
/// historical analysis and the live diff.
pub fn list_untracked(repo: &Path, ignores: &GlobSet) -> Result<Vec<UntrackedFile>> {
    let out = Command::new("git")
        .args(["ls-files", "--others", "--exclude-standard", "-z"])
        .current_dir(repo)
        .output()
        .context("failed to invoke `git ls-files` — is git on PATH?")?;
    if !out.status.success() {
        anyhow::bail!(
            "git ls-files exited with {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
    }

    let mut files = Vec::new();
    for raw in out.stdout.split(|&b| b == 0) {
        if raw.is_empty() {
            continue;
        }
        // Non-UTF-8 paths are vanishingly rare in modern repos and
        // already handled lossily across the analyzer (see
        // mmk-git/src/lib.rs's head_paths note). Skip them rather
        // than fail the review.
        let Ok(path_str) = std::str::from_utf8(raw) else {
            continue;
        };
        if !ignores.is_empty() && ignores.is_match(path_str) {
            continue;
        }
        let abs = repo.join(path_str);
        let Ok(bytes) = std::fs::read(&abs) else {
            continue;
        };
        if is_binary(&bytes) {
            continue;
        }
        files.push(UntrackedFile {
            path: PathBuf::from(path_str),
            line_count: u64::from(count_lines(&bytes)),
        });
    }
    Ok(files)
}
