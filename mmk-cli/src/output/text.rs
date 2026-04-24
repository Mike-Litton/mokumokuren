//! Ranked top-N text table.

use anyhow::Result;
use mmk_config::Config;
use mmk_core::HotspotEntry;
use mmk_git::AnalyzeOutput;
use std::io::Write;

pub fn write<W: Write>(
    w: &mut W,
    ranked: &[HotspotEntry],
    analysis: &AnalyzeOutput,
    duration_ms: u64,
    _config: &Config,
) -> Result<()> {
    if ranked.is_empty() {
        writeln!(
            w,
            "no hotspots: {} commits analyzed in {duration_ms} ms",
            analysis.counts.commits_analyzed
        )?;
        return Ok(());
    }

    let path_width = ranked
        .iter()
        .map(|e| e.path.to_string_lossy().len())
        .max()
        .unwrap_or(4)
        .max(4);

    writeln!(
        w,
        "{:>4}  {:<pw$}  {:>8}  {:>14}  {:>8}  {:>12}",
        "rank",
        "path",
        "loc",
        "weighted_churn",
        "commits",
        "hotspot",
        pw = path_width,
    )?;
    writeln!(
        w,
        "{:->4}  {:-<pw$}  {:->8}  {:->14}  {:->8}  {:->12}",
        "",
        "",
        "",
        "",
        "",
        "",
        pw = path_width,
    )?;
    for e in ranked {
        writeln!(
            w,
            "{:>4}  {:<pw$}  {:>8}  {:>14.2}  {:>8}  {:>12.2}",
            e.hotspot_rank,
            e.path.to_string_lossy(),
            e.loc,
            e.weighted_churn,
            e.commits_touching,
            e.hotspot_score,
            pw = path_width,
        )?;
    }
    writeln!(
        w,
        "\n{} commits analyzed ({} filtered) in {} ms",
        analysis.counts.commits_analyzed, analysis.counts.commits_filtered_bulk, duration_ms,
    )?;
    Ok(())
}
