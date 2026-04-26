//! Layer-labeled findings — the unified shape every event-driven
//! subcommand emits (`review`, `pre-edit`, `drift`, `session-summary`).
//!
//! Text mode groups findings by layer (`HOTSPOT:` / `COUPLING:` / …)
//! with one body line per finding so a human reviewer or `grep` can
//! scan it. JSON mode is a flat array with stable `{layer, severity,
//! message}` keys for LLM-harness consumers.

use anyhow::Result;
use serde::Serialize;
use std::io::Write;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Layer {
    Hotspot,
    Coupling,
    Drift,
    Budget,
    // v0.4 reserved slots — declared now so adding AST-based adapters
    // later doesn't bump the schema.
    Health,
    Anchor,
}

impl Layer {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Hotspot => "HOTSPOT",
            Self::Coupling => "COUPLING",
            Self::Drift => "DRIFT",
            Self::Budget => "BUDGET",
            Self::Health => "HEALTH",
            Self::Anchor => "ANCHOR",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Warn,
    Info,
    Ok,
}

impl Severity {
    #[must_use]
    pub const fn glyph(self) -> &'static str {
        match self {
            Self::Warn => "⚠",
            Self::Info => "ⓘ",
            Self::Ok => "✓",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    pub layer: Layer,
    pub severity: Severity,
    pub message: String,
}

impl Finding {
    pub fn new(layer: Layer, severity: Severity, message: impl Into<String>) -> Self {
        Self {
            layer,
            severity,
            message: message.into(),
        }
    }
}

const LAYER_ORDER: [Layer; 6] = [
    Layer::Hotspot,
    Layer::Coupling,
    Layer::Drift,
    Layer::Budget,
    Layer::Health,
    Layer::Anchor,
];

/// Group by layer, write `LAYER:` header, then one indented line per
/// finding. Empty input writes nothing.
pub fn render_text<W: Write>(w: &mut W, findings: &[Finding]) -> Result<()> {
    if findings.is_empty() {
        return Ok(());
    }
    for layer in LAYER_ORDER {
        let mut group = findings.iter().filter(|f| f.layer == layer).peekable();
        if group.peek().is_none() {
            continue;
        }
        writeln!(w, "{}:", layer.label())?;
        for f in group {
            writeln!(w, "  {} {}", f.severity.glyph(), f.message)?;
        }
    }
    Ok(())
}

/// Flat JSON array; no grouping, no headers. Pretty-printed to match
/// the rest of the CLI's JSON output.
pub fn render_json<W: Write>(w: &mut W, findings: &[Finding]) -> Result<()> {
    serde_json::to_writer_pretty(&mut *w, findings)?;
    writeln!(w)?;
    Ok(())
}
