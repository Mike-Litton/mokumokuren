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
    analyze_health_for_subject, apply_coupling_file, apply_health_file, apply_monotonic_gate,
    apply_sensor_file, complexity_to_finding, coupling_findings_with_signal,
    health_severity_for_review, health_to_finding, list_directory_siblings, load_bodies,
    load_config_file, resolve_patterns, structure_to_finding_with_signal, CouplingEmission,
    CouplingProse,
};
use crate::hook::HookEnvelope;
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
    envelope: Option<&HookEnvelope>,
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
        emit_empty(args, envelope, mode, stdout)?;
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
    if let Some(file_s) = file_cfg.sensor.as_ref() {
        apply_sensor_file(
            &mut cfg.sensor.structure,
            &mut cfg.sensor.complexity,
            &mut cfg.sensor.budget_ramp,
            &mut cfg.sensor.cohesion,
            file_s,
        );
    }
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
    // Explicit --coupling-threshold wins; fall back to the
    // (deprecated) --blast-radius-threshold so existing CLI
    // invocations keep working until users migrate. Both are routed
    // to confidence_threshold — the active gate.
    if let Some(t) = args.coupling_threshold.or(args.blast_radius_threshold) {
        cfg.coupling.threshold = t;
        cfg.coupling.confidence_threshold = t;
    }

    let started = Instant::now();

    // STRUCTURE + COMPLEXITY are per-file: their cost scales with the
    // changed-file count, which the bulk-self-filter already caps.
    // Compute them once here so they're visible in BOTH the bulk path
    // (where the analyzer-based layers are suppressed) and the
    // normal path. The expensive-history layers (HOTSPOT/COUPLING) and
    // the analyzer-driven greenfield signal still gate on bulk below.
    let sensor_items = compute_sensor_findings(&cwd, &changed, &cfg);

    // Bulk self-filter: when the diff itself blows the BUDGET cap
    // (default `max_files = 15`, `max_lines = 1000`), the
    // history-graph layers (HOTSPOT, COUPLING, DRIFT, greenfield)
    // are intentionally skipped. The reason is signal collapse,
    // not cost:
    //
    // - COUPLING asks "given you edited A, you should also have
    //   touched B." The `excluded_partners` filter drops any
    //   partner already in the changed_set. On a diff with
    //   hundreds of files, every historical partner of every
    //   changed file is in the diff by construction — COUPLING's
    //   "missed partner" question is trivially answered "no" for
    //   nearly all pairs, and the few survivors are statistical
    //   artifacts.
    // - HOTSPOT degenerates symmetrically: dozens of fires per
    //   review, each "true" but each redundant against the BUDGET
    //   message that's the actual point. Information per finding
    //   collapses.
    // - The historical analyzer (`mmk_git::analyze`) already drops
    //   commits with > `max_files` files from the *baseline* —
    //   that's the same `bulk` filter applied to the working tree
    //   here, so the working-tree diff isn't being scored against
    //   a baseline that would have included it anyway.
    //
    // Per-file sensors (STRUCTURE, COMPLEXITY) still run — their
    // cost scales per changed file (not with diff size) and their
    // signal stays meaningful per-file at any total diff size.
    //
    // The LOC-cap framing is research-grounded (Cohen 2006
    // SmartBear/Cisco data: defect-detection-rate degrades sharply
    // past ~200 LOC per review and floors out past ~400 LOC). The
    // file-count cap is an engineering heuristic — there is no
    // peer-reviewed file-count threshold in the literature.
    //
    // The `ignore_for_budget` globset removes generated-file class
    // paths from the BUDGET-trigger denominator. Both gross (full
    // diff) and net (post-filter) counts are surfaced so silent
    // dropping doesn't re-create the v0.4 class of bug where mmk's
    // measurement disagreed with the user's reality.
    let counts = budget_counts(&changed, &cfg.bulk.ignore_for_budget);
    if counts.files_net > cfg.bulk.max_files || counts.lines_net > u64::from(cfg.bulk.max_lines) {
        let mut findings = bulk_self_findings(&counts, &cfg);
        let sensor_findings = apply_monotonic_gate(&cwd, None, sensor_items, args.no_dedup);
        findings.extend(sensor_findings);
        return emit_bulk(
            args, envelope, mode, &changed, &counts, &findings, started, stdout,
        );
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

    // History-layer findings (HOTSPOT / COUPLING / BUDGET) are
    // signal-tagged so COUPLING repeats with neither k nor n
    // worsening drop through the same gate that handles
    // STRUCTURE / COMPLEXITY. Sensor + history items merge into one
    // gate call so the LRU cap and persistence pass run once.
    let mut tagged: Vec<(Finding, Option<crate::monotonic::MonotonicSignal>)> =
        compute_findings_with_signals(
            &changed,
            &ranked,
            &analysis.commits,
            &commits_touching,
            &cfg,
            args.top,
            &counts,
        );
    tagged.extend(sensor_items);

    let mut findings =
        apply_monotonic_gate(&cwd, analysis.head_sha.as_deref(), tagged, args.no_dedup);

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

    // DEDUP: suppress emission iff (same findings hash) AND (same
    // HEAD SHA) AND (within TTL of the prior emission). All three
    // boundaries match the agent's mental model — if the picture
    // changed since last time, show me. Hook-shape only: analyze /
    // eval / session-summary / drift are user-invoked and stay
    // verbose. Under hook output, the agent gets an explicit
    // `systemMessage` instead of silence.
    let suppressed =
        !args.no_dedup && maybe_suppress_review(&cwd, &findings, &analysis, args.gate).is_some();
    if suppressed && envelope.is_none() {
        return Ok(verdict_for(args.gate, &findings));
    }

    if let Some(env) = envelope_for_hook(envelope) {
        crate::output::hook_json::write_post_tool_use(
            stdout,
            &env.hook_event_name,
            if suppressed { &[] } else { &findings },
            suppressed,
            analysis.head_sha.as_deref(),
            matches!(args.gate, Gate::Warn),
        )?;
        return Ok(verdict_for(args.gate, &findings));
    }

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
            &counts,
        )?,
    }
    Ok(verdict_for(args.gate, &findings))
}

