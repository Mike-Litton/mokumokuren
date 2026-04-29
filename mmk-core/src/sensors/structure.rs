//! STRUCTURE sensor — directory-convention detection.
//!
//! Given a query path P and a list of sibling paths in the same
//! directory, group siblings by **shape token** (extension + suffix
//! before the extension, e.g. `.service.ts`, `.test.ts`, plain
//! `.tsx`), pick the shape matching P, and aggregate the imports +
//! export templates that the majority of those siblings share.
//!
//! Pre-edit fires informatively (here's what the directory expects).
//! Review fires when a file's imports / exports diverge from the
//! detected convention.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use mmk_config::StructureCfg;
use mmk_health::ImportFact;

use super::FilesMap;

/// Render mode for a sensor invocation.
///
/// Whether the sensor is being asked for the pre-edit (informational
/// "here's the convention") wording or the review (divergence)
/// wording. The detected convention is the same in both modes; the
/// caller picks how to render it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StructureMode {
    PreEditNew,
    PreEditExisting,
    Review,
}

/// Inputs for a single STRUCTURE sensor invocation.
#[derive(Debug, Clone)]
pub struct StructureInput<'a> {
    /// The path being analyzed. Need not exist on disk (pre-edit
    /// new-file path); existence is encoded in `mode` instead.
    pub path: &'a Path,
    /// Other paths in the same directory, including untracked ones.
    /// The sensor filters these to the same shape token as `path`.
    pub siblings: &'a [PathBuf],
    /// File-body map covering every sibling the caller wants
    /// considered. Sources missing from the map are skipped.
    pub bodies: &'a FilesMap,
    /// `path`'s body if available (for review-mode divergence
    /// computation). Empty / None for new-file pre-edit.
    pub subject_body: Option<&'a str>,
    pub mode: StructureMode,
    pub cfg: &'a StructureCfg,
}

/// What STRUCTURE detected for the directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectoryConvention {
    /// Total siblings (including the subject if it was already on
    /// disk) sharing the same shape token.
    pub sibling_count: u32,
    /// `(ext, suffix)` shape token. `suffix` is empty for plain
    /// extensions; `index.<ext>` files use the synthetic suffix
    /// `__index__` so they never aggregate with siblings of the
    /// same extension.
    pub shape_ext: String,
    pub shape_suffix: String,
    /// Imports declared by ≥ majority of the same-shape siblings,
    /// alphabetised. Each entry's `symbols` is the intersection
    /// of imported symbols across siblings using that source.
    pub common_imports: Vec<ImportFact>,
    /// Export-name templates (e.g. `Create*Dialog`) declared by
    /// ≥ majority of siblings. Alphabetised.
    pub common_export_templates: Vec<String>,
}

/// One STRUCTURE finding to render in `messages`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructureFinding {
    pub path: PathBuf,
    pub kind: StructureFindingKind,
    pub convention: DirectoryConvention,
    /// `true` when the subject's stem matches a configured
    /// `role_patterns` entry (factory / contribution / registration
    /// / etc.). Role files legitimately diverge from sibling shape
    /// conventions — the formatter demotes Warn to Info and reframes
    /// the prose so the agent sees "role divergence is expected"
    /// rather than "fix this divergence." See
    /// [`mmk_config::DEFAULT_STRUCTURE_ROLE_PATTERNS`].
    pub is_role: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StructureFindingKind {
    /// Pre-edit, file does not yet exist.
    PreEditNew,
    /// Pre-edit, file already exists.
    PreEditExisting,
    /// Review, file conforms to the directory baseline. Surfaced
    /// only when `cfg.report_conformance` is true.
    ReviewConforming,
    /// Review, file diverges. Carries the missing common-imports
    /// (subset of `convention.common_imports`) and the missing
    /// expected export templates (subset of
    /// `convention.common_export_templates`).
    ReviewDivergent {
        missing_imports: Vec<ImportFact>,
        missing_templates: Vec<String>,
    },
}

/// Top-level entry point. Returns at most one finding per call —
/// shape detection collapses to a single answer per (directory,
/// path) pair, and the caller groups multiple subjects itself.
#[must_use]
pub fn compute_structure_finding(input: &StructureInput<'_>) -> Option<StructureFinding> {
    if !input.cfg.enabled {
        return None;
    }
    let token = shape_token(input.path)?;
    let same_shape = filter_same_shape(input.path, input.siblings, &token);
    let n = u32::try_from(same_shape.len()).unwrap_or(u32::MAX);
    if n < input.cfg.min_siblings {
        return None;
    }

    let convention = aggregate_convention(input, &token, &same_shape, n);
    if convention.common_imports.is_empty() && convention.common_export_templates.is_empty() {
        return None;
    }

    let kind = match input.mode {
        StructureMode::PreEditNew => StructureFindingKind::PreEditNew,
        StructureMode::PreEditExisting => StructureFindingKind::PreEditExisting,
        StructureMode::Review => review_kind(input, &convention)?,
    };

    Some(StructureFinding {
        path: input.path.to_path_buf(),
        kind,
        convention,
        is_role: is_role_file(input.path, &input.cfg.role_patterns),
    })
}

