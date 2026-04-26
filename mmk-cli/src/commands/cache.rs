//! `mmk cache info` / `mmk cache clear`.
//!
//! Both subcommands operate on the cache for the *current* Git
//! repository — the one discovered via `mmk_git::discover_work_dir`.

use anyhow::{Context, Result};
use mmk_git::cache::{cache_path, Cache};
use std::io::Write;

use crate::args::{CacheArgs, CacheCommand};

pub fn run<O: Write, E: Write>(args: &CacheArgs, stdout: &mut O, _stderr: &mut E) -> Result<()> {
    let cwd = std::env::current_dir().context("failed to determine current directory")?;
    let workdir = mmk_git::discover_work_dir(&cwd).context("not inside a Git repository")?;
    let git_dir = workdir.join(".git");
    let path = cache_path(&git_dir)?;

    match args.command {
        CacheCommand::Info => {
            writeln!(stdout, "cache file: {}", path.display())?;
            if !path.exists() {
                writeln!(
                    stdout,
                    "status:     no cache yet — `mmk analyze` will create one"
                )?;
                return Ok(());
            }
            let meta =
                std::fs::metadata(&path).with_context(|| format!("stat {}", path.display()))?;
            let cache = Cache::load(&path)?;
            writeln!(stdout, "size:       {} bytes", meta.len())?;
            writeln!(stdout, "entries:    {} commits", cache.entries.len())?;
            writeln!(stdout, "schema:     v{}", cache.version)?;
        }
        CacheCommand::Clear => {
            if path.exists() {
                std::fs::remove_file(&path)
                    .with_context(|| format!("remove {}", path.display()))?;
                writeln!(stdout, "removed {}", path.display())?;
            } else {
                writeln!(stdout, "no cache to clear ({})", path.display())?;
            }
        }
    }
    Ok(())
}