/// Thin alias: returns `Some(env)` when the envelope carries a
/// non-empty `hook_event_name`. The empty-name case happens when a
/// caller pipes some other JSON in but the field is missing — fall
/// through to standard CLI output rather than emitting hook-shape
/// JSON the hook harness can't parse.
fn envelope_for_hook(env: Option<&HookEnvelope>) -> Option<&HookEnvelope> {
    env.filter(|e| !e.hook_event_name.is_empty())
}

/// Per-diff BUDGET accounting: the gross totals (every changed
/// file / line in the diff), the net totals (after applying the
/// `bulk.ignore_for_budget` globset), and the count of files
/// dropped. The struct is populated once per `mmk review`
/// invocation and threaded through the bulk-self-filter, the
/// over-cap trigger, the under-cap ramp, and the JSON envelope so
/// every consumer sees the same numbers.
#[derive(Debug, Clone)]
pub(crate) struct BudgetCounts {
    pub files_gross: u32,
    pub files_net: u32,
    pub lines_gross: u64,
    pub lines_net: u64,
    /// Globs from `bulk.ignore_for_budget` actively in effect. Echoed
    /// into JSON for diagnostic transparency; absent / empty when no
    /// globs are configured.
    pub ignored_for_budget: Vec<String>,
}

impl BudgetCounts {
    /// `true` when the ignore-for-budget globset matched at least one
    /// file in the diff. Drives the JSON sub-block presence and
    /// whether the prose carries the gross/net split.
    pub(crate) const fn has_ignored(&self) -> bool {
        self.files_net != self.files_gross || self.lines_net != self.lines_gross
    }
}

