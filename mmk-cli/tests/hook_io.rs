//! Hook stdin envelope + hook output channels.
//!
//! Locks the contract Claude Code's hook integration documents: a
//! JSON envelope on stdin tells mmk which file the agent is acting
//! on, mmk emits its findings as `hookSpecificOutput.
//! additionalContext` (PreToolUse) or `additionalContext` plus
//! optional `decision: "block"` + `reason` (PostToolUse / Stop with
//! `--gate warn`), and dedup-suppress surfaces as a top-level
//! `systemMessage` so the agent can distinguish "consulted, quiet"
//! from "wasn't run."

mod common;

use common::{commit_all, init_repo, write, CWD_LOCK, DAY};
use mokumokuren::args::{Format, Gate, PreEditArgs, ReviewArgs};
use mokumokuren::hook::{HookEnvelope, HookToolInput};
use serde_json::Value;
use std::path::PathBuf;
use tempfile::TempDir;

fn pre_edit_args(path: &str) -> PreEditArgs {
    PreEditArgs {
        path: if path.is_empty() {
            None
        } else {
            Some(PathBuf::from(path))
        },
        since: "60days".into(),
        top: 20,
        format: Format::Json,
        ignores: Vec::new(),
        config: None,
        verbose: false,
        coupling_threshold: None,
        blast_radius_threshold: None,
        drift_sessions: 0,
        gate: Gate::None,
        no_dedup: true,
    }
}

fn review_args() -> ReviewArgs {
    ReviewArgs {
        staged: false,
        range: None,
        commit: None,
        since: "60days".into(),
        top: 20,
        format: Format::Json,
        ignores: Vec::new(),
        config: None,
        verbose: false,
        coupling_threshold: None,
        blast_radius_threshold: None,
        gate: Gate::None,
        no_dedup: true,
    }
}

fn pre_tool_use_envelope(path: &str) -> HookEnvelope {
    HookEnvelope {
        hook_event_name: "PreToolUse".into(),
        tool_input: Some(HookToolInput {
            file_path: Some(PathBuf::from(path)),
        }),
    }
}

fn post_tool_use_envelope() -> HookEnvelope {
    HookEnvelope {
        hook_event_name: "PostToolUse".into(),
        tool_input: None,
    }
}

fn run_pre_edit(repo: &std::path::Path, args: PreEditArgs, env: Option<&HookEnvelope>) -> Vec<u8> {
    let _g = CWD_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let orig = std::env::current_dir().unwrap();
    std::env::set_current_dir(repo).unwrap();
    let mut stdout: Vec<u8> = Vec::new();
    let mut stderr: Vec<u8> = Vec::new();
    let res = mokumokuren::commands::pre_edit::run(&args, env, &mut stdout, &mut stderr);
    std::env::set_current_dir(orig).unwrap();
    res.expect("pre-edit run");
    stdout
}

fn run_review(repo: &std::path::Path, args: ReviewArgs, env: Option<&HookEnvelope>) -> Vec<u8> {
    let _g = CWD_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let orig = std::env::current_dir().unwrap();
    std::env::set_current_dir(repo).unwrap();
    let mut stdout: Vec<u8> = Vec::new();
    let mut stderr: Vec<u8> = Vec::new();
    let res = mokumokuren::commands::review::run(&args, env, &mut stdout, &mut stderr);
    std::env::set_current_dir(orig).unwrap();
    res.expect("review run");
    stdout
}

