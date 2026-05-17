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
    /// Present when the per-commit bulk filter dropped at least one
    /// commit from the analysis window. Descriptive metadata about
    /// what the analyzer saw, not a Finding the agent should act on
    /// — operational BUDGET fires on the agent's diff and lives in
    /// the `findings` array. Added in v0.8 so consumers can render
    /// the truncation as a Diagnostic instead of as a Finding.
    #[serde(skip_serializing_if = "Option::is_none")]
    window_truncation: Option<WindowTruncationBlock>,
}

#[derive(Serialize)]
struct WindowTruncationBlock {
    commits_dropped: u64,
    total_commits: u64,
    max_files: u32,
    max_lines: u32,
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
    /// Suppressed (`None` → key omitted) when the session has zero
    /// commits, so the empty-session view doesn't bury the ANCHOR
    /// nudge under a window-wide hotspot list the user can fetch
    /// from `mmk analyze` if they want it.
    #[serde(skip_serializing_if = "Option::is_none")]
    files: Option<Vec<FileEntry<'a>>>,
    /// The ranking computed only over commits since the resolved base.
    session_files: Vec<FileEntry<'a>>,
    session: SessionBlock<'a>,
    /// DRIFT + BUDGET findings overlaid on the session view. Always
    /// present (possibly empty) so harnesses see a stable shape.
    findings: &'a [crate::output::findings::Finding],
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
            window_truncation: None,
        },
        files,
        blast_radius: blast_block,
    };

    serde_json::to_writer_pretty(&mut *w, &report)?;
    writeln!(w)?;
    Ok(())
}

/// Write a session report — the window ranking plus the session
/// ranking, delta block, and the DRIFT/BUDGET findings overlay.
/// Used by `mmk session-summary` (and the `session` alias).
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
    findings: &[crate::output::findings::Finding],
) -> Result<()> {
    let analysis = &session_out.window;

    let session_n = session_out.session_commits.len();
    let files: Option<Vec<FileEntry<'_>>> = if session_n == 0 {
        // Empty session: the WINDOW ranking is noise (locale `.po`,
        // generated artifacts) that buries the ANCHOR nudge. Drop it
        // entirely; the absent key signals "fetch from `mmk analyze`."
        None
    } else {
        Some(
            window_ranked
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
                .collect(),
        )
    };
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

    let (base_ref, base_sha, base_via) = session_out
        .base
        .as_ref()
        .map_or((None, None, "head_minus_one"), |r| {
            (r.label.as_deref(), Some(r.oid.to_string()), r.via.as_str())
        });

    let blast_block = blast.map(|(root, threshold, nodes)| BlastRadiusBlock {
        root: root.to_string_lossy().into_owned(),
        hops: 1,
        threshold,
        nodes,
    });

    let window_truncation = if analysis.counts.commits_filtered_bulk > 0 {
        Some(WindowTruncationBlock {
            commits_dropped: analysis.counts.commits_filtered_bulk,
            total_commits: analysis.counts.commits_seen,
            max_files: config.bulk.max_files,
            max_lines: config.bulk.max_lines,
        })
    } else {
        None
    };

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
            window_truncation,
        },
        files,
        session_files,
        session: SessionBlock {
            base_ref,
            base_sha,
            base_resolved_via: base_via,
            delta,
        },
        findings,
        blast_radius: blast_block,
    };

    serde_json::to_writer_pretty(&mut *w, &report)?;
    writeln!(w)?;
    Ok(())
}

#[derive(Serialize)]
struct ReviewBlock<'a> {
    /// One of `working_tree`, `staged`, `range`, `commit`. Lets the
    /// consumer reason about what was compared without re-deriving
    /// from CLI args.
    mode: &'static str,
    diff: ReviewDiffBlock<'a>,
}

#[derive(Serialize)]
struct ReviewDiffBlock<'a> {
    files_changed: u32,
    lines_added: u64,
    lines_deleted: u64,
    files: Vec<ReviewDiffFile<'a>>,
    /// Fraction of changed paths the historical analyzer has never
    /// seen. Optional: omitted when not computed (e.g. bulk-self
    /// path) or zero. Lets a consumer reason about why
    /// HOTSPOT/COUPLING/DRIFT are silent without re-deriving from
    /// the diff.
    #[serde(skip_serializing_if = "Option::is_none")]
    new_file_fraction: Option<f64>,
    /// v0.6 BUDGET accounting block. Present only when
    /// `bulk.ignore_for_budget` matched at least one file in the
    /// diff — at that point gross / net diverge and the agent
    /// needs to see both. When the globset is empty or didn't
    /// match, this key is omitted (gross == net, redundant with
    /// `files_changed` / `lines_added` + `lines_deleted`).
    #[serde(skip_serializing_if = "Option::is_none")]
    budget: Option<ReviewBudgetBlock>,
}