/// Compute gross + net counts against the `bulk.ignore_for_budget`
/// globset. Pure: no I/O, easy to test.
pub(crate) fn budget_counts(changed: &[ChangedFile], globs: &[String]) -> BudgetCounts {
    let files_gross = u32::try_from(changed.len()).unwrap_or(u32::MAX);
    let lines_gross: u64 = changed.iter().map(|c| c.added + c.deleted).sum();
    let globset = build_budget_ignore_globset(globs);
    let (files_net, lines_net) = globset.as_ref().map_or((files_gross, lines_gross), |set| {
        let mut fn_ = 0u32;
        let mut ln = 0u64;
        for c in changed {
            if set.is_match(&c.path) {
                continue;
            }
            fn_ = fn_.saturating_add(1);
            ln = ln.saturating_add(c.added + c.deleted);
        }
        (fn_, ln)
    });
    BudgetCounts {
        files_gross,
        files_net,
        lines_gross,
        lines_net,
        ignored_for_budget: globs.to_vec(),
    }
}

fn build_budget_ignore_globset(globs: &[String]) -> Option<GlobSet> {
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

/// Side-effecting dedup gate. Returns `Some(verdict)` if the current
/// fire matches the prior recorded emission and should be silently
/// dropped; `None` if the caller should proceed to emit. On a `None`
/// return, the new emission is recorded as the latest baseline.
fn maybe_suppress_review(
    cwd: &Path,
    findings: &[Finding],
    analysis: &mmk_git::AnalyzeOutput,
    gate: Gate,
) -> Option<Verdict> {
    // No HEAD = unborn repo; nothing to dedup against.
    let head_sha = analysis.head_sha.as_deref()?;
    // discover_work_dir returns the worktree; the dedup file sits
    // under the .git dir to share keys across worktrees.
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

fn emit_empty<O: Write>(
    args: &ReviewArgs,
    envelope: Option<&HookEnvelope>,
    mode: ReviewMode,
    stdout: &mut O,
) -> Result<()> {
    if let Some(env) = envelope_for_hook(envelope) {
        return crate::output::hook_json::write_post_tool_use(
            stdout,
            &env.hook_event_name,
            &[],
            false,
            None,
            matches!(args.gate, Gate::Warn),
        );
    }
    match args.format {
        Format::Text => Ok(()),
        Format::Json => crate::output::json::write_review_empty(stdout, mode),
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_bulk<O: Write>(
    args: &ReviewArgs,
    envelope: Option<&HookEnvelope>,
    mode: ReviewMode,
    changed: &[ChangedFile],
    counts: &BudgetCounts,
    findings: &[Finding],
    started: Instant,
    stdout: &mut O,
) -> Result<Verdict> {
    let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    if let Some(env) = envelope_for_hook(envelope) {
        crate::output::hook_json::write_post_tool_use(
            stdout,
            &env.hook_event_name,
            findings,
            false,
            None,
            matches!(args.gate, Gate::Warn),
        )?;
        return Ok(verdict_for(args.gate, findings));
    }
    match args.format {
        Format::Text => render_text(stdout, findings)?,
        Format::Json => {
            crate::output::json::write_review_bulk(
                stdout,
                mode,
                changed,
                counts,
                findings,
                duration_ms,
            )?;
        }
    }
    Ok(verdict_for(args.gate, findings))
}

/// Compute STRUCTURE + COMPLEXITY findings for the diff's changed paths.
///
/// Both sensors are per-file and don't depend on the historical
/// `analyze` pass — their cost scales with `changed.len()`, which the
/// bulk-self-filter already caps. Pulled out of the main `run` so the
/// bulk path can also surface them: structural / per-function signal
/// is at least as relevant on a sweep as on a normal review.
///
/// Returns `(finding, Option<MonotonicSignal>)` pairs so the
/// orchestration code can apply the monotonic-worsening gate without
/// re-parsing finding messages. STRUCTURE findings carry `None`
/// (per-directory-shape; the whole-set dedup already covers them);
/// COMPLEXITY findings carry the `(path, function, kind)` key plus
/// the metric so equal-or-improving repeats can drop.
fn compute_sensor_findings(
    cwd: &Path,
    changed: &[ChangedFile],
    cfg: &Config,
) -> Vec<(Finding, Option<crate::monotonic::MonotonicSignal>)> {
    let mut out = Vec::new();
    if !(cfg.sensor.structure.enabled || cfg.sensor.complexity.enabled) {
        return out;
    }
    let cap = cfg.sensor.structure.top_imports_to_show;
    let pct = (cfg.sensor.structure.import_majority * 100.0).round() as u32;
    for c in changed {
        let siblings = list_directory_siblings(cwd, &c.path);
        let mut all_paths = siblings.clone();
        if !all_paths.iter().any(|p| p == &c.path) {
            all_paths.push(c.path.clone());
        }
        let bodies = load_bodies(cwd, &all_paths);

        if cfg.sensor.structure.enabled {
            let subject_body = bodies.get(&c.path).map(String::as_str);
            let input = mmk_core::sensors::StructureInput {
                path: &c.path,
                siblings: &siblings,
                bodies: &bodies,
                subject_body,
                mode: mmk_core::sensors::StructureMode::Review,
                cfg: &cfg.sensor.structure,
            };
            if let Some(sf) = mmk_core::sensors::compute_structure_finding(&input) {
                out.push(structure_to_finding_with_signal(&sf, cap, pct));
            }
        }
        if cfg.sensor.complexity.enabled {
            let input = mmk_core::sensors::ComplexityInput {
                path: &c.path,
                siblings: &siblings,
                bodies: &bodies,
                cfg: &cfg.sensor.complexity,
            };
            for cf in mmk_core::sensors::compute_complexity_findings(&input) {
                let signal = complexity_monotonic_signal(&cf);
                out.push((complexity_to_finding(&cf), Some(signal)));
            }
        }
    }
    out
}

/// Build the per-finding monotonic key + axes for a COMPLEXITY
/// finding. `kind` is encoded in the key so a Nesting finding and a
/// Size finding on the same `(path, function)` get independent
/// suppression — they measure different things and can move
/// independently.
fn complexity_monotonic_signal(
    f: &mmk_core::sensors::ComplexityFinding,
) -> crate::monotonic::MonotonicSignal {
    let kind = match f.kind {
        mmk_core::sensors::ComplexityFindingKind::Nesting => "nesting",
        mmk_core::sensors::ComplexityFindingKind::Size => "loc",
    };
    let key = format!("complexity::{kind}::{}::{}", f.path.display(), f.function);
    crate::monotonic::MonotonicSignal {
        key,
        axes: vec![f.actual],
    }
}

/// BUDGET findings emitted on the bulk-self-filter path.
///
/// The `suppressed = true` flag flowed into the message formatters
/// produces wording that explicitly names the skipped layers
/// (HOTSPOT/COUPLING) and why — without it, an agent reading the
/// hook output would see an empty HOTSPOT/COUPLING block and could
/// misread the silence as "all clear" rather than "uncomputed at
/// this scale." See [`messages::budget_files`] for the full
/// rationale on the wording choice.
pub(crate) fn bulk_self_findings(counts: &BudgetCounts, cfg: &Config) -> Vec<Finding> {
    let mut findings = Vec::new();
    let triggers = mmk_core::budget::check_diff_budget(
        &mmk_core::budget::BudgetCheck {
            files_changed: counts.files_net,
            lines_changed: counts.lines_net,
        },
        &cfg.bulk,
    );
    for t in triggers {
        let msg = match t {
            mmk_core::budget::BudgetTrigger::FilesExceeded { actual, max } => {
                let gross = counts.has_ignored().then_some(counts.files_gross);
                messages::budget_files(actual, max, gross, true)
            }
            mmk_core::budget::BudgetTrigger::LinesExceeded { actual, max } => {
                let gross = counts.has_ignored().then_some(counts.lines_gross);
                messages::budget_lines(actual, max, gross, true)
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

#[allow(clippy::too_many_arguments)]
pub(crate) fn compute_findings(
    changed: &[ChangedFile],
    ranked: &[mmk_core::HotspotEntry],
    commits: &[mmk_core::types::Commit],
    commits_touching: &ahash::AHashMap<PathBuf, u32>,
    cfg: &Config,
    top: usize,
    counts: &BudgetCounts,
) -> Vec<Finding> {
    compute_findings_with_signals(changed, ranked, commits, commits_touching, cfg, top, counts)
        .into_iter()
        .map(|(f, _)| f)
        .collect()
}

/// Same shape as [`compute_findings`] but each finding is paired with
/// an optional [`MonotonicSignal`]. COUPLING entries carry signals
/// (key `coupling::<subject>::<partner>`, axes `[k, n]`); HOTSPOT and
/// BUDGET findings stay untagged — HOTSPOT today fires once per
/// changed-and-ranked file (re-fire on every edit is *itself* the
/// signal: the file is back in scope), BUDGET is whole-set deduped.
#[allow(clippy::too_many_arguments)]
pub(crate) fn compute_findings_with_signals(
    changed: &[ChangedFile],
    ranked: &[mmk_core::HotspotEntry],
    commits: &[mmk_core::types::Commit],
    commits_touching: &ahash::AHashMap<PathBuf, u32>,
    cfg: &Config,
    top: usize,
    counts: &BudgetCounts,
) -> Vec<(Finding, Option<crate::monotonic::MonotonicSignal>)> {
    let mut findings: Vec<(Finding, Option<crate::monotonic::MonotonicSignal>)> = Vec::new();
    let changed_set: AHashSet<PathBuf> = changed.iter().map(|c| c.path.clone()).collect();

    // HOTSPOT — changed file is ranked ≤ top.
    for c in changed {
        if let Some(entry) = ranked.iter().find(|e| e.path == c.path) {
            if (entry.hotspot_rank as usize) <= top {
                findings.push((
                    Finding::new(
                        Layer::Hotspot,
                        Severity::Warn,
                        messages::hotspot(&c.path, entry.hotspot_rank, top),
                    ),
                    None,
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
            findings.extend(coupling_findings_with_signal(CouplingEmission {
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

    // COHESION — sits next to COUPLING because both walk the same
    // co-change graph. COUPLING flags missing partners of ONE
    // changed file; COHESION flags whether the changed files
    // *together* form a single cluster or split into multiple
    // disjoint ones (the structural fingerprint of a tangled diff).
    // Severity is Info — pattern-naming, not gating; promotion to
    // Warn waits for replay-grade evidence that tangled diffs
    // measurably predict revert.
    if cfg.sensor.cohesion.enabled && !changed_set.is_empty() {
        let components = coupling::connected_components_by_wilson(
            commits,
            &changed_set,
            cfg.sensor.cohesion.confidence_threshold,
            cfg.sensor.cohesion.min_sample_size,
        );
        // Drop singleton greenfield files: a 1-file component on a
        // path the analyzer has never seen carries no historical
        // signal either way, so counting it as "another cluster"
        // would over-report tangled diffs on legitimate
        // additions-alongside-changes. Multi-file singletons
        // (real-but-isolated edits) still count.
        let qualifying: Vec<&Vec<PathBuf>> = components
            .iter()
            .filter(|c| {
                let n_files = u32::try_from(c.len()).unwrap_or(u32::MAX);
                if n_files < cfg.sensor.cohesion.min_files_per_cluster {
                    let all_greenfield = c
                        .iter()
                        .all(|p| !commits_touching.contains_key(p.as_path()));
                    return !all_greenfield && n_files >= 1;
                }
                true
            })
            .filter(|c| {
                u32::try_from(c.len()).unwrap_or(u32::MAX)
                    >= cfg.sensor.cohesion.min_files_per_cluster
            })
            .collect();
        if qualifying.len() >= 2 {
            let total_files: usize = qualifying.iter().map(|c| c.len()).sum();
            let cluster_sizes: Vec<usize> = qualifying.iter().map(|c| c.len()).collect();
            // Cap on detail-form rendering: when the total cluster
            // path count exceeds 8 the path enumeration blows the
            // line length without adding signal — the cluster
            // sizes already convey the structural picture.
            let detail_paths: Option<Vec<Vec<String>>> = if total_files <= 8 {
                Some(
                    qualifying
                        .iter()
                        .map(|c| c.iter().map(|p| p.display().to_string()).collect())
                        .collect(),
                )
            } else {
                None
            };
            findings.push((
                Finding::new(
                    Layer::Cohesion,
                    Severity::Info,
                    messages::cohesion_tangled(&cluster_sizes, detail_paths.as_deref()),
                ),
                None,
            ));
        }
    }

    // BUDGET — over-cap triggers via check_diff_budget; under-cap
    // ramp via budget_progress so the meter is visible from 50%
    // upward instead of snapping at 100%. Both gates evaluate the
    // *net* counts (after `bulk.ignore_for_budget`); the prose
    // surfaces the gross totals when they differ so the agent can
    // see what was excluded.
    let check = mmk_core::budget::BudgetCheck {
        files_changed: counts.files_net,
        lines_changed: counts.lines_net,
    };
    let triggers = mmk_core::budget::check_diff_budget(&check, &cfg.bulk);
    if triggers.is_empty() {
        // Under cap: ramp tier may fire when explicitly opted in.
        // Ramp shipped behind a flag because n=1 evidence isn't enough
        // to default-on a continuous-feedback signal that competes for
        // attention with the other layers; eval --replay measures
        // whether ramp findings correlate with course-correction.
        if cfg.sensor.budget_ramp.enabled {
            let progress = mmk_core::budget::budget_progress(&check, &cfg.bulk);
            match mmk_core::budget::budget_tier(&progress) {
                mmk_core::budget::BudgetTier::Approaching => {
                    findings.push((
                        Finding::new(
                            Layer::Budget,
                            Severity::Info,
                            messages::budget_ramp(
                                progress.files.0,
                                progress.files.1,
                                progress.lines.0,
                                progress.lines.1,
                                false,
                            ),
                        ),
                        None,
                    ));
                }
                mmk_core::budget::BudgetTier::Near => {
                    findings.push((
                        Finding::new(
                            Layer::Budget,
                            Severity::Warn,
                            messages::budget_ramp(
                                progress.files.0,
                                progress.files.1,
                                progress.lines.0,
                                progress.lines.1,
                                true,
                            ),
                        ),
                        None,
                    ));
                }
                mmk_core::budget::BudgetTier::Quiet | mmk_core::budget::BudgetTier::Over => {}
            }
        }
    } else {
        for t in triggers {
            let msg = match t {
                mmk_core::budget::BudgetTrigger::FilesExceeded { actual, max } => {
                    let gross = counts.has_ignored().then_some(counts.files_gross);
                    messages::budget_files(actual, max, gross, false)
                }
                mmk_core::budget::BudgetTrigger::LinesExceeded { actual, max } => {
                    let gross = counts.has_ignored().then_some(counts.lines_gross);
                    messages::budget_lines(actual, max, gross, false)
                }
            };
            findings.push((Finding::new(Layer::Budget, Severity::Warn, msg), None));
        }
    }

    findings
}

/// Pre-edit's hook to read the *current working-tree* diff vs
/// HEAD without going through the `ReviewArgs` plumbing. Returns
/// only the file count + line count form the per-edit BUDGET
/// ramp uses; binary entries / ignored entries are filtered the
/// same way `collect_diff` does in WorkingTree mode.
pub(crate) fn collect_working_tree_diff(
    cwd: &Path,
    ignores: &[String],
) -> Result<Vec<ChangedFile>> {
    let args = ReviewArgs {
        staged: false,
        range: None,
        commit: None,
        since: "180days".into(),
        top: 20,
        format: Format::Json,
        ignores: Vec::new(),
        config: None,
        verbose: false,
        coupling_threshold: None,
        blast_radius_threshold: None,
        gate: Gate::None,
        no_dedup: true,
    };
    collect_diff(cwd, ReviewMode::WorkingTree, &args, ignores)
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
