//! `mmk drift` — historical-pattern drift across recent sessions.
//!
//! Re-runs `analyze_at` at K session-boundary commits, builds K
//! ranking snapshots, then asks `mmk_core::drift::compute_drift`
//! which paths climbed in a majority of K-1 transitions.
//!
//! Slow path on purpose: K × analyze cost. Use end-of-session or in
//! PR review, not in the per-edit hook.

use anyhow::{Context, Result};
use mmk_config::{Config, ConfigFile};
use mmk_core::drift::{compute_drift, DriftFinding, Snapshot};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::args::{DriftArgs, Format};
use crate::output::findings::{render_text, Finding, Layer, Severity};
use crate::output::messages;

pub fn run<O: Write, E: Write>(args: &DriftArgs, stdout: &mut O, stderr: &mut E) -> Result<()> {
    let cwd = std::env::current_dir().context("failed to determine current directory")?;

    let window = humantime::parse_duration(&args.since)
        .with_context(|| format!("invalid --since value: {}", args.since))?;
    let window_days = u32::try_from(window.as_secs() / 86_400)
        .unwrap_or(u32::MAX)
        .max(1);

    let (file_cfg, file_path) = load_config_file(&cwd, args.config.as_deref())?;

    let mut cfg = Config::default();
    cfg.window.days = window_days;
    cfg.hotspot.top_n = args.top;
    cfg.ignores = file_cfg.ignore;
    cfg.ignores.extend(args.ignores.iter().cloned());

    if args.verbose {
        match &file_path {
            Some(p) => writeln!(stderr, "loaded config from {}", p.display())?,
            None => writeln!(stderr, "no mokumokuren.toml found; running with defaults")?,
        }
    }

    let started = Instant::now();
    let walker = mmk_git::RepoWalker::open(&cwd)?;
    let boundaries = walker.find_session_boundaries(args.sessions)?;

    let snapshots: Vec<Snapshot> = boundaries
        .iter()
        .map(|oid| -> Result<Snapshot> {
            let analysis = mmk_git::analyze_at(&cwd, &cfg, *oid)?;
            let now_ts = analysis.head_timestamp.unwrap_or(0);
            let weighted =
                mmk_core::churn::weighted_churn(&analysis.commits, now_ts, cfg.tau_seconds());
            let relative = mmk_core::churn::relative_churn(&weighted, &analysis.loc);
            let commits_touching = mmk_core::churn::commits_touching(&analysis.commits);
            let last_modified = mmk_core::last_modified(&analysis.commits);
            let ranking = mmk_core::hotspot::rank(
                mmk_core::hotspot::RankInputs {
                    weighted: &weighted,
                    relative: &relative,
                    loc: &analysis.loc,
                    commits_touching: &commits_touching,
                    last_modified: &last_modified,
                },
                cfg.hotspot.top_n,
            );
            Ok(Snapshot {
                label: oid.to_string(),
                ranking,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    let drift_findings = compute_drift(&snapshots);
    let findings: Vec<Finding> = drift_findings.iter().map(drift_to_finding).collect();

    let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);

    match args.format {
        Format::Text => render_text(stdout, &findings)?,
        Format::Json => crate::output::json::write_drift(
            stdout,
            args.base.as_deref(),
            args.sessions,
            &snapshots,
            &drift_findings,
            duration_ms,
        )?,
    }
    Ok(())
}

fn drift_to_finding(d: &DriftFinding) -> Finding {
    Finding::new(
        Layer::Drift,
        Severity::Warn,
        messages::drift(
            &d.path,
            d.climb_transitions,
            d.total_transitions,
            d.latest_rank,
        ),
    )
}

fn load_config_file(cwd: &Path, explicit: Option<&Path>) -> Result<(ConfigFile, Option<PathBuf>)> {
    if let Some(path) = explicit {
        let cfg = ConfigFile::load_from_path(path)
            .with_context(|| format!("failed to load config from {}", path.display()))?;
        return Ok((cfg, Some(path.to_path_buf())));
    }
    if let Some(repo_root) = mmk_git::discover_work_dir(cwd) {
        let candidate = repo_root.join("mokumokuren.toml");
        if candidate.exists() {
            let cfg = ConfigFile::load_from_path(&candidate)
                .with_context(|| format!("failed to load config from {}", candidate.display()))?;
            return Ok((cfg, Some(candidate)));
        }
    }
    Ok((ConfigFile::default(), None))
}
