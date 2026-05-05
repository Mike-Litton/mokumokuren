//! Shared plumbing for the event-driven subcommands (`review`,
//! `pre-edit`, `eval`, etc.).
//!
//! "Discover the repo's `mokumokuren.toml` and apply it on top of an
//! in-memory `Config`" lives here so each subcommand wires through
//! one helper rather than copy-pasting the load-and-merge block.
//! The deprecation surface for `[coupling] threshold` is also
//! consolidated as a single, testable returns-warnings function —
//! see [`apply_coupling_file`].

use anyhow::{Context, Result};
use globset::GlobSet;
use mmk_config::{
    BudgetRampCfg, CohesionCfg, ComplexityCfg, ConfigFile, CouplingCfg, CouplingFile, HealthFile,
    HealthTsCfg, SensorFile, StructureCfg,
};
use mmk_core::CouplingEntry;
use mmk_health::{HealthFinding, HealthPattern};
use std::path::{Path, PathBuf};

use crate::monotonic::MonotonicSignal;
use crate::output::findings::{Finding, Layer, Severity};
use crate::output::messages;

/// Locate and parse `mokumokuren.toml`.
///
/// `explicit` wins when set (`--config <PATH>` on the CLI). With no
/// explicit path, the function walks up from `cwd` looking for the
/// repo's git work-dir and tries `<root>/mokumokuren.toml`. Returns
/// `(ConfigFile, Some(path))` if a file was loaded, or
/// `(ConfigFile::default(), None)` otherwise.
pub fn load_config_file(
    cwd: &Path,
    explicit: Option<&Path>,
) -> Result<(ConfigFile, Option<PathBuf>)> {
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

/// Deprecation diagnostic emitted when an old config knob is in effect.
///
/// Pure data — the caller decides how/whether to surface it (verbose
/// stderr today; could drive a structured warnings field in JSON
/// later without changing this code).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CouplingDeprecation {
    /// `[coupling] threshold` (or the equivalent CLI flag) is set.
    /// COUPLING is gated on the Wilson lower bound now, so the field
    /// is silently re-mapped to `confidence_threshold` for back-compat.
    LegacyThreshold,
}

impl CouplingDeprecation {
    /// One-line message. Stable text so CI grep-on-stderr can match it.
    #[must_use]
    pub const fn message(&self) -> &'static str {
        match self {
            Self::LegacyThreshold => {
                "warning: [coupling] threshold is deprecated; COUPLING is gated by \
                 confidence_threshold + min_sample_size (Wilson lower bound on \
                 conditional probability)."
            }
        }
    }
}

/// Apply a parsed `[coupling]` block onto the in-memory config.
///
/// Pure: returns the deprecation diagnostics that fired so the caller
/// can render them on stderr (verbose mode) or thread them into a
/// structured warnings field. Does no I/O.
///
/// Mapping rules:
/// - `threshold` (legacy) → both `cfg.threshold` (preserved for the
///   diagnostic echo in the `config` JSON block) AND
///   `confidence_threshold` (so the active gate honors the user's
///   intent). Returns `LegacyThreshold` deprecation.
/// - `confidence_threshold` → `cfg.confidence_threshold` (no warning).
/// - `min_sample_size` → `cfg.min_sample_size`.
/// - `ignore_partners` → replaces `cfg.ignore_partners` when non-empty.
pub fn apply_coupling_file(
    cfg: &mut CouplingCfg,
    file_cp: &CouplingFile,
) -> Vec<CouplingDeprecation> {
    let mut warnings = Vec::new();
    if let Some(t) = file_cp.threshold {
        cfg.threshold = t;
        cfg.confidence_threshold = t;
        warnings.push(CouplingDeprecation::LegacyThreshold);
    }
    if let Some(ct) = file_cp.confidence_threshold {
        cfg.confidence_threshold = ct;
    }
    if let Some(n) = file_cp.min_sample_size {
        cfg.min_sample_size = n;
    }
    if !file_cp.ignore_partners.is_empty() {
        cfg.ignore_partners.clone_from(&file_cp.ignore_partners);
    }
    warnings
}

/// Prose variant for a COUPLING finding.
///
/// Review and pre-edit answer related-but-different questions, so the
/// wording differs (review names a partner not in the diff; pre-edit
/// states the historical co-edit fact). Capturing the choice as an
/// enum keeps the vocabulary in one place — see
/// [`crate::output::messages`] for the actual format strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CouplingProse {
    ReviewMissed,
    PreEditExpected,
}

