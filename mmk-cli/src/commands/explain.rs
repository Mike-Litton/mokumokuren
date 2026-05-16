//! `mmk explain --finding <id>` — addressable per-commit evidence
//! behind a borderline finding's claim.
//!
//! The pipeline: parse the fingerprint, run `mmk_git::analyze` for the
//! configured window, filter the analyzer's commits to those whose
//! deltas touch either pair member, and emit the chronological
//! evidence + aggregate timeline.
//!
//! Scoped to COUPLING. The information-theoretic argument for that
//! scope: drill-down is required only when the finding's summary
//! statistic destroys information the agent needs to make the
//! decision, and COUPLING's K-of-N collapses temporal distribution
//! that the agent otherwise has no way to recover. (Introduced in
//! v0.11.)

use anyhow::{anyhow, Context, Result};
use mmk_config::Config;
use mmk_core::types::Commit;
use serde::Serialize;
use std::io::Write;
use std::path::PathBuf;

use crate::args::{ExplainArgs, Format};
use crate::commands::common::load_config_file;

pub fn run<O: Write, E: Write>(args: &ExplainArgs, stdout: &mut O, stderr: &mut E) -> Result<()> {
    let cwd = std::env::current_dir().context("failed to determine current directory")?;

    let parsed = parse_finding_id(&args.finding)?;
    let FingerprintFields::Coupling { subject, partner } = parsed;

    let window = humantime::parse_duration(&args.since)
        .with_context(|| format!("invalid --since value: {}", args.since))?;
    let window_days = u32::try_from(window.as_secs() / 86_400)
        .unwrap_or(u32::MAX)
        .max(1);

    let (file_cfg, file_path) = load_config_file(&cwd, args.config.as_deref())?;

    let mut cfg = Config::default();
    cfg.window.days = window_days;
    cfg.ignores = file_cfg.ignore;
    cfg.ignores.extend(args.ignores.iter().cloned());

    if args.verbose {
        match &file_path {
            Some(p) => writeln!(stderr, "loaded config from {}", p.display())?,
            None => writeln!(stderr, "no mokumokuren.toml found; running with defaults")?,
        }
    }

    let analysis = mmk_git::analyze(&cwd, &cfg)?;
    let evidence = collect_evidence(&analysis.commits, &subject, &partner);
    let either_touching = count_either_touching(&analysis.commits, &subject, &partner);

    match args.format {
        Format::Text => write_text(
            stdout,
            &args.finding,
            &subject,
            &partner,
            &evidence,
            either_touching,
        )?,
        Format::Json => write_json(stdout, &args.finding, &evidence, either_touching)?,
    }

    Ok(())
}

#[derive(Debug)]
enum FingerprintFields {
    Coupling { subject: PathBuf, partner: PathBuf },
}

fn parse_finding_id(id: &str) -> Result<FingerprintFields> {
    // Layer prefix is the first colon-delimited segment; the rest is
    // the per-layer body. COUPLING's body is `<subject>:<partner>`,
    // both rendered through `Path::display()` so paths containing
    // forward slashes (the common case) survive the round-trip
    // unchanged. A path with an embedded `:` in its components would
    // make this ambiguous; that case isn't supported — the explicit
    // error below points the caller at the limitation.
    let mut parts = id.splitn(2, ':');
    let layer = parts.next().unwrap_or("");
    let body = parts.next().ok_or_else(|| {
        anyhow!("finding id must look like `coupling:<subject>:<partner>` (got `{id}`)")
    })?;

    match layer {
        "coupling" => {
            // Splitn over the body — first `:` separates subject from
            // partner. Embedded `:` characters in a path would land
            // inside the partner; we reject the case to avoid silent
            // misattribution.
            let mut body_parts = body.splitn(2, ':');
            let subject = body_parts
                .next()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow!("coupling fingerprint missing subject (got `{id}`)"))?;
            let partner = body_parts
                .next()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow!("coupling fingerprint missing partner (got `{id}`)"))?;
            if partner.contains(':') {
                return Err(anyhow!(
                    "coupling fingerprint has an extra `:` segment; only \
                     `coupling:<subject>:<partner>` is supported (got `{id}`)"
                ));
            }
            Ok(FingerprintFields::Coupling {
                subject: PathBuf::from(subject),
                partner: PathBuf::from(partner),
            })
        }
        other => Err(anyhow!(
            "unknown finding layer `{other}` — only `coupling:<a>:<b>` is supported"
        )),
    }
}

