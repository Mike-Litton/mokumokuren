//! `mmk sensors` — sensor-to-command discovery surface.
//!
//! Two modes:
//!   - `mmk sensors list` prints the sensor-to-command matrix so an
//!     operator without source access can see which findings each
//!     subcommand emits.
//!   - `mmk sensors describe <name>` prints the per-sensor reference
//!     (purpose, when it fires, severity, configuration knob).
//!
//! The single source of truth is [`SENSOR_CATALOG`]: the matrix
//! rendering, the JSON envelope, the per-command "Sensor coverage"
//! help lines (consumed by `args.rs`), and the `describe` lookup all
//! read from this table. Adding a new sensor → add one row here.

use anyhow::{anyhow, Result};
use serde::Serialize;
use std::io::Write;

use crate::args::{Format, SensorsAction, SensorsArgs};

/// One row in the sensor-to-command matrix.
///
/// `commands` lists every subcommand that can emit this finding.
/// `mode` distinguishes delta-vs-history detectors (HOTSPOT, COUPLING,
/// EVASION, …) from static / per-file detectors (STRUCTURE,
/// COMPLEXITY, BroadCatchDebt, …) — agents that filter by mode
/// (`jq '.sensors[] | select(.mode == "delta")'`) get a clean cut.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct SensorEntry {
    pub name: &'static str,
    pub layer: &'static str,
    /// Optional pattern token under HEALTH (`broad_exception`,
    /// `broad_catch_debt`, etc.). `None` for non-HEALTH sensors.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern: Option<&'static str>,
    pub mode: &'static str,
    pub default_severity: &'static str,
    pub description: &'static str,
    pub config_key: &'static str,
    pub config_subkeys: &'static [&'static str],
    pub commands: &'static [&'static str],
    pub since: &'static str,
}

