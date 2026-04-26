//! `mmk eval` — sample N recent commits and aggregate a noise-floor
//! report.
//!
//! Built so a new adopter can answer "do my mmk defaults fit my
//! repo?" in under a minute. The aggregation surfaces three deciding
//! signals: firing rate, layer mix, and the Wilson-95 %-lower-bound
//! distribution of COUPLING findings — letting the user decide
//! whether to raise `[coupling] confidence_threshold`.

use anyhow::{Context, Result};
use mmk_config::Config;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use crate::args::{EvalArgs, Format};
use crate::commands::common::{apply_coupling_file, apply_health_file, load_config_file};
use crate::commands::review::{
    bulk_self_findings, collect_diff, compute_findings, ChangedFile, ReviewMode,
};
use crate::output::findings::{Finding, Layer};

pub fn run<O: Write, E: Write>(args: &EvalArgs, stdout: &mut O, stderr: &mut E) -> Result<()> {
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

    if args.verbose {
        match &file_path {
            Some(p) => writeln!(stderr, "loaded config from {}", p.display())?,
            None => writeln!(stderr, "no mokumokuren.toml found; running with defaults")?,
        }
    }

    let started = Instant::now();
    let shas = list_recent_commits(&cwd, args.sample)?;
    if shas.is_empty() {
        anyhow::bail!("no non-merge commits found in the current repository");
    }

    // One analyze pass anchored on HEAD; same baseline for every
    // sampled commit. Avoids the K × analyze cost — the reason eval
    // is fast enough to run interactively.
    let analysis = mmk_git::analyze(&cwd, &cfg)?;
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

    let mut report = AggregateReport {
        commits_sampled: shas.len(),
        ..AggregateReport::default()
    };

    for sha in &shas {
        let changed = match commit_diff(&cwd, sha) {
            Ok(c) => c,
            Err(e) => {
                if args.verbose {
                    let _ = writeln!(stderr, "skipping {sha}: {e:#}");
                }
                continue;
            }
        };
        if changed.is_empty() {
            continue;
        }

        let files_n = u32::try_from(changed.len()).unwrap_or(u32::MAX);
        let lines_n: u64 = changed.iter().map(|c| c.added + c.deleted).sum();
        let findings = if files_n > cfg.bulk.max_files || lines_n > u64::from(cfg.bulk.max_lines) {
            bulk_self_findings(files_n, lines_n, &cfg)
        } else {
            compute_findings(
                &changed,
                &ranked,
                &analysis.commits,
                &commits_touching,
                &cfg,
                args.top,
            )
        };
        report.absorb(&findings);
    }

    if args.learn {
        report.learn_suggestions = synthesize_learn_suggestions(
            &report.partner_subjects,
            &commits_touching,
            &analysis.commits,
        );
    }

    let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    report.duration_ms = duration_ms;
    report.threshold = cfg.coupling.confidence_threshold;

    match args.format {
        Format::Text => write_text(stdout, &report)?,
        Format::Json => write_json(stdout, &report)?,
    }
    Ok(())
}

