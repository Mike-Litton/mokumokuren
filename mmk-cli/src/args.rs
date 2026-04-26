use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(
    name = "mmk",
    bin_name = "mmk",
    version,
    about = "Evidence-based Git health metrics for humans and LLM agents.",
    arg_required_else_help = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Compute hotspots for the current Git repository.
    Analyze(AnalyzeArgs),
    /// Compare the current session's churn ranking against a baseline
    /// ref. Surfaces what shifted "since I started" — entered top-N,
    /// rank climbs, thrash ratio, commit entropy.
    Session(SessionArgs),
    /// Write a starter `mokumokuren.toml` config file.
    Init(InitArgs),
    /// Inspect or clear the per-commit delta cache.
    Cache(CacheArgs),
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum Format {
    Text,
    Json,
}

#[derive(Debug, Parser)]
pub struct AnalyzeArgs {
    /// Analysis window (e.g. `180d`, `90days`, `6weeks`).
    #[arg(long, default_value = "180days")]
    pub since: String,

    /// Maximum number of hotspots to emit.
    #[arg(long, default_value_t = 20)]
    pub top: usize,

    /// Output format.
    #[arg(long, value_enum, default_value_t = Format::Text)]
    pub format: Format,

    /// Ignore paths matching this glob. Repeatable. Unioned with any
    /// `ignore` entries from `mokumokuren.toml`.
    #[arg(long = "ignore", value_name = "GLOB")]
    pub ignores: Vec<String>,

    /// Path to a config file. Defaults to `mokumokuren.toml` at the
    /// repo root if present.
    #[arg(long, value_name = "PATH")]
    pub config: Option<PathBuf>,

    /// Print extra progress/warnings on stderr.
    #[arg(short, long)]
    pub verbose: bool,

    /// Return only the coupling list for the given path. When set,
    /// suppresses the ranked `files` block; output is just the
    /// co-change partners of `<PATH>`.
    #[arg(long = "couples-of", value_name = "PATH")]
    pub couples_of: Option<PathBuf>,

    /// Render an indented `couples:` block under each ranked file in
    /// text output. Off by default — keeps the table grep-friendly.
    #[arg(long)]
    pub couples: bool,

    /// Emit a 1-hop blast-radius neighborhood for the given path
    /// alongside the ranked output. Each node is a co-changing partner
    /// at jaccard ≥ the effective threshold.
    #[arg(long = "blast-radius", value_name = "PATH")]
    pub blast_radius: Option<PathBuf>,

    /// Override the Jaccard threshold for `--blast-radius`.
    /// Falls back to `[blast_radius] threshold` in
    /// `mokumokuren.toml`, then to the built-in default (0.10).
    #[arg(long = "blast-radius-threshold", value_name = "FLOAT")]
    pub blast_radius_threshold: Option<f64>,
}

#[derive(Debug, Parser)]
pub struct SessionArgs {
    /// Analysis window for the *baseline* ranking. The session ranking
    /// is whatever subset of those commits is reachable from HEAD but
    /// not from the resolved base.
    #[arg(long, default_value = "180days")]
    pub since: String,

    /// Maximum number of hotspots to emit in each ranking.
    #[arg(long, default_value_t = 20)]
    pub top: usize,

    /// Output format.
    #[arg(long, value_enum, default_value_t = Format::Text)]
    pub format: Format,

    /// Explicit base ref to compare against (e.g. `main`, `origin/main`).
    /// Mutually exclusive with `--since-commit`.
    #[arg(long, value_name = "REF", conflicts_with = "since_commit")]
    pub base: Option<String>,

    /// Explicit base commit SHA. Mutually exclusive with `--base`.
    #[arg(long = "since-commit", value_name = "SHA", conflicts_with = "base")]
    pub since_commit: Option<String>,

    /// Ignore paths matching this glob. Repeatable. Unioned with any
    /// `ignore` entries from `mokumokuren.toml`.
    #[arg(long = "ignore", value_name = "GLOB")]
    pub ignores: Vec<String>,

    /// Path to a config file. Defaults to `mokumokuren.toml` at the
    /// repo root if present.
    #[arg(long, value_name = "PATH")]
    pub config: Option<PathBuf>,

    /// Print extra progress/warnings on stderr.
    #[arg(short, long)]
    pub verbose: bool,

    /// Emit a 1-hop blast-radius neighborhood for the given path
    /// alongside the session report.
    #[arg(long = "blast-radius", value_name = "PATH")]
    pub blast_radius: Option<PathBuf>,

    /// Override the Jaccard threshold for `--blast-radius`.
    /// Falls back to `[blast_radius] threshold` in
    /// `mokumokuren.toml`, then to the built-in default (0.10).
    #[arg(long = "blast-radius-threshold", value_name = "FLOAT")]
    pub blast_radius_threshold: Option<f64>,
}

#[derive(Debug, Parser)]
pub struct InitArgs {
    /// Overwrite an existing `mokumokuren.toml`.
    #[arg(long)]
    pub force: bool,
}

#[derive(Debug, Parser)]
pub struct CacheArgs {
    #[command(subcommand)]
    pub command: CacheCommand,
}

#[derive(Debug, Subcommand)]
pub enum CacheCommand {
    /// Print the cache location, entry count, and on-disk size for the
    /// current repository.
    Info,
    /// Delete the cache for the current repository. Next `analyze` rebuilds.
    Clear,
}