#[derive(Serialize)]
struct ReviewBudgetBlock {
    files_gross: u32,
    files_net: u32,
    lines_gross: u64,
    lines_net: u64,
    /// Globs from `bulk.ignore_for_budget` actively in effect.
    /// Echoed for diagnostic transparency.
    ignored_for_budget: Vec<String>,
}

#[derive(Serialize)]
struct ReviewDiffFile<'a> {
    path: &'a str,
    added: u64,
    deleted: u64,
}

#[derive(Serialize)]
struct ReviewReport<'a> {
    schema_version: &'static str,
    crate_version: &'static str,
    repo: RepoBlock<'a>,
    config: &'a Config,
    analysis: AnalysisBlock,
    review: ReviewBlock<'a>,
    findings: &'a [crate::output::findings::Finding],
    #[serde(skip_serializing_if = "Option::is_none")]
    health: Option<HealthBlock<'a>>,
    /// `cohesion` block — present only when COHESION fired with the
    /// full per-cluster file split. The structured form lets a
    /// harness render the split as a commit-split proposal without
    /// parsing `findings[].message`.
    #[serde(skip_serializing_if = "Option::is_none")]
    cohesion: Option<CohesionBlock>,
}

#[derive(Serialize)]
struct ReviewEmptyReport {
    schema_version: &'static str,
    crate_version: &'static str,
    review: ReviewEmptyBlock,
    findings: Vec<()>,
}

#[derive(Serialize)]
struct ReviewEmptyBlock {
    mode: &'static str,
    diff: ReviewEmptyDiff,
}

#[derive(Serialize)]
struct ReviewEmptyDiff {
    files_changed: u32,
    lines_added: u64,
    lines_deleted: u64,
    files: Vec<()>,
}

/// Write a `mmk review` JSON envelope: standard `repo`/`config`/
/// `analysis` blocks plus a `review` block (mode + per-file diff
/// numstat), the `findings` array, the optional `health` block, and
/// the optional `cohesion` block.
///
/// Used when there are changes to review; clean-tree calls go
/// through [`write_review_empty`].
#[allow(clippy::too_many_arguments)]
pub(crate) fn write_review<W: Write>(
    w: &mut W,
    mode: crate::commands::review::ReviewMode,
    changed: &[crate::commands::review::ChangedFile],
    findings: &[crate::output::findings::Finding],
    analysis: &AnalyzeOutput,
    duration_ms: u64,
    config: &Config,
    health_matches: &[mmk_health::HealthFinding],
    health_patterns: &[mmk_health::HealthPattern],
    new_file_fraction: Option<f64>,
    counts: &crate::commands::review::BudgetCounts,
    cohesion_tangles: &[crate::commands::review::CohesionTangle],
) -> Result<()> {
    let path_strs: Vec<String> = changed
        .iter()
        .map(|c| c.path.to_string_lossy().into_owned())
        .collect();
    let files: Vec<ReviewDiffFile<'_>> = changed
        .iter()
        .zip(path_strs.iter())
        .map(|(c, s)| ReviewDiffFile {
            path: s.as_str(),
            added: c.added,
            deleted: c.deleted,
        })
        .collect();
    let lines_added: u64 = changed.iter().map(|c| c.added).sum();
    let lines_deleted: u64 = changed.iter().map(|c| c.deleted).sum();

    let report = ReviewReport {
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
            window_truncation: None,
        },
        review: ReviewBlock {
            mode: mode.as_str(),
            diff: ReviewDiffBlock {
                files_changed: u32::try_from(changed.len()).unwrap_or(u32::MAX),
                lines_added,
                lines_deleted,
                files,
                new_file_fraction,
                budget: review_budget_block(counts),
            },
        },
        findings,
        health: health_block(health_matches, health_patterns),
        cohesion: cohesion_block(cohesion_tangles),
    };

    serde_json::to_writer_pretty(&mut *w, &report)?;
    writeln!(w)?;
    Ok(())
}

fn review_budget_block(
    counts: &crate::commands::review::BudgetCounts,
) -> Option<ReviewBudgetBlock> {
    if !counts.has_ignored() {
        return None;
    }
    Some(ReviewBudgetBlock {
        files_gross: counts.files_gross,
        files_net: counts.files_net,
        lines_gross: counts.lines_gross,
        lines_net: counts.lines_net,
        ignored_for_budget: counts.ignored_for_budget.clone(),
    })
}

#[derive(Serialize)]
struct PreEditBlock<'a> {
    path: &'a str,
}

/// `health` block — present only when the Health adapter ran AND
/// returned at least one match. Surfaces the pattern + related
/// list structurally so consumers can read it without parsing
/// `findings[].message`.
#[derive(Serialize)]
struct HealthBlock<'a> {
    /// Pattern tokens evaluated for this run (echoed from
    /// `cfg.health.ts.patterns`). Lets the consumer see *what was
    /// asked*, not just what fired.
    patterns_evaluated: Vec<&'static str>,
    matches: Vec<HealthMatchEntry<'a>>,
}

