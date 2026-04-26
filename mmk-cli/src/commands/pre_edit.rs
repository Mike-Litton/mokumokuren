//! `mmk pre-edit <PATH>` — historical context for a single file
//! before the agent edits it. Composes hotspot rank + coupling
//! lookup into the unified findings format. The `PreToolUse:Edit`
//! hook target.
//!
//! Drift findings hook in once Step 4's `compute_drift` lands; this
//! file is wired to skip drift when `args.drift_sessions == 0`.

use ahash::AHashSet;
use anyhow::{Context, Result};
use mmk_config::Config;
use mmk_core::coupling;
use mmk_core::drift::{compute_drift, Snapshot};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::args::{Format, PreEditArgs};
use crate::commands::analyze::COUPLES_PER_FILE;
use crate::commands::common::{
    analyze_health_for_subject, apply_coupling_file, apply_health_file, coupling_findings,
    health_to_finding, load_config_file, resolve_patterns, CouplingEmission, CouplingProse,
};
use crate::commands::review::{build_partner_globset, verdict_for};
use crate::output::findings::{render_text, Finding, Layer, Severity};
use crate::output::messages;
use crate::Verdict;

pub fn run<O: Write, E: Write>(
    args: &PreEditArgs,
    stdout: &mut O,
    stderr: &mut E,
) -> Result<Verdict> {
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
    if let Some(file_cp) = file_cfg.coupling.as_ref() {
        let warns = apply_coupling_file(&mut cfg.coupling, file_cp);
        if args.verbose {
            for w in &warns {
                writeln!(stderr, "{}", w.message())?;
            }
        }
    }
    if let Some(file_h) = file_cfg.health.as_ref() {
        apply_health_file(&mut cfg.health.ts, file_h);
    }
    if let Some(t) = args.coupling_threshold.or(args.blast_radius_threshold) {
        cfg.coupling.threshold = t;
        cfg.coupling.confidence_threshold = t;
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

    let mut findings = compute_findings(
        &args.path,
        &ranked,
        &analysis.commits,
        &commits_touching,
        &cfg,
        args.top,
    );

    // HEALTH: structural-pattern adapter. Pre-edit treats every
    // Health finding as informational (the agent hasn't acted yet
    // — surfaces neighbors but doesn't demand edits).
    let health_patterns = resolve_patterns(&cfg.health.ts.patterns);
    let health_matches: Vec<mmk_health::HealthFinding> = if cfg.health.ts.enabled {
        let peer_paths: Vec<PathBuf> = analysis.loc.keys().cloned().collect();
        analyze_health_for_subject(&cwd, &args.path, &peer_paths, &health_patterns)
    } else {
        Vec::new()
    };
    for h in &health_matches {
        findings.push(health_to_finding(h, Severity::Info));
    }

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
                    messages::drift(
                        &d.path,
                        d.climb_transitions,
                        d.total_transitions,
                        d.latest_rank,
                    ),
                ));
            }
        }
    }

    // No-signal fall-through: when no other layer fires, emit one
    // OK finding so the agent can distinguish "mmk was consulted but
    // had nothing to say" from "mmk wasn't run." Sits under
    // Layer::Coupling + Severity::Ok because the absence-of-coupling
    // signal is the typical trigger.
    if findings.is_empty() {
        let n = commits_touching.get(&args.path).copied().unwrap_or(0);
        let rank = ranked
            .iter()
            .find(|e| e.path == args.path)
            .map(|e| e.hotspot_rank);
        findings.push(Finding::new(
            Layer::Coupling,
            Severity::Ok,
            messages::quiet_file(&args.path, n, cfg.window.days, rank),
        ));
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
            &health_matches,
            &health_patterns,
        )?,
    }
    Ok(verdict_for(args.gate, &findings))
}

fn compute_findings(
    target: &Path,
    ranked: &[mmk_core::HotspotEntry],
    commits: &[mmk_core::types::Commit],
    commits_touching: &ahash::AHashMap<PathBuf, u32>,
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
                messages::hotspot(target, entry.hotspot_rank, top),
            ));
        }
    }

    // COUPLING — delegate to the shared helper. Quiet files
    // (n < min_sample_size) yield zero findings here and are picked
    // up by the OK-finding fall-through in `run`.
    let mut targets: AHashSet<PathBuf> = AHashSet::new();
    targets.insert(target.to_path_buf());
    let mut couples =
        coupling::compute_conditional_couples_for(commits, &targets, COUPLES_PER_FILE);
    let n = commits_touching.get(target).copied().unwrap_or(0);
    if let Some(partners) = couples.remove(target) {
        let ignore_set = build_partner_globset(&cfg.coupling.ignore_partners);
        let no_excluded: AHashSet<PathBuf> = AHashSet::new();
        findings.extend(coupling_findings(CouplingEmission {
            subject: target,
            n,
            partners: &partners,
            cfg: &cfg.coupling,
            ignore_set: ignore_set.as_ref(),
            excluded_partners: &no_excluded,
            severity: Severity::Info,
            prose: CouplingProse::PreEditExpected,
        }));
    }

    findings
}
