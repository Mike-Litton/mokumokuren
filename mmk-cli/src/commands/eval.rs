//! `mmk eval` — sample N recent commits and aggregate a noise-floor
//! report.
//!
//! Built so a new adopter can answer "do my mmk defaults fit my
//! repo?" in under a minute, without writing scratch shell. The
//! aggregation surfaces three things the v0.3 four-repo eval showed
//! were the deciding signals: firing rate, layer mix, and the
//! jaccard distribution of COUPLING findings (so the user knows
//! whether to raise `[coupling] threshold`).

use anyhow::{Context, Result};
use mmk_config::{Config, ConfigFile};
use serde::Serialize;
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use crate::args::{EvalArgs, Format};
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
        if let Some(t) = file_cp.threshold {
            cfg.coupling.threshold = t;
        }
        if !file_cp.ignore_partners.is_empty() {
            cfg.coupling
                .ignore_partners
                .clone_from(&file_cp.ignore_partners);
        }
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
            compute_findings(&changed, &ranked, &analysis.commits, &cfg, args.top)
        };
        report.absorb(&findings);
    }

    let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    report.duration_ms = duration_ms;
    report.threshold = cfg.coupling.threshold;

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
    /// Bucketed jaccard distribution for COUPLING findings:
    /// "0.10-0.30", "0.30-0.50", "0.50+".
    jaccard_buckets: BTreeMap<&'static str, usize>,
    /// Effective coupling threshold for context.
    threshold: f64,
    duration_ms: u64,
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
                if let Some(partner) = parse_partner(&f.message) {
                    *self.noisy_partners.entry(partner).or_default() += 1;
                }
                if let Some(j) = parse_jaccard(&f.message) {
                    *self.jaccard_buckets.entry(jaccard_bucket(j)).or_default() += 1;
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

/// COUPLING messages have two shapes:
///   review:    `<file> edited; expected partner <X> not touched (jaccard 0.NN)`
///   pre-edit:  `<file> historically co-changes with <X> (jaccard 0.NN)`
/// We pull `<X>` between "partner " / "with " and the next sentinel
/// (" not touched" for review, " (jaccard" for pre-edit).
fn parse_partner(msg: &str) -> Option<String> {
    let (key, sentinel) = if let Some(idx) = msg.find("partner ") {
        (idx + "partner ".len(), " not touched")
    } else if let Some(idx) = msg.find("with ") {
        (idx + "with ".len(), " (jaccard")
    } else {
        return None;
    };
    let rest = &msg[key..];
    let end = rest.find(sentinel)?;
    Some(rest[..end].trim().to_owned())
}

fn parse_jaccard(msg: &str) -> Option<f64> {
    let idx = msg.find("jaccard ")? + "jaccard ".len();
    let rest = &msg[idx..];
    let end = rest.find(')').unwrap_or(rest.len());
    rest[..end].trim().parse().ok()
}

const fn jaccard_bucket(j: f64) -> &'static str {
    if j < 0.30 {
        "0.10-0.30"
    } else if j < 0.50 {
        "0.30-0.50"
    } else {
        "0.50+"
    }
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

    if !r.jaccard_buckets.is_empty() {
        writeln!(w, "  jaccard distribution (coupling findings):")?;
        let total_coupling: usize = r.jaccard_buckets.values().sum();
        for bucket in ["0.10-0.30", "0.30-0.50", "0.50+"] {
            let n = r.jaccard_buckets.get(bucket).copied().unwrap_or(0);
            let pct = if total_coupling == 0 {
                0.0
            } else {
                100.0 * n as f64 / total_coupling as f64
            };
            let bars = "#".repeat((pct / 5.0) as usize);
            writeln!(w, "    {bucket:<10}  {bars:<20} {pct:.0}%")?;
        }
        writeln!(w)?;

        let low_pct = r.jaccard_buckets.get("0.10-0.30").copied().unwrap_or(0) as f64
            / total_coupling.max(1) as f64;
        if low_pct > 0.5 && r.threshold < 0.30 {
            writeln!(
                w,
                "  consider raising [coupling] threshold to 0.30 — \
                 over half of COUPLING findings sit in the 0.10-0.30 \
                 noise band on this repo."
            )?;
        }
    }

    Ok(())
}

fn write_json<W: Write>(w: &mut W, r: &AggregateReport) -> Result<()> {
    serde_json::to_writer_pretty(&mut *w, r)?;
    writeln!(w)?;
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::{jaccard_bucket, parse_jaccard, parse_partner};

    #[test]
    fn parse_partner_extracts_review_partner_without_trailing_text() {
        // Regression: an earlier version sliced from "partner " all
        // the way to " (jaccard", swallowing " not touched" into the
        // captured partner — which made `noisy_partners` keys like
        // "Cargo.toml not touched" instead of "Cargo.toml".
        let msg = "core/a.rs edited; expected partner core/b.rs not touched (jaccard 0.75)";
        assert_eq!(parse_partner(msg).as_deref(), Some("core/b.rs"));
    }

    #[test]
    fn parse_partner_extracts_pre_edit_partner() {
        let msg = "core/a.rs historically co-changes with core/b.rs (jaccard 0.75)";
        assert_eq!(parse_partner(msg).as_deref(), Some("core/b.rs"));
    }

    #[test]
    fn parse_partner_returns_none_on_unrelated_message() {
        assert!(parse_partner("just some other text").is_none());
    }

    #[test]
    fn parse_jaccard_pulls_float_from_message() {
        let msg = "core/a.rs edited; expected partner core/b.rs not touched (jaccard 0.42)";
        assert!((parse_jaccard(msg).unwrap() - 0.42).abs() < 1e-6);
    }

    #[test]
    fn jaccard_bucket_partitions_at_0_30_and_0_50() {
        assert_eq!(jaccard_bucket(0.10), "0.10-0.30");
        assert_eq!(jaccard_bucket(0.29), "0.10-0.30");
        assert_eq!(jaccard_bucket(0.30), "0.30-0.50");
        assert_eq!(jaccard_bucket(0.49), "0.30-0.50");
        assert_eq!(jaccard_bucket(0.50), "0.50+");
        assert_eq!(jaccard_bucket(0.99), "0.50+");
    }
}
