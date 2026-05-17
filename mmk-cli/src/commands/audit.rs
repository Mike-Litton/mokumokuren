//! `mmk audit` — static codebase snapshot.
//!
//! Walks every health-eligible TS/TSX/JS/JSX file at HEAD and runs
//! the per-file sensors (STRUCTURE, COMPLEXITY) plus the non-delta
//! HEALTH patterns (`test_pair`). Skips the history-dependent layers
//! (HOTSPOT, COUPLING, DRIFT, BUDGET) and the delta-mode patterns
//! (EVASION, TEST_WEAKENING) — no diff to score against.
//!
//! Built for one-shot codebase audits — operators who want to see
//! structural divergence without first running an agent edit loop.
//! Distinct from `review`'s per-edit hot path.

use anyhow::{Context, Result};
use mmk_config::Config;
use std::io::Write;

use crate::args::{AuditArgs, Format};
use crate::commands::common::{
    analyze_health_for_subject, apply_health_file, apply_sensor_file, build_ignore_set,
    enabled_audit_health_patterns, enumerate_eligible_files, health_severity_for_review,
    health_to_finding, list_directory_siblings, load_bodies, load_config_file,
};
use crate::commands::per_file_sensors;
use crate::commands::review::verdict_for;
use crate::output::findings::{render_text, Finding};
use crate::output::messages::NO_SIGNAL_PREFIX;
use crate::Verdict;
use mmk_health::HealthFinding;

pub fn run<O: Write, E: Write>(
    args: &AuditArgs,
    stdout: &mut O,
    stderr: &mut E,
) -> Result<Verdict> {
    let cwd = std::env::current_dir().context("failed to determine current directory")?;
    let repo_root = mmk_git::discover_work_dir(&cwd)
        .ok_or_else(|| anyhow::anyhow!("not inside a git repository"))?;

    let (file_cfg, file_path) = load_config_file(&cwd, args.config.as_deref())?;

    let mut ignores = file_cfg.ignore.clone();
    ignores.extend(args.ignores.iter().cloned());
    let mut cfg = Config {
        ignores,
        ..Config::default()
    };

    if let Some(file_h) = file_cfg.health.as_ref() {
        apply_health_file(&mut cfg.health.ts, file_h);
    }
    if let Some(file_s) = file_cfg.sensor.as_ref() {
        apply_sensor_file(
            &mut cfg.sensor.structure,
            &mut cfg.sensor.complexity,
            &mut cfg.sensor.budget_ramp,
            &mut cfg.sensor.cohesion,
            file_s,
        );
    }

    if args.verbose {
        match &file_path {
            Some(p) => writeln!(stderr, "loaded config from {}", p.display())?,
            None => writeln!(stderr, "no mokumokuren.toml found; running with defaults")?,
        }
    }

    let ignores = build_ignore_set(&cfg.ignores)?;
    let files = enumerate_eligible_files(&repo_root, &ignores)?;

    let audit_health_patterns = enabled_audit_health_patterns(&cfg.health.ts.patterns);
    let log_identifiers = &cfg.health.ts.broad_exception.log_identifiers;

    let mut findings: Vec<Finding> = Vec::new();
    // Keep the structured `HealthFinding`s alongside the rendered
    // `Finding`s so the JSON envelope can expose `health.matches[]`
    // with the per-pattern `detail` payload intact. Text mode still
    // renders from `findings[]` only — the structured form is for
    // harnesses that want to filter without regex-parsing the
    // message.
    let mut health_matches: Vec<HealthFinding> = Vec::new();
    for path in &files {
        let siblings = list_directory_siblings(&repo_root, path);
        let mut all_paths = siblings.clone();
        if !all_paths.iter().any(|p| p == path) {
            all_paths.push(path.clone());
        }
        let bodies = load_bodies(&repo_root, &all_paths);
        let subject_body = bodies.get(path).map(String::as_str);
        let ctx = per_file_sensors::PerFileCtx {
            path,
            siblings: &siblings,
            bodies: &bodies,
            subject_body,
        };
        // STRUCTURE / COMPLEXITY in Review mode (the divergence vs
        // conformance distinction matches what the operator wants to
        // see in an audit: "is this file the odd one out in its
        // directory?"). HEAD body is `None` — no diff to weight
        // COMPLEXITY's delta filter against; a function over cap
        // surfaces regardless of whether the agent edited it.
        let per_file = per_file_sensors::compute_per_file_findings(
            &ctx,
            &cfg.sensor,
            per_file_sensors::PerFileMode::Review,
            None,
        );
        findings.extend(per_file.into_iter().map(|(f, _)| f));

        if cfg.health.ts.enabled && !audit_health_patterns.is_empty() {
            // Audit doesn't have an `analysis.loc.keys()` to seed peer
            // paths with; pass the per-file siblings only.
            // `analyze_health_for_subject` augments with mirrored
            // `test/` directories internally, which covers TestPair.
            for h in analyze_health_for_subject(
                &repo_root,
                path,
                None,
                &siblings,
                &audit_health_patterns,
                log_identifiers,
            ) {
                let severity = health_severity_for_review(h.pattern);
                findings.push(health_to_finding(&h, severity));
                health_matches.push(h);
            }
        }
    }

    match args.format {
        Format::Text => {
            if findings.is_empty() {
                writeln!(
                    stdout,
                    "{NO_SIGNAL_PREFIX}audited {} files; no findings",
                    files.len()
                )?;
            } else {
                render_text(stdout, &findings)?;
            }
        }
        Format::Json => {
            let health_block = (!health_matches.is_empty()).then(|| AuditHealthBlock {
                patterns_evaluated: audit_health_patterns.iter().map(|p| p.token()).collect(),
                matches: &health_matches,
            });
            let envelope = AuditEnvelope {
                schema_version: crate::output::schema::SCHEMA_VERSION,
                files_audited: u32::try_from(files.len()).unwrap_or(u32::MAX),
                findings: &findings,
                health: health_block,
            };
            serde_json::to_writer_pretty(&mut *stdout, &envelope)?;
            writeln!(stdout)?;
        }
    }
    Ok(verdict_for(args.gate, &findings))
}

#[derive(serde::Serialize)]
struct AuditEnvelope<'a> {
    schema_version: &'static str,
    files_audited: u32,
    findings: &'a [Finding],
    /// Structured `HealthFinding`s alongside the flattened
    /// `findings[]`. Each entry carries the full
    /// `mmk_health::HealthFinding` shape, including the per-pattern
    /// `detail` payload. Absent when no health pattern fired
    /// (mirrors `mmk review`'s `health` block shape).
    #[serde(skip_serializing_if = "Option::is_none")]
    health: Option<AuditHealthBlock<'a>>,
}

#[derive(serde::Serialize)]
struct AuditHealthBlock<'a> {
    patterns_evaluated: Vec<&'static str>,
    matches: &'a [HealthFinding],
}