/// `true` iff `path`'s stem matches any role-pattern entry.
///
/// Patterns are stem-suffix matches: `*<suffix>` checks whether the
/// file stem (segment before final extension) ends with `<suffix>`.
/// Patterns without the `*` prefix are silently skipped — every
/// shipped pattern carries it.
#[must_use]
pub fn is_role_file(path: &Path, patterns: &[String]) -> bool {
    let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
        return false;
    };
    patterns
        .iter()
        .filter_map(|p| p.strip_prefix('*'))
        .any(|suffix| stem.ends_with(suffix))
}

fn review_kind(
    input: &StructureInput<'_>,
    convention: &DirectoryConvention,
) -> Option<StructureFindingKind> {
    // The subject's own facts are needed for divergence; without a
    // body we can't compute the missing-imports set.
    let body = input.subject_body?;
    let subject_facts = mmk_health::extract_with_imports(input.path, body).unwrap_or_default();

    let subject_imports: ahash::AHashSet<&str> = subject_facts
        .imports
        .iter()
        .map(|i| i.source.as_str())
        .collect();
    let missing_imports: Vec<ImportFact> = convention
        .common_imports
        .iter()
        .filter(|c| !subject_imports.contains(c.source.as_str()))
        .cloned()
        .collect();

    let subject_templates: ahash::AHashSet<&str> = subject_facts
        .exports
        .iter()
        .map(|e| e.template_stem.as_str())
        .collect();
    let missing_templates: Vec<String> = convention
        .common_export_templates
        .iter()
        .filter(|t| !subject_templates.contains(t.as_str()))
        .cloned()
        .collect();

    let missing_count = u32::try_from(missing_imports.len()).unwrap_or(u32::MAX);
    if missing_count >= input.cfg.divergence_min_missing || !missing_templates.is_empty() {
        return Some(StructureFindingKind::ReviewDivergent {
            missing_imports,
            missing_templates,
        });
    }
    if input.cfg.report_conformance {
        return Some(StructureFindingKind::ReviewConforming);
    }
    None
}

/// Public re-export so the COMPLEXITY sensor can group siblings by
/// the same shape token without re-deriving the rules.
#[must_use]
pub fn shape_token_pub(path: &Path) -> Option<(String, String)> {
    shape_token(path)
}

/// Compute `(ext, suffix)` for `path`, returning `None` for paths
/// without an extension. `index.<ext>` files use the synthetic
/// suffix `__index__` so they never aggregate across directories
/// or with siblings of the same extension.
fn shape_token(path: &Path) -> Option<(String, String)> {
    let ext = path.extension().and_then(|e| e.to_str())?.to_lowercase();
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    if stem == "index" {
        return Some((ext, "__index__".to_owned()));
    }
    if let Some(dot) = stem.rfind('.') {
        let suffix = stem[dot + 1..].to_owned();
        return Some((ext, suffix));
    }
    Some((ext, String::new()))
}

fn filter_same_shape(
    subject: &Path,
    siblings: &[PathBuf],
    token: &(String, String),
) -> Vec<PathBuf> {
    siblings
        .iter()
        .filter(|p| p.as_path() != subject)
        .filter(|p| shape_token(p).as_ref() == Some(token))
        .cloned()
        .collect()
}

