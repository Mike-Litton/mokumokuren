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
    /// End-of-feature / PR-review summary: compare the current
    /// session's churn ranking against a baseline ref and overlay
    /// DRIFT (with `--drift-sessions K`) and BUDGET findings on top
    /// of the entered-top-N / rank-climbs / commit-entropy block.
    /// `mmk session` is kept as an alias for backward compat.
    #[command(name = "session-summary", alias = "session")]
    SessionSummary(SessionArgs),
    /// Emit findings comparing a diff (working-tree by default) against
    /// the historical baseline. Built for the agent edit loop:
    /// `PostToolUse:Edit` → `mmk review` → findings before any commit.
    Review(ReviewArgs),
    /// Emit findings about a single path *before* editing it. Built
    /// for the `PreToolUse:Edit` hook: feeds the agent the historical
    /// context (rank, expected partners, drift) for the file it's
    /// about to touch.
    #[command(name = "pre-edit")]
    PreEdit(PreEditArgs),
    /// Re-run analyze at K historical session boundaries and emit
    /// DRIFT findings for files that climbed in a majority of
    /// transitions. Slow path (K × analyze cost); intended for
    /// end-of-session / PR-review use, not the per-edit hook.
    Drift(DriftArgs),
    /// Write a starter `mokumokuren.toml` config file.
    Init(InitArgs),
    /// Sample recent commits, run `mmk review` against each, and emit
    /// a noise-floor report. Lets a new user calibrate
    /// `[coupling] threshold` and `ignore_partners` for their repo.
    Eval(EvalArgs),
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

    /// Number of recent sessions to inspect for DRIFT findings.
    /// Defaults to 0 (drift skipped). Set to 5 for a typical
    /// end-of-session view.
    #[arg(long = "drift-sessions", value_name = "K", default_value_t = 0)]
    pub drift_sessions: usize,

    /// Exit-code policy (see `mmk review --help`).
    #[arg(long, value_enum, default_value_t = Gate::None)]
    pub gate: Gate,
}

#[derive(Debug, Parser)]
pub struct ReviewArgs {
    /// Compare the staged index against HEAD. Default mode (no flag)
    /// is working tree vs HEAD — the per-edit hot path.
    #[arg(long, conflicts_with_all = ["range", "commit"])]
    pub staged: bool,

    /// Compare a committed range `A..B`. Used for end-of-feature
    /// review (`--range main..HEAD`) without going through
    /// session-summary.
    #[arg(long, value_name = "A..B", conflicts_with = "commit")]
    pub range: Option<String>,

    /// Compare a single commit against its first parent.
    #[arg(long, value_name = "SHA")]
    pub commit: Option<String>,

    /// Window for the historical baseline (couples + ranking).
    #[arg(long, default_value = "180days")]
    pub since: String,

    /// Top-N hotspot threshold. Files at rank ≤ this fire HOTSPOT.
    #[arg(long, default_value_t = 20)]
    pub top: usize,

    /// Output format.
    #[arg(long, value_enum, default_value_t = Format::Text)]
    pub format: Format,

    /// Ignore paths matching this glob. Repeatable. Unioned with
    /// `mokumokuren.toml`.
    #[arg(long = "ignore", value_name = "GLOB")]
    pub ignores: Vec<String>,

    /// Path to a config file. Defaults to `mokumokuren.toml` at the
    /// repo root if present.
    #[arg(long, value_name = "PATH")]
    pub config: Option<PathBuf>,

    /// Print extra progress/warnings on stderr.
    #[arg(short, long)]
    pub verbose: bool,

    /// Override the Jaccard threshold for COUPLING findings.
    /// Falls back to `[coupling] threshold` in `mokumokuren.toml`,
    /// then to the built-in default (0.30).
    #[arg(long = "coupling-threshold", value_name = "FLOAT")]
    pub coupling_threshold: Option<f64>,

    /// Deprecated alias for `--coupling-threshold`. Kept so existing
    /// CLI invocations don't break; users should migrate to
    /// `--coupling-threshold`.
    #[arg(long = "blast-radius-threshold", value_name = "FLOAT", hide = true)]
    pub blast_radius_threshold: Option<f64>,

    /// Exit-code policy. `none` (default) always exits 0 unless mmk
    /// itself errors. `warn` exits 1 if any warn-severity finding
    /// fires; `error` exits 1 if any error-severity finding fires.
    #[arg(long, value_enum, default_value_t = Gate::None)]
    pub gate: Gate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Gate {
    None,
    Warn,
    Error,
}

#[derive(Debug, Parser)]
pub struct PreEditArgs {
    /// File to look up. Relative to the repo root.
    pub path: PathBuf,

    /// Window for the historical baseline (couples + ranking).
    #[arg(long, default_value = "180days")]
    pub since: String,

    /// Top-N hotspot threshold. Path firing at rank ≤ this gets the
    /// HOTSPOT finding.
    #[arg(long, default_value_t = 20)]
    pub top: usize,

    /// Output format.
    #[arg(long, value_enum, default_value_t = Format::Text)]
    pub format: Format,

    /// Ignore paths matching this glob. Repeatable. Unioned with
    /// `mokumokuren.toml`.
    #[arg(long = "ignore", value_name = "GLOB")]
    pub ignores: Vec<String>,