fn list_recent_commits(cwd: &Path, n: usize) -> Result<Vec<String>> {
    let out = Command::new("git")
        .args(["log", "--no-merges", &format!("-{n}"), "--format=%H"])
        .current_dir(cwd)
        .output()
        .context("failed to invoke `git log` — is git on PATH?")?;
    if !out.status.success() {
        anyhow::bail!(
            "git log exited with {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::to_owned)
        .collect())
}

fn commit_diff(cwd: &Path, sha: &str) -> Result<Vec<ChangedFile>> {
    // Stub up a ReviewArgs with --commit so we can re-use the same
    // diff parser; constructing it once here is cheaper than copying
    // the parsing logic.
    let args = crate::args::ReviewArgs {
        staged: false,
        range: None,
        commit: Some(sha.to_owned()),
        since: "180days".into(),
        top: 20,
        format: Format::Text,
        ignores: Vec::new(),
        config: None,
        verbose: false,
        coupling_threshold: None,
        blast_radius_threshold: None,
        gate: crate::args::Gate::None,
    };
    collect_diff(cwd, ReviewMode::Commit, &args)
}

#[derive(Debug, Default, Serialize)]
struct AggregateReport {
    commits_sampled: usize,
    commits_with_findings: usize,
    total_findings: usize,
    by_layer: BTreeMap<&'static str, usize>,
    /// COUPLING-finding count per partner path. Only the missed
    /// partner (the second `<file>` in the message) is counted.
    noisy_partners: BTreeMap<String, usize>,
    /// Bucketed Wilson-95 %-lower-bound distribution for COUPLING
    /// findings: `"0.00-0.20"`, `"0.20-0.40"`, `"0.40+"`.
    wilson_lower_buckets: BTreeMap<&'static str, usize>,
    /// Set of distinct subjects that fired COUPLING for each
    /// partner. Drives the `--learn` heuristic: a partner blamed
    /// across many unrelated subjects is system-level noise, not
    /// pairwise coupling. Skipped from JSON output (callers consume
    /// the synthesized `learn_suggestions` block, not the raw set).
    #[serde(skip)]
    partner_subjects: BTreeMap<String, BTreeSet<String>>,
    /// Effective `coupling.confidence_threshold` for context.
    threshold: f64,
    /// Suggested `[coupling] ignore_partners` additions, populated
    /// when `--learn` is set. Empty otherwise. Each entry is a
    /// glob-style partner path plus the supporting evidence.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    learn_suggestions: Vec<LearnSuggestion>,
    duration_ms: u64,
}

/// One adoption suggestion from `--learn`. Includes the supporting
/// evidence so the user can sanity-check the heuristic before
/// pasting the path into their `mokumokuren.toml`.
#[derive(Debug, Clone, Serialize)]
pub struct LearnSuggestion {
    pub partner: String,
    /// Number of distinct subject files that fired this partner.
    pub subject_count: usize,
    /// `co_change(subject, partner) / commits_touching(partner)`,
    /// averaged over the firing subjects. Low values indicate the
    /// partner moves for reasons unrelated to any one subject — the
    /// system-level-noise signature.
    pub mean_inverse_conditional_probability: f64,
}

impl AggregateReport {
    fn absorb(&mut self, findings: &[Finding]) {
        if !findings.is_empty() {
            self.commits_with_findings += 1;
        }
        for f in findings {
            self.total_findings += 1;
            *self.by_layer.entry(layer_label(f.layer)).or_default() += 1;
            if f.layer == Layer::Coupling {
                let partner = parse_partner(&f.message);
                let subject = parse_subject(&f.message);
                if let Some(p) = partner.as_deref() {
                    *self.noisy_partners.entry(p.to_owned()).or_default() += 1;
                }
                if let (Some(p), Some(s)) = (partner, subject) {
                    self.partner_subjects.entry(p).or_default().insert(s);
                }
                if let Some(w) = parse_wilson_lower(&f.message) {
                    *self
                        .wilson_lower_buckets
                        .entry(wilson_lower_bucket(w))
                        .or_default() += 1;
                }
            }
        }
    }
}

const fn layer_label(l: Layer) -> &'static str {
    match l {
        Layer::Hotspot => "hotspot",
        Layer::Coupling => "coupling",
        Layer::Drift => "drift",
        Layer::Budget => "budget",
        Layer::Health => "health",
        Layer::Anchor => "anchor",
    }
}

/// COUPLING messages come in two shapes:
///   review:    `<file> edited; expected partner <X> not touched (...)`
///   pre-edit:  `<file> historically co-changes with <X> (...)`
/// Pull `<X>` between "partner " / "with " and the next sentinel
/// (" not touched" for review, " (" for pre-edit).
fn parse_partner(msg: &str) -> Option<String> {
    let (after_keyword, end_sentinel) = if let Some(idx) = msg.find("partner ") {
        (idx + "partner ".len(), " not touched")
    } else if let Some(idx) = msg.find("with ") {
        (idx + "with ".len(), " (")
    } else {
        return None;
    };
    let rest = &msg[after_keyword..];
    let end = rest.find(end_sentinel)?;
    Some(rest[..end].trim().to_owned())
}

/// Extract the subject (the file the user edited / queried) from a
/// COUPLING message. The subject is the leading path before either
/// " edited;" (review) or " historically " (pre-edit).
fn parse_subject(msg: &str) -> Option<String> {
    for sentinel in [" edited;", " historically "] {
        if let Some(end) = msg.find(sentinel) {
            return Some(msg[..end].trim().to_owned());
        }
    }
    None
}

/// Extract the Wilson 95 % lower bound from a COUPLING message.
/// The `"Wilson 95% lower X.YZ"` suffix is appended by both the
/// review and pre-edit emitters.
fn parse_wilson_lower(msg: &str) -> Option<f64> {
    let key = "Wilson 95% lower ";
    let idx = msg.find(key)? + key.len();
    let rest = &msg[idx..];
    let end = rest.find(')').unwrap_or(rest.len());
    rest[..end].trim().parse().ok()
}

/// Bucket a Wilson 95 % lower bound into a coarse band. Boundaries
/// land on the default `confidence_threshold = 0.20` and the next
/// round step at 0.40 — gives a "below firing threshold / just
/// above / well above" visualization.
const fn wilson_lower_bucket(w: f64) -> &'static str {
    if w < 0.20 {
        "0.00-0.20"
    } else if w < 0.40 {
        "0.20-0.40"
    } else {
        "0.40+"
    }
}