fn aggregate_convention(
    input: &StructureInput<'_>,
    token: &(String, String),
    same_shape: &[PathBuf],
    n_total: u32,
) -> DirectoryConvention {
    // Per-source: how many siblings imported it, plus the running
    // intersection of imported symbols across those siblings.
    let mut import_count: BTreeMap<String, u32> = BTreeMap::new();
    let mut import_symbols: BTreeMap<String, Option<Vec<String>>> = BTreeMap::new();
    let mut template_count: BTreeMap<String, u32> = BTreeMap::new();

    let mut considered = 0_u32;
    for path in same_shape {
        let Some(body) = input.bodies.get(path) else {
            continue;
        };
        let facts = if input.cfg.linescan_fallback {
            mmk_health::extract_with_imports(path, body)
        } else {
            mmk_health::extract(path, body)
        };
        let Some(facts) = facts else { continue };
        considered += 1;
        // Collapse repeated imports of the same source within one
        // sibling (e.g. two `from "react"` lines) to a single hit.
        let mut seen: ahash::AHashSet<String> = ahash::AHashSet::new();
        for imp in &facts.imports {
            if !seen.insert(imp.source.clone()) {
                continue;
            }
            *import_count.entry(imp.source.clone()).or_insert(0) += 1;
            let entry = import_symbols.entry(imp.source.clone()).or_insert(None);
            *entry = Some(intersect_or_init(entry.as_deref(), &imp.symbols));
        }
        let mut seen_tpl: ahash::AHashSet<String> = ahash::AHashSet::new();
        for exp in &facts.exports {
            if exp.template_stem == exp.name {
                continue; // not a template-shaped export
            }
            if seen_tpl.insert(exp.template_stem.clone()) {
                *template_count.entry(exp.template_stem.clone()).or_insert(0) += 1;
            }
        }
    }

    let n_for_majority = considered.max(1);
    let import_floor = majority_floor(n_for_majority, input.cfg.import_majority);
    let template_floor = majority_floor(n_for_majority, input.cfg.export_template_majority);

    let mut common_imports: Vec<ImportFact> = import_count
        .into_iter()
        .filter(|(_, c)| *c >= import_floor)
        .map(|(source, _)| ImportFact {
            symbols: import_symbols
                .remove(&source)
                .and_then(|s| s)
                .unwrap_or_default(),
            source,
        })
        .collect();
    common_imports.sort_by(|a, b| a.source.cmp(&b.source));

    let mut common_export_templates: Vec<String> = template_count
        .into_iter()
        .filter(|(_, c)| *c >= template_floor)
        .map(|(t, _)| t)
        .collect();
    common_export_templates.sort();

    DirectoryConvention {
        sibling_count: n_total,
        shape_ext: token.0.clone(),
        shape_suffix: token.1.clone(),
        common_imports,
        common_export_templates,
    }
}

fn intersect_or_init(prev: Option<&[String]>, next: &[String]) -> Vec<String> {
    prev.map_or_else(
        || next.to_vec(),
        |prev| {
            let set: ahash::AHashSet<&str> = next.iter().map(String::as_str).collect();
            prev.iter()
                .filter(|s| set.contains(s.as_str()))
                .cloned()
                .collect()
        },
    )
}