/// Inputs for [`coupling_findings`].
///
/// Bundled so adding a future argument (e.g. a Health-layer
/// integration that wants to re-categorize findings by structural
/// pattern) doesn't break call sites that don't care about it.
#[derive(Debug, Clone, Copy)]
pub struct CouplingEmission<'a> {
    pub subject: &'a Path,
    /// `commits_touching(subject)` — the binomial sample size the
    /// gate evaluates against `cfg.min_sample_size`.
    pub n: u32,
    pub partners: &'a [CouplingEntry],
    pub cfg: &'a CouplingCfg,
    pub ignore_set: Option<&'a GlobSet>,
    /// Paths to skip when emitting findings. In review mode it's
    /// the set of changed files (so "missed partner" doesn't fire
    /// on a partner the user *did* touch). In pre-edit it's
    /// typically empty.
    pub excluded_partners: &'a ahash::AHashSet<PathBuf>,
    pub severity: Severity,
    pub prose: CouplingProse,
}

/// Emit COUPLING findings under the Wilson gate.
///
/// One finding per partner of `input.subject` that clears
/// `n ≥ min_sample_size` AND `wilson_lower_95 ≥ confidence_threshold`,
/// after filtering by `ignore_set` and `excluded_partners`. Pure:
/// no repository or commits work happens here, which keeps the
/// unit-test surface small.
#[must_use]
pub fn coupling_findings(input: CouplingEmission<'_>) -> Vec<Finding> {
    coupling_findings_with_signal(input)
        .into_iter()
        .map(|(f, _)| f)
        .collect()
}

/// Same as [`coupling_findings`] but tagged for the monotonic gate.
///
/// Each finding is paired with a [`MonotonicSignal`] whose key
/// encodes `(subject, partner)` and whose axes are `[k, n]`
/// (`k = co_change_count`, `n = commits_touching(subject)`). Lets
/// the per-key gate suppress re-fires where neither axis worsened
/// — the COUPLING analogue of COMPLEXITY's nesting / LOC monotonic
/// dedup.
#[must_use]
pub fn coupling_findings_with_signal(
    input: CouplingEmission<'_>,
) -> Vec<(Finding, Option<MonotonicSignal>)> {
    if input.n < input.cfg.min_sample_size {
        return Vec::new();
    }
    let mut out = Vec::new();
    for p in input.partners {
        if p.wilson_lower_95 < input.cfg.confidence_threshold {
            continue;
        }
        if input.excluded_partners.contains(&p.partner) {
            continue;
        }
        if input.ignore_set.is_some_and(|set| set.is_match(&p.partner)) {
            continue;
        }
        let message = match input.prose {
            CouplingProse::ReviewMissed => messages::coupling_review_missed(
                input.subject,
                &p.partner,
                p.co_change_count,
                input.n,
                p.wilson_lower_95,
                input.cfg.min_sample_size,
                input.cfg.confidence_threshold,
            ),
            CouplingProse::PreEditExpected => messages::coupling_pre_edit(
                input.subject,
                &p.partner,
                p.co_change_count,
                input.n,
                p.wilson_lower_95,
                input.cfg.min_sample_size,
                input.cfg.confidence_threshold,
            ),
        };
        let signal = MonotonicSignal {
            key: format!(
                "coupling::{}::{}",
                input.subject.display(),
                p.partner.display()
            ),
            axes: vec![p.co_change_count, input.n],
        };
        let id = messages::coupling_id(input.subject, &p.partner);
        out.push((
            Finding::with_id(Layer::Coupling, input.severity, message, id),
            Some(signal),
        ));
    }
    out
}

