//! `mmk cache info` / `mmk cache clear`.
//!
//! Both subcommands operate on the caches for the *current* Git
//! repository — the one discovered via `mmk_git::discover_work_dir`.
//! Three caches share the same per-repo directory: per-commit deltas,
//! revwalk results, and head-tree enumeration.

use anyhow::{Context, Result};
use mmk_git::cache::{
    cache_path, head_tree_cache_path, revwalk_cache_path, Cache, HeadTreeCache, RevwalkCache,
};
use std::io::Write;
use std::path::Path;

use crate::args::{CacheArgs, CacheCommand, CacheScope};

pub fn run<O: Write, E: Write>(args: &CacheArgs, stdout: &mut O, _stderr: &mut E) -> Result<()> {
    let cwd = std::env::current_dir().context("failed to determine current directory")?;
    let workdir = mmk_git::discover_work_dir(&cwd).context("not inside a Git repository")?;
    let git_dir = workdir.join(".git");
    let deltas = cache_path(&git_dir)?;
    let revwalk = revwalk_cache_path(&git_dir)?;
    let head_tree = head_tree_cache_path(&git_dir)?;

    match &args.command {
        CacheCommand::Info => {
            report_deltas(stdout, &deltas)?;
            writeln!(stdout)?;
            report_revwalk(stdout, &revwalk)?;
            writeln!(stdout)?;
            report_head_tree(stdout, &head_tree)?;
        }
        CacheCommand::Clear(clear_args) => match clear_args.scope {
            CacheScope::All => {
                clear_path(stdout, &deltas)?;
                clear_path(stdout, &revwalk)?;
                clear_path(stdout, &head_tree)?;
            }
            CacheScope::Deltas => clear_path(stdout, &deltas)?,
            CacheScope::Revwalk => clear_path(stdout, &revwalk)?,
            CacheScope::Loc => clear_path(stdout, &head_tree)?,
        },
    }
    Ok(())
}

fn report_deltas<O: Write>(stdout: &mut O, path: &Path) -> Result<()> {
    writeln!(stdout, "deltas (per-commit):")?;
    writeln!(stdout, "  file:    {}", path.display())?;
    if !path.exists() {
        writeln!(stdout, "  status:  no cache yet")?;
        return Ok(());
    }
    let meta = std::fs::metadata(path).with_context(|| format!("stat {}", path.display()))?;
    let cache = Cache::load(path)?;
    writeln!(stdout, "  size:    {} bytes", meta.len())?;
    writeln!(stdout, "  entries: {} commits", cache.entries.len())?;
    writeln!(stdout, "  schema:  v{}", cache.version)?;
    Ok(())
}

fn report_revwalk<O: Write>(stdout: &mut O, path: &Path) -> Result<()> {
    writeln!(stdout, "revwalk:")?;
    writeln!(stdout, "  file:    {}", path.display())?;
    if !path.exists() {
        writeln!(stdout, "  status:  no cache yet")?;
        return Ok(());
    }
    let meta = std::fs::metadata(path).with_context(|| format!("stat {}", path.display()))?;
    let cache = RevwalkCache::load(path)?;
    writeln!(stdout, "  size:    {} bytes", meta.len())?;
    writeln!(stdout, "  entries: {} keys", cache.entries.len())?;
    writeln!(stdout, "  schema:  v{}", cache.version)?;
    Ok(())
}

fn report_head_tree<O: Write>(stdout: &mut O, path: &Path) -> Result<()> {
    writeln!(stdout, "head-tree:")?;
    writeln!(stdout, "  file:    {}", path.display())?;
    if !path.exists() {
        writeln!(stdout, "  status:  no cache yet")?;
        return Ok(());
    }
    let meta = std::fs::metadata(path).with_context(|| format!("stat {}", path.display()))?;
    let cache = HeadTreeCache::load(path)?;
    writeln!(stdout, "  size:    {} bytes", meta.len())?;
    writeln!(stdout, "  entries: {} keys", cache.entries.len())?;
    writeln!(stdout, "  schema:  v{}", cache.version)?;
    Ok(())
}

fn clear_path<O: Write>(stdout: &mut O, path: &Path) -> Result<()> {
    if path.exists() {
        std::fs::remove_file(path).with_context(|| format!("remove {}", path.display()))?;
        writeln!(stdout, "removed {}", path.display())?;
    } else {
        writeln!(stdout, "no cache to clear ({})", path.display())?;
    }
    Ok(())
}
