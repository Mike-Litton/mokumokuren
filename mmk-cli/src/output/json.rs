//! JSON output for `mmk analyze`.

use anyhow::Result;
use mmk_config::Config;
use mmk_core::session::SessionDelta;
use mmk_core::{CouplingEntry, HotspotEntry, NeighborhoodNode};
use mmk_git::{AnalyzeOutput, SessionAnalyzeOutput};
use serde::Serialize;
use std::io::Write;

#[derive(Serialize)]
struct Report<'a> {
    /// Schema-contract version. Bumps with breaking JSON changes; pin
    /// against this in consumers.
    schema_version: &'static str,
    /// Cargo crate version of the producing `mmk` build. Diagnostic
    /// only — do not pin against.
    crate_version: &'static str,
    repo: RepoBlock<'a>,
    config: &'a Config,
    analysis: AnalysisBlock,
    files: Vec<FileEntry<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    blast_radius: Option<BlastRadiusBlock<'a>>,
}

#[derive(Serialize)]
struct RepoBlock<'a> {
    head_sha: Option<&'a str>,
    head_timestamp: Option<String>,
    is_shallow: bool,
    warnings: &'a [String],
}

#[derive(Serialize)]
struct AnalysisBlock {
    commits_seen: u64,
    commits_analyzed: u64,
    commits_filtered: CommitsFilteredBlock,
    files_ignored: FilesIgnoredBlock,
    duration_ms: u64,
}

#[derive(Serialize)]
struct CommitsFilteredBlock {
    bulk: u64,
}

#[derive(Serialize)]
struct FilesIgnoredBlock {
    /// Change-events on paths that aren't in HEAD (renamed away,
    /// deleted, or matched by an ignore glob during diff walk).
    deleted_from_head: u64,
    /// HEAD-tree paths excluded by an ignore glob. Useful for sanity-
    /// checking that the user's `mokumokuren.toml` is doing something.
    head_paths_ignored: u64,
}

#[derive(Serialize)]
struct FileEntry<'a> {
    path: String,
    loc: u32,
    weighted_churn: f64,
    relative_churn: f64,
    hotspot_score: f64,
    hotspot_rank: u32,
    commits_touching: u32,
    last_modified: Option<String>,
    top_couples: &'a [CouplingEntry],
}

#[derive(Serialize)]
struct CouplesOfBlock<'a> {
    path: String,
    entries: &'a [CouplingEntry],
}

#[derive(Serialize)]
struct BlastRadiusBlock<'a> {
    root: String,
    hops: u32,
    /// Effective Jaccard threshold applied to the neighborhood —
    /// echo of the value resolved from CLI override / TOML config /
    /// built-in default. Lets a downstream consumer see what filter
    /// produced the listed nodes without having to guess.
    threshold: f64,
    nodes: &'a [NeighborhoodNode],
}

#[derive(Serialize)]
struct SessionBlock<'a> {
    base_ref: Option<&'a str>,
    base_sha: Option<String>,
    base_resolved_via: &'static str,
    #[serde(flatten)]
    delta: &'a SessionDelta,
}

#[derive(Serialize)]
struct SessionReport<'a> {
    schema_version: &'static str,
    crate_version: &'static str,
    repo: RepoBlock<'a>,
    config: &'a Config,
    analysis: AnalysisBlock,
    /// The full-window ranking — same shape as `analyze`'s `files`.
    files: Vec<FileEntry<'a>>,
    /// The ranking computed only over commits since the resolved base.
    session_files: Vec<FileEntry<'a>>,
    session: SessionBlock<'a>,
    #[serde(skip_serializing_if = "Option::is_none")]
    blast_radius: Option<BlastRadiusBlock<'a>>,
}

#[derive(Serialize)]
struct CouplesOfReport<'a> {
    schema_version: &'static str,
    crate_version: &'static str,
    repo: RepoBlock<'a>,
    config: &'a Config,
    analysis: AnalysisBlock,
    couples_of: CouplesOfBlock<'a>,
    #[serde(skip_serializing_if = "Option::is_none")]
    blast_radius: Option<BlastRadiusBlock<'a>>,
}

