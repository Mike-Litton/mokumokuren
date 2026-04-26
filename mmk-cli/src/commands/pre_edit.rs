//! `mmk pre-edit <PATH>` — historical context for a single file
//! before the agent edits it. Composes hotspot rank + coupling
//! lookup into the unified findings format. The `PreToolUse:Edit`
//! hook target.
//!
//! Drift findings hook in once Step 4's `compute_drift` lands; this
//! file is wired to skip drift when `args.drift_sessions == 0`.

use ahash::AHashSet;
use anyhow::{Context, Result};
use mmk_config::{Config, ConfigFile};
use mmk_core::coupling;
use mmk_core::drift::{compute_drift, Snapshot};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::args::{Format, PreEditArgs};
use crate::commands::analyze::COUPLES_PER_FILE;
use crate::output::findings::{render_text, Finding, Layer, Severity};

pub fn run<O: Write, E: Write>(args: &PreEditArgs, stdout: &mut O, stderr: &mut E) -> Result<()> {
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
    if let Some(file_br) = file_cfg.blast_radius.as_ref() {
        cfg.blast_radius.threshold = file_br.threshold;
    }
    if let Some(t) = args.blast_radius_threshold {
        cfg.blast_radius.threshold = t;
    }

    let started = Instant::now();
    let analysis = mmk_git::analyze(&cwd, &cfg)?;

    if args.verbose {
        match &file_path {
            Some(p) => writeln!(stderr, "loaded config from {}", p.display())?,
            None => writeln!(stderr, "no mokumokuren.toml found; running with defaults")?,
        }
    }

    let now_ts = analysis.head_timestamp.unwrap_or(0);
    let weighted = mmk_core::churn::weighted_churn(&analysis.commits, now_ts, cfg.tau_seconds());
    let relative = mmk_core::churn::relative_churn(&weighted, &analysis.loc);
    let commits_touching = mmk_core::churn::commits_touching(&analysis.commits);
    let last_modified = mmk_core::last_modified(&analysis.commits);
    let ranked = mmk_core::hotspot::rank(
        mmk_core::hotspot::RankInputs {
            weighted: &weighted,
            relative: &relative,
            loc: &analysis.loc,
            commits_touching: &commits_touching,
            last_modified: &last_modified,
        },
        cfg.hotspot.top_n,
    );

    let mut findings = compute_findings(&args.path, &ranked, &analysis.commits, &cfg, args.top);

    // DRIFT: only if the user opted in via --drift-sessions K. Slow
    // path (K x analyze) — kept out of the default per-edit hook.
    if args.drift_sessions > 0 {
        let walker = mmk_git::RepoWalker::open(&cwd)?;
        let boundaries = walker.find_session_boundaries(args.drift_sessions)?;
        let snapshots: Vec<Snapshot> = boundaries
            .iter()
            .map(|oid| -> Result<Snapshot> {
                let snap_analysis = mmk_git::analyze_at(&cwd, &cfg, *oid)?;
                let now_ts = snap_analysis.head_timestamp.unwrap_or(0);
                let weighted = mmk_core::churn::weighted_churn(
                    &snap_analysis.commits,
                    now_ts,
                    cfg.tau_seconds(),
                );
                let relative = mmk_core::churn::relative_churn(&weighted, &snap_analysis.loc);
                let commits_touching = mmk_core::churn::commits_touching(&snap_analysis.commits);
                let last_modified = mmk_core::last_modified(&snap_analysis.commits);
                let ranking = mmk_core::hotspot::rank(
                    mmk_core::hotspot::RankInputs {
                        weighted: &weighted,
                        relative: &relative,
                        loc: &snap_analysis.loc,
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
        for d in compute_drift(&snapshots) {
            if d.path == args.path {
                findings.push(Finding::new(
                    Layer::Drift,
                    Severity::Warn,
                    format!(
                        "{} climbed in {}/{} sessions; latest rank #{}",
                        d.path.display(),
                        d.climb_transitions,
                        d.total_transitions,
                        d.latest_rank
                    ),
                ));
            }
        }
    }

    let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);

    match args.format {
        Format::Text => render_text(stdout, &findings)?,
        Format::Json => crate::output::json::write_pre_edit(
            stdout,
            &args.path,
            &findings,
            &analysis,
            duration_ms,
            &cfg,
        )?,
    }
    Ok(())
}

fn compute_findings(
    target: &Path,
    ranked: &[mmk_core::HotspotEntry],
    commits: &[mmk_core::types::Commit],
    cfg: &Config,
    top: usize,
) -> Vec<Finding> {
    let mut findings = Vec::new();

    // HOTSPOT — surface the rank if the file is in the top-N. Same
    // threshold semantics as `mmk review`.
    if let Some(entry) = ranked.iter().find(|e| e.path == target) {
        if (entry.hotspot_rank as usize) <= top {
            findings.push(Finding::new(
                Layer::Hotspot,
                Severity::Warn,
                format!(
                    "{} ranks #{} (top-{} hotspot)",
                    target.display(),
                    entry.hotspot_rank,
                    top
                ),
            ));
        }
    }

    // COUPLING — list every partner above threshold. Pre-edit is
    // *informational* (the agent hasn't acted yet), so severity is
    // Info, not Warn. The hook reader treats it as "you should
    // probably re-read these too."
    let mut targets: AHashSet<PathBuf> = AHashSet::new();
    targets.insert(target.to_path_buf());
    let mut couples = coupling::top_couples_for(commits, &targets, COUPLES_PER_FILE);
    if let Some(partners) = couples.remove(target) {
        let threshold = cfg.blast_radius.threshold;
        for p in partners {
            if p.jaccard >= threshold {
                findings.push(Finding::new(
                    Layer::Coupling,
                    Severity::Info,
                    format!(
                        "{} historically co-changes with {} (jaccard {:.2})",
                        target.display(),
                        p.partner.display(),
                        p.jaccard
                    ),
                ));
            }
        }
    }

    findings
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
