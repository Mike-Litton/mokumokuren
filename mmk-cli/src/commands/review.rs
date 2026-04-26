//! `mmk review` — compare a diff against the historical baseline and
//! emit layer-labeled findings.
//!
//! Default mode is *working tree vs HEAD*: the agent edit-loop hot
//! path. `--staged` reads the index; `--range A..B` and
//! `--commit <SHA>` review committed work without going through
//! session-summary.
//!
//! Findings are HOTSPOT (changed file is in top-N), COUPLING
//! (changed file's expected partner is not also touched), and BUDGET
//! (diff exceeds `bulk.max_files` or `bulk.max_lines`).

use ahash::AHashSet;
use anyhow::{Context, Result};
use mmk_config::{Config, ConfigFile};
use mmk_core::coupling;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use crate::args::{Format, ReviewArgs};
use crate::commands::analyze::COUPLES_PER_FILE;
use crate::output::findings::{render_text, Finding, Layer, Severity};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReviewMode {
    WorkingTree,
    Staged,
    Range,
    Commit,
}

impl ReviewMode {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::WorkingTree => "working_tree",
            Self::Staged => "staged",
            Self::Range => "range",
            Self::Commit => "commit",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ChangedFile {
    pub(crate) path: PathBuf,
    pub(crate) added: u64,
    pub(crate) deleted: u64,
}

pub fn run<O: Write, E: Write>(args: &ReviewArgs, stdout: &mut O, stderr: &mut E) -> Result<()> {
    let cwd = std::env::current_dir().context("failed to determine current directory")?;

    let mode = if args.staged {
        ReviewMode::Staged
    } else if args.range.is_some() {
        ReviewMode::Range
    } else if args.commit.is_some() {
        ReviewMode::Commit
    } else {
        ReviewMode::WorkingTree
    };

    let changed = collect_diff(&cwd, mode, args)?;

    // Clean tree / no-op range: text mode says nothing, JSON mode
    // emits the envelope with empty findings so harnesses can still
    // parse a stable shape.
    if changed.is_empty() {
        return emit_empty(args, mode, stdout);
    }

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
    cfg.ignores.extend(args.ignores.iter().cloned());
    if let Some(file_br) = file_cfg.blast_radius.as_ref() {
        cfg.blast_radius.threshold = file_br.threshold;
    }
    if let Some(t) = args.blast_radius_threshold {
        cfg.blast_radius.threshold = t;
    }

    let started = Instant::now();
    let analysis = mmk_git::analyze(&cwd, &cfg)?;

    if args.verbose {
        match &file_path {
            Some(p) => writeln!(stderr, "loaded config from {}", p.display())?,
            None => writeln!(stderr, "no mokumokuren.toml found; running with defaults")?,
        }
    }

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

    let findings = compute_findings(&changed, &ranked, &analysis.commits, &cfg, args.top);

    let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);

    match args.format {
        Format::Text => render_text(stdout, &findings)?,
        Format::Json => crate::output::json::write_review(
            stdout,
            mode,
            &changed,
            &findings,
            &analysis,
            duration_ms,
            &cfg,
        )?,
    }
    Ok(())
}

fn emit_empty<O: Write>(args: &ReviewArgs, mode: ReviewMode, stdout: &mut O) -> Result<()> {
    match args.format {
        Format::Text => Ok(()),
        Format::Json => crate::output::json::write_review_empty(stdout, mode),
    }
}