fn rfc3339(ts: i64) -> Option<String> {
    if ts <= 0 {
        return None;
    }
    let secs = u64::try_from(ts).ok()?;
    let st = std::time::UNIX_EPOCH.checked_add(std::time::Duration::from_secs(secs))?;
    let formatted = humantime::format_rfc3339_seconds(st).to_string();
    Some(formatted)
}

/// Write a pretty-printed JSON report to `w`. Schema is the
/// [`Report`] struct: `schema_version` is the contract consumers pin
/// against, `crate_version` is diagnostic only.
pub fn write<W: Write>(
    w: &mut W,
    ranked: &[HotspotEntry],
    analysis: &AnalyzeOutput,
    duration_ms: u64,
    config: &Config,
    blast: Option<(&std::path::Path, f64, &[NeighborhoodNode])>,
) -> Result<()> {
    let files = ranked
        .iter()
        .map(|e| FileEntry {
            path: e.path.to_string_lossy().into_owned(),
            loc: e.loc,
            weighted_churn: e.weighted_churn,
            relative_churn: e.relative_churn,
            hotspot_score: e.hotspot_score,
            hotspot_rank: e.hotspot_rank,
            commits_touching: e.commits_touching,
            last_modified: rfc3339(e.last_modified),
            top_couples: &e.top_couples,
        })
        .collect();

    let blast_block = blast.map(|(root, threshold, nodes)| BlastRadiusBlock {
        root: root.to_string_lossy().into_owned(),
        hops: 1,
        threshold,
        nodes,
    });

    let report = Report {
        schema_version: crate::output::schema::SCHEMA_VERSION,
        crate_version: env!("CARGO_PKG_VERSION"),
        repo: RepoBlock {
            head_sha: analysis.head_sha.as_deref(),
            head_timestamp: analysis.head_timestamp.and_then(rfc3339),
            is_shallow: analysis.is_shallow,
            warnings: &analysis.warnings,
        },
        config,
        analysis: AnalysisBlock {
            commits_seen: analysis.counts.commits_seen,
            commits_analyzed: analysis.counts.commits_analyzed,
            commits_filtered: CommitsFilteredBlock {
                bulk: analysis.counts.commits_filtered_bulk,
            },
            files_ignored: FilesIgnoredBlock {
                deleted_from_head: analysis.counts.non_head_events,
                head_paths_ignored: analysis.counts.head_paths_ignored,
            },
            duration_ms,
        },
        files,
        blast_radius: blast_block,
    };

    serde_json::to_writer_pretty(&mut *w, &report)?;
    writeln!(w)?;
    Ok(())
}

