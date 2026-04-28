//! Claude Code hook-output writers.
//!
//! Three output shapes:
//! - `PreToolUse` → `hookSpecificOutput.additionalContext` carries
//!   the body. Non-blocking; the agent reads but isn't forced to
//!   yield. Blocking in the *Edit* phase is reserved for `--gate warn`
//!   on `PostToolUse`.
//! - `PostToolUse` / `Stop` → same `additionalContext` shape on
//!   non-Warn fires. With `--gate warn` AND a Warn-severity finding,
//!   add `decision: "block"` + `reason` so the agent must yield.
//! - Dedup-suppress (any phase) → top-level `systemMessage` with
//!   "mmk ran but had nothing new to say"; replaces the silent
//!   no-op so the agent can distinguish "consulted, quiet" from
//!   "wasn't run."
//!
//! The body is rendered through the existing `findings::render_text`
//! writer so hook output and CLI text output share one source of
//! truth — wording fixes don't have to be replicated in two places.

use anyhow::Result;
use serde::Serialize;
use std::io::Write;

use crate::output::findings::{render_text, Finding, Severity};

/// `--gate warn` + Warn finding present → block message Claude Code
/// will surface as a yield to the agent.
const BLOCK_REASON_PREFIX: &str = "mmk: warn-severity finding(s):";

/// Suppression notice surfaced via `systemMessage`.
fn dedup_message(head_sha: Option<&str>) -> String {
    head_sha.map_or_else(
        || "mmk: prior findings unchanged".to_owned(),
        |sha| {
            let short = if sha.len() >= 7 { &sha[..7] } else { sha };
            format!("mmk: prior findings unchanged since HEAD {short}")
        },
    )
}

#[derive(Serialize)]
struct PreToolUseOutput {
    #[serde(rename = "hookSpecificOutput")]
    hook_specific_output: HookSpecificOutput,
    #[serde(rename = "systemMessage", skip_serializing_if = "Option::is_none")]
    system_message: Option<String>,
}

#[derive(Serialize)]
struct HookSpecificOutput {
    #[serde(rename = "hookEventName")]
    hook_event_name: &'static str,
    #[serde(rename = "additionalContext", skip_serializing_if = "Option::is_none")]
    additional_context: Option<String>,
}

#[derive(Serialize)]
struct PostToolUseOutput {
    #[serde(rename = "hookSpecificOutput")]
    hook_specific_output: HookSpecificOutput,
    #[serde(skip_serializing_if = "Option::is_none")]
    decision: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
    #[serde(rename = "systemMessage", skip_serializing_if = "Option::is_none")]
    system_message: Option<String>,
}

/// Render the findings body to a string via the same writer the CLI
/// text path uses. Empty findings → empty string.
fn body_for_findings(findings: &[Finding]) -> String {
    let mut buf: Vec<u8> = Vec::new();
    let _ = render_text(&mut buf, findings);
    String::from_utf8(buf).unwrap_or_default()
}

/// Hook-shape output for `PreToolUse`. Always non-blocking: the
/// `additionalContext` channel injects the finding body into the
/// agent's next turn without forcing a yield.
pub fn write_pre_tool_use<W: Write>(
    w: &mut W,
    findings: &[Finding],
    suppressed: bool,
    head_sha: Option<&str>,
) -> Result<()> {
    let body = body_for_findings(findings);
    let (additional_context, system_message) = if suppressed {
        (None, Some(dedup_message(head_sha)))
    } else if body.is_empty() {
        (None, Some("mmk: no findings".to_owned()))
    } else {
        (Some(body), None)
    };
    let out = PreToolUseOutput {
        hook_specific_output: HookSpecificOutput {
            hook_event_name: "PreToolUse",
            additional_context,
        },
        system_message,
    };
    serde_json::to_writer_pretty(&mut *w, &out)?;
    writeln!(w)?;
    Ok(())
}

