use ahash::AHashSet;
use anyhow::{Context, Result};
use mmk_config::{Config, ConfigFile};
use mmk_core::coupling;
use std::io::Write;
use std::path::PathBuf;
use std::time::Instant;

use crate::args::{AnalyzeArgs, Format};
use crate::output::{json, text};

/// Couples emitted per hotspot entry. Hardcoded — adding a config knob
/// for it adds surface without buying signal at the resolution
/// `mmk` currently produces.
pub(crate) const COUPLES_PER_FILE: usize = 5;

pub fn run<O: Write, E: Write>(args: &AnalyzeArgs, stdout: &mut O, stderr: &mut E) -> Result<()> {
    let cwd = std::env::current_dir().context("failed to determine current directory")?;

    let window = humantime::parse_duration(&args.since)
        .with_context(|| format!("invalid --since value: {}", args.since))?;
    let window_days = u32::try_from(window.as_secs() / 86_400)
        .unwrap_or(u32::MAX)
        .max(1);

    // Load mokumokuren.toml — explicit `--config` wins; otherwise look at
    // the Git repo root. Discovery uses gix to find the work tree, since
    // `cwd` may be deep inside a subdirectory.
    let (file_cfg, file_path) = load_config_file(&cwd, args.config.as_deref())?;

    let mut cfg = Config::default();
    cfg.window.days = window_days;
    cfg.hotspot.top_n = args.top;
    // Union: file first (so CLI flags appear after — order matters only
    // for diagnostics, not glob semantics).
    cfg.ignores = file_cfg.ignore;
    cfg.ignores.extend(args.ignores.iter().cloned());
    // Effective blast-radius threshold: CLI override → TOML config →
    // built-in default. The CLI value is single-source-of-truth here;
    // analyze.rs and session.rs compose the same precedence.
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
        if analysis.counts.head_paths_ignored > 0 {
            writeln!(
                stderr,
                "{} HEAD path(s) excluded by ignore globs",
                analysis.counts.head_paths_ignored
            )?;
        }
        for warn in &analysis.warnings {
            writeln!(stderr, "warning: {warn}")?;
        }
    }

    let now_ts = analysis.head_timestamp.unwrap_or_else(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64)
    });

    // `MMK_TRACE=1` prints per-phase wall times for the post-diff
    // aggregation. Mirrors the gate in `mmk_git::analyze`.
    let trace = std::env::var_os("MMK_TRACE").is_some();
    let phase = |name: &str, t: Instant| {
        if trace {
            eprintln!(
                "[mmk] {name}: {:>6.1} ms",
                t.elapsed().as_secs_f64() * 1000.0
            );
        }
    };

    let t = Instant::now();
    let weighted = mmk_core::churn::weighted_churn(&analysis.commits, now_ts, cfg.tau_seconds());
    phase("weighted_churn", t);
    let t = Instant::now();
    let relative = mmk_core::churn::relative_churn(&weighted, &analysis.loc);
    phase("relative_churn", t);
    let t = Instant::now();
    let commits_touching = mmk_core::churn::commits_touching(&analysis.commits);
    phase("commits_touching", t);
    let t = Instant::now();
    let last_modified = mmk_core::last_modified(&analysis.commits);
    phase("last_modified", t);
    let t = Instant::now();
    let mut ranked = mmk_core::hotspot::rank(
        mmk_core::hotspot::RankInputs {
            weighted: &weighted,
            relative: &relative,
            loc: &analysis.loc,
            commits_touching: &commits_touching,
            last_modified: &last_modified,
        },
        cfg.hotspot.top_n,
    );
    phase("hotspot::rank", t);

    // Coupling: walk commits once for the top-N targets and attach the
    // top-K partners to each ranked entry. `rank()` stays cheap and
    // pure; coupling is a separate, optional pass.
    let t = Instant::now();
    let targets: AHashSet<PathBuf> = ranked.iter().map(|e| e.path.clone()).collect();
    if !targets.is_empty() {
        let mut couples = coupling::top_couples_for(&analysis.commits, &targets, COUPLES_PER_FILE);
        for entry in &mut ranked {
            if let Some(list) = couples.remove(&entry.path) {
                entry.top_couples = list;
            }
        }
    }
    phase("coupling::top_couples_for", t);

    // Optional 1-hop blast-radius lookup. Threshold is the effective
    // value resolved from CLI/TOML/default into `cfg.blast_radius`.
    let blast_threshold = cfg.blast_radius.threshold;
    let blast_nodes = args
        .blast_radius
        .as_ref()
        .map(|p| {
            let nodes = coupling::neighborhood(&analysis.commits, p, 1, blast_threshold)?;
            Ok::<_, anyhow::Error>((p.clone(), nodes))
        })
        .transpose()?;
    let blast_ref: Option<(&std::path::Path, f64, &[mmk_core::NeighborhoodNode])> = blast_nodes
        .as_ref()
        .map(|(p, n)| (p.as_path(), blast_threshold, n.as_slice()));

    let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);

    match args.format {
        Format::Text => text::write(stdout, &ranked, &analysis, duration_ms, &cfg, blast_ref)?,
        Format::Json => {
            json::write(stdout, &ranked, &analysis, duration_ms, &cfg, blast_ref)?;
        }
    }
    Ok(())
}

/// Resolve which config file to load and load it. Precedence:
/// 1. Explicit `--config <path>` (must exist; error if missing).
/// 2. `mokumokuren.toml` at the Git repo root (auto-discovered).
/// 3. Default-empty.
fn load_config_file(
    cwd: &std::path::Path,
    explicit: Option<&std::path::Path>,
) -> Result<(ConfigFile, Option<PathBuf>)> {
    if let Some(path) = explicit {
        let cfg = ConfigFile::load_from_path(path)
            .with_context(|| format!("failed to load config from {}", path.display()))?;
        return Ok((cfg, Some(path.to_path_buf())));
    }
    if let Some(repo_root) = discover_repo_root(cwd) {
        let candidate = repo_root.join("mokumokuren.toml");
        if candidate.exists() {
            let cfg = ConfigFile::load_from_path(&candidate)
                .with_context(|| format!("failed to load config from {}", candidate.display()))?;
            return Ok((cfg, Some(candidate)));
        }
    }
    Ok((ConfigFile::default(), None))
}

/// Walk up from `start` to find the Git work-tree root. Returns `None`
/// if not in a Git repo (the analyze step will then fail with a clearer
/// error than a missing config).
fn discover_repo_root(start: &std::path::Path) -> Option<PathBuf> {
    mmk_git::discover_work_dir(start)
}