#[derive(Serialize)]
struct HealthMatchEntry<'a> {
    pattern: &'static str,
    subject: String,
    related: Vec<String>,
    #[serde(skip)]
    _phantom: std::marker::PhantomData<&'a ()>,
}

fn health_block<'a>(
    matches: &[mmk_health::HealthFinding],
    patterns: &[mmk_health::HealthPattern],
) -> Option<HealthBlock<'a>> {
    if matches.is_empty() {
        return None;
    }
    Some(HealthBlock {
        patterns_evaluated: patterns.iter().map(|p| p.token()).collect(),
        matches: matches
            .iter()
            .map(|m| HealthMatchEntry {
                pattern: m.pattern.token(),
                subject: m.subject.display().to_string(),
                related: m.related.iter().map(|r| r.display().to_string()).collect(),
                _phantom: std::marker::PhantomData,
            })
            .collect(),
    })
}

/// `cohesion` block — surfaces the COHESION cluster decomposition.
/// Always carries the *full* qualifying split (the prose text caps
/// at 8 paths to stay legible; the structured form has no cap so
/// harnesses can render it as a commit-split proposal).
#[derive(Serialize)]
pub(crate) struct CohesionBlock {
    pub(crate) tangles: Vec<TangleEntry>,
}

#[derive(Serialize)]
pub(crate) struct TangleEntry {
    /// One vec per qualifying cluster; paths are sorted within each
    /// cluster (lex by path) and clusters are sorted by their
    /// smallest path. Deterministic ordering matches the
    /// canonical_cluster_signature used for monotonic dedup.
    pub(crate) clusters: Vec<Vec<String>>,
}

/// Build the cohesion block from the qualifying clusters.
/// Returns `None` when no tangle qualified — keeps the JSON shape
/// additive (`cohesion` field is absent rather than empty).
pub(crate) fn cohesion_block(
    tangles: &[crate::commands::review::CohesionTangle],
) -> Option<CohesionBlock> {
    if tangles.is_empty() {
        return None;
    }
    let entries: Vec<TangleEntry> = tangles
        .iter()
        .map(|qualifying| {
            let mut clusters: Vec<Vec<String>> = qualifying
                .iter()
                .map(|c| {
                    let mut paths: Vec<String> =
                        c.iter().map(|p| p.display().to_string()).collect();
                    paths.sort();
                    paths
                })
                .collect();
            clusters.sort_by(|a, b| a.first().cmp(&b.first()));
            TangleEntry { clusters }
        })
        .collect();
    Some(CohesionBlock { tangles: entries })
}

#[derive(Serialize)]
struct PreEditReport<'a> {
    schema_version: &'static str,
    crate_version: &'static str,
    repo: RepoBlock<'a>,
    config: &'a Config,
    analysis: AnalysisBlock,
    pre_edit: PreEditBlock<'a>,
    findings: &'a [crate::output::findings::Finding],
    #[serde(skip_serializing_if = "Option::is_none")]
    health: Option<HealthBlock<'a>>,
}

/// Write a `mmk pre-edit` JSON envelope: the standard `repo` /
/// `config` / `analysis` blocks plus a `pre_edit.path` echo, the
/// `findings` array, and the optional `health` block.
#[allow(clippy::too_many_arguments)]
pub(crate) fn write_pre_edit<W: Write>(
    w: &mut W,
    target: &std::path::Path,
    findings: &[crate::output::findings::Finding],
    analysis: &AnalyzeOutput,
    duration_ms: u64,
    config: &Config,
    health_matches: &[mmk_health::HealthFinding],
    health_patterns: &[mmk_health::HealthPattern],
) -> Result<()> {
    let path = target.to_string_lossy();
    let report = PreEditReport {
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
            window_truncation: None,
        },
        pre_edit: PreEditBlock { path: &path },
        findings,
        health: health_block(health_matches, health_patterns),
    };

    serde_json::to_writer_pretty(&mut *w, &report)?;
    writeln!(w)?;
    Ok(())
}

#[derive(Serialize)]
struct DriftReport<'a> {
    schema_version: &'static str,
    crate_version: &'static str,
    drift: DriftBlock<'a>,
    findings: Vec<DriftFindingDto<'a>>,
    duration_ms: u64,
}

#[derive(Serialize)]
struct DriftBlock<'a> {
    /// Echoed from `--base` for diagnostics; the boundary walk
    /// currently always anchors on HEAD.
    base: Option<&'a str>,
    sessions: usize,
    /// One label per snapshot (currently the commit OID at the
    /// session boundary). Surfacing labels rather than full
    /// rankings keeps the JSON small; consumers wanting per-snapshot
    /// rankings can re-run analyze at each label.
    snapshot_labels: Vec<&'a str>,
}

