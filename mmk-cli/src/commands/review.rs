//! `mmk review` — compare a diff against the historical baseline and
//! emit layer-labeled findings.
//!
//! Default mode is *working tree vs HEAD*: the agent edit-loop hot
//! path. `--staged` reads the index; `--range A..B` and
//! `--commit <SHA>` review committed work without going through
//! session-summary.
//!
//! Findings are HOTSPOT (changed file is in top-N), COUPLING
//! (changed file's expected partner is not also touched), and BUDGET
//! (diff exceeds `bulk.max_files` or `bulk.max_lines`).

use ahash::AHashSet;
use anyhow::{Context, Result};
use globset::{Glob, GlobSet, GlobSetBuilder};
use mmk_config::Config;
use mmk_core::coupling;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use crate::args::{Format, Gate, ReviewArgs};
use crate::commands::analyze::COUPLES_PER_FILE;
use crate::commands::common::{
    analyze_health_for_subject, apply_coupling_file, apply_health_file, coupling_findings,
    health_severity_for_review, health_to_finding, load_config_file, resolve_patterns,
    CouplingEmission, CouplingProse,
};
use crate::output::findings::{render_text, Finding, Layer, Severity};
use crate::output::messages;
use crate::Verdict;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReviewMode {
    WorkingTree,
    Staged,
    Range,
    Commit,
}

impl ReviewMode {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::WorkingTree => "working_tree",
            Self::Staged => "staged",
            Self::Range => "range",
            Self::Commit => "commit",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ChangedFile {
    pub(crate) path: PathBuf,
    pub(crate) added: u64,
    pub(crate) deleted: u64,
}

pub fn run<O: Write, E: Write>(
    args: &ReviewArgs,
    stdout: &mut O,
    stderr: &mut E,
) -> Result<Verdict> {
    let cwd = std::env::current_dir().context("failed to determine current directory")?;

    let mode = if args.staged {
        ReviewMode::Staged
    } else if args.range.is_some() {
        ReviewMode::Range
    } else if args.commit.is_some() {
        ReviewMode::Commit
    } else {
        ReviewMode::WorkingTree
    };

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

    let changed = collect_diff(&cwd, mode, args, &cfg.ignores)?;

    // Clean tree / no-op range: text mode says nothing, JSON mode
    // emits the envelope with empty findings so harnesses can still
    // parse a stable shape.
    if changed.is_empty() {
        emit_empty(args, mode, stdout)?;
        return Ok(Verdict::Ok);
    }

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
    if let Some(file_b) = file_cfg.bulk.as_ref() {
        if let Some(t) = file_b.greenfield_threshold {
            cfg.bulk.greenfield_threshold = t;
        }
    }
    // Explicit --coupling-threshold wins; fall back to the
    // (deprecated) --blast-radius-threshold so existing CLI
    // invocations keep working until users migrate. Both are routed
    // to confidence_threshold — the active gate.
    if let Some(t) = args.coupling_threshold.or(args.blast_radius_threshold) {
        cfg.coupling.threshold = t;
        cfg.coupling.confidence_threshold = t;
    }

    let started = Instant::now();

    // §1c bulk self-filter: if the input diff itself trips the bulk
    // thresholds, this is a sweep / vendored snapshot — emit one
    // BUDGET finding and skip the (expensive, noisy) HOTSPOT/COUPLING
    // analysis. Mirrors how the analyzer drops bulk commits from
    // history.
    let files_n = u32::try_from(changed.len()).unwrap_or(u32::MAX);
    let lines_n: u64 = changed.iter().map(|c| c.added + c.deleted).sum();
    if files_n > cfg.bulk.max_files || lines_n > u64::from(cfg.bulk.max_lines) {
        let findings = bulk_self_findings(files_n, lines_n, &cfg);
        return emit_bulk(args, mode, &changed, &findings, started, stdout);
    }

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
        &changed,
        &ranked,
        &analysis.commits,
        &commits_touching,
        &cfg,
        args.top,
    );

    // HEALTH: structural-pattern adapter. Pattern C is **Warn**
    // when the implementation moved without its test partner; A/B
    // stay Info. The existing peer-touched filter for COUPLING
    // applies analogously: if the test partner is in the changed
    // set, suppress the Warn (the agent did touch it).
    let health_patterns = resolve_patterns(&cfg.health.ts.patterns);
    let mut health_matches: Vec<mmk_health::HealthFinding> = Vec::new();
    if cfg.health.ts.enabled {
        let peer_paths: Vec<PathBuf> = analysis.loc.keys().cloned().collect();
        let changed_set: AHashSet<PathBuf> = changed.iter().map(|c| c.path.clone()).collect();
        for c in &changed {
            for h in analyze_health_for_subject(&cwd, &c.path, &peer_paths, &health_patterns) {
                if h.pattern == mmk_health::HealthPattern::TestPair
                    && h.related.iter().all(|p| changed_set.contains(p))
                {
                    // Test partner *was* touched in this diff —
                    // suppress, same shape as COUPLING's "partner
                    // also touched" filter.
                    continue;
                }
                let severity = health_severity_for_review(h.pattern);
                findings.push(health_to_finding(&h, severity));
                health_matches.push(h);
            }
        }
    }