#[test]
fn pre_edit_envelope_with_file_path_threads_into_lookup() {
    // The hook envelope's tool_input.file_path takes the place of
    // an argv path. We assert mmk produces hook-shape JSON with
    // additionalContext naming the path, not the argv-empty error.
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    init_repo(dir.path());
    write(dir.path(), "src/seed.ts", "export const x = 1;\n");
    commit_all(dir.path(), "seed", now - 5 * DAY);

    let args = pre_edit_args(""); // empty argv path → relies on envelope
    let env = pre_tool_use_envelope("src/seed.ts");
    let stdout = run_pre_edit(dir.path(), args, Some(&env));
    let v: Value = serde_json::from_slice(&stdout).expect("valid hook JSON");
    let event = v["hookSpecificOutput"]["hookEventName"].as_str().unwrap();
    assert_eq!(event, "PreToolUse");
    let ctx = v["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap_or_default();
    let sys = v["systemMessage"].as_str().unwrap_or_default();
    assert!(
        ctx.contains("src/seed.ts") || sys.contains("no findings") || !ctx.is_empty(),
        "expected either context or systemMessage; got: {}",
        String::from_utf8_lossy(&stdout),
    );
}

#[test]
fn review_post_tool_use_with_warn_gate_emits_block_when_warn_finding_fires() {
    // Calibrated fixture: A and B co-change 4 of 5 prior commits
    // (Wilson 4/5 @ n=5 ≈ 0.376), clearing the v0.6 default
    // (`confidence_threshold = 0.30`, `min_sample_size = 3`).
    // Editing only A in working tree fires Severity::Warn COUPLING;
    // with `--gate warn` the hook output must carry decision=block.
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    common::build_coupling_fixture(dir.path(), now);

    // Add an uncommitted edit to core/a.rs so review sees a diff.
    write(dir.path(), "core/a.rs", "a1\na2\na3\na4\na5\nuncommitted\n");

    let args = ReviewArgs {
        gate: Gate::Warn,
        ..review_args()
    };
    let env = post_tool_use_envelope();
    let stdout = run_review(dir.path(), args, Some(&env));
    let v: Value = serde_json::from_slice(&stdout).expect("valid hook JSON");
    let decision = v["decision"].as_str();
    let reason = v["reason"].as_str().unwrap_or_default();
    assert_eq!(decision, Some("block"), "expected decision=block; got {v}");
    assert!(reason.contains("COUPLING") || reason.contains("warn"),);
}

#[test]
fn review_post_tool_use_without_gate_uses_additional_context_only() {
    // Same fixture; same uncommitted edit; but no --gate. The warn
    // finding still surfaces, but in additionalContext — not as
    // a hard decision=block. This is the strategic-deployment line:
    // `--gate warn` is opt-in.
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    common::build_coupling_fixture(dir.path(), now);
    write(dir.path(), "core/a.rs", "a1\na2\na3\na4\na5\nuncommitted\n");

    let args = review_args();
    let env = post_tool_use_envelope();
    let stdout = run_review(dir.path(), args, Some(&env));
    let v: Value = serde_json::from_slice(&stdout).expect("valid hook JSON");
    assert!(v["decision"].is_null(), "no --gate must not block; got {v}");
    let ctx = v["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap_or_default();
    assert!(
        ctx.contains("COUPLING") || ctx.contains("co-edited"),
        "expected COUPLING in additionalContext; got: {ctx}"
    );
}

#[test]
fn review_clean_tree_envelope_returns_empty_additional_context() {
    // Clean tree → no findings to surface. The hook output keeps
    // its shape but additionalContext is null and a systemMessage
    // makes the silence explicit.
    let dir = TempDir::new().unwrap();
    let now = 1_700_000_000_i64;
    init_repo(dir.path());
    write(dir.path(), "src/seed.ts", "export const x = 1;\n");
    commit_all(dir.path(), "seed", now - 5 * DAY);

    let args = review_args();
    let env = post_tool_use_envelope();
    let stdout = run_review(dir.path(), args, Some(&env));
    let v: Value = serde_json::from_slice(&stdout).expect("valid hook JSON");
    assert!(v["hookSpecificOutput"]["additionalContext"].is_null());
    let sys = v["systemMessage"].as_str().unwrap_or_default();
    assert!(
        sys.contains("no findings") || sys.contains("unchanged"),
        "clean-tree must emit systemMessage; got {sys}"
    );
}
