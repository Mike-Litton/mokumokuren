use anyhow::{Context, Result};
use mmk_config::Config;
use std::io::Write;
use std::time::Instant;

use crate::args::{AnalyzeArgs, Format};

pub fn run<O: Write, E: Write>(args: &AnalyzeArgs, stdout: &mut O, stderr: &mut E) -> Result<()> {
    let cwd = std::env::current_dir().context("failed to determine current directory")?;

    let window = humantime::parse_duration(&args.since)
        .with_context(|| format!("invalid --since value: {}", args.since))?;
    let window_days = u32::try_from(window.as_secs() / 86_400)
        .unwrap_or(u32::MAX)
        .max(1);

    let mut cfg = Config::default();
    cfg.window.days = window_days;
    cfg.hotspot.top_n = args.top;
    cfg.ignores.clone_from(&args.ignores);

    let started = Instant::now();
    let analysis = mmk_git::analyze(&cwd, &cfg)?;

    if args.verbose {
        for warn in &analysis.warnings {
            writeln!(stderr, "warning: {warn}")?;
        }
    }

    let now_ts = analysis.head_timestamp.unwrap_or_else(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    });

    let weighted = mmk_core::churn::weighted_churn(&analysis.commits, now_ts, cfg.tau_seconds());
    let relative = mmk_core::churn::relative_churn(&weighted, &analysis.loc);
    let commits_touching = mmk_core::churn::commits_touching(&analysis.commits);
    let last_modified = mmk_core::last_modified(&analysis.commits);
    let ranked = mmk_core::hotspot::rank(
        &weighted,
        &relative,
        &analysis.loc,
        &commits_touching,
        &last_modified,
        cfg.hotspot.top_n,
    );

    let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);

    match args.format {
        Format::Text => crate::output::text::write(stdout, &ranked, &analysis, duration_ms, &cfg)?,
        Format::Json => crate::output::json::write(stdout, &ranked, &analysis, duration_ms, &cfg)?,
    }
    Ok(())
}