    // GREENFIELD signal: when most of the diff is paths the historical
    // analyzer hasn't seen, the HOTSPOT/COUPLING/DRIFT layers
    // structurally have nothing to say. Emit one Info finding so the
    // agent reads silence as expected, not as "mmk decided to be
    // quiet." HEALTH is structural and may still fire — that's signal,
    // not noise.
    let changed_paths: Vec<PathBuf> = changed.iter().map(|c| c.path.clone()).collect();
    let new_frac = mmk_core::budget::new_file_fraction(&changed_paths, &commits_touching);
    if new_frac > cfg.bulk.greenfield_threshold {
        let history_layer_fired = findings.iter().any(|f| {
            matches!(
                f.layer,
                Layer::Hotspot | Layer::Coupling | Layer::Drift | Layer::Budget
            )
        });
        if !history_layer_fired {
            let new_count = changed_paths
                .iter()
                .filter(|p| !commits_touching.contains_key(p.as_path()))
                .count();
            findings.push(Finding::new(
                Layer::Coupling,
                Severity::Info,
                messages::greenfield_signal(new_count, changed_paths.len()),
            ));
        }
    }

    let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);

    match args.format {
        Format::Text => render_text(stdout, &findings)?,
        Format::Json => crate::output::json::write_review(
            stdout,
            mode,
            &changed,
            &findings,
            &analysis,
            duration_ms,
            &cfg,
            &health_matches,
            &health_patterns,
            Some(new_frac),
        )?,
    }
    Ok(verdict_for(args.gate, &findings))
}

fn emit_empty<O: Write>(args: &ReviewArgs, mode: ReviewMode, stdout: &mut O) -> Result<()> {
    match args.format {
        Format::Text => Ok(()),
        Format::Json => crate::output::json::write_review_empty(stdout, mode),
    }
}

fn emit_bulk<O: Write>(
    args: &ReviewArgs,
    mode: ReviewMode,
    changed: &[ChangedFile],
    findings: &[Finding],
    started: Instant,
    stdout: &mut O,
) -> Result<Verdict> {
    let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    match args.format {
        Format::Text => render_text(stdout, findings)?,
        Format::Json => {
            crate::output::json::write_review_bulk(stdout, mode, changed, findings, duration_ms)?;
        }
    }
    Ok(verdict_for(args.gate, findings))
}

pub(crate) fn bulk_self_findings(files_n: u32, lines_n: u64, cfg: &Config) -> Vec<Finding> {
    let mut findings = Vec::new();
    let triggers = mmk_core::budget::check_diff_budget(
        &mmk_core::budget::BudgetCheck {
            files_changed: files_n,
            lines_changed: lines_n,
        },
        &cfg.bulk,
    );
    for t in triggers {
        let msg = match t {
            mmk_core::budget::BudgetTrigger::FilesExceeded { actual, max } => {
                messages::budget_files(actual, max, true)
            }
            mmk_core::budget::BudgetTrigger::LinesExceeded { actual, max } => {
                messages::budget_lines(actual, max, true)
            }
        };
        findings.push(Finding::new(Layer::Budget, Severity::Warn, msg));
    }
    findings
}

pub(crate) fn verdict_for(gate: Gate, findings: &[Finding]) -> Verdict {
    match gate {
        // Gate::Error reserved for future error-severity findings;
        // no such severity exists today, so it behaves like None.
        Gate::None | Gate::Error => Verdict::Ok,
        Gate::Warn => {
            if findings.iter().any(|f| f.severity == Severity::Warn) {
                Verdict::GateTriggered
            } else {
                Verdict::Ok
            }
        }
    }
}