/// Apply a parsed `[sensor]` block onto the in-memory config.
///
/// Pure: only data merging. Each subblock is independently optional;
/// fields left unset fall through to the in-code defaults.
pub fn apply_sensor_file(
    structure: &mut StructureCfg,
    complexity: &mut ComplexityCfg,
    budget_ramp: &mut BudgetRampCfg,
    cohesion: &mut CohesionCfg,
    file_s: &SensorFile,
) {
    if let Some(b) = file_s.budget_ramp.as_ref() {
        if let Some(v) = b.enabled {
            budget_ramp.enabled = v;
        }
    }
    if let Some(s) = file_s.structure.as_ref() {
        if let Some(v) = s.enabled {
            structure.enabled = v;
        }
        if let Some(v) = s.min_siblings {
            structure.min_siblings = v;
        }
        if let Some(v) = s.import_majority {
            structure.import_majority = v;
        }
        if let Some(v) = s.export_template_majority {
            structure.export_template_majority = v;
        }
        if let Some(v) = s.top_imports_to_show {
            structure.top_imports_to_show = v;
        }
        if let Some(v) = s.divergence_min_missing {
            structure.divergence_min_missing = v;
        }
        if let Some(v) = s.report_conformance {
            structure.report_conformance = v;
        }
        if let Some(v) = s.linescan_fallback {
            structure.linescan_fallback = v;
        }
        if let Some(v) = s.role_patterns.as_ref() {
            structure.role_patterns.clone_from(v);
        }
    }
    if let Some(c) = file_s.complexity.as_ref() {
        if let Some(v) = c.enabled {
            complexity.enabled = v;
        }
        if let Some(v) = c.nesting_ratio_threshold {
            complexity.nesting_ratio_threshold = v;
        }
        if let Some(v) = c.nesting_absolute_max {
            complexity.nesting_absolute_max = v;
        }
        if let Some(v) = c.loc_ratio_threshold {
            complexity.loc_ratio_threshold = v;
        }
        if let Some(v) = c.loc_absolute_max {
            complexity.loc_absolute_max = v;
        }
        if let Some(v) = c.min_directory_siblings {
            complexity.min_directory_siblings = v;
        }
        if let Some(v) = c.delta_warn_pct {
            complexity.delta_warn_pct = v;
        }
        if let Some(v) = c.delta_warn_abs {
            complexity.delta_warn_abs = v;
        }
    }
    if let Some(co) = file_s.cohesion.as_ref() {
        if let Some(v) = co.enabled {
            cohesion.enabled = v;
        }
        if let Some(v) = co.confidence_threshold {
            cohesion.confidence_threshold = v;
        }
        if let Some(v) = co.min_sample_size {
            cohesion.min_sample_size = v;
        }
        if let Some(v) = co.min_files_per_cluster {
            cohesion.min_files_per_cluster = v;
        }
    }
}

/// Apply a parsed `[health]` block onto the in-memory config.
///
/// Pure: only data merging. No tree-sitter, no I/O — that lives in
/// the call sites that actually run analysis.
pub fn apply_health_file(cfg: &mut HealthTsCfg, file_h: &HealthFile) {
    if let Some(ts) = file_h.ts.as_ref() {
        if let Some(enabled) = ts.enabled {
            cfg.enabled = enabled;
        }
        if let Some(patterns) = ts.patterns.as_ref() {
            cfg.patterns.clone_from(patterns);
        }
    }
}

/// Resolve configured pattern tokens to `HealthPattern` enums.
///
/// Silently drops any unknown tokens. Stable order is preserved so
/// `health.patterns_evaluated[]` in JSON output matches the user's
/// config.
#[must_use]
pub fn resolve_patterns(tokens: &[String]) -> Vec<HealthPattern> {
    tokens
        .iter()
        .filter_map(|t| HealthPattern::from_token(t))
        .collect()
}

/// Wrap a [`HealthFinding`] from `mmk-health` into the CLI's unified
/// `Finding` shape with the right severity by mode.
///
/// - Pre-edit: every Health finding is informational (the agent
///   hasn't acted yet, the message is "consider this neighbor").
/// - Review: Pattern C and BroadException are **Warn** (the
///   implementation moved without its test partner; or a broad
///   handler was added). Patterns A and B remain Info — they
///   surface architectural neighbors without demanding edits.
#[must_use]
pub fn health_to_finding(h: &HealthFinding, severity: Severity) -> Finding {
    let message = match h.pattern {
        HealthPattern::Registration => messages::health_registration(&h.subject, &h.related),
        HealthPattern::Service => messages::health_service(&h.subject, &h.related),
        HealthPattern::TestPair => messages::health_test_pair(&h.subject, &h.related),
        // BroadException's `related` field is empty by convention —
        // the finding is about the subject only. The detector tracks
        // the *delta*; we surface "1+" rather than re-reading the
        // detector's internal counter, since v0.7's HealthFinding
        // shape carries no numeric payload.
        HealthPattern::BroadException => messages::health_broad_exception(&h.subject, 1),
    };
    Finding::new(Layer::Health, severity, message)
}