#[derive(Debug, Serialize)]
struct EvidenceRow {
    sha: String,
    ts: i64,
    deltas: Vec<EvidenceDelta>,
}

#[derive(Debug, Serialize)]
struct EvidenceDelta {
    path: String,
    added: u32,
    deleted: u32,
}

/// Filter `commits` to the co-change rows — commits whose deltas
/// include BOTH `subject` and `partner` — and sort newest-first.
///
/// The aggregate count of "commits touching either" is computed
/// separately on the full set; only the co-change rows belong in
/// `evidence` because that's what the agent verifies against the
/// COUPLING claim. Partner-only commits show up as the difference
/// between `commits_touching_either` and `co_change_count`; the
/// agent can `git log -- <path>` them if they want the full ledger.
fn collect_evidence(commits: &[Commit], subject: &PathBuf, partner: &PathBuf) -> Vec<EvidenceRow> {
    let mut rows: Vec<EvidenceRow> = commits
        .iter()
        .filter_map(|c| {
            let touches_subject = c.deltas.iter().any(|d| &d.path == subject);
            let touches_partner = c.deltas.iter().any(|d| &d.path == partner);
            if !(touches_subject && touches_partner) {
                return None;
            }
            let deltas: Vec<EvidenceDelta> = c
                .deltas
                .iter()
                .filter(|d| &d.path == subject || &d.path == partner)
                .map(|d| EvidenceDelta {
                    path: d.path.display().to_string(),
                    added: d.added,
                    deleted: d.deleted,
                })
                .collect();
            Some(EvidenceRow {
                sha: c.info.sha.clone(),
                ts: c.info.timestamp,
                deltas,
            })
        })
        .collect();
    // Newest-first so the agent reads the recent burst before the
    // historical context.
    rows.sort_by_key(|r| std::cmp::Reverse(r.ts));
    rows
}

/// Number of commits that touched either pair member, regardless of
/// whether the partner appeared. This is the denominator the
/// COUPLING-message K-of-N reports against, surfaced separately so
/// the partner-only count can be derived without re-walking the log.
fn count_either_touching(commits: &[Commit], subject: &PathBuf, partner: &PathBuf) -> u32 {
    let n = commits
        .iter()
        .filter(|c| {
            c.deltas
                .iter()
                .any(|d| &d.path == subject || &d.path == partner)
        })
        .count();
    u32::try_from(n).unwrap_or(u32::MAX)
}

#[derive(Debug, Serialize)]
struct EvidenceReport<'a> {
    finding: &'a str,
    co_change_count: u32,
    commits_touching_either: u32,
    co_change_span_days: u64,
    co_change_first_ts: Option<i64>,
    co_change_last_ts: Option<i64>,
    evidence: &'a [EvidenceRow],
}

fn aggregate_stats(rows: &[EvidenceRow], commits_touching_either: u32) -> AggregateStats {
    let co_change_count = u32::try_from(rows.len()).unwrap_or(u32::MAX);
    let (first, last, span_days) = if let (Some(min), Some(max)) = (
        rows.iter().map(|r| r.ts).min(),
        rows.iter().map(|r| r.ts).max(),
    ) {
        let span_secs = (max - min).max(0);
        let span_days = (span_secs as u64).div_ceil(86_400);
        (Some(min), Some(max), span_days)
    } else {
        (None, None, 0)
    };
    AggregateStats {
        co_change_count,
        commits_touching_either,
        first,
        last,
        span_days,
    }
}

struct AggregateStats {
    co_change_count: u32,
    commits_touching_either: u32,
    first: Option<i64>,
    last: Option<i64>,
    span_days: u64,
}

