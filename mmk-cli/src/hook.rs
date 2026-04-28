//! Claude Code hook protocol — stdin envelope + structured output.
//!
//! Claude Code passes a single JSON object on the hook process's
//! stdin describing the tool call that triggered the hook. mmk reads
//! that envelope to learn (a) which file the agent is about to edit
//! (`tool_input.file_path`), and (b) which event fired
//! (`hook_event_name` — `PreToolUse`, `PostToolUse`, `Stop`). When
//! the envelope is absent (manual `mmk pre-edit foo.rs` from a
//! shell), mmk falls back to argv parsing and emits its normal text
//! / JSON output.
//!
//! The envelope-driven path also picks the *output* shape: rather
//! than the bare findings array, mmk wraps its output in the
//! hook-output JSON Claude Code expects (`hookSpecificOutput.
//! additionalContext` for non-blocking inject; `decision: "block"` +
//! `reason` for hard-block under `--gate warn`; top-level
//! `systemMessage` for "ran but had nothing new to say").
//!
//! Auto-detection (envelope present → hook output) keeps the wiring
//! snippet a single line of shell with no `--format hook-json` flag
//! to remember.

use anyhow::{Context, Result};
use serde::Deserialize;
use std::io::{IsTerminal, Read};
use std::path::PathBuf;

/// Minimal view of Claude Code's hook envelope.
///
/// Only the fields mmk actually consumes are deserialized; unknown
/// fields are ignored so future Claude Code versions don't break the
/// integration.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct HookEnvelope {
    /// `PreToolUse`, `PostToolUse`, `Stop`, etc. mmk auto-detects
    /// "I was invoked from a hook" by the presence of any non-empty
    /// value here.
    #[serde(default)]
    pub hook_event_name: String,
    #[serde(default)]
    pub tool_input: Option<HookToolInput>,
}

/// `tool_input` — Claude Code passes the matched tool's arguments
/// here. For `Edit`/`Write` the path lives in `file_path`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct HookToolInput {
    #[serde(default)]
    pub file_path: Option<PathBuf>,
}

impl HookEnvelope {
    /// Echo the file path the hook is acting on, if any. Pulled out
    /// so subcommands don't have to navigate the nested option chain.
    #[must_use]
    pub fn file_path(&self) -> Option<&std::path::Path> {
        self.tool_input.as_ref()?.file_path.as_deref()
    }
}

/// Read one JSON envelope from stdin.
///
/// Returns `Ok(None)` when stdin is a terminal (manual invocation —
/// argv mode) or when nothing was piped in (whitespace-only). Returns
/// `Err` when bytes were piped but parsing failed — surfacing a loud
/// failure beats silently downgrading to argv mode and producing
/// findings against the wrong file.
pub fn read_envelope_from_stdin() -> Result<Option<HookEnvelope>> {
    let stdin = std::io::stdin();
    if stdin.is_terminal() {
        return Ok(None);
    }
    let mut buf = String::new();
    stdin
        .lock()
        .read_to_string(&mut buf)
        .context("failed to read stdin")?;
    if buf.trim().is_empty() {
        return Ok(None);
    }
    let env: HookEnvelope = serde_json::from_str(&buf)
        .with_context(|| "failed to parse hook envelope JSON from stdin")?;
    Ok(Some(env))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_pre_tool_use_envelope_with_file_path() {
        let raw = r#"{
            "session_id": "x",
            "transcript_path": "/tmp/x",
            "cwd": "/r",
            "hook_event_name": "PreToolUse",
            "tool_input": { "file_path": "src/foo.rs" }
        }"#;
        let env: HookEnvelope = serde_json::from_str(raw).unwrap();
        assert_eq!(env.hook_event_name, "PreToolUse");
        assert_eq!(env.file_path(), Some(std::path::Path::new("src/foo.rs")));
    }

    #[test]
    fn parses_envelope_without_tool_input() {
        let raw = r#"{ "hook_event_name": "Stop" }"#;
        let env: HookEnvelope = serde_json::from_str(raw).unwrap();
        assert_eq!(env.hook_event_name, "Stop");
        assert!(env.file_path().is_none());
    }

    #[test]
    fn ignores_unknown_fields() {
        let raw = r#"{ "hook_event_name": "PreToolUse",
                       "matcher": "Edit",
                       "permission_decision": "allow" }"#;
        let env: HookEnvelope = serde_json::from_str(raw).unwrap();
        assert_eq!(env.hook_event_name, "PreToolUse");
    }
}