/// Catalog of every sensor mmk emits, plus which commands surface
/// each one. Read by:
///   - `mmk sensors list` (text matrix + JSON envelope)
///   - `mmk sensors describe <name>` (per-sensor lookup)
///   - `args.rs` per-command help text (the "Sensor coverage" line
///     on every subcommand's clap doc-comment is sourced from here)
///
/// Order matches the layer-rendering order in
/// `output::findings::LAYER_ORDER`, then HEALTH patterns, then
/// audit-only HEALTH patterns.
pub const SENSOR_CATALOG: &[SensorEntry] = &[
    SensorEntry {
        name: "HOTSPOT",
        layer: "Hotspot",
        pattern: None,
        mode: "history",
        default_severity: "Warn",
        description: "Changed file ranks within the top-N by weighted churn × LOC.",
        config_key: "[hotspot] top_n",
        config_subkeys: &[],
        commands: &["analyze", "review", "pre-edit", "drift", "session-summary", "eval"],
        since: "0.1.0",
    },
    SensorEntry {
        name: "COUPLING",
        layer: "Coupling",
        pattern: None,
        mode: "history",
        default_severity: "Warn",
        description: "Changed file's expected co-edit partner is missing from the diff (Wilson lower-bound on conditional co-change probability).",
        config_key: "[coupling]",
        config_subkeys: &["confidence_threshold", "min_sample_size", "ignore_partners"],
        commands: &["analyze", "review", "pre-edit", "session-summary", "eval"],
        since: "0.1.0",
    },
    SensorEntry {
        name: "COHESION",
        layer: "Cohesion",
        pattern: None,
        mode: "history",
        default_severity: "Warn",
        description: "Working-tree diff partitions into multiple disjoint clusters on the historical co-change graph (tangled-diff fingerprint).",
        config_key: "[sensor.cohesion]",
        config_subkeys: &[
            "confidence_threshold",
            "min_sample_size",
            "min_files_per_cluster",
        ],
        commands: &["review"],
        since: "0.6.0",
    },
    SensorEntry {
        name: "DRIFT",
        layer: "Drift",
        pattern: None,
        mode: "history",
        default_severity: "Info",
        description: "File climbed in rank across a majority of the K most recent session boundaries.",
        config_key: "(driven by `--drift-sessions` / `--top`)",
        config_subkeys: &[],
        commands: &["drift", "session-summary"],
        since: "0.3.0",
    },
    SensorEntry {
        name: "BUDGET",
        layer: "Budget",
        pattern: None,
        mode: "static",
        default_severity: "Warn",
        description: "Working-tree diff exceeds the per-diff cap (file count or LOC), or crosses the 200-LOC review-effectiveness floor.",
        config_key: "[bulk]",
        config_subkeys: &["max_files", "max_lines", "review_quality_lines", "ignore_for_budget"],
        commands: &["review"],
        since: "0.1.0",
    },
    SensorEntry {
        name: "STRUCTURE",
        layer: "Structure",
        pattern: None,
        mode: "static",
        default_severity: "Warn",
        description: "File diverges from a directory-shape convention (≥3 sibling files share an import / export template the subject is missing).",
        config_key: "[sensor.structure]",
        config_subkeys: &[
            "min_siblings",
            "import_majority",
            "export_template_majority",
            "role_patterns",
        ],
        commands: &["review", "pre-edit", "audit"],
        since: "0.5.0",
    },
    SensorEntry {
        name: "COMPLEXITY",
        layer: "Complexity",
        pattern: None,
        mode: "static",
        default_severity: "Warn",
        description: "Per-function nesting depth or LOC exceeds the directory ratio or absolute cap.",
        config_key: "[sensor.complexity]",
        config_subkeys: &[
            "nesting_ratio_threshold",
            "nesting_absolute_max",
            "loc_ratio_threshold",
            "loc_absolute_max",
            "delta_warn_pct",
            "delta_warn_abs",
        ],
        commands: &["review", "audit"],
        since: "0.5.0",
    },
    SensorEntry {
        name: "HEALTH:test_pair",
        layer: "Health",
        pattern: Some("test_pair"),
        mode: "static",
        default_severity: "Warn",
        description: "File has a `<name>.test.ts` / `<name>.spec.ts` partner not present in the diff.",
        config_key: "[health.ts] patterns",
        config_subkeys: &[],
        commands: &["review", "audit"],
        since: "0.4.0",
    },
    SensorEntry {
        name: "HEALTH:registration",
        layer: "Health",
        pattern: Some("registration"),
        mode: "static",
        default_severity: "Info",
        description: "File matches the action / contribution registration shape; surfaces sibling registration files as architectural precedent.",
        config_key: "[health.ts] patterns",
        config_subkeys: &[],
        commands: &["review", "audit"],
        since: "0.4.0",
    },
    SensorEntry {
        name: "HEALTH:service",
        layer: "Health",
        pattern: Some("service"),
        mode: "static",
        default_severity: "Info",
        description: "File declares an `interface IFoo` plus `registerSingleton(IFoo, FooImpl)`; surfaces top consumers importing the interface.",
        config_key: "[health.ts] patterns",
        config_subkeys: &[],
        commands: &["review", "audit"],
        since: "0.4.0",
    },
    SensorEntry {
        name: "HEALTH:broad_exception",
        layer: "Health",
        pattern: Some("broad_exception"),
        mode: "delta",
        default_severity: "Warn",
        description: "Newly added non-top-level broad TS/JS catch handler (empty body, no parameter, typed any/unknown/Error, or log-and-swallow shape) not present at HEAD.",
        config_key: "[health.ts] patterns",
        config_subkeys: &["[health.ts.broad_exception] log_identifiers"],
        commands: &["review"],
        since: "0.7.0",
    },
    SensorEntry {
        name: "HEALTH:broad_catch_debt",
        layer: "Health",
        pattern: Some("broad_catch_debt"),
        mode: "static",
        default_severity: "Info",
        description: "Static count of non-top-level broad TS/JS catch handlers in the working tree, no HEAD comparison.",
        config_key: "[health.ts] patterns",
        config_subkeys: &["[health.ts.broad_exception] log_identifiers"],
        commands: &["audit"],
        since: "0.12.0",
    },
];

