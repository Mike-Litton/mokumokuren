//! Verifies that every `mmk ...` invocation in a fenced ```shell
//! block in the project README actually runs against the canonical
//! fixture. Catches drift the moment a README example refers to a
//! flag, subcommand, or path the tool no longer exposes.
//!
//! Orthogonality tag: protects **users** of both modes — agent
//! harness authors copy-pasting from the README and humans doing a
//! first-time read.
//!
//! Mechanism: the test parses the README, extracts each fenced
//! `shell` block, and for every line strips comments / pipes,
//! tokenizes on whitespace, finds the first `mmk` (or
//! `mokumokuren`) token, and parses the remaining tokens through
//! clap. The parsed command is then dispatched through the lib API
//! against `build_canonical_fixture`. `mmk init` writes to CWD and
//! has its own dedicated test, so it's skipped here.

mod common;

use clap::Parser;
use common::{build_canonical_fixture, CWD_LOCK};
use mokumokuren::args::{Cli, Command};
use std::fs;
use tempfile::TempDir;

const SKIP_TOKENS: &[&str] = &["init", "cache", "eval"];

fn extract_shell_blocks(readme: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_block = false;
    let mut current = String::new();
    for line in readme.lines() {
        if line.trim_start().starts_with("```shell") {
            in_block = true;
            current.clear();
            continue;
        }
        if in_block && line.trim_start().starts_with("```") {
            in_block = false;
            out.push(std::mem::take(&mut current));
            continue;
        }
        if in_block {
            current.push_str(line);
            current.push('\n');
        }
    }
    out
}

fn extract_mmk_commands(block: &str) -> Vec<Vec<String>> {
    let mut out = Vec::new();
    for raw in block.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // Truncate at first pipe — we only test the mmk side.
        let head = line.split('|').next().unwrap().trim();
        let tokens: Vec<&str> = head.split_whitespace().collect();
        // Find the `mmk` / `mokumokuren` invocation start.
        let Some(start) = tokens
            .iter()
            .position(|t| *t == "mmk" || *t == "mokumokuren")
        else {
            continue;
        };
        let cmd: Vec<String> = tokens[start..].iter().map(|s| (*s).to_string()).collect();
        // The first arg after `mmk` is the subcommand. Skip if in
        // SKIP_TOKENS (e.g. `init`).
        if let Some(sub) = cmd.get(1) {
            if SKIP_TOKENS.iter().any(|skip| sub == skip) {
                continue;
            }
        }
        out.push(cmd);
    }
    out
}

fn run_cmd_against_fixture(cmd: &[String]) -> Result<(), String> {
    // Strip surrounding quotes from glob arguments like 'docs/**'
    // — this is the only shell-quoting we'll see in the README.
    let normalized: Vec<String> = cmd
        .iter()
        .map(|t| t.trim_matches('\'').trim_matches('"').to_string())
        .collect();

    // Parse via clap so the test catches any flag/subcommand drift.
    let cli = Cli::try_parse_from(&normalized)
        .map_err(|e| format!("clap rejected `{}`: {e}", normalized.join(" ")))?;

    let _g = CWD_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = TempDir::new().unwrap();
    let now: i64 = 1_700_000_000;
    build_canonical_fixture(dir.path(), now);

    let orig = std::env::current_dir().unwrap();
    std::env::set_current_dir(dir.path()).unwrap();

    let mut stdout: Vec<u8> = Vec::new();
    let mut stderr: Vec<u8> = Vec::new();
    let result: anyhow::Result<()> = match cli.command {
        Command::Analyze(a) => mokumokuren::commands::analyze::run(&a, &mut stdout, &mut stderr),
        Command::SessionSummary(a) => {
            mokumokuren::commands::session::run(&a, &mut stdout, &mut stderr).map(|_| ())
        }
        Command::Review(a) => {
            mokumokuren::commands::review::run(&a, &mut stdout, &mut stderr).map(|_| ())
        }
        Command::PreEdit(a) => {
            mokumokuren::commands::pre_edit::run(&a, &mut stdout, &mut stderr).map(|_| ())
        }
        Command::Drift(a) => mokumokuren::commands::drift::run(&a, &mut stdout, &mut stderr),
        Command::Init(_) | Command::Eval(_) | Command::Cache(_) => {
            unreachable!("filtered by SKIP_TOKENS")
        }
    };

    std::env::set_current_dir(orig).unwrap();
    result.map_err(|e| format!("running `{}` failed: {e:#}", normalized.join(" ")))
}

#[test]
fn readme_shell_blocks_extract_at_least_one_mmk_command() {
    // Sanity: if the README ever loses all its examples, this test
    // fails so we notice. Extraction-side regression guard.
    let readme = fs::read_to_string("../README.md").expect("read README.md");
    let blocks = extract_shell_blocks(&readme);
    assert!(
        blocks
            .iter()
            .flat_map(|b| extract_mmk_commands(b))
            .next()
            .is_some(),
        "expected at least one `mmk` command in a fenced ```shell block"
    );
}

// Run via clap (parse-only, no dispatch). Catches flag drift even
// when the canonical fixture wouldn't satisfy a particular path.
#[test]
fn readme_mmk_commands_parse_via_clap() {
    let readme = fs::read_to_string("../README.md").expect("read README.md");
    let blocks = extract_shell_blocks(&readme);
    let mut failures = Vec::new();
    for block in &blocks {
        for cmd in extract_mmk_commands(block) {
            let normalized: Vec<String> = cmd
                .iter()
                .map(|t| t.trim_matches('\'').trim_matches('"').to_string())
                .collect();
            if let Err(e) = Cli::try_parse_from(&normalized) {
                failures.push(format!("`{}`: {e}", normalized.join(" ")));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "README contains commands that no longer parse:\n{}",
        failures.join("\n")
    );
}

// End-to-end: every README `mmk` command runs cleanly against the
// canonical fixture. Strongest contract; catches both flag drift and
// runtime regressions.
#[test]
fn readme_mmk_commands_run_clean_on_canonical_fixture() {
    let readme = fs::read_to_string("../README.md").expect("read README.md");
    let blocks = extract_shell_blocks(&readme);
    let mut failures = Vec::new();
    for block in &blocks {
        for cmd in extract_mmk_commands(block) {
            if let Err(e) = run_cmd_against_fixture(&cmd) {
                failures.push(e);
            }
        }
    }
    assert!(
        failures.is_empty(),
        "README contains commands that fail at runtime:\n{}",
        failures.join("\n")
    );
}
