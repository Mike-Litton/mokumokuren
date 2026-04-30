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
use crate::output::messages::EmptyDiffSummary;

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
///
/// Pre-edit has no working-tree diff yet, so the empty-findings
/// systemMessage carries no diff size — callers always pass
/// `diff_summary: None` here. The signature parallels
/// [`write_post_tool_use`] for consistency.
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
        // Empty-findings line travels via `additionalContext`, the
        // same channel real findings use. `systemMessage` alone
        // surfaces only to the human user, leaving "mmk ran, found
        // nothing" invisible to the agent.
        let line = head_sha.map_or_else(
            || "[no actionable signal] no findings".to_owned(),
            |sha| {
                let short = if sha.len() >= 7 { &sha[..7] } else { sha };
                crate::output::messages::empty_review_line(short, None)
            },
        );
        (Some(line), None)
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
///
/// `diff_summary` shapes the empty-findings systemMessage: `None`
/// renders the clean-tree form `[no actionable signal] no findings
/// (HEAD <sha7>)`; `Some` renders the diff-bearing form
/// `[no actionable signal] no findings (N file[s], +M LOC vs HEAD
/// <sha7>)`. `mmk review` callers pass `Some` whenever a real diff
/// produced zero findings; pre-edit always passes `None`.
pub fn write_post_tool_use<W: Write>(
    w: &mut W,
    event_name: &str,
    findings: &[Finding],
    suppressed: bool,
    head_sha: Option<&str>,
    diff_summary: Option<EmptyDiffSummary>,
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
        // Same routing as `write_pre_tool_use`: empty-findings line
        // travels via `additionalContext` so the agent sees it.
        let line = head_sha.map_or_else(
            || "[no actionable signal] no findings".to_owned(),
            |sha| {
                let short = if sha.len() >= 7 { &sha[..7] } else { sha };
                crate::output::messages::empty_review_line(short, diff_summary.as_ref())
            },
        );
        (Some(line), None, None, None)
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
            None,
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
            None,
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
            None,
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

    #[test]
    fn post_tool_use_empty_findings_with_diff_summary_includes_size() {
        // A diff with zero findings still names what was reviewed
        // (file count + LOC) so silence is unambiguous. The line
        // travels via `additionalContext` — the channel real
        // findings use — not `systemMessage`, which surfaces only
        // to the human user and so leaves the line invisible to
        // the agent's next-turn context.
        let mut buf = Vec::new();
        write_post_tool_use(
            &mut buf,
            "PostToolUse",
            &[],
            false,
            Some("4bb7928abc"),
            Some(EmptyDiffSummary {
                file_count: 1,
                loc: 34,
            }),
            false,
        )
        .unwrap();
        let v: Value = serde_json::from_slice(&buf).unwrap();
        let ctx = v["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .expect("empty-findings line must reach the agent via additionalContext");
        assert!(
            ctx.contains("1 file") && ctx.contains("+34 LOC") && ctx.contains("HEAD 4bb7928"),
            "expected diff-bearing clean-state line in additionalContext; got: {ctx}"
        );
        assert!(
            v["systemMessage"].is_null(),
            "empty-findings line must not also fan-out to systemMessage; got: {}",
            v["systemMessage"]
        );
    }
}
