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
use mmk_config::{ConfigFile, CouplingCfg, CouplingFile, HealthFile, HealthTsCfg};
use mmk_core::CouplingEntry;
use mmk_health::{HealthFinding, HealthPattern};
use std::path::{Path, PathBuf};

use crate::output::findings::{Finding, Layer, Severity};

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
/// Review and pre-edit answer related-but-different questions ("you
/// edited A but missed B" vs "you're about to edit A; B has
/// historically come along"), so the wording differs. Capturing the
/// choice as an enum keeps the vocabulary in one place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CouplingProse {
    /// `<subject> edited; expected partner <X> not touched (...)`.
    ReviewMissed,
    /// `<subject> historically co-changes with <X> (...)`.
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
        let body = format!(
            "{}/{} = {:.0}% historical co-edit, Wilson 95% lower {:.2}",
            p.co_change_count,
            input.n,
            p.conditional_probability * 100.0,
            p.wilson_lower_95,
        );
        let message = match input.prose {
            CouplingProse::ReviewMissed => format!(
                "{} edited; expected partner {} not touched ({body})",
                input.subject.display(),
                p.partner.display(),
            ),
            CouplingProse::PreEditExpected => format!(
                "{} historically co-changes with {} ({body})",
                input.subject.display(),
                p.partner.display(),
            ),
        };
        out.push(Finding::new(Layer::Coupling, input.severity, message));
    }
    out
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
/// - Review: Pattern C is **Warn** (the implementation was edited
///   but its test partner is still untouched in the diff). Patterns
///   A and B remain Info — they surface architectural neighbors
///   without demanding edits.
#[must_use]
pub fn health_to_finding(h: &HealthFinding, severity: Severity) -> Finding {
    let label = match h.pattern {
        HealthPattern::Registration => {
            "matches the action-registration pattern; nearby precedents:"
        }
        HealthPattern::Service => "matches the service-decl pattern; consumers:",
        HealthPattern::TestPair => "has a test partner not touched in this diff:",
    };
    let related: Vec<String> = h.related.iter().map(|p| p.display().to_string()).collect();
    let message = format!("{} {label} {}", h.subject.display(), related.join(", "));
    Finding::new(Layer::Health, severity, message)
}

/// Pick the severity for a Health finding given the call site.
///
/// Captured here so the rule lives in one place — drift would
/// otherwise mean Pattern C silently downgrades across
/// review/pre-edit.
#[must_use]
pub const fn health_severity_for_review(p: HealthPattern) -> Severity {
    match p {
        HealthPattern::TestPair => Severity::Warn,
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
/// CWD, which the CLI sets to the repo root.
#[must_use]
pub fn analyze_health_for_subject(
    repo_root: &Path,
    subject: &Path,
    peer_paths: &[PathBuf],
    enabled: &[HealthPattern],
) -> Vec<HealthFinding> {
    if !is_typescript_path(subject) {
        return Vec::new();
    }
    let abs_subject = repo_root.join(subject);
    let body = std::fs::read_to_string(&abs_subject).unwrap_or_default();
    mmk_health::ts::analyze_ts(subject, &body, peer_paths, enabled)
}

fn is_typescript_path(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e == "ts" || e == "tsx")
}

#[cfg(test)]
mod tests {
    use super::{
        apply_coupling_file, apply_health_file, health_severity_for_review, health_to_finding,
        resolve_patterns, CouplingDeprecation,
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
    fn health_severity_for_review_only_warns_on_test_pair() {
        assert_eq!(
            health_severity_for_review(HealthPattern::TestPair),
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
}