/// Pick the severity for a Health finding given the call site.
///
/// Captured here so the rule lives in one place — drift would
/// otherwise mean Pattern C / BroadException silently downgrade
/// across review/pre-edit.
#[must_use]
pub const fn health_severity_for_review(p: HealthPattern) -> Severity {
    match p {
        HealthPattern::TestPair | HealthPattern::BroadException => Severity::Warn,
        HealthPattern::Registration | HealthPattern::Service => Severity::Info,
    }
}

/// Run the TypeScript Health adapter against `subject`.
///
/// Reads the file body from `repo_root / subject` once; if the read
/// fails (file isn't on disk yet, permissions issue) returns no
/// findings — Health is opportunistic, not load-bearing. `peer_paths`
/// should be repo-relative paths matching `analyze.loc.keys()`;
/// Pattern B's `read_to_string` resolves them against the process
/// CWD, which the CLI sets to the repo root. `head_body` carries the
/// file's content at HEAD when available — EVASION uses it to
/// compute the working-vs-HEAD broad-handler delta. `None` is
/// correct for new files (no HEAD blob) and for pre-edit (no diff
/// to score against yet).
///
/// `peer_paths` is supplemented here with the subject's working-tree
/// directory listing (and any `test/` subdirectory) before being
/// passed down. Reason: `analyze.loc.keys()` only contains paths
/// with recent churn — TestPair otherwise misses stable, untouched
/// test partners. The supplement is opportunistic; a missing
/// directory just yields no extra entries.
#[must_use]
pub fn analyze_health_for_subject(
    repo_root: &Path,
    subject: &Path,
    head_body: Option<&str>,
    peer_paths: &[PathBuf],
    enabled: &[HealthPattern],
) -> Vec<HealthFinding> {
    if !is_health_eligible_path(subject) {
        return Vec::new();
    }
    let abs_subject = repo_root.join(subject);
    let body = std::fs::read_to_string(&abs_subject).unwrap_or_default();
    let augmented = augment_peer_paths_with_working_tree(repo_root, subject, peer_paths);
    mmk_health::ts::analyze_ts(subject, &body, head_body, &augmented, enabled)
}

/// Add the subject's working-tree directory siblings (plus any
/// `test/` sibling subdirectory) to `base`. Deduplicates against
/// the existing entries; preserves their order. Pure: returns a new
/// owned vec.
fn augment_peer_paths_with_working_tree(
    repo_root: &Path,
    subject: &Path,
    base: &[PathBuf],
) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = base.to_vec();
    let mut seen: ahash::AHashSet<PathBuf> = base.iter().cloned().collect();
    let push = |p: PathBuf, out: &mut Vec<PathBuf>, seen: &mut ahash::AHashSet<PathBuf>| {
        if seen.insert(p.clone()) {
            out.push(p);
        }
    };
    // Same-directory siblings.
    for s in list_directory_siblings(repo_root, subject) {
        push(s, &mut out, &mut seen);
    }
    // `test/` subdirectories: TestPair partners may live at the
    // same level (`<dir>/test/`) or under a mirrored `test/` at any
    // ancestor (vscode-style, e.g. `vs/editor/test/common/commands/`
    // mirroring `vs/editor/common/commands/`). Enumerate every such
    // directory and add the file it contains for the matching stem.
    let parent = subject.parent().unwrap_or_else(|| Path::new(""));
    let stem = subject
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    for dir_rel in mmk_health::ts::test_pair::mirrored_test_parents(parent) {
        let abs_dir = repo_root.join(&dir_rel);
        let Ok(entries) = std::fs::read_dir(&abs_dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(ft) = entry.file_type() else { continue };
            if !ft.is_file() {
                continue;
            }
            let name = entry.file_name();
            let Some(name_str) = name.to_str() else {
                continue;
            };
            // Cheap pre-filter: stem must match and the suffix must
            // look like a test/spec partner. Keeps unrelated files
            // in `test/` directories out of `peer_paths`.
            if !name_str.starts_with(stem) {
                continue;
            }
            push(dir_rel.join(name_str), &mut out, &mut seen);
        }
    }
    out
}