/// Write a session report — the window ranking plus the session
/// ranking and the delta block. Used by `mmk session`.
#[allow(clippy::too_many_arguments)]
pub fn write_session<W: Write>(
    w: &mut W,
    window_ranked: &[HotspotEntry],
    session_ranked: &[HotspotEntry],
    delta: &SessionDelta,
    session_out: &SessionAnalyzeOutput,
    duration_ms: u64,
    config: &Config,
    blast: Option<(&std::path::Path, f64, &[NeighborhoodNode])>,
) -> Result<()> {
    let analysis = &session_out.window;

    let files: Vec<FileEntry<'_>> = window_ranked
        .iter()
        .map(|e| FileEntry {
            path: e.path.to_string_lossy().into_owned(),
            loc: e.loc,
            weighted_churn: e.weighted_churn,
            relative_churn: e.relative_churn,
            hotspot_score: e.hotspot_score,
            hotspot_rank: e.hotspot_rank,
            commits_touching: e.commits_touching,
            last_modified: rfc3339(e.last_modified),
            top_couples: &e.top_couples,
        })
        .collect();
    let session_files: Vec<FileEntry<'_>> = session_ranked
        .iter()
        .map(|e| FileEntry {
            path: e.path.to_string_lossy().into_owned(),
            loc: e.loc,
            weighted_churn: e.weighted_churn,
            relative_churn: e.relative_churn,
            hotspot_score: e.hotspot_score,
            hotspot_rank: e.hotspot_rank,
            commits_touching: e.commits_touching,
            last_modified: rfc3339(e.last_modified),
            top_couples: &e.top_couples,
        })
        .collect();

    let (base_ref, base_sha, base_via) = session_out.base.as_ref().map_or(
        (None, None, "head_minus_one"),
        |r| (r.label.as_deref(), Some(r.oid.to_string()), r.via.as_str()),
    );

    let blast_block = blast.map(|(root, threshold, nodes)| BlastRadiusBlock {
        root: root.to_string_lossy().into_owned(),
        hops: 1,
        threshold,
        nodes,
    });

    let report = SessionReport {
        schema_version: crate::output::schema::SCHEMA_VERSION,
        crate_version: env!("CARGO_PKG_VERSION"),
        repo: RepoBlock {
            head_sha: analysis.head_sha.as_deref(),
            head_timestamp: analysis.head_timestamp.and_then(rfc3339),
            is_shallow: analysis.is_shallow,
            warnings: &analysis.warnings,
        },
        config,
        analysis: AnalysisBlock {
            commits_seen: analysis.counts.commits_seen,
            commits_analyzed: analysis.counts.commits_analyzed,
            commits_filtered: CommitsFilteredBlock {
                bulk: analysis.counts.commits_filtered_bulk,
            },
            files_ignored: FilesIgnoredBlock {
                deleted_from_head: analysis.counts.non_head_events,
                head_paths_ignored: analysis.counts.head_paths_ignored,
            },
            duration_ms,
        },
        files,
        session_files,
        session: SessionBlock {
            base_ref,
            base_sha,
            base_resolved_via: base_via,
            delta,
        },
        blast_radius: blast_block,
    };

    serde_json::to_writer_pretty(&mut *w, &report)?;
    writeln!(w)?;
    Ok(())
}

/// Write a `--couples-of <PATH>` report: the coupling list for one
/// path, no ranked `files` block. Same envelope (`schema_version`,
/// `repo`, `config`, `analysis`) so harnesses can use the same
/// dispatcher.
pub fn write_couples_of<W: Write>(
    w: &mut W,
    path: &std::path::Path,
    entries: &[CouplingEntry],
    analysis: &AnalyzeOutput,
    duration_ms: u64,
    config: &Config,
    blast: Option<(&std::path::Path, f64, &[NeighborhoodNode])>,
) -> Result<()> {
    let blast_block = blast.map(|(root, threshold, nodes)| BlastRadiusBlock {
        root: root.to_string_lossy().into_owned(),
        hops: 1,
        threshold,
        nodes,
    });

    let report = CouplesOfReport {
        schema_version: crate::output::schema::SCHEMA_VERSION,
        crate_version: env!("CARGO_PKG_VERSION"),
        repo: RepoBlock {
            head_sha: analysis.head_sha.as_deref(),
            head_timestamp: analysis.head_timestamp.and_then(rfc3339),
            is_shallow: analysis.is_shallow,
            warnings: &analysis.warnings,
        },
        config,
        analysis: AnalysisBlock {
            commits_seen: analysis.counts.commits_seen,
            commits_analyzed: analysis.counts.commits_analyzed,
            commits_filtered: CommitsFilteredBlock {
                bulk: analysis.counts.commits_filtered_bulk,
            },
            files_ignored: FilesIgnoredBlock {
                deleted_from_head: analysis.counts.non_head_events,
                head_paths_ignored: analysis.counts.head_paths_ignored,
            },
            duration_ms,
        },
        couples_of: CouplesOfBlock {
            path: path.to_string_lossy().into_owned(),
            entries,
        },
        blast_radius: blast_block,
    };

    serde_json::to_writer_pretty(&mut *w, &report)?;
    writeln!(w)?;
    Ok(())
}