fn write_json<W: Write>(
    w: &mut W,
    finding: &str,
    rows: &[EvidenceRow],
    commits_touching_either: u32,
) -> Result<()> {
    let stats = aggregate_stats(rows, commits_touching_either);
    let report = EvidenceReport {
        finding,
        co_change_count: stats.co_change_count,
        commits_touching_either: stats.commits_touching_either,
        co_change_span_days: stats.span_days,
        co_change_first_ts: stats.first,
        co_change_last_ts: stats.last,
        evidence: rows,
    };
    serde_json::to_writer_pretty(&mut *w, &report)?;
    writeln!(w)?;
    Ok(())
}

fn write_text<W: Write>(
    w: &mut W,
    _finding: &str,
    subject: &std::path::Path,
    partner: &std::path::Path,
    rows: &[EvidenceRow],
    commits_touching_either: u32,
) -> Result<()> {
    let stats = aggregate_stats(rows, commits_touching_either);
    // The three header facts the agent reads before deciding:
    // co-change density (already in COUPLING's message), temporal
    // concentration (the new fact `explain` adds), and partner-only
    // commits (single-merge-storm tell). One short factual sentence
    // per line — same convention as `messages.rs`, no editorial.
    writeln!(
        w,
        "{} and {} co-changed in {} of {} commits.",
        subject.display(),
        partner.display(),
        stats.co_change_count,
        stats.commits_touching_either,
    )?;
    if let (Some(first), Some(last)) = (stats.first, stats.last) {
        writeln!(
            w,
            "co-changes span {} days; first {}, last {}.",
            stats.span_days,
            short_date(first),
            short_date(last),
        )?;
    } else {
        writeln!(w, "no co-changes in this window.")?;
    }
    let partner_only_count = stats
        .commits_touching_either
        .saturating_sub(stats.co_change_count);
    writeln!(
        w,
        "{} of {} commits touched {} without {}.",
        partner_only_count,
        stats.commits_touching_either,
        subject.display(),
        partner.display(),
    )?;

    if !rows.is_empty() {
        writeln!(w)?;
        for r in rows {
            let short = if r.sha.len() >= 7 {
                &r.sha[..7]
            } else {
                &r.sha
            };
            write!(w, "  {}  {}", short_date(r.ts), short)?;
            for d in &r.deltas {
                write!(w, "  +{} -{} {}", d.added, d.deleted, d.path)?;
            }
            writeln!(w)?;
        }
    }
    Ok(())
}

fn short_date(ts: i64) -> String {
    if ts <= 0 {
        return "????-??-??".to_string();
    }
    let Ok(secs) = u64::try_from(ts) else {
        return "????-??-??".to_string();
    };
    let Some(st) = std::time::UNIX_EPOCH.checked_add(std::time::Duration::from_secs(secs)) else {
        return "????-??-??".to_string();
    };
    let formatted = humantime::format_rfc3339_seconds(st).to_string();
    formatted.get(..10).unwrap_or("????-??-??").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_coupling_id_round_trips_two_segment_paths() {
        let parsed = parse_finding_id("coupling:core/a.rs:core/b.rs").unwrap();
        let FingerprintFields::Coupling { subject, partner } = parsed;
        assert_eq!(subject, PathBuf::from("core/a.rs"));
        assert_eq!(partner, PathBuf::from("core/b.rs"));
    }

    #[test]
    fn parse_unknown_layer_errors_clearly() {
        let err = parse_finding_id("hotspot:foo:bar").unwrap_err();
        assert!(
            err.to_string().contains("unknown finding layer"),
            "expected layer error, got: {err}"
        );
    }

    #[test]
    fn parse_missing_partner_errors_clearly() {
        let err = parse_finding_id("coupling:core/a.rs").unwrap_err();
        assert!(
            err.to_string().contains("missing partner"),
            "expected missing-partner error, got: {err}"
        );
    }

    #[test]
    fn parse_completely_malformed_id_errors_clearly() {
        let err = parse_finding_id("just-a-name").unwrap_err();
        assert!(
            err.to_string().contains("must look like"),
            "expected fingerprint-shape error, got: {err}"
        );
    }

    #[test]
    fn parse_extra_colon_segment_rejected() {
        let err = parse_finding_id("coupling:a:b:c").unwrap_err();
        assert!(
            err.to_string().contains("extra `:`"),
            "expected extra-segment rejection, got: {err}"
        );
    }
}