/// Every subcommand the matrix considers, in display order. Column
/// ordering must match the matrix-rendering loop below.
pub const COMMANDS_IN_MATRIX: &[&str] = &[
    "analyze",
    "review",
    "pre-edit",
    "drift",
    "audit",
    "session-summary",
    "eval",
];

/// Comma-separated coverage list for a given subcommand, drawn
/// from `SENSOR_CATALOG`.
///
/// The "Sensor coverage:" lines in `args.rs` clap doc-comments are
/// the operator-facing surface but are inlined string literals
/// (clap doc-comments are compile-time only — they can't call
/// functions). This helper is kept so downstream tooling can
/// programmatically derive the same coverage matrix without
/// scraping `--help` output, and so a follow-up parity test can
/// assert the inline strings match what the catalog says.
#[must_use]
pub fn coverage_for_command(cmd: &str) -> String {
    let mut names: Vec<&'static str> = SENSOR_CATALOG
        .iter()
        .filter(|e| e.commands.contains(&cmd))
        .map(|e| e.name)
        .collect();
    // Stable order (catalog order is already deliberate).
    names.dedup();
    names.join(", ")
}

pub fn run<O: Write, E: Write>(args: &SensorsArgs, out: &mut O, _err: &mut E) -> Result<()> {
    match &args.action {
        SensorsAction::List(list_args) => render_list(list_args.format, out),
        SensorsAction::Describe(describe_args) => {
            render_describe(&describe_args.name, describe_args.format, out)
        }
    }
}

fn render_list<O: Write>(format: Format, out: &mut O) -> Result<()> {
    match format {
        Format::Text => render_list_text(out),
        Format::Json => render_list_json(out),
    }
}

fn render_list_text<O: Write>(out: &mut O) -> Result<()> {
    // Compute column widths once for alignment.
    let name_width = SENSOR_CATALOG
        .iter()
        .map(|e| e.name.len())
        .max()
        .unwrap_or(8)
        .max("sensor".len());
    write!(out, "{:<name_width$}", "sensor")?;
    for cmd in COMMANDS_IN_MATRIX {
        write!(out, "  {cmd:<8}")?;
    }
    writeln!(out)?;
    for entry in SENSOR_CATALOG {
        write!(out, "{:<name_width$}", entry.name)?;
        for cmd in COMMANDS_IN_MATRIX {
            let cell = if entry.commands.contains(cmd) {
                "✓"
            } else {
                ""
            };
            write!(out, "  {cell:<8}")?;
        }
        writeln!(out)?;
    }
    Ok(())
}

#[derive(Debug, Serialize)]
struct ListEnvelope<'a> {
    schema_version: &'static str,
    sensors: &'a [SensorEntry],
}

fn render_list_json<O: Write>(out: &mut O) -> Result<()> {
    let env = ListEnvelope {
        schema_version: crate::output::schema::SCHEMA_VERSION,
        sensors: SENSOR_CATALOG,
    };
    serde_json::to_writer_pretty(&mut *out, &env)?;
    writeln!(out)?;
    Ok(())
}

fn lookup(name: &str) -> Option<&'static SensorEntry> {
    SENSOR_CATALOG
        .iter()
        .find(|e| e.name.eq_ignore_ascii_case(name) || matches_pattern_token(e, name))
}

fn matches_pattern_token(entry: &SensorEntry, name: &str) -> bool {
    entry.pattern.is_some_and(|p| p.eq_ignore_ascii_case(name))
}

fn render_describe<O: Write>(name: &str, format: Format, out: &mut O) -> Result<()> {
    let entry = lookup(name).ok_or_else(|| {
        anyhow!("no sensor named `{name}`. Run `mmk sensors list` to see available sensors.")
    })?;
    match format {
        Format::Text => render_describe_text(entry, out),
        Format::Json => render_describe_json(entry, out),
    }
}

