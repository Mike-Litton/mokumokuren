//! Ranked top-N text table.

use anyhow::Result;
use mmk_config::Config;
use mmk_core::session::SessionDelta;
use mmk_core::{CouplingEntry, HotspotEntry, NeighborhoodNode};
use mmk_git::{AnalyzeOutput, SessionAnalyzeOutput};
use std::io::Write;
use std::path::Path;

/// Write a human-readable ranked top-N table to `w`.
///
/// Column widths auto-size to the longest path so output stays
/// aligned. When `with_couples` is true, an indented `couples:`
/// block is rendered under each row.
pub fn write<W: Write>(
    w: &mut W,
    ranked: &[HotspotEntry],
    analysis: &AnalyzeOutput,
    duration_ms: u64,
    _config: &Config,
    with_couples: bool,
    blast: Option<(&Path, f64, &[NeighborhoodNode])>,
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
        if with_couples && !e.top_couples.is_empty() {
            writeln!(w, "      couples:")?;
            for c in &e.top_couples {
                writeln!(
                    w,
                    "        {:<pw$}  jaccard {:.2}  co-change {}",
                    c.partner.to_string_lossy(),
                    c.jaccard,
                    c.co_change_count,
                    pw = path_width,
                )?;
            }
        }
    }

    if let Some((root, threshold, nodes)) = blast {
        writeln!(
            w,
            "\nblast_radius for {} (1-hop, threshold {:.2}):",
            root.display(),
            threshold
        )?;
        if nodes.is_empty() {
            writeln!(w, "  (no partners above threshold)")?;
        } else {
            for n in nodes {
                writeln!(
                    w,
                    "  {}  jaccard {:.2}  co-change {}",
                    n.path.display(),
                    n.jaccard,
                    n.co_change_count
                )?;
            }
        }
    }

    writeln!(
        w,
        "\n{} commits analyzed ({} filtered) in {} ms",
        analysis.counts.commits_analyzed, analysis.counts.commits_filtered_bulk, duration_ms,
    )?;
    Ok(())
}

/// Render a `mmk session` text report.
///
/// When `suppress_window` is set the WINDOW ranking table is replaced
/// with a single-line suppression notice. Used when the session is
/// empty: the WINDOW table would bury the ANCHOR nudge under
/// generated-file noise the user can fetch from `mmk analyze` if
/// they want it.
#[allow(clippy::too_many_arguments)]
pub fn write_session<W: Write>(
    w: &mut W,
    window_ranked: &[HotspotEntry],
    session_ranked: &[HotspotEntry],
    delta: &SessionDelta,
    session_out: &SessionAnalyzeOutput,
    duration_ms: u64,
    blast: Option<(&Path, f64, &[NeighborhoodNode])>,
    suppress_window: bool,
    bulk_max_files: u32,
    bulk_max_lines: u32,
) -> Result<()> {
    writeln!(w, "session:")?;
    match &session_out.base {
        Some(r) => writeln!(
            w,
            "  base: {} ({})  via {}",
            r.label.as_deref().unwrap_or("(SHA)"),
            r.oid,
            r.via.as_str()
        )?,
        None => writeln!(w, "  base: (none — root commit?)")?,
    }
    writeln!(w, "  commit_entropy: {:.3}", delta.commit_entropy)?;
    writeln!(w, "  entered_top_n ({}):", delta.entered_top_n.len())?;
    for p in &delta.entered_top_n {
        writeln!(w, "    {}", p.display())?;
    }
    writeln!(w, "  rank_climbs ({}):", delta.rank_climbs.len())?;
    for c in &delta.rank_climbs {
        writeln!(w, "    {}  +{}", c.path.display(), c.delta)?;
    }
    writeln!(w, "  churn_of_churn ({}):", delta.churn_of_churn.len())?;
    for c in &delta.churn_of_churn {
        writeln!(w, "    {}  {:.2}", c.path.display(), c.ratio)?;
    }

    writeln!(w, "\nsession ranking ({} files):", session_ranked.len())?;
    write_simple_table(w, session_ranked)?;
    if suppress_window {
        writeln!(
            w,
            "\n{}",
            crate::output::messages::session_window_suppressed(),
        )?;
    } else {
        writeln!(w, "\nwindow ranking ({} files):", window_ranked.len())?;
        write_simple_table(w, window_ranked)?;
    }

    if let Some((root, threshold, nodes)) = blast {
        writeln!(
            w,
            "\nblast_radius for {} (1-hop, threshold {:.2}):",
            root.display(),
            threshold
        )?;
        if nodes.is_empty() {
            writeln!(w, "  (no partners above threshold)")?;
        } else {
            for n in nodes {
                writeln!(
                    w,
                    "  {}  jaccard {:.2}  co-change {}",
                    n.path.display(),
                    n.jaccard,
                    n.co_change_count
                )?;
            }
        }
    }

    writeln!(
        w,
        "\n{} commits in window, {} in session — {} ms",
        session_out.window.commits.len(),
        session_out.session_commits.len(),
        duration_ms,
    )?;

    // Diagnostics: descriptive metadata about the analysis itself —
    // not findings the agent should act on. Window-truncation lives
    // here as of v0.8 (was a Layer::Budget Warn through v0.7); it
    // describes what the analyzer saw, while operational BUDGET
    // (diff-vs-cap) describes what the agent did.
    let counts = &session_out.window.counts;
    if counts.commits_filtered_bulk > 0 {
        writeln!(w, "\nDiagnostics:")?;
        writeln!(
            w,
            "  {}",
            crate::output::messages::session_budget(
                counts.commits_filtered_bulk,
                counts.commits_seen,
                bulk_max_files,
                bulk_max_lines,
            ),
        )?;
    }
    Ok(())
}

