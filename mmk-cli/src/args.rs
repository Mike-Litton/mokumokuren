use clap::{Parser, Subcommand, ValueEnum};

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

    /// Ignore paths matching this glob. Repeatable.
    #[arg(long = "ignore", value_name = "GLOB")]
    pub ignores: Vec<String>,

    /// Print extra progress/warnings on stderr.
    #[arg(short, long)]
    pub verbose: bool,
}
