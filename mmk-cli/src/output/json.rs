//! JSON output for `mmk analyze`.

use anyhow::Result;
use mmk_config::Config;
use mmk_core::HotspotEntry;
use mmk_git::AnalyzeOutput;
use serde::Serialize;
use std::io::Write;

#[derive(Serialize)]
struct Report<'a> {
    version: &'static str,
    repo: RepoBlock<'a>,
    config: &'a Config,
    analysis: AnalysisBlock,
    files: Vec<FileEntry<'a>>,
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
    #[serde(skip)]
    _marker: std::marker::PhantomData<&'a ()>,
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
/// [`Report`] struct; `version` tracks the crate version so consumers
/// can spot a breaking change.
pub fn write<W: Write>(
    w: &mut W,
    ranked: &[HotspotEntry],
    analysis: &AnalyzeOutput,
    duration_ms: u64,
    config: &Config,
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
            _marker: std::marker::PhantomData,
        })
        .collect();

    let report = Report {
        version: env!("CARGO_PKG_VERSION"),
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
    };

    serde_json::to_writer_pretty(&mut *w, &report)?;
    writeln!(w)?;
    Ok(())
}