/// Extensions the Health TypeScript adapter knows how to parse.
///
/// Includes `.ts`, `.tsx`, `.js`, and `.jsx` — the TSX grammar
/// (selected per-file in `mmk_health::ts::parse_for`) is a superset
/// that handles JSX-bearing files in either language. Pub-crate so
/// `commands::review` can use the same predicate when filtering
/// changed files for HEAD-blob fetching, instead of duplicating the
/// match arms.
pub(crate) fn is_health_eligible_path(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| matches!(e, "ts" | "tsx" | "js" | "jsx"))
}

// ---- STRUCTURE / COMPLEXITY plumbing ---------------------------

/// List the working-tree paths in `subject`'s parent directory.
///
/// One level deep, not recursive. Reads the directory directly so
/// the result includes untracked files — the same shape rule the
/// working-tree review uses.
///
/// Returns paths *relative to `repo_root`* matching the entries in
/// the analyzer's `loc.keys()`. Errors (missing directory, I/O)
/// silently return `Vec::new()` — STRUCTURE / COMPLEXITY are
/// opportunistic, not load-bearing.
#[must_use]
pub fn list_directory_siblings(repo_root: &Path, subject: &Path) -> Vec<PathBuf> {
    let dir_rel = subject.parent().unwrap_or_else(|| Path::new(""));
    let abs_dir = repo_root.join(dir_rel);
    let Ok(entries) = std::fs::read_dir(&abs_dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_file() {
            continue;
        }
        let name = entry.file_name();
        let Some(name_str) = name.to_str() else {
            continue;
        };
        out.push(dir_rel.join(name_str));
    }
    out
}

/// Read every sibling's body into a [`mmk_core::sensors::FilesMap`].
/// Files that fail to read silently fall out — sensors handle a
/// missing entry by treating that sibling as not contributing.
#[must_use]
pub fn load_bodies(repo_root: &Path, paths: &[PathBuf]) -> mmk_core::sensors::FilesMap {
    let mut out = mmk_core::sensors::FilesMap::default();
    for p in paths {
        let abs = repo_root.join(p);
        if let Ok(body) = std::fs::read_to_string(&abs) {
            out.insert(p.clone(), body);
        }
    }
    out
}

/// Translate a [`mmk_core::sensors::StructureFinding`] into a CLI
/// [`Finding`]. Severity is `Info` — STRUCTURE is suggestive, not a
/// gate; the agent reads the convention and decides.
#[must_use]
pub fn structure_to_finding(
    f: &mmk_core::sensors::StructureFinding,
    cap: usize,
    majority_pct: u32,
) -> Finding {
    structure_to_finding_with_signal(f, cap, majority_pct).0
}

/// Same as [`structure_to_finding`] but tagged for the monotonic gate.
///
/// `ReviewDivergent` findings carry a signal keyed
/// `structure::<path>` with axes
/// `[missing_imports_count, missing_templates_count]`. Other kinds
/// (PreEdit / Conforming) carry `None` — they don't repeat-fire
/// across edits, so the whole-set dedup is enough.
#[must_use]
pub fn structure_to_finding_with_signal(
    f: &mmk_core::sensors::StructureFinding,
    cap: usize,
    majority_pct: u32,
) -> (Finding, Option<MonotonicSignal>) {
    use mmk_core::sensors::StructureFindingKind as K;
    let dir = f
        .path
        .parent()
        .map_or_else(|| PathBuf::from(""), Path::to_path_buf);
    let common_imports: Vec<String> = f
        .convention
        .common_imports
        .iter()
        .map(|i| i.source.clone())
        .collect();
    let total = common_imports.len();
    let templates: Vec<String> = f.convention.common_export_templates.clone();

    let bundle = messages::StructurePreEdit {
        path: &f.path,
        dir: &dir,
        sibling_count: f.convention.sibling_count,
        shape_ext: &f.convention.shape_ext,
        shape_suffix: &f.convention.shape_suffix,
        common_imports: &common_imports,
        total_common_imports: total,
        cap,
        majority_pct,
        common_templates: &templates,
    };
    let (severity, message, signal) = match &f.kind {
        K::PreEditNew => (
            Severity::Info,
            messages::structure_pre_edit_new(&bundle),
            None,
        ),
        K::PreEditExisting => (
            Severity::Info,
            messages::structure_pre_edit_existing(&bundle),
            None,
        ),
        K::ReviewConforming => (
            Severity::Ok,
            messages::structure_review_conforming(&f.path, &dir, f.convention.sibling_count),
            None,
        ),
        K::ReviewDivergent {
            missing_imports,
            missing_templates,
        } => {
            let missing_sources: Vec<String> =
                missing_imports.iter().map(|i| i.source.clone()).collect();
            let imports_count = u32::try_from(missing_imports.len()).unwrap_or(u32::MAX);
            let templates_count = u32::try_from(missing_templates.len()).unwrap_or(u32::MAX);
            let key = format!("structure::{}", f.path.display());
            // Role-file demotion (v0.8): factories / registrations /
            // contribution files legitimately diverge from sibling
            // shape conventions. Demote Warn → Info and reframe the
            // prose to flag the role status, so the agent reads
            // "expected divergence" rather than "fix this."
            if f.is_role {
                (
                    Severity::Info,
                    messages::structure_review_role(
                        &f.path,
                        &dir,
                        f.convention.sibling_count,
                        &f.convention.shape_ext,
                        &f.convention.shape_suffix,
                    ),
                    Some(MonotonicSignal {
                        key,
                        axes: vec![imports_count, templates_count],
                    }),
                )
            } else {
                (
                    Severity::Warn,
                    messages::structure_review_divergent(
                        &f.path,
                        &missing_sources,
                        total,
                        missing_templates,
                    ),
                    Some(MonotonicSignal {
                        key,
                        axes: vec![imports_count, templates_count],
                    }),
                )
            }
        }
    };
    (Finding::new(Layer::Structure, severity, message), signal)
}

