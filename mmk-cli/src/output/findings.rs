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
    /// Tangled-diff detection (`[sensor.cohesion]`) — fires when a
    /// working-tree diff spans multiple disjoint coupling-graph
    /// components. Reads as "this looks like two changes carried in
    /// one diff," the structural fingerprint Herzig & Zeller (2013)
    /// identified as elevating revert / review cost.
    Cohesion,
    Drift,
    Budget,
    /// Structural-pattern adapter findings (mmk-health). Populated
    /// when `[health.<lang>]` is enabled and a pattern matches.
    Health,
    /// Directory-convention sensor (`[sensor.structure]`).
    Structure,
    /// Per-function structural-budget sensor (`[sensor.complexity]`).
    Complexity,
    /// Reserved for ADR / CHANGELOG / PR-history surfacing —
    /// declared so adding it later doesn't bump the schema.
    Anchor,
}

impl Layer {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Hotspot => "HOTSPOT",
            Self::Coupling => "COUPLING",
            Self::Cohesion => "COHESION",
            Self::Drift => "DRIFT",
            Self::Budget => "BUDGET",
            Self::Health => "HEALTH",
            Self::Structure => "STRUCTURE",
            Self::Complexity => "COMPLEXITY",
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
    /// Line-prefix marker shown in text output. `Warn` and `Ok` use
    /// glyphs that stand out typographically; `Info` uses the
    /// bracketed `[info]` because the `ⓘ` glyph tokenized too close
    /// to `⚠` for agent consumers, and the bracketed form matches
    /// mmk's other low-priority prefixes (`[no actionable signal]`,
    /// `[low-confidence n=N]`).
    #[must_use]
    pub const fn marker(self) -> &'static str {
        match self {
            Self::Warn => "⚠",
            Self::Info => "[info]",
            Self::Ok => "✓",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    pub layer: Layer,
    pub severity: Severity,
    pub message: String,
    /// Stable fingerprint for `mmk explain --finding <id>` — the
    /// addressable join-key the agent passes back when it wants the
    /// per-commit evidence behind a borderline claim. Populated for
    /// COUPLING in v0.11; `None` for layers that don't yet have
    /// explain support, serialised as `null` so the absence is
    /// explicit instead of inferred from a missing key.
    pub id: Option<String>,
}

impl Finding {
    /// Constructor without an explain id. Kept so the 30-plus existing
    /// emission sites compile unchanged; new COUPLING emissions go
    /// through [`Finding::with_id`].
    #[must_use]
    pub const fn new(layer: Layer, severity: Severity, message: String) -> Self {
        Self {
            layer,
            severity,
            message,
            id: None,
        }
    }

    /// Constructor that attaches the addressable [`mmk explain`] id.
    #[must_use]
    pub const fn with_id(layer: Layer, severity: Severity, message: String, id: String) -> Self {
        Self {
            layer,
            severity,
            message,
            id: Some(id),
        }
    }
}

// Layer rendering order. COHESION sits next to COUPLING because the
// two answer related questions on the same co-change graph: COUPLING
// flags a *missing* partner of one edited file; COHESION flags
// *multiple disjoint clusters* in the diff. Reading them adjacently
// keeps the agent's mental model of "co-change graph signals" in
// one place.
const LAYER_ORDER: [Layer; 9] = [
    Layer::Hotspot,
    Layer::Coupling,
    Layer::Cohesion,
    Layer::Drift,
    Layer::Budget,
    Layer::Health,
    Layer::Structure,
    Layer::Complexity,
    Layer::Anchor,
];

/// Group by layer, write `LAYER:` header, then one indented line per
/// finding. Empty input writes nothing. Severity → line prefix
/// mapping lives on [`Severity::marker`].
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
            match &f.id {
                Some(id) => writeln!(w, "  {} {} [id={id}]", f.severity.marker(), f.message)?,
                None => writeln!(w, "  {} {}", f.severity.marker(), f.message)?,
            }
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