/// Parse `git diff --numstat` output. Binary files (numstat
/// emits `- -` for added/deleted) are skipped because they don't
/// contribute line-budget signal and have no rank data anyway.
///
/// In `WorkingTree` mode the result is augmented with untracked
/// (not-yet-`git add`-ed) files via `mmk_git::list_untracked`. The
/// other modes (Staged / Range / Commit) operate on index/commit
/// state where untracked is by definition out of scope.
pub(crate) fn collect_diff(
    cwd: &Path,
    mode: ReviewMode,
    args: &ReviewArgs,
    ignores: &[String],
) -> Result<Vec<ChangedFile>> {
    let mut cmd = Command::new("git");
    cmd.arg("diff").arg("--numstat").current_dir(cwd);
    match mode {
        ReviewMode::WorkingTree => {
            cmd.arg("HEAD");
        }
        ReviewMode::Staged => {
            cmd.arg("--cached");
        }
        ReviewMode::Range => {
            cmd.arg(args.range.as_ref().expect("range mode requires --range"));
        }
        ReviewMode::Commit => {
            let sha = args.commit.as_ref().expect("commit mode requires --commit");
            cmd.arg(format!("{sha}^..{sha}"));
        }
    }

    let out = cmd
        .output()
        .context("failed to invoke `git diff` — is git on PATH?")?;
    if !out.status.success() {
        anyhow::bail!(
            "git diff exited with {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
    }

    let mut files = Vec::new();
    for raw in String::from_utf8_lossy(&out.stdout).lines() {
        let mut parts = raw.splitn(3, '\t');
        let added = parts.next().unwrap_or("-");
        let deleted = parts.next().unwrap_or("-");
        let Some(path) = parts.next() else { continue };
        // Binary files: `-` for both columns. Drop them — they don't
        // contribute line-budget signal and we don't have rank data
        // for binary blobs anyway.
        if added == "-" && deleted == "-" {
            continue;
        }
        let added: u64 = added.parse().unwrap_or(0);
        let deleted: u64 = deleted.parse().unwrap_or(0);
        files.push(ChangedFile {
            path: PathBuf::from(path),
            added,
            deleted,
        });
    }

    if mode == ReviewMode::WorkingTree {
        let globset = mmk_git::build_globset(ignores)
            .context("failed to compile ignore globs for untracked enumeration")?;
        let untracked = mmk_git::list_untracked(cwd, &globset)?;
        let known: AHashSet<PathBuf> = files.iter().map(|c| c.path.clone()).collect();
        for u in untracked {
            // `git diff HEAD` reports a stage-deletion as one entry;
            // an untracked file at the same path is a separate entry.
            // Dedup defensively in case of any overlap.
            if known.contains(&u.path) {
                continue;
            }
            files.push(ChangedFile {
                path: u.path,
                added: u.line_count,
                deleted: 0,
            });
        }
    }

    Ok(files)
}

pub(crate) fn compute_findings(
    changed: &[ChangedFile],
    ranked: &[mmk_core::HotspotEntry],
    commits: &[mmk_core::types::Commit],
    commits_touching: &ahash::AHashMap<PathBuf, u32>,
    cfg: &Config,
    top: usize,
) -> Vec<Finding> {
    let mut findings = Vec::new();
    let changed_set: AHashSet<PathBuf> = changed.iter().map(|c| c.path.clone()).collect();

    // HOTSPOT — changed file is ranked ≤ top.
    for c in changed {
        if let Some(entry) = ranked.iter().find(|e| e.path == c.path) {
            if (entry.hotspot_rank as usize) <= top {
                findings.push(Finding::new(
                    Layer::Hotspot,
                    Severity::Warn,
                    messages::hotspot(&c.path, entry.hotspot_rank, top),
                ));
            }
        }
    }

    // COUPLING — delegate to the shared helper. The Wilson gate
    // (Wilson lower ≥ confidence_threshold AND n ≥ min_sample_size)
    // and the per-partner glob filter live in one place.
    if !changed_set.is_empty() {
        let couples_map =
            coupling::compute_conditional_couples_for(commits, &changed_set, COUPLES_PER_FILE);
        let ignore_set = build_partner_globset(&cfg.coupling.ignore_partners);
        for c in changed {
            let n = commits_touching.get(&c.path).copied().unwrap_or(0);
            let Some(partners) = couples_map.get(&c.path) else {
                continue;
            };
            findings.extend(coupling_findings(CouplingEmission {
                subject: &c.path,
                n,
                partners,
                cfg: &cfg.coupling,
                ignore_set: ignore_set.as_ref(),
                excluded_partners: &changed_set,
                severity: Severity::Warn,
                prose: CouplingProse::ReviewMissed,
            }));
        }
    }

    // BUDGET — delegated to mmk_core::budget::check_diff_budget so
    // review and session-summary share the exact threshold logic.
    let files_n = u32::try_from(changed.len()).unwrap_or(u32::MAX);
    let lines_n: u64 = changed.iter().map(|c| c.added + c.deleted).sum();
    let triggers = mmk_core::budget::check_diff_budget(
        &mmk_core::budget::BudgetCheck {
            files_changed: files_n,
            lines_changed: lines_n,
        },
        &cfg.bulk,
    );
    for t in triggers {
        let msg = match t {
            mmk_core::budget::BudgetTrigger::FilesExceeded { actual, max } => {
                messages::budget_files(actual, max, false)
            }
            mmk_core::budget::BudgetTrigger::LinesExceeded { actual, max } => {
                messages::budget_lines(actual, max, false)
            }
        };
        findings.push(Finding::new(Layer::Budget, Severity::Warn, msg));
    }

    findings
}

pub(crate) fn build_partner_globset(globs: &[String]) -> Option<GlobSet> {
    if globs.is_empty() {
        return None;
    }
    let mut builder = GlobSetBuilder::new();
    for g in globs {
        if let Ok(glob) = Glob::new(g) {
            builder.add(glob);
        }
    }
    builder.build().ok()
}