/// Translate a [`mmk_core::sensors::ComplexityFinding`] into a CLI
/// [`Finding`] with delta-weighted severity.
///
/// Severity rules (v0.8):
/// - new file / new function (no HEAD baseline) → `Warn`
/// - existing function with `Δ ≥ delta_warn_pct × head_actual` OR
///   `Δ ≥ delta_warn_abs` → `Warn`
/// - otherwise (small Δ on a pre-existing problem) → `Info`
///
/// Lets the agent see "you made it materially worse" (Warn) vs.
/// "you nudged an inherited problem" (Info) without losing the fact
/// that a borderline metric is still over cap.
#[must_use]
pub fn complexity_to_finding(
    f: &mmk_core::sensors::ComplexityFinding,
    cfg: &mmk_config::ComplexityCfg,
) -> Finding {
    use mmk_core::sensors::ComplexityFindingKind as K;
    let message = match f.kind {
        K::Nesting => messages::complexity_review_nesting(
            &f.path,
            &f.function,
            f.actual,
            f.cap,
            f.head_actual,
            f.directory_median,
        ),
        K::Size => messages::complexity_review_size(
            &f.path,
            &f.function,
            f.actual,
            f.cap,
            f.head_actual,
            f.directory_median,
        ),
    };
    let severity = complexity_severity(f, cfg);
    Finding::new(Layer::Complexity, severity, message)
}

/// Pick the severity for a complexity finding given the cfg
/// thresholds. Pulled out so it's unit-testable in isolation —
/// the formatter is otherwise pure rendering.
#[must_use]
pub fn complexity_severity(
    f: &mmk_core::sensors::ComplexityFinding,
    cfg: &mmk_config::ComplexityCfg,
) -> Severity {
    let Some(head) = f.head_actual else {
        return Severity::Warn;
    };
    let delta = f.actual.saturating_sub(head);
    let pct_threshold = (f64::from(head) * cfg.delta_warn_pct).ceil() as u32;
    if delta >= cfg.delta_warn_abs || delta >= pct_threshold {
        Severity::Warn
    } else {
        Severity::Info
    }
}

