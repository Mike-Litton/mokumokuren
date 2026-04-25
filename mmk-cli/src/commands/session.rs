use ahash::AHashSet;
use anyhow::{Context, Result};
use mmk_config::{Config, ConfigFile};
use mmk_core::coupling;
use std::io::Write;
use std::path::PathBuf;
use std::time::Instant;

use crate::args::{Format, SessionArgs};
use crate::commands::analyze::COUPLES_PER_FILE;

pub fn run<O: Write, E: Write>(args: &SessionArgs, stdout: &mut O, stderr: &mut E) -> Result<()> {
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
    cfg.ignores.extend(args.ignores.iter().cloned());
    if let Some(file_br) = file_cfg.blast_radius.as_ref() {
        cfg.blast_radius.threshold = file_br.threshold;
    }
    if let Some(t) = args.blast_radius_threshold {
        cfg.blast_radius.threshold = t;
    }

    let started = Instant::now();
    let session_out = mmk_git::analyze_session(
        &cwd,
        &cfg,
        args.base.as_deref(),
        args.since_commit.as_deref(),
    )?;

    if args.verbose {
        match &file_path {
            Some(p) => writeln!(stderr, "loaded config from {}", p.display())?,
            None => writeln!(stderr, "no mokumokuren.toml found; running with defaults")?,
        }
        for warn in &session_out.window.warnings {
            writeln!(stderr, "warning: {warn}")?;
        }
    }

    let now_ts = session_out.window.head_timestamp.unwrap_or_else(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    });

    // Window ranking — the baseline "what's hot in the full window".
    let weighted = mmk_core::churn::weighted_churn(
        &session_out.window.commits,
        now_ts,
        cfg.tau_seconds(),
    );
    let relative = mmk_core::churn::relative_churn(&weighted, &session_out.window.loc);
    let commits_touching = mmk_core::churn::commits_touching(&session_out.window.commits);
    let last_modified = mmk_core::last_modified(&session_out.window.commits);
    let mut window_ranked = mmk_core::hotspot::rank(
        &weighted,
        &relative,
        &session_out.window.loc,
        &commits_touching,
        &last_modified,
        cfg.hotspot.top_n,
    );

    // Session ranking — only commits since the resolved base.
    // LOC denominator here is the **session base epoch** (see
    // `SessionAnalyzeOutput` docs): `session_relative_churn =
    // session_weighted_churn / base_LOC`, so a file truncated
    // post-session doesn't get an artificially inflated ratio.
    let s_weighted =
        mmk_core::churn::weighted_churn(&session_out.session_commits, now_ts, cfg.tau_seconds());
    let s_relative = mmk_core::churn::relative_churn(&s_weighted, &session_out.session_loc);
    let s_commits_touching = mmk_core::churn::commits_touching(&session_out.session_commits);
    let s_last_modified = mmk_core::last_modified(&session_out.session_commits);
    let mut session_ranked = mmk_core::hotspot::rank(
        &s_weighted,
        &s_relative,
        &session_out.session_loc,
        &s_commits_touching,
        &s_last_modified,
        cfg.hotspot.top_n,
    );

    // Couples: compute against the union of paths in either ranking
    // and attach to whichever entries hold them. Single coupling pass
    // over the full window keeps the cost bounded.
    let mut targets: AHashSet<PathBuf> = AHashSet::new();
    for e in window_ranked.iter().chain(session_ranked.iter()) {
        targets.insert(e.path.clone());
    }
    if !targets.is_empty() {
        let couples =
            coupling::top_couples_for(&session_out.window.commits, &targets, COUPLES_PER_FILE);
        for entry in window_ranked.iter_mut().chain(session_ranked.iter_mut()) {
            if let Some(list) = couples.get(&entry.path) {
                entry.top_couples = list.clone();
            }
        }
    }

    let delta = mmk_core::session::compute_delta(
        &window_ranked,
        &session_ranked,
        &session_out.session_commits,
    );

    let blast_threshold = cfg.blast_radius.threshold;
    let blast_nodes = args.blast_radius.as_ref().map(|p| {
        let nodes = coupling::neighborhood(
            &session_out.window.commits,
            p,
            1,
            blast_threshold,
        );
        (p.clone(), nodes)
    });
    let blast_ref: Option<(&std::path::Path, f64, &[mmk_core::NeighborhoodNode])> = blast_nodes
        .as_ref()
        .map(|(p, n)| (p.as_path(), blast_threshold, n.as_slice()));

    let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);

    match args.format {
        Format::Text => crate::output::text::write_session(
            stdout,
            &window_ranked,
            &session_ranked,
            &delta,
            &session_out,
            duration_ms,
            blast_ref,
        )?,
        Format::Json => crate::output::json::write_session(
            stdout,
            &window_ranked,
            &session_ranked,
            &delta,
            &session_out,
            duration_ms,
            &cfg,
            blast_ref,
        )?,
    }
    Ok(())
}

fn load_config_file(
    cwd: &std::path::Path,
    explicit: Option<&std::path::Path>,
) -> Result<(ConfigFile, Option<PathBuf>)> {
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