fn write_simple_table<W: Write>(w: &mut W, entries: &[HotspotEntry]) -> Result<()> {
    if entries.is_empty() {
        writeln!(w, "  (empty)")?;
        return Ok(());
    }
    let pw = entries
        .iter()
        .map(|e| e.path.to_string_lossy().len())
        .max()
        .unwrap_or(4)
        .max(4);
    for e in entries {
        writeln!(
            w,
            "  {:>3}  {:<pw$}  hotspot {:>10.2}  commits {:>4}",
            e.hotspot_rank,
            e.path.to_string_lossy(),
            e.hotspot_score,
            e.commits_touching,
            pw = pw,
        )?;
    }
    Ok(())
}

/// Render a `--couples-of <PATH>` listing.
pub fn write_couples_of<W: Write>(
    w: &mut W,
    path: &Path,
    entries: &[CouplingEntry],
    analysis: &AnalyzeOutput,
    duration_ms: u64,
    blast: Option<(&Path, f64, &[NeighborhoodNode])>,
) -> Result<()> {
    writeln!(w, "couples for {}:", path.display())?;
    if entries.is_empty() {
        writeln!(w, "  (no co-changes in window)")?;
    } else {
        let pw = entries
            .iter()
            .map(|c| c.partner.to_string_lossy().len())
            .max()
            .unwrap_or(4);
        for c in entries {
            writeln!(
                w,
                "  {:<pw$}  jaccard {:.2}  co-change {}",
                c.partner.to_string_lossy(),
                c.jaccard,
                c.co_change_count,
                pw = pw,
            )?;
        }
    }

    if let Some((root, threshold, nodes)) = blast {
        writeln!(
            w,
            "\nblast_radius for {} (1-hop, threshold {:.2}):",
            root.display(),
            threshold
        )?;
        if nodes.is_empty() {
            writeln!(w, "  (no partners above threshold)")?;
        } else {
            for n in nodes {
                writeln!(
                    w,
                    "  {}  jaccard {:.2}  co-change {}",
                    n.path.display(),
                    n.jaccard,
                    n.co_change_count
                )?;
            }
        }
    }

    writeln!(
        w,
        "\n{} commits analyzed ({} filtered) in {} ms",
        analysis.counts.commits_analyzed, analysis.counts.commits_filtered_bulk, duration_ms,
    )?;
    Ok(())
}