/// Apply the per-key monotonic-worsening gate to a list of findings
/// paired with optional signals.
///
/// Findings tagged with a [`MonotonicSignal`] are dropped when the
/// prior emission for the same key is within TTL AND no axis has
/// strictly worsened since. Findings without a signal pass through
/// unchanged. `no_dedup = true` (the `--no-dedup` flag) bypasses the
/// gate entirely so eval / replay runs see every fire.
///
/// Lives in `common.rs` so review and pre-edit share one home for
/// the git-dir discovery + load/save plumbing — the per-fire dedup
/// shape is the same in both.
#[must_use]
pub fn apply_monotonic_gate(
    cwd: &Path,
    head_sha: Option<&str>,
    items: Vec<(Finding, Option<MonotonicSignal>)>,
    no_dedup: bool,
) -> Vec<Finding> {
    if no_dedup {
        return items.into_iter().map(|(f, _)| f).collect();
    }
    let git_dir = mmk_git::discover_work_dir(cwd).and_then(|wd| {
        let g = wd.join(".git");
        g.exists().then_some(g)
    });
    let Some(git_dir) = git_dir else {
        return items.into_iter().map(|(f, _)| f).collect();
    };
    let Some(path) = crate::monotonic::store_path(&git_dir) else {
        return items.into_iter().map(|(f, _)| f).collect();
    };
    let now = crate::dedup::now_unix();
    let ttl = crate::monotonic::ttl_seconds();
    let mut store = crate::monotonic::load(&path, now, ttl);
    let (kept, recorded) = crate::monotonic::apply(items, &store, now, ttl);
    if !recorded.is_empty() {
        for sig in recorded {
            crate::monotonic::record(&mut store, sig.key, sig.axes, now, head_sha.unwrap_or(""));
        }
        crate::monotonic::cap_lru(&mut store);
        crate::monotonic::save(&path, &store);
    }
    kept
}

#[cfg(test)]
mod tests {
    use super::{
        apply_coupling_file, apply_health_file, complexity_severity, health_severity_for_review,
        health_to_finding, resolve_patterns, CouplingDeprecation,
    };
    use mmk_config::{CouplingCfg, CouplingFile, HealthFile, HealthTsCfg, HealthTsFile};
    use mmk_health::{HealthFinding, HealthPattern};
    use std::path::PathBuf;

    use crate::output::findings::{Layer, Severity};

    #[test]
    fn legacy_threshold_routes_to_confidence_and_returns_deprecation() {
        let mut cfg = CouplingCfg::default();
        let file = CouplingFile {
            threshold: Some(0.42),
            ..CouplingFile::default()
        };
        let warns = apply_coupling_file(&mut cfg, &file);
        assert!((cfg.confidence_threshold - 0.42).abs() < 1e-12);
        assert!((cfg.threshold - 0.42).abs() < 1e-12);
        assert_eq!(warns, vec![CouplingDeprecation::LegacyThreshold]);
    }

    #[test]
    fn explicit_confidence_threshold_does_not_warn() {
        let mut cfg = CouplingCfg::default();
        let file = CouplingFile {
            confidence_threshold: Some(0.30),
            ..CouplingFile::default()
        };
        let warns = apply_coupling_file(&mut cfg, &file);
        assert!((cfg.confidence_threshold - 0.30).abs() < 1e-12);
        assert!(warns.is_empty(), "no deprecation expected; got {warns:?}");
    }

    #[test]
    fn explicit_confidence_overrides_legacy_when_both_set() {
        // Real-world migration: a user adding the new key while the
        // old one is still in their toml. The new key wins; the
        // deprecation still fires so the user sees they can drop the
        // old key.
        let mut cfg = CouplingCfg::default();
        let file = CouplingFile {
            threshold: Some(0.10),
            confidence_threshold: Some(0.30),
            ..CouplingFile::default()
        };
        let warns = apply_coupling_file(&mut cfg, &file);
        assert!((cfg.confidence_threshold - 0.30).abs() < 1e-12);
        assert_eq!(warns, vec![CouplingDeprecation::LegacyThreshold]);
    }

    #[test]
    fn min_sample_size_and_ignore_partners_apply_without_warning() {
        let mut cfg = CouplingCfg::default();
        let file = CouplingFile {
            min_sample_size: Some(8),
            ignore_partners: vec!["**/CHANGELOG.md".into()],
            ..CouplingFile::default()
        };
        let warns = apply_coupling_file(&mut cfg, &file);
        assert_eq!(cfg.min_sample_size, 8);
        assert_eq!(cfg.ignore_partners, vec!["**/CHANGELOG.md".to_string()]);
        assert!(warns.is_empty());
    }

    #[test]
    fn empty_ignore_partners_does_not_clear_existing() {
        let mut cfg = CouplingCfg {
            ignore_partners: vec!["**/keep.md".into()],
            ..CouplingCfg::default()
        };
        let file = CouplingFile::default();
        let warns = apply_coupling_file(&mut cfg, &file);
        assert_eq!(cfg.ignore_partners, vec!["**/keep.md".to_string()]);
        assert!(warns.is_empty());
    }