/// Minimum distinct subjects a partner must fire across to be a
/// `--learn` candidate. Set conservatively so a 50-commit sample on
/// a small repo can still produce useful suggestions.
const LEARN_MIN_SUBJECT_COUNT: usize = 3;

/// Synthesize `--learn` suggestions from the post-aggregation state.
///
/// Surfaces every partner blamed across `≥ LEARN_MIN_SUBJECT_COUNT`
/// distinct subjects. Breadth alone is the gate: legit pairwise
/// coupling fires from one subject's history, so a partner appearing
/// across multiple unrelated subjects is the noise pattern (CHANGELOG,
/// archived-version snapshots, lockstep manifest files).
///
/// `mean_inverse_conditional_probability` is reported as evidence
/// rather than used as a filter — it would mis-classify legit
/// 1-to-N parent-file patterns (e.g. an `index.ts` re-exporting many
/// children) as noise. The user reads the suggestion + evidence and
/// decides whether to add the path to `ignore_partners`.
fn synthesize_learn_suggestions(
    partner_subjects: &BTreeMap<String, BTreeSet<String>>,
    commits_touching: &ahash::AHashMap<PathBuf, u32>,
    commits: &[mmk_core::types::Commit],
) -> Vec<LearnSuggestion> {
    let mut out = Vec::new();
    for (partner, subjects) in partner_subjects {
        if subjects.len() < LEARN_MIN_SUBJECT_COUNT {
            continue;
        }
        let partner_path = PathBuf::from(partner);
        let n_partner = commits_touching.get(&partner_path).copied().unwrap_or(0);
        let mean_inv = if n_partner == 0 {
            0.0
        } else {
            let sum_inv: f64 = subjects
                .iter()
                .map(|s| {
                    let co = co_change_count(commits, &PathBuf::from(s), &partner_path);
                    f64::from(co) / f64::from(n_partner)
                })
                .sum();
            sum_inv / subjects.len() as f64
        };
        out.push(LearnSuggestion {
            partner: partner.clone(),
            subject_count: subjects.len(),
            mean_inverse_conditional_probability: mean_inv,
        });
    }
    // Highest-breadth suggestions first; tie-break on path so output is stable.
    out.sort_by(|a, b| {
        b.subject_count
            .cmp(&a.subject_count)
            .then_with(|| a.partner.cmp(&b.partner))
    });
    out
}

/// Count distinct commits touching both `a` and `b`. One pass over
/// the window — we only call this for the handful of `--learn`
/// candidates so the cost stays in the noise.
fn co_change_count(commits: &[mmk_core::types::Commit], a: &Path, b: &Path) -> u32 {
    let mut n = 0_u32;
    for c in commits {
        let mut has_a = false;
        let mut has_b = false;
        for d in &c.deltas {
            if d.path == a {
                has_a = true;
            }
            if d.path == b {
                has_b = true;
            }
            if has_a && has_b {
                break;
            }
        }
        if has_a && has_b {
            n = n.saturating_add(1);
        }
    }
    n
}

