use anyhow::{Context, Result};
use mmk_config::{Config, ConfigFile};
use std::io::Write;
use std::path::PathBuf;
use std::time::Instant;

use crate::args::{AnalyzeArgs, Format};

pub fn run<O: Write, E: Write>(args: &AnalyzeArgs, stdout: &mut O, stderr: &mut E) -> Result<()> {
    let cwd = std::env::current_dir().context("failed to determine current directory")?;

    let window = humantime::parse_duration(&args.since)
        .with_context(|| format!("invalid --since value: {}", args.since))?;
    let window_days = u32::try_from(window.as_secs() / 86_400)
        .unwrap_or(u32::MAX)
        .max(1);

    // Load mokumokuren.toml — explicit `--config` wins; otherwise look at
    // the Git repo root. Discovery uses gix to find the work tree, since
    // `cwd` may be deep inside a subdirectory.
    let (file_cfg, file_path) = load_config_file(&cwd, args.config.as_deref())?;

    let mut cfg = Config::default();
    cfg.window.days = window_days;
    cfg.hotspot.top_n = args.top;
    // Union: file first (so CLI flags appear after — order matters only
    // for diagnostics, not glob semantics).
    cfg.ignores = file_cfg.ignore;
    cfg.ignores.extend(args.ignores.iter().cloned());

    let started = Instant::now();
    let analysis = mmk_git::analyze(&cwd, &cfg)?;

    if args.verbose {
        match &file_path {
            Some(p) => writeln!(stderr, "loaded config from {}", p.display())?,
            None => writeln!(stderr, "no mokumokuren.toml found; running with defaults")?,
        }
        if analysis.counts.head_paths_ignored > 0 {
            writeln!(
                stderr,
                "{} HEAD path(s) excluded by ignore globs",
                analysis.counts.head_paths_ignored
            )?;
        }
        for warn in &analysis.warnings {
            writeln!(stderr, "warning: {warn}")?;
        }
    }

    let now_ts = analysis.head_timestamp.unwrap_or_else(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    });

    let weighted = mmk_core::churn::weighted_churn(&analysis.commits, now_ts, cfg.tau_seconds());
    let relative = mmk_core::churn::relative_churn(&weighted, &analysis.loc);
    let commits_touching = mmk_core::churn::commits_touching(&analysis.commits);
    let last_modified = mmk_core::last_modified(&analysis.commits);
    let ranked = mmk_core::hotspot::rank(
        &weighted,
        &relative,
        &analysis.loc,
        &commits_touching,
        &last_modified,
        cfg.hotspot.top_n,
    );

    let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);

    match args.format {
        Format::Text => crate::output::text::write(stdout, &ranked, &analysis, duration_ms, &cfg)?,
        Format::Json => crate::output::json::write(stdout, &ranked, &analysis, duration_ms, &cfg)?,
    }
    Ok(())
}

/// Resolve which config file to load and load it. Precedence:
/// 1. Explicit `--config <path>` (must exist; error if missing).
/// 2. `mokumokuren.toml` at the Git repo root (auto-discovered).
/// 3. Default-empty.
fn load_config_file(
    cwd: &std::path::Path,
    explicit: Option<&std::path::Path>,
) -> Result<(ConfigFile, Option<PathBuf>)> {
    if let Some(path) = explicit {
        let cfg = ConfigFile::load_from_path(path)
            .with_context(|| format!("failed to load config from {}", path.display()))?;
        return Ok((cfg, Some(path.to_path_buf())));
    }
    if let Some(repo_root) = discover_repo_root(cwd) {
        let candidate = repo_root.join("mokumokuren.toml");
        if candidate.exists() {
            let cfg = ConfigFile::load_from_path(&candidate)
                .with_context(|| format!("failed to load config from {}", candidate.display()))?;
            return Ok((cfg, Some(candidate)));
        }
    }
    Ok((ConfigFile::default(), None))
}

/// Walk up from `start` to find the Git work-tree root. Returns `None`
/// if not in a Git repo (the analyze step will then fail with a clearer
/// error than a missing config).
fn discover_repo_root(start: &std::path::Path) -> Option<PathBuf> {
    mmk_git::discover_work_dir(start)
}