#[derive(Serialize)]
struct DriftFindingDto<'a> {
    layer: &'static str,
    severity: &'static str,
    path: &'a str,
    climb_transitions: u32,
    total_transitions: u32,
    latest_rank: u32,
}

pub(crate) fn write_drift<W: Write>(
    w: &mut W,
    base: Option<&str>,
    sessions: usize,
    snapshots: &[mmk_core::drift::Snapshot],
    findings: &[mmk_core::drift::DriftFinding],
    duration_ms: u64,
) -> Result<()> {
    let labels: Vec<&str> = snapshots.iter().map(|s| s.label.as_str()).collect();
    let path_strs: Vec<String> = findings
        .iter()
        .map(|f| f.path.to_string_lossy().into_owned())
        .collect();
    let dtos: Vec<DriftFindingDto<'_>> = findings
        .iter()
        .zip(path_strs.iter())
        .map(|(f, p)| DriftFindingDto {
            layer: "drift",
            severity: "warn",
            path: p.as_str(),
            climb_transitions: f.climb_transitions,
            total_transitions: f.total_transitions,
            latest_rank: f.latest_rank,
        })
        .collect();

    let report = DriftReport {
        schema_version: crate::output::schema::SCHEMA_VERSION,
        crate_version: env!("CARGO_PKG_VERSION"),
        drift: DriftBlock {
            base,
            sessions,
            snapshot_labels: labels,
        },
        findings: dtos,
        duration_ms,
    };
    serde_json::to_writer_pretty(&mut *w, &report)?;
    writeln!(w)?;
    Ok(())
}

#[derive(Serialize)]
struct ReviewBulkReport<'a> {
    schema_version: &'static str,
    crate_version: &'static str,
    review: ReviewBlock<'a>,
    findings: &'a [crate::output::findings::Finding],
    duration_ms: u64,
}

/// `mmk review` envelope for the bulk-self-filter path: the input
/// diff itself tripped `bulk.max_files` / `max_lines`, so HOTSPOT and
/// COUPLING were intentionally suppressed. Emits the diff numstat +
/// the single BUDGET finding without paying the analyze cost.
pub(crate) fn write_review_bulk<W: Write>(
    w: &mut W,
    mode: crate::commands::review::ReviewMode,
    changed: &[crate::commands::review::ChangedFile],
    counts: &crate::commands::review::BudgetCounts,
    findings: &[crate::output::findings::Finding],
    duration_ms: u64,
) -> Result<()> {
    let path_strs: Vec<String> = changed
        .iter()
        .map(|c| c.path.to_string_lossy().into_owned())
        .collect();
    let files: Vec<ReviewDiffFile<'_>> = changed
        .iter()
        .zip(path_strs.iter())
        .map(|(c, s)| ReviewDiffFile {
            path: s.as_str(),
            added: c.added,
            deleted: c.deleted,
        })
        .collect();
    let lines_added: u64 = changed.iter().map(|c| c.added).sum();
    let lines_deleted: u64 = changed.iter().map(|c| c.deleted).sum();

    let report = ReviewBulkReport {
        schema_version: crate::output::schema::SCHEMA_VERSION,
        crate_version: env!("CARGO_PKG_VERSION"),
        review: ReviewBlock {
            mode: mode.as_str(),
            diff: ReviewDiffBlock {
                files_changed: u32::try_from(changed.len()).unwrap_or(u32::MAX),
                lines_added,
                lines_deleted,
                files,
                new_file_fraction: None,
                budget: review_budget_block(counts),
            },
        },
        findings,
        duration_ms,
    };

    serde_json::to_writer_pretty(&mut *w, &report)?;
    writeln!(w)?;
    Ok(())
}

/// Minimal `mmk review` envelope for the clean-tree / no-op-range
/// case: skips `repo`/`config`/`analysis` (we didn't run analyze)
/// and emits `findings: []`. Lets a hook see the same top-level
/// `findings` key on every invocation without paying the analyze
/// cost when there's nothing to review.
pub(crate) fn write_review_empty<W: Write>(
    w: &mut W,
    mode: crate::commands::review::ReviewMode,
) -> Result<()> {
    let report = ReviewEmptyReport {
        schema_version: crate::output::schema::SCHEMA_VERSION,
        crate_version: env!("CARGO_PKG_VERSION"),
        review: ReviewEmptyBlock {
            mode: mode.as_str(),
            diff: ReviewEmptyDiff {
                files_changed: 0,
                lines_added: 0,
                lines_deleted: 0,
                files: Vec::new(),
            },
        },
        findings: Vec::new(),
    };
    serde_json::to_writer_pretty(&mut *w, &report)?;
    writeln!(w)?;
    Ok(())
}