/// Hook-shape output for `PostToolUse` / `Stop`.
///
/// Adds `decision: "block"` + `reason` when the caller asked for
/// `--gate warn` and at least one Warn-severity finding fired —
/// that's the strategic choice Claude Code's hook contract
/// surfaces as a hard yield.
pub fn write_post_tool_use<W: Write>(
    w: &mut W,
    event_name: &str,
    findings: &[Finding],
    suppressed: bool,
    head_sha: Option<&str>,
    block_on_warn: bool,
) -> Result<()> {
    let body = body_for_findings(findings);
    let warn_count = findings
        .iter()
        .filter(|f| f.severity == Severity::Warn)
        .count();
    let (additional_context, system_message, decision, reason) = if suppressed {
        (None, Some(dedup_message(head_sha)), None, None)
    } else if body.is_empty() {
        (None, Some("mmk: no findings".to_owned()), None, None)
    } else if block_on_warn && warn_count > 0 {
        let reason_str = format!("{BLOCK_REASON_PREFIX}\n{body}");
        (None, None, Some("block"), Some(reason_str))
    } else {
        (Some(body), None, None, None)
    };
    let static_event_name = match event_name {
        "Stop" => "Stop",
        _ => "PostToolUse",
    };
    let out = PostToolUseOutput {
        hook_specific_output: HookSpecificOutput {
            hook_event_name: static_event_name,
            additional_context,
        },
        decision,
        reason,
        system_message,
    };
    serde_json::to_writer_pretty(&mut *w, &out)?;
    writeln!(w)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::findings::Layer;
    use serde_json::Value;

    fn warn_finding() -> Finding {
        Finding::new(Layer::Coupling, Severity::Warn, "noisy")
    }
    fn info_finding() -> Finding {
        Finding::new(Layer::Coupling, Severity::Info, "quiet")
    }

    #[test]
    fn pre_tool_use_emits_additional_context_for_findings() {
        let mut buf = Vec::new();
        write_pre_tool_use(&mut buf, &[info_finding()], false, Some("abcdef0")).unwrap();
        let v: Value = serde_json::from_slice(&buf).unwrap();
        let ctx = v["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap();
        assert!(ctx.contains("quiet"), "body should be threaded; got {ctx}");
    }

    #[test]
    fn pre_tool_use_emits_system_message_on_dedup_suppress() {
        let mut buf = Vec::new();
        write_pre_tool_use(&mut buf, &[], true, Some("abcdef0123456")).unwrap();
        let v: Value = serde_json::from_slice(&buf).unwrap();
        assert!(v["hookSpecificOutput"]["additionalContext"].is_null());
        let msg = v["systemMessage"].as_str().unwrap();
        assert!(msg.contains("abcdef0"), "expected short-sha; got {msg}");
    }

    #[test]
    fn post_tool_use_blocks_on_warn_with_gate() {
        let mut buf = Vec::new();
        write_post_tool_use(
            &mut buf,
            "PostToolUse",
            &[warn_finding()],
            false,
            Some("abc1234"),
            true,
        )
        .unwrap();
        let v: Value = serde_json::from_slice(&buf).unwrap();
        assert_eq!(v["decision"].as_str(), Some("block"));
        assert!(v["reason"].as_str().unwrap().contains("noisy"));
    }

    #[test]
    fn post_tool_use_does_not_block_without_gate_even_with_warn() {
        let mut buf = Vec::new();
        write_post_tool_use(
            &mut buf,
            "PostToolUse",
            &[warn_finding()],
            false,
            Some("abc1234"),
            false,
        )
        .unwrap();
        let v: Value = serde_json::from_slice(&buf).unwrap();
        assert!(v["decision"].is_null());
        let ctx = v["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap();
        assert!(ctx.contains("noisy"));
    }

    #[test]
    fn post_tool_use_info_only_with_gate_does_not_block() {
        let mut buf = Vec::new();
        write_post_tool_use(
            &mut buf,
            "PostToolUse",
            &[info_finding()],
            false,
            Some("abc1234"),
            true,
        )
        .unwrap();
        let v: Value = serde_json::from_slice(&buf).unwrap();
        assert!(v["decision"].is_null());
        let ctx = v["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap();
        assert!(ctx.contains("quiet"));
    }
}