fn render_describe_text<O: Write>(entry: &SensorEntry, out: &mut O) -> Result<()> {
    writeln!(out, "{}", entry.name)?;
    writeln!(out, "  layer:            {}", entry.layer)?;
    if let Some(p) = entry.pattern {
        writeln!(out, "  pattern:          {p}")?;
    }
    writeln!(out, "  mode:             {}", entry.mode)?;
    writeln!(out, "  default severity: {}", entry.default_severity)?;
    writeln!(out, "  commands:         {}", entry.commands.join(", "))?;
    writeln!(out, "  config:           {}", entry.config_key)?;
    if !entry.config_subkeys.is_empty() {
        writeln!(
            out,
            "  config knobs:     {}",
            entry.config_subkeys.join(", ")
        )?;
    }
    writeln!(out, "  since:            {}", entry.since)?;
    writeln!(out)?;
    writeln!(out, "{}", entry.description)?;
    if let Some(extra) = long_description(entry) {
        writeln!(out)?;
        writeln!(out, "{extra}")?;
    }
    Ok(())
}

#[derive(Debug, Serialize)]
struct DescribeEnvelope<'a> {
    schema_version: &'static str,
    #[serde(flatten)]
    entry: &'a SensorEntry,
    #[serde(skip_serializing_if = "Option::is_none")]
    long_description: Option<&'static str>,
}

fn render_describe_json<O: Write>(entry: &'static SensorEntry, out: &mut O) -> Result<()> {
    let env = DescribeEnvelope {
        schema_version: crate::output::schema::SCHEMA_VERSION,
        entry,
        long_description: long_description(entry),
    };
    serde_json::to_writer_pretty(&mut *out, &env)?;
    writeln!(out)?;
    Ok(())
}

/// Optional reference text shown alongside the catalog row. Kept
/// terse — operators read `--help` for the one-liner and `describe`
/// for the multi-paragraph background.
fn long_description(entry: &SensorEntry) -> Option<&'static str> {
    match entry.name {
        "HEALTH:broad_exception" => Some(
            "EVASION targets the \"evasive repairs with try-except blocks\" failure mode named in arXiv:2509.13941. \
             The detector compares working tree against HEAD; only the *addition* of a broad non-top-level catch handler fires. \
             The set of \"broad\" shapes covers empty body, missing parameter, parameter typed `any`/`unknown`/`Error`, and \
             (v0.12) the log-and-swallow shape — body composed exclusively of member-call expressions on a configured log \
             identifier (default `logger`, `log`, `console`; extend via `[health.ts.broad_exception] log_identifiers`).",
        ),
        "HEALTH:broad_catch_debt" => Some(
            "Static-mode counterpart to EVASION. Reports the count of broad non-top-level catch handlers in the working \
             tree at HEAD without comparing to a previous body. Use this on first contact with a codebase that accumulated \
             evasion debt before mmk was enabled. Reuses the same `is_broad` predicate as EVASION, so the log-and-swallow \
             shape and the `log_identifiers` config knob apply uniformly.",
        ),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_entries_have_unique_names() {
        let mut seen = std::collections::HashSet::new();
        for entry in SENSOR_CATALOG {
            assert!(
                seen.insert(entry.name),
                "duplicate sensor name in catalog: {}",
                entry.name
            );
        }
    }

    #[test]
    fn coverage_for_review_mentions_health() {
        let cov = coverage_for_command("review");
        assert!(cov.contains("HEALTH:broad_exception"), "got: {cov}");
        assert!(!cov.contains("HEALTH:broad_catch_debt"), "got: {cov}");
    }

    #[test]
    fn coverage_for_audit_includes_broad_catch_debt() {
        let cov = coverage_for_command("audit");
        assert!(cov.contains("HEALTH:broad_catch_debt"), "got: {cov}");
        assert!(cov.contains("STRUCTURE"), "got: {cov}");
        assert!(!cov.contains("HOTSPOT"), "got: {cov}");
    }

    #[test]
    fn lookup_supports_full_name_and_pattern_token() {
        assert!(lookup("HEALTH:broad_catch_debt").is_some());
        assert!(lookup("broad_catch_debt").is_some());
        assert!(lookup("nonexistent_sensor").is_none());
    }
}
