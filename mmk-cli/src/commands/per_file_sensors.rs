//! Per-file sensor seam.
//!
//! `STRUCTURE` and `COMPLEXITY` are per-file sensors: their cost
//! scales with the count of subject files, not the history graph.
//! `review` invokes both for every changed file; `pre-edit` invokes
//! `STRUCTURE` only (complexity needs the working-tree body to
//! compare against, and pre-edit fires *before* the agent's edit, so
//! there is nothing to measure yet). This helper centralizes that
//! asymmetry — when a third per-file sensor lands, this is the one
//! place to wire it.
//!
//! Returns `(Finding, Option<MonotonicSignal>)` pairs in the order
//! produced by each sensor. The caller hands the result to
//! `apply_monotonic_gate`; layer ordering is applied at render time
//! by `output::findings::LAYER_ORDER`, not here.

use std::path::{Path, PathBuf};

use mmk_config::SensorCfg;
use mmk_core::sensors::{
    self, ComplexityFinding, ComplexityFindingKind, ComplexityInput, FilesMap, StructureInput,
    StructureMode,
};

use crate::commands::common::{complexity_to_finding, structure_to_finding_with_signal};
use crate::monotonic::MonotonicSignal;
use crate::output::findings::Finding;

/// Which command is asking, and (for `Structure`) what mode to use.
/// `Review` runs both sensors; pre-edit modes run `STRUCTURE` only.
#[derive(Debug, Clone, Copy)]
pub(crate) enum PerFileMode {
    Review,
    PreEditNew,
    PreEditExisting,
}

/// Inputs that all per-file sensors share. Caller is responsible for
/// loading sibling bodies (`load_bodies`) and the subject body where
/// applicable; the helper does no I/O.
pub(crate) struct PerFileCtx<'a> {
    pub path: &'a Path,
    pub siblings: &'a [PathBuf],
    pub bodies: &'a FilesMap,
    /// Subject's working-tree body. `Some` for `Review`; `None` for
    /// pre-edit (the subject hasn't been read because the agent is
    /// about to edit it).
    pub subject_body: Option<&'a str>,
}

/// Run all enabled per-file sensors against `ctx`. Caller dedups /
/// orders downstream.
///
/// `head_body` is the subject's body at HEAD, used by COMPLEXITY's
/// delta-vs-HEAD baseline filter (suppress findings where the
/// function's metric didn't worsen vs. HEAD). Ignored when
/// `mode != Review` because pre-edit doesn't run COMPLEXITY.
pub(crate) fn compute_per_file_findings(
    ctx: &PerFileCtx<'_>,
    cfg: &SensorCfg,
    mode: PerFileMode,
    head_body: Option<&str>,
) -> Vec<(Finding, Option<MonotonicSignal>)> {
    let mut out = Vec::new();
    if cfg.structure.enabled {
        let structure_mode = match mode {
            PerFileMode::Review => StructureMode::Review,
            PerFileMode::PreEditNew => StructureMode::PreEditNew,
            PerFileMode::PreEditExisting => StructureMode::PreEditExisting,
        };
        let input = StructureInput {
            path: ctx.path,
            siblings: ctx.siblings,
            bodies: ctx.bodies,
            subject_body: ctx.subject_body,
            mode: structure_mode,
            cfg: &cfg.structure,
        };
        if let Some(sf) = sensors::compute_structure_finding(&input) {
            let cap = cfg.structure.top_imports_to_show;
            let pct = (cfg.structure.import_majority * 100.0).round() as u32;
            out.push(structure_to_finding_with_signal(&sf, cap, pct));
        }
    }

    if matches!(mode, PerFileMode::Review) && cfg.complexity.enabled {
        let input = ComplexityInput {
            path: ctx.path,
            siblings: ctx.siblings,
            bodies: ctx.bodies,
            cfg: &cfg.complexity,
        };
        let raw = sensors::compute_complexity_findings(&input);
        let filtered = filter_complexity_by_head_baseline(ctx.path, raw, head_body);
        for cf in filtered {
            let signal = complexity_monotonic_signal(&cf);
            out.push((complexity_to_finding(&cf, &cfg.complexity), Some(signal)));
        }
    }
    out
}

/// Build the per-finding monotonic key + axes for a COMPLEXITY
/// finding. `kind` is encoded in the key so a Nesting finding and a
/// Size finding on the same `(path, function)` get independent
/// suppression — they measure different things and can move
/// independently.
fn complexity_monotonic_signal(f: &ComplexityFinding) -> MonotonicSignal {
    let kind = match f.kind {
        ComplexityFindingKind::Nesting => "nesting",
        ComplexityFindingKind::Size => "loc",
    };
    let key = format!("complexity::{kind}::{}::{}", f.path.display(), f.function);
    MonotonicSignal {
        key,
        axes: vec![f.actual],
    }
}

/// Drop COMPLEXITY findings whose function exists at HEAD with the
/// same-or-better metric value. Keeps findings on:
/// - new files (no HEAD body to compare against),
/// - newly-added functions (no matching name at HEAD),
/// - functions whose metric strictly worsened vs. HEAD.
///
/// Function identity is compared by `FunctionFact.qualified_name`
/// (`ClassName::methodName` for methods; bare name for top-level
/// functions). v0.8 used `FunctionFact.name` and silently
/// cross-attributed methods that shared a bare name across classes
/// in one file (`constructor`, `dispose`, `init`, …) — the first
/// AST match won, so an agent shrinking `Inner::constructor` could
/// see COMPLEXITY fire on the same file with `+N vs HEAD` computed
/// against `Outer::constructor`'s baseline. v0.9's qualified-name
/// match closes that off.
///
/// Known weakness: a rename (`parse` → `parseV4`) still leaves the
/// working-tree function with no HEAD match, so it fires as if it
/// were newly added — even though the body is structurally
/// identical. This false-fire is acceptable because renames are
/// rare in feature work and the resulting finding is still
/// factually true (the renamed function *is* over the cap); the
/// cost is one extra Warn per rename. A tighter check would
/// require structural matching across renames, which costs more
/// than the false-fire it prevents.
///
/// HEAD-parse failures (rare — tree-sitter is error-tolerant) are
/// treated conservatively: keep the finding rather than silently
/// drop a real over-cap signal because we couldn't compute a
/// baseline.
fn filter_complexity_by_head_baseline(
    subject: &Path,
    findings: Vec<ComplexityFinding>,
    head_body: Option<&str>,
) -> Vec<ComplexityFinding> {
    let Some(body) = head_body else {
        return findings;
    };
    let Some(head_facts) = mmk_health::extract(subject, body) else {
        return findings;
    };
    findings
        .into_iter()
        .filter_map(|mut f| {
            let Some(hf) = head_facts
                .functions
                .iter()
                .find(|hf| hf.qualified_name == f.function)
            else {
                return Some(f);
            };
            let head_actual = match f.kind {
                ComplexityFindingKind::Nesting => hf.max_nesting_depth,
                ComplexityFindingKind::Size => hf.loc,
            };
            if f.actual > head_actual {
                f.head_actual = Some(head_actual);
                Some(f)
            } else {
                None
            }
        })
        .collect()
}