    /// Path to a config file. Defaults to `mokumokuren.toml` at the
    /// repo root if present.
    #[arg(long, value_name = "PATH")]
    pub config: Option<PathBuf>,

    /// Print extra progress/warnings on stderr.
    #[arg(short, long)]
    pub verbose: bool,

    /// Override the Jaccard threshold for COUPLING findings.
    /// Falls back to `[coupling] threshold` in `mokumokuren.toml`,
    /// then to the built-in default (0.30).
    #[arg(long = "coupling-threshold", value_name = "FLOAT")]
    pub coupling_threshold: Option<f64>,

    /// Deprecated alias for `--coupling-threshold`. Kept so existing
    /// CLI invocations don't break.
    #[arg(long = "blast-radius-threshold", value_name = "FLOAT", hide = true)]
    pub blast_radius_threshold: Option<f64>,

    /// Number of recent sessions to inspect for DRIFT findings.
    /// Defaults to 0 (drift skipped) until Step 4 lands the
    /// `compute_drift` engine; once enabled, set to 5 to match the
    /// `mmk drift` default.
    #[arg(long = "drift-sessions", value_name = "K", default_value_t = 0)]
    pub drift_sessions: usize,

    /// Exit-code policy (see `mmk review --help`).
    #[arg(long, value_enum, default_value_t = Gate::None)]
    pub gate: Gate,
}

#[derive(Debug, Parser)]
pub struct DriftArgs {
    /// How many session snapshots to compute. K-1 transitions get
    /// inspected for climbs.
    #[arg(long, default_value_t = 5)]
    pub sessions: usize,

    /// Base ref label (informational; the boundary walk currently
    /// always starts at HEAD). Surfaced in the JSON `drift.base` so
    /// consumers can label the result.
    #[arg(long, value_name = "REF")]
    pub base: Option<String>,

    /// Window for each snapshot's analyze.
    #[arg(long, default_value = "180days")]
    pub since: String,

    /// Top-N retained per snapshot ranking. Lower = faster + tighter
    /// climb signal.
    #[arg(long, default_value_t = 20)]
    pub top: usize,

    /// Output format.
    #[arg(long, value_enum, default_value_t = Format::Text)]
    pub format: Format,

    /// Ignore paths matching this glob. Repeatable.
    #[arg(long = "ignore", value_name = "GLOB")]
    pub ignores: Vec<String>,

    /// Path to a config file. Defaults to `mokumokuren.toml` at the
    /// repo root if present.
    #[arg(long, value_name = "PATH")]
    pub config: Option<PathBuf>,

    /// Print extra progress/warnings on stderr.
    #[arg(short, long)]
    pub verbose: bool,
}

#[derive(Debug, Parser)]
pub struct InitArgs {
    /// Overwrite an existing `mokumokuren.toml`.
    #[arg(long)]
    pub force: bool,

    /// Bundled profile to write. Default = the generic starter
    /// (no opinionated ignores). `js-ts`, `rust`, `python`, `go`
    /// ship ecosystem-specific defaults.
    #[arg(long, value_name = "NAME")]
    pub profile: Option<String>,
}

#[derive(Debug, Parser)]
pub struct EvalArgs {
    /// Number of recent (non-merge) commits to sample.
    #[arg(long, default_value_t = 50)]
    pub sample: usize,

    /// Window for the historical baseline used by each `mmk review`
    /// invocation. Same semantics as `mmk review --since`.
    #[arg(long, default_value = "180days")]
    pub since: String,

    /// Top-N hotspot threshold passed to each underlying review.
    #[arg(long, default_value_t = 20)]
    pub top: usize,

    /// Output format.
    #[arg(long, value_enum, default_value_t = Format::Text)]
    pub format: Format,

    /// Path to a config file. Defaults to `mokumokuren.toml` at the
    /// repo root if present.
    #[arg(long, value_name = "PATH")]
    pub config: Option<PathBuf>,

    /// Print extra progress on stderr.
    #[arg(short, long)]
    pub verbose: bool,

    /// Append a `[coupling] ignore_partners = [...]` suggestion block
    /// to the report, listing partners that fire across many
    /// unrelated subjects (high breadth, low inverse conditional
    /// probability — typically system-level files like CHANGELOG that
    /// move whenever anything ships).
    #[arg(long)]
    pub learn: bool,
}

#[derive(Debug, Parser)]
pub struct CacheArgs {
    #[command(subcommand)]
    pub command: CacheCommand,
}

#[derive(Debug, Subcommand)]
pub enum CacheCommand {
    /// Print the cache location, entry count, and on-disk size for the
    /// current repository (covers per-commit deltas, revwalk, and
    /// head-tree caches).
    Info,
    /// Delete cache files for the current repository. By default
    /// removes all three caches; pass `--scope` to target one.
    Clear(CacheClearArgs),
}

#[derive(Debug, Parser)]
pub struct CacheClearArgs {
    /// Which cache(s) to clear.
    #[arg(long, value_enum, default_value_t = CacheScope::All)]
    pub scope: CacheScope,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum CacheScope {
    /// All caches: per-commit deltas, revwalk, head-tree.
    All,
    /// Per-commit `(added, deleted)` deltas.
    Deltas,
    /// Cached revwalk results, keyed by `(anchor_sha, since_ts)`.
    Revwalk,
    /// Cached HEAD/anchor tree enumeration, keyed by `(commit_sha, ignores_hash)`.
    Loc,
}