/// Shell to `git` for diff parsing. Faster to write and externally
/// validated. Format is `--numstat`: tab-separated `added deleted path`,
/// one file per line. Binary files come back as `- - path` and are
/// skipped (no line counts).
fn collect_diff(cwd: &Path, mode: ReviewMode, args: &ReviewArgs) -> Result<Vec<ChangedFile>> {
    let mut cmd = Command::new("git");
    cmd.arg("diff").arg("--numstat").current_dir(cwd);
    match mode {
        ReviewMode::WorkingTree => {
            cmd.arg("HEAD");
        }
        ReviewMode::Staged => {
            cmd.arg("--cached");
        }
        ReviewMode::Range => {
            cmd.arg(args.range.as_ref().expect("range mode requires --range"));
        }
        ReviewMode::Commit => {
            let sha = args.commit.as_ref().expect("commit mode requires --commit");
            cmd.arg(format!("{sha}^..{sha}"));
        }
    }

    let out = cmd
        .output()
        .context("failed to invoke `git diff` — is git on PATH?")?;
    if !out.status.success() {
        anyhow::bail!(
            "git diff exited with {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
    }

    let mut files = Vec::new();
    for raw in String::from_utf8_lossy(&out.stdout).lines() {
        let mut parts = raw.splitn(3, '\t');
        let added = parts.next().unwrap_or("-");
        let deleted = parts.next().unwrap_or("-");
        let Some(path) = parts.next() else { continue };
        // Binary files: `-` for both columns. Drop them — they don't
        // contribute line-budget signal and we don't have rank data
        // for binary blobs anyway.
        if added == "-" && deleted == "-" {
            continue;
        }
        let added: u64 = added.parse().unwrap_or(0);
        let deleted: u64 = deleted.parse().unwrap_or(0);
        files.push(ChangedFile {
            path: PathBuf::from(path),
            added,
            deleted,
        });
    }
    Ok(files)
}

fn compute_findings(
    changed: &[ChangedFile],
    ranked: &[mmk_core::HotspotEntry],
    commits: &[mmk_core::types::Commit],
    cfg: &Config,
    top: usize,
) -> Vec<Finding> {
    let mut findings = Vec::new();
    let changed_set: AHashSet<PathBuf> = changed.iter().map(|c| c.path.clone()).collect();

    // HOTSPOT — changed file is ranked ≤ top.
    for c in changed {
        if let Some(entry) = ranked.iter().find(|e| e.path == c.path) {
            if (entry.hotspot_rank as usize) <= top {
                findings.push(Finding::new(
                    Layer::Hotspot,
                    Severity::Warn,
                    format!(
                        "{} ranks #{} (top-{} hotspot)",
                        c.path.display(),
                        entry.hotspot_rank,
                        top
                    ),
                ));
            }
        }
    }

    // COUPLING — changed file's expected partner above threshold not
    // also touched. Threshold reuses cfg.blast_radius.threshold so
    // operators have one knob for "what counts as a meaningful
    // partner."
    if !changed_set.is_empty() {
        let couples_map = coupling::top_couples_for(commits, &changed_set, COUPLES_PER_FILE);
        let threshold = cfg.blast_radius.threshold;
        for c in changed {
            let Some(partners) = couples_map.get(&c.path) else {
                continue;
            };
            for p in partners {
                if p.jaccard >= threshold && !changed_set.contains(&p.partner) {
                    findings.push(Finding::new(
                        Layer::Coupling,
                        Severity::Warn,
                        format!(
                            "{} edited; expected partner {} not touched (jaccard {:.2})",
                            c.path.display(),
                            p.partner.display(),
                            p.jaccard
                        ),
                    ));
                }
            }
        }
    }

    // BUDGET — delegated to mmk_core::budget::check_diff_budget so
    // review and session-summary share the exact threshold logic.
    let files_n = u32::try_from(changed.len()).unwrap_or(u32::MAX);
    let lines_n: u64 = changed.iter().map(|c| c.added + c.deleted).sum();
    let triggers = mmk_core::budget::check_diff_budget(
        &mmk_core::budget::BudgetCheck {
            files_changed: files_n,
            lines_changed: lines_n,
        },
        &cfg.bulk,
    );
    for t in triggers {
        let msg = match t {
            mmk_core::budget::BudgetTrigger::FilesExceeded { actual, max } => {
                format!("diff touches {actual} files; bulk.max_files = {max}")
            }
            mmk_core::budget::BudgetTrigger::LinesExceeded { actual, max } => {
                format!("diff is {actual} lines; bulk.max_lines = {max}")
            }
        };
        findings.push(Finding::new(Layer::Budget, Severity::Warn, msg));
    }

    findings
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