fn write_text<W: Write>(w: &mut W, r: &AggregateReport) -> Result<()> {
    let secs = r.duration_ms as f64 / 1000.0;
    writeln!(
        w,
        "[mmk eval] sampled {} commits in {secs:.1}s",
        r.commits_sampled
    )?;
    writeln!(w)?;

    let pct = if r.commits_sampled == 0 {
        0.0
    } else {
        100.0 * r.commits_with_findings as f64 / r.commits_sampled as f64
    };
    let median = if r.commits_with_findings == 0 {
        0
    } else {
        r.total_findings / r.commits_with_findings.max(1)
    };
    writeln!(
        w,
        "  firing rate:           {pct:.0}% ({}/{})",
        r.commits_with_findings, r.commits_sampled
    )?;
    writeln!(w, "  median findings/commit: {median}")?;

    let total = r.total_findings.max(1);
    let mut layer_pcts: Vec<(String, usize)> = r
        .by_layer
        .iter()
        .map(|(k, v)| {
            (
                format!("{:.0}% {}", 100.0 * *v as f64 / total as f64, k),
                *v,
            )
        })
        .collect();
    layer_pcts.sort_by(|a, b| b.1.cmp(&a.1));
    let mix = layer_pcts
        .iter()
        .map(|(s, _)| s.clone())
        .collect::<Vec<_>>()
        .join(", ");
    writeln!(w, "  layer mix:             {mix}")?;
    writeln!(w)?;

    if !r.noisy_partners.is_empty() {
        writeln!(w, "  top noisy partners (in COUPLING findings):")?;
        let mut sorted: Vec<(&String, &usize)> = r.noisy_partners.iter().collect();
        sorted.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
        for (partner, count) in sorted.iter().take(10) {
            writeln!(w, "    {count:>3}  {partner}")?;
        }
        writeln!(w)?;
    }

    if !r.wilson_lower_buckets.is_empty() {
        writeln!(w, "  Wilson 95% lower distribution (coupling findings):")?;
        let total_coupling: usize = r.wilson_lower_buckets.values().sum();
        for bucket in ["0.00-0.20", "0.20-0.40", "0.40+"] {
            let n = r.wilson_lower_buckets.get(bucket).copied().unwrap_or(0);
            let pct = if total_coupling == 0 {
                0.0
            } else {
                100.0 * n as f64 / total_coupling as f64
            };
            let bars = "#".repeat((pct / 5.0) as usize);
            writeln!(w, "    {bucket:<10}  {bars:<20} {pct:.0}%")?;
        }
        writeln!(w)?;
    }

    if !r.learn_suggestions.is_empty() {
        writeln!(w, "suggested mokumokuren.toml additions:")?;
        writeln!(w)?;
        writeln!(w, "  [coupling]")?;
        writeln!(w, "  ignore_partners = [")?;
        for s in &r.learn_suggestions {
            writeln!(
                w,
                "      {:?},  # fires across {} unrelated subjects \
                 (mean P(subject|partner) {:.2})",
                s.partner, s.subject_count, s.mean_inverse_conditional_probability,
            )?;
        }
        writeln!(w, "  ]")?;
        writeln!(w)?;
    }

    Ok(())
}

fn write_json<W: Write>(w: &mut W, r: &AggregateReport) -> Result<()> {
    serde_json::to_writer_pretty(&mut *w, r)?;
    writeln!(w)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{parse_partner, parse_subject, parse_wilson_lower, wilson_lower_bucket};

    #[test]
    fn parse_partner_extracts_review_partner_without_trailing_text() {
        // Regression: an earlier version sliced from "partner " all
        // the way to the next paren, swallowing " not touched" into
        // the captured partner — which made `noisy_partners` keys
        // like "Cargo.toml not touched" instead of "Cargo.toml".
        let msg = "core/a.rs edited; expected partner core/b.rs not touched \
                   (54/203 = 27% historical co-edit, Wilson 95% lower 0.21)";
        assert_eq!(parse_partner(msg).as_deref(), Some("core/b.rs"));
    }

    #[test]
    fn parse_partner_extracts_pre_edit_partner() {
        let msg = "core/a.rs historically co-changes with core/b.rs \
                   (3/4 = 75% historical co-edit, Wilson 95% lower 0.30)";
        assert_eq!(parse_partner(msg).as_deref(), Some("core/b.rs"));
    }

    #[test]
    fn parse_partner_returns_none_on_unrelated_message() {
        assert!(parse_partner("just some other text").is_none());
    }

    #[test]
    fn parse_wilson_lower_pulls_float_from_message() {
        let msg = "core/a.rs edited; expected partner core/b.rs not touched \
                   (54/203 = 27% historical co-edit, Wilson 95% lower 0.21)";
        assert!((parse_wilson_lower(msg).unwrap() - 0.21).abs() < 1e-6);
    }

    #[test]
    fn parse_subject_extracts_review_subject() {
        let msg = "core/a.rs edited; expected partner core/b.rs not touched \
                   (54/203 = 27% historical co-edit, Wilson 95% lower 0.21)";
        assert_eq!(parse_subject(msg).as_deref(), Some("core/a.rs"));
    }

    #[test]
    fn parse_subject_extracts_pre_edit_subject() {
        let msg = "core/a.rs historically co-changes with core/b.rs \
                   (3/4 = 75% historical co-edit, Wilson 95% lower 0.30)";
        assert_eq!(parse_subject(msg).as_deref(), Some("core/a.rs"));
    }

    #[test]
    fn parse_subject_returns_none_on_unrelated_message() {
        assert!(parse_subject("just some other text").is_none());
    }

    #[test]
    fn wilson_lower_bucket_partitions_at_0_20_and_0_40() {
        assert_eq!(wilson_lower_bucket(0.10), "0.00-0.20");
        assert_eq!(wilson_lower_bucket(0.19), "0.00-0.20");
        assert_eq!(wilson_lower_bucket(0.20), "0.20-0.40");
        assert_eq!(wilson_lower_bucket(0.39), "0.20-0.40");
        assert_eq!(wilson_lower_bucket(0.40), "0.40+");
        assert_eq!(wilson_lower_bucket(0.99), "0.40+");
    }
}
