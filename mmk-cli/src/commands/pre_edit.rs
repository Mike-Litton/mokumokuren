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

use crate::args::{Format, Gate, PreEditArgs};
use crate::commands::analyze::COUPLES_PER_FILE;
use crate::commands::common::{
    analyze_health_for_subject, apply_coupling_file, apply_health_file, apply_monotonic_gate,
    apply_sensor_file, coupling_findings_with_signal, health_to_finding, list_directory_siblings,
    load_bodies, load_config_file, resolve_patterns, structure_to_finding_with_signal,
    CouplingEmission, CouplingProse,
};
use crate::commands::review::{build_partner_globset, collect_working_tree_diff, verdict_for};
use crate::hook::HookEnvelope;
use crate::monotonic::MonotonicSignal;
use crate::output::findings::{render_text, Finding, Layer, Severity};
use crate::output::messages;
use crate::Verdict;

pub fn run<O: Write, E: Write>(
    args: &PreEditArgs,
    envelope: Option<&HookEnvelope>,
    stdout: &mut O,
    stderr: &mut E,
) -> Result<Verdict> {
    let cwd = std::env::current_dir().context("failed to determine current directory")?;

    // Argv path is optional so a stdin hook envelope can supply it
    // instead. The envelope wins when present — Claude Code's
    // `tool_input.file_path` is the authoritative source under a
    // hook recipe, and falling back to argv would surface a stale
    // path if both happened to be set.
    let raw_path = envelope
        .and_then(HookEnvelope::file_path)
        .map(std::path::Path::to_path_buf)
        .or_else(|| args.path.clone())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "mmk pre-edit: no path supplied (argv empty and stdin envelope had no \
                 tool_input.file_path)"
            )
        })?;

    // Hook integrations (Claude Code's PreToolUse) pass
    // `tool_input.file_path` as an absolute path. The analyzer keys
    // its lookup tables on repo-relative paths, so an absolute path
    // misses every layer and falls through to the OK "new file (no
    // history)" — silently degrading the hook output. Normalize
    // here so manual relative-path invocations and hook
    // absolute-path invocations produce identical signal.
    let path = normalize_repo_relative(&cwd, &raw_path);

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
    if let Some(file_s) = file_cfg.sensor.as_ref() {
        apply_sensor_file(
            &mut cfg.sensor.structure,
            &mut cfg.sensor.complexity,
            &mut cfg.sensor.budget_ramp,
            &mut cfg.sensor.cohesion,
            file_s,
        );
    }
    // The bulk filter governs which historical commits enter the
    // coupling/hotspot baseline. pre-edit's analyze pass needs the
    // same override review applies — without it, repos with wide
    // commit grain see "no analyzable history" wording on files
    // their own contributors edit routinely.
    if let Some(file_b) = file_cfg.bulk.as_ref() {
        if let Some(t) = file_b.greenfield_threshold {
            cfg.bulk.greenfield_threshold = t;
        }
        if !file_b.ignore_for_budget.is_empty() {
            cfg.bulk
                .ignore_for_budget
                .clone_from(&file_b.ignore_for_budget);
        }
        if let Some(v) = file_b.max_files {
            cfg.bulk.max_files = v;
        }
        if let Some(v) = file_b.max_lines {
            cfg.bulk.max_lines = v;
        }
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

    let mut tagged = compute_findings(
        &path,
        &ranked,
        &analysis.commits,
        &commits_touching,
        &cfg,
        args.top,
    );

    // STRUCTURE: directory-convention sensor. Pre-edit fires
    // informationally — the agent reads the convention and conforms
    // before writing. Computed early so it joins the rest through
    // one monotonic-gate pass.
    if cfg.sensor.structure.enabled {
        let abs = cwd.join(&path);
        let mode = if abs.exists() {
            mmk_core::sensors::StructureMode::PreEditExisting
        } else {
            mmk_core::sensors::StructureMode::PreEditNew
        };
        let siblings = list_directory_siblings(&cwd, &path);
        let bodies = load_bodies(&cwd, &siblings);
        let input = mmk_core::sensors::StructureInput {
            path: &path,
            siblings: &siblings,
            bodies: &bodies,
            subject_body: None,
            mode,
            cfg: &cfg.sensor.structure,
        };
        if let Some(sf) = mmk_core::sensors::compute_structure_finding(&input) {
            let cap = cfg.sensor.structure.top_imports_to_show;
            let pct = (cfg.sensor.structure.import_majority * 100.0).round() as u32;
            tagged.push(structure_to_finding_with_signal(&sf, cap, pct));
        }
    }

    let mut findings =
        apply_monotonic_gate(&cwd, analysis.head_sha.as_deref(), tagged, args.no_dedup);

    // HEALTH: structural-pattern adapter. Pre-edit treats every
    // Health finding as informational (the agent hasn't acted yet
    // — surfaces neighbors but doesn't demand edits). EVASION
    // requires a working-vs-HEAD diff; pre-edit fires *before* the
    // agent's edit, so the working tree and HEAD are identical for
    // this subject — passing `head_body: None` keeps the detector
    // dormant under pre-edit semantics.
    let health_patterns: Vec<mmk_health::HealthPattern> = resolve_patterns(&cfg.health.ts.patterns)
        .into_iter()
        .filter(|p| !matches!(p, mmk_health::HealthPattern::BroadException))
        .collect();
    let health_matches: Vec<mmk_health::HealthFinding> = if cfg.health.ts.enabled {
        let peer_paths: Vec<PathBuf> = analysis.loc.keys().cloned().collect();
        analyze_health_for_subject(&cwd, &path, None, &peer_paths, &health_patterns)
    } else {
        Vec::new()
    };
    for h in &health_matches {
        findings.push(health_to_finding(h, Severity::Info));
    }

    // BUDGET (continuous-feedback ramp): under-cap meter so the
    // agent sees the cap climbing *before* the next edit. Opt-in
    // via [sensor.budget_ramp] enabled = true — see review.rs for
    // why this didn't ship default-on. Same wording / tiers as
    // review's ramp; single source of truth in messages::budget_ramp.
    if cfg.sensor.budget_ramp.enabled {
        if let Ok(working_changed) = collect_working_tree_diff(&cwd, &cfg.ignores) {
            // v0.6: read net counts so a generated-file regeneration
            // doesn't keep the pre-edit ramp pinned at high% across
            // every edit in the session.
            let counts = crate::commands::review::budget_counts(
                &working_changed,
                &cfg.bulk.ignore_for_budget,
            );
            let check = mmk_core::budget::BudgetCheck {
                files_changed: counts.files_net,
                lines_changed: counts.lines_net,
            };
            let progress = mmk_core::budget::budget_progress(&check, &cfg.bulk);
            match mmk_core::budget::budget_tier(&progress) {
                mmk_core::budget::BudgetTier::Approaching => {
                    findings.push(Finding::new(
                        Layer::Budget,
                        Severity::Info,
                        messages::budget_ramp(
                            progress.files.0,
                            progress.files.1,
                            progress.lines.0,
                            progress.lines.1,
                            false,
                        ),
                    ));
                }
                mmk_core::budget::BudgetTier::Near => {
                    findings.push(Finding::new(
                        Layer::Budget,
                        Severity::Warn,
                        messages::budget_ramp(
                            progress.files.0,
                            progress.files.1,
                            progress.lines.0,
                            progress.lines.1,
                            true,
                        ),
                    ));
                }
                // Over and Quiet: no pre-edit ramp surface. Over is
                // handled by review's existing over-cap message; Quiet
                // is the noise floor.
                mmk_core::budget::BudgetTier::Quiet | mmk_core::budget::BudgetTier::Over => {}
            }
        }
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
            if d.path == path {
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
        let n = commits_touching.get(&path).copied().unwrap_or(0);
        let rank = ranked
            .iter()
            .find(|e| e.path == path)
            .map(|e| e.hotspot_rank);
        // `mmk_git::path_in_head` walks HEAD's tree directly so the
        // predicate answers "is this path tracked at HEAD?" rather
        // than the previous `analysis.loc.contains_key()` which only
        // covered paths with churn in the analysis window — every
        // existing file whose history all fell to the bulk filter
        // (or simply had no commits in the window) was misreported
        // as new. See `messages::quiet_file` for the wording the
        // distinction drives.
        let present_in_head = mmk_git::path_in_head(&cwd, path.as_path());
        findings.push(Finding::new(
            Layer::Coupling,
            Severity::Ok,
            messages::quiet_file(&path, n, cfg.window.days, rank, present_in_head),
        ));
    }

    let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);

    // DEDUP: same shape as `mmk review`. Same baseline + same
    // findings + within TTL → silent. Any of the three changing
    // re-emits the full output. Hook callers see the suppression
    // through a `systemMessage` instead of empty stdout.
    let suppressed =
        !args.no_dedup && maybe_suppress_pre_edit(&cwd, &findings, &analysis, args.gate).is_some();
    if suppressed && envelope.is_none() {
        return Ok(verdict_for(args.gate, &findings));
    }

    if let Some(env) = envelope_for_hook(envelope) {
        let block = matches!(args.gate, Gate::Warn);
        // PreToolUse cannot block in the Edit phase by Claude
        // Code's contract; that's a strategic-deployment choice
        // surfaced via PostToolUse. Pre-edit is always
        // `additionalContext`-only. Stop / PostToolUse honor
        // `--gate warn` per the plan.
        if env.hook_event_name == "PreToolUse" {
            crate::output::hook_json::write_pre_tool_use(
                stdout,
                if suppressed { &[] } else { &findings },
                suppressed,
                analysis.head_sha.as_deref(),
            )?;
        } else {
            crate::output::hook_json::write_post_tool_use(
                stdout,
                &env.hook_event_name,
                if suppressed { &[] } else { &findings },
                suppressed,
                analysis.head_sha.as_deref(),
                block,
            )?;
        }
        return Ok(verdict_for(args.gate, &findings));
    }

    match args.format {
        Format::Text => render_text(stdout, &findings)?,
        Format::Json => crate::output::json::write_pre_edit(
            stdout,
            &path,
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

fn envelope_for_hook(env: Option<&HookEnvelope>) -> Option<&HookEnvelope> {
    env.filter(|e| !e.hook_event_name.is_empty())
}

fn maybe_suppress_pre_edit(
    cwd: &Path,
    findings: &[Finding],
    analysis: &mmk_git::AnalyzeOutput,
    gate: crate::args::Gate,
) -> Option<Verdict> {
    let head_sha = analysis.head_sha.as_deref()?;
    let git_dir = mmk_git::discover_work_dir(cwd).and_then(|wd| {
        let git = wd.join(".git");
        git.exists().then_some(git)
    })?;
    let path = crate::dedup::dedup_path(&git_dir)?;
    let hash = crate::dedup::hash_findings(findings);
    let prior = crate::dedup::load_record(&path);
    let now = crate::dedup::now_unix();
    let ttl = crate::dedup::ttl_seconds();
    if crate::dedup::should_suppress(hash, head_sha, prior.as_ref(), now, ttl) {
        return Some(verdict_for(gate, findings));
    }
    crate::dedup::record_emission(
        &path,
        &crate::dedup::DedupRecord {
            findings_hash: hash,
            head_sha: head_sha.to_string(),
            emitted_at: now,
        },
    );
    None
}

/// Resolve `input` to a repo-relative path so it matches the
/// analyzer's keying.
///
/// Hook integrations pass absolute paths (Claude Code's
/// `tool_input.file_path`); manual invocations pass relative paths.
/// Both must produce the same downstream lookup key — anything else
/// silently degrades hook output to the OK fall-through.
fn normalize_repo_relative(cwd: &Path, input: &Path) -> PathBuf {
    let abs = if input.is_absolute() {
        input.to_path_buf()
    } else {
        cwd.join(input)
    };
    let Some(repo_root) = mmk_git::discover_work_dir(cwd) else {
        return input.to_path_buf();
    };
    abs.strip_prefix(&repo_root)
        .map_or_else(|_| input.to_path_buf(), Path::to_path_buf)
}

fn compute_findings(
    target: &Path,
    ranked: &[mmk_core::HotspotEntry],
    commits: &[mmk_core::types::Commit],
    commits_touching: &ahash::AHashMap<PathBuf, u32>,
    cfg: &Config,
    top: usize,
) -> Vec<(Finding, Option<MonotonicSignal>)> {
    let mut findings: Vec<(Finding, Option<MonotonicSignal>)> = Vec::new();

    // HOTSPOT — surface the rank if the file is in the top-N. Same
    // threshold semantics as `mmk review`.
    if let Some(entry) = ranked.iter().find(|e| e.path == target) {
        if (entry.hotspot_rank as usize) <= top {
            findings.push((
                Finding::new(
                    Layer::Hotspot,
                    Severity::Warn,
                    messages::hotspot(target, entry.hotspot_rank, top),
                ),
                None,
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
        findings.extend(coupling_findings_with_signal(CouplingEmission {
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