/// Smallest count `c` such that `c / n ≥ majority` — i.e.
/// `ceil(majority × n)`, with a floor of 1 so a single sibling can
/// still satisfy a 0.5 majority on n=2.
fn majority_floor(n: u32, majority: f64) -> u32 {
    let raw = (f64::from(n) * majority).ceil() as i64;
    let floor = raw.max(1).min(i64::from(n));
    u32::try_from(floor).unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mmk_config::StructureCfg;
    use std::path::PathBuf;

    fn cfg() -> StructureCfg {
        StructureCfg::default()
    }

    fn ts(path: &str, body: &str) -> (PathBuf, String) {
        (PathBuf::from(path), body.to_owned())
    }

    fn make_bodies(entries: &[(PathBuf, String)]) -> FilesMap {
        let mut m = FilesMap::default();
        for (p, b) in entries {
            m.insert(p.clone(), b.clone());
        }
        m
    }

    #[test]
    fn shape_token_distinguishes_test_from_plain() {
        assert_eq!(
            shape_token(Path::new("a/b/foo.test.ts")),
            Some(("ts".into(), "test".into()))
        );
        assert_eq!(
            shape_token(Path::new("a/b/foo.ts")),
            Some(("ts".into(), String::new()))
        );
    }

    #[test]
    fn shape_token_index_uses_synthetic_suffix() {
        assert_eq!(
            shape_token(Path::new("a/b/index.ts")),
            Some(("ts".into(), "__index__".into()))
        );
    }

    #[test]
    fn aggregates_common_imports_at_majority() {
        // 4 siblings; 3 import zod (75 %), 1 imports react (25 %).
        // The current default (0.85) would reject zod too — this
        // test exercises the threshold mechanism on a sub-unanimous
        // fixture, so it pins an explicit 0.66 threshold. → zod
        // survives, react does not.
        let body_with = "import { z } from 'zod';\nexport function CreateAwardDialog(){}\n";
        let body_no = "import React from 'react';\nexport function Helper(){}\n";
        let entries = vec![
            ts("dlg/award.tsx", body_with),
            ts("dlg/goal.tsx", body_with),
            ts("dlg/job.tsx", body_with),
            ts("dlg/extra.tsx", body_no),
        ];
        let bodies = make_bodies(&entries);
        let siblings: Vec<PathBuf> = entries.iter().map(|(p, _)| p.clone()).collect();

        let cfg = StructureCfg {
            import_majority: 0.66,
            export_template_majority: 0.66,
            ..StructureCfg::default()
        };
        let input = StructureInput {
            path: Path::new("dlg/new.tsx"),
            siblings: &siblings,
            bodies: &bodies,
            subject_body: None,
            mode: StructureMode::PreEditNew,
            cfg: &cfg,
        };
        let f = compute_structure_finding(&input).expect("should fire");
        let sources: Vec<&str> = f
            .convention
            .common_imports
            .iter()
            .map(|i| i.source.as_str())
            .collect();
        assert!(sources.contains(&"zod"));
        assert!(!sources.contains(&"react"));
    }

    #[test]
    fn detects_export_template_across_siblings() {
        let entries = vec![
            ts(
                "dlg/award.tsx",
                "export function CreateAwardDialog(){ return null; }\n",
            ),
            ts(
                "dlg/goal.tsx",
                "export function CreateGoalDialog(){ return null; }\n",
            ),
            ts(
                "dlg/job.tsx",
                "export function CreateJobDialog(){ return null; }\n",
            ),
        ];
        let bodies = make_bodies(&entries);
        let siblings: Vec<PathBuf> = entries.iter().map(|(p, _)| p.clone()).collect();

        let cfg = cfg();
        let input = StructureInput {
            path: Path::new("dlg/new.tsx"),
            siblings: &siblings,
            bodies: &bodies,
            subject_body: None,
            mode: StructureMode::PreEditNew,
            cfg: &cfg,
        };
        let f = compute_structure_finding(&input).expect("should fire");
        assert!(f
            .convention
            .common_export_templates
            .contains(&"Create*Dialog".to_owned()));
    }

    #[test]
    fn min_siblings_floor_suppresses_when_below() {
        // 2 siblings — below default min_siblings=3.
        let entries = vec![
            ts("dlg/a.tsx", "import { z } from 'zod';\nexport const A=1;\n"),
            ts("dlg/b.tsx", "import { z } from 'zod';\nexport const B=2;\n"),
        ];
        let bodies = make_bodies(&entries);
        let siblings: Vec<PathBuf> = entries.iter().map(|(p, _)| p.clone()).collect();
        let cfg = cfg();
        let input = StructureInput {
            path: Path::new("dlg/new.tsx"),
            siblings: &siblings,
            bodies: &bodies,
            subject_body: None,
            mode: StructureMode::PreEditNew,
            cfg: &cfg,
        };
        assert!(compute_structure_finding(&input).is_none());
    }

    #[test]
    fn review_mode_emits_divergence_when_imports_missing() {
        let entries = vec![
            ts(
                "dlg/award.tsx",
                "import { z } from 'zod';\nimport React from 'react';\nexport function CreateAwardDialog(){}\n",
            ),
            ts(
                "dlg/goal.tsx",
                "import { z } from 'zod';\nimport React from 'react';\nexport function CreateGoalDialog(){}\n",
            ),
            ts(
                "dlg/job.tsx",
                "import { z } from 'zod';\nimport React from 'react';\nexport function CreateJobDialog(){}\n",
            ),
        ];
        let bodies = make_bodies(&entries);
        let siblings: Vec<PathBuf> = entries.iter().map(|(p, _)| p.clone()).collect();
        let cfg = cfg();
        let subject_body = "export const x = 1;\n";
        let input = StructureInput {
            path: Path::new("dlg/divergent.tsx"),
            siblings: &siblings,
            bodies: &bodies,
            subject_body: Some(subject_body),
            mode: StructureMode::Review,
            cfg: &cfg,
        };
        let f = compute_structure_finding(&input).expect("should fire");
        match &f.kind {
            StructureFindingKind::ReviewDivergent {
                missing_imports,
                missing_templates,
            } => {
                let sources: Vec<&str> =
                    missing_imports.iter().map(|i| i.source.as_str()).collect();
                assert!(sources.contains(&"zod"));
                assert!(sources.contains(&"react"));
                assert!(missing_templates.contains(&"Create*Dialog".to_owned()));
            }
            other => panic!("expected divergence; got {other:?}"),
        }
    }

    #[test]
    fn review_mode_silent_on_conforming_when_report_conformance_off() {
        let entries = vec![
            ts(
                "dlg/award.tsx",
                "import { z } from 'zod';\nexport function CreateAwardDialog(){}\n",
            ),
            ts(
                "dlg/goal.tsx",
                "import { z } from 'zod';\nexport function CreateGoalDialog(){}\n",
            ),
            ts(
                "dlg/job.tsx",
                "import { z } from 'zod';\nexport function CreateJobDialog(){}\n",
            ),
        ];
        let bodies = make_bodies(&entries);
        let siblings: Vec<PathBuf> = entries.iter().map(|(p, _)| p.clone()).collect();
        let cfg = cfg();
        let conforming = "import { z } from 'zod';\nexport function CreateNewDialog(){}\n";
        let input = StructureInput {
            path: Path::new("dlg/new.tsx"),
            siblings: &siblings,
            bodies: &bodies,
            subject_body: Some(conforming),
            mode: StructureMode::Review,
            cfg: &cfg,
        };
        assert!(compute_structure_finding(&input).is_none());
    }
}