    #[test]
    fn apply_health_file_flips_enabled_and_replaces_patterns() {
        let mut cfg = HealthTsCfg::default();
        let file = HealthFile {
            ts: Some(HealthTsFile {
                enabled: Some(true),
                patterns: Some(vec!["test_pair".into()]),
            }),
        };
        apply_health_file(&mut cfg, &file);
        assert!(cfg.enabled);
        assert_eq!(cfg.patterns, vec!["test_pair".to_string()]);
    }

    #[test]
    fn apply_health_file_partial_block_only_updates_provided_fields() {
        let mut cfg = HealthTsCfg::default();
        let original_patterns = cfg.patterns.clone();
        let file = HealthFile {
            ts: Some(HealthTsFile {
                enabled: Some(true),
                patterns: None,
            }),
        };
        apply_health_file(&mut cfg, &file);
        assert!(cfg.enabled);
        assert_eq!(
            cfg.patterns, original_patterns,
            "patterns left unset must be untouched"
        );
    }

    #[test]
    fn resolve_patterns_drops_unknown_tokens() {
        let toks = vec![
            "test_pair".to_string(),
            "totally_made_up".to_string(),
            "registration".to_string(),
        ];
        let resolved = resolve_patterns(&toks);
        assert_eq!(
            resolved,
            vec![HealthPattern::TestPair, HealthPattern::Registration]
        );
    }

    #[test]
    fn health_severity_for_review_warns_on_test_pair_and_broad_exception() {
        assert_eq!(
            health_severity_for_review(HealthPattern::TestPair),
            Severity::Warn
        );
        assert_eq!(
            health_severity_for_review(HealthPattern::BroadException),
            Severity::Warn
        );
        assert_eq!(
            health_severity_for_review(HealthPattern::Registration),
            Severity::Info
        );
        assert_eq!(
            health_severity_for_review(HealthPattern::Service),
            Severity::Info
        );
    }

    #[test]
    fn health_to_finding_renders_layer_health_with_subject_and_related() {
        let h = HealthFinding {
            pattern: HealthPattern::TestPair,
            subject: PathBuf::from("src/foo.ts"),
            related: vec![PathBuf::from("src/foo.test.ts")],
        };
        let f = health_to_finding(&h, Severity::Warn);
        assert_eq!(f.layer, Layer::Health);
        assert_eq!(f.severity, Severity::Warn);
        assert!(f.message.contains("src/foo.ts"));
        assert!(f.message.contains("src/foo.test.ts"));
    }

    #[test]
    fn complexity_severity_demotes_tiny_delta_on_existing_function_to_info() {
        // A +1-LOC edit to a pre-existing 46-LOC function (cap 80,
        // ratio gate) is Info: the agent inherited the over-cap
        // state, the contribution is below both delta thresholds.
        use mmk_core::sensors::{ComplexityFinding, ComplexityFindingKind};
        let f = ComplexityFinding {
            path: PathBuf::from("a/b.ts"),
            function: "Lane::moveCardTo".to_owned(),
            kind: ComplexityFindingKind::Size,
            actual: 47,
            cap: 80,
            head_actual: Some(46),
            directory_median: Some(22),
        };
        let cfg = mmk_config::ComplexityCfg::default();
        assert_eq!(
            complexity_severity(&f, &cfg),
            Severity::Info,
            "+1 LOC delta on a 46-LOC function must be Info, not Warn"
        );
    }

    #[test]
    fn complexity_severity_warns_when_delta_clears_absolute_threshold() {
        // Control: +25 LOC clears delta_warn_abs (default 20) and so
        // earns Warn even on a pre-existing over-cap function.
        use mmk_core::sensors::{ComplexityFinding, ComplexityFindingKind};
        let f = ComplexityFinding {
            path: PathBuf::from("a/b.ts"),
            function: "Lane::moveCardTo".to_owned(),
            kind: ComplexityFindingKind::Size,
            actual: 71,
            cap: 80,
            head_actual: Some(46),
            directory_median: Some(22),
        };
        let cfg = mmk_config::ComplexityCfg::default();
        assert_eq!(
            complexity_severity(&f, &cfg),
            Severity::Warn,
            "+25 LOC delta crosses delta_warn_abs — must be Warn"
        );
    }
}
