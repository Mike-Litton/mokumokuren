//! COMPLEXITY sensor — per-function structural budget.
//!
//! For each function in `path`, fire a finding when its nesting
//! depth or LOC is either:
//!   - above an absolute cap (catches uniformly bad directories), or
//!   - above a relative ratio versus the directory median (catches
//!     outliers within a permissive directory).
//!
//! The relative path needs ≥ `min_directory_siblings` files of the
//! same shape to compute a stable median; below that floor, only the
//! absolute thresholds apply.

use std::path::{Path, PathBuf};

use mmk_config::ComplexityCfg;
use mmk_health::FunctionFact;

use super::FilesMap;

#[derive(Debug, Clone)]
pub struct ComplexityInput<'a> {
    pub path: &'a Path,
    /// Other paths in the same directory, used to compute the
    /// median.
    pub siblings: &'a [PathBuf],
    /// File-body map covering the subject + every sibling the
    /// caller wants considered.
    pub bodies: &'a FilesMap,
    pub cfg: &'a ComplexityCfg,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComplexityFinding {
    pub path: PathBuf,
    pub function: String,
    pub kind: ComplexityFindingKind,
    /// Actual measurement on the function.
    pub actual: u32,
    /// Median across same-shape directory siblings, when available.
    /// `None` indicates the relative threshold didn't apply (too
    /// few siblings); the finding fired on the absolute threshold.
    pub directory_median: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComplexityFindingKind {
    Nesting,
    Size,
}

#[must_use]
pub fn compute_complexity_findings(input: &ComplexityInput<'_>) -> Vec<ComplexityFinding> {
    if !input.cfg.enabled {
        return Vec::new();
    }
    let Some(body) = input.bodies.get(input.path) else {
        return Vec::new();
    };
    // Refuse to fire if the language doesn't have a real adapter
    // (line-scan can't measure nesting).
    let Some(facts) = mmk_health::extract(input.path, body) else {
        return Vec::new();
    };

    let same_shape: Vec<&PathBuf> = input
        .siblings
        .iter()
        .filter(|p| p.as_path() != input.path)
        .filter(|p| same_shape_token(input.path, p))
        .collect();

    let median_nesting;
    let median_loc;
    let n_siblings = u32::try_from(same_shape.len()).unwrap_or(u32::MAX);
    if n_siblings >= input.cfg.min_directory_siblings {
        let mut nestings: Vec<u32> = Vec::new();
        let mut locs: Vec<u32> = Vec::new();
        for sib in &same_shape {
            let Some(b) = input.bodies.get(sib.as_path()) else {
                continue;
            };
            let Some(f) = mmk_health::extract(sib.as_path(), b) else {
                continue;
            };
            for fun in f.functions {
                nestings.push(fun.max_nesting_depth);
                locs.push(fun.loc);
            }
        }
        median_nesting = median(&mut nestings);
        median_loc = median(&mut locs);
    } else {
        median_nesting = None;
        median_loc = None;
    }

    let mut out = Vec::new();
    for fun in &facts.functions {
        if let Some(f) = check_nesting(input.path, fun, input.cfg, median_nesting) {
            out.push(f);
        }
        if let Some(f) = check_size(input.path, fun, input.cfg, median_loc) {
            out.push(f);
        }
    }
    out
}

fn check_nesting(
    path: &Path,
    fun: &FunctionFact,
    cfg: &ComplexityCfg,
    median: Option<u32>,
) -> Option<ComplexityFinding> {
    if fun.max_nesting_depth > cfg.nesting_absolute_max {
        return Some(ComplexityFinding {
            path: path.to_path_buf(),
            function: fun.name.clone(),
            kind: ComplexityFindingKind::Nesting,
            actual: fun.max_nesting_depth,
            directory_median: median,
        });
    }
    if let Some(m) = median {
        if m == 0 {
            return None;
        }
        let ratio = f64::from(fun.max_nesting_depth) / f64::from(m);
        if ratio > cfg.nesting_ratio_threshold {
            return Some(ComplexityFinding {
                path: path.to_path_buf(),
                function: fun.name.clone(),
                kind: ComplexityFindingKind::Nesting,
                actual: fun.max_nesting_depth,
                directory_median: Some(m),
            });
        }
    }
    None
}

fn check_size(
    path: &Path,
    fun: &FunctionFact,
    cfg: &ComplexityCfg,
    median: Option<u32>,
) -> Option<ComplexityFinding> {
    if fun.loc > cfg.loc_absolute_max {
        return Some(ComplexityFinding {
            path: path.to_path_buf(),
            function: fun.name.clone(),
            kind: ComplexityFindingKind::Size,
            actual: fun.loc,
            directory_median: median,
        });
    }
    if let Some(m) = median {
        if m == 0 {
            return None;
        }
        let ratio = f64::from(fun.loc) / f64::from(m);
        if ratio > cfg.loc_ratio_threshold {
            return Some(ComplexityFinding {
                path: path.to_path_buf(),
                function: fun.name.clone(),
                kind: ComplexityFindingKind::Size,
                actual: fun.loc,
                directory_median: Some(m),
            });
        }
    }
    None
}

fn same_shape_token(a: &Path, b: &Path) -> bool {
    super::structure::shape_token_pub(a) == super::structure::shape_token_pub(b)
}

fn median(values: &mut [u32]) -> Option<u32> {
    if values.is_empty() {
        return None;
    }
    values.sort_unstable();
    let mid = values.len() / 2;
    Some(values[mid])
}

#[cfg(test)]
mod tests {
    use super::*;
    use mmk_config::ComplexityCfg;
    use std::path::PathBuf;

    fn cfg() -> ComplexityCfg {
        ComplexityCfg::default()
    }

    fn make_bodies(entries: &[(PathBuf, String)]) -> FilesMap {
        let mut m = FilesMap::default();
        for (p, b) in entries {
            m.insert(p.clone(), b.clone());
        }
        m
    }

    #[test]
    fn nesting_absolute_threshold_fires() {
        let body = r"
function deep() {
  if (a) { if (b) { if (c) { if (d) { if (e) { if (f) { if (g) { return 1; } } } } } } }
}
";
        let entries = vec![(PathBuf::from("a.ts"), body.to_owned())];
        let bodies = make_bodies(&entries);
        let cfg = cfg();
        let input = ComplexityInput {
            path: Path::new("a.ts"),
            siblings: &[],
            bodies: &bodies,
            cfg: &cfg,
        };
        let findings = compute_complexity_findings(&input);
        assert!(
            findings
                .iter()
                .any(|f| f.kind == ComplexityFindingKind::Nesting),
            "expected absolute-threshold nesting finding; got: {findings:?}"
        );
    }

    #[test]
    fn refuses_to_fire_without_real_adapter() {
        // .rs file → adapter returns None → COMPLEXITY refuses.
        let entries = vec![(
            PathBuf::from("a.rs"),
            "fn deep() { if a { if b { if c { return 1; } } } }".to_owned(),
        )];
        let bodies = make_bodies(&entries);
        let cfg = cfg();
        let input = ComplexityInput {
            path: Path::new("a.rs"),
            siblings: &[],
            bodies: &bodies,
            cfg: &cfg,
        };
        assert!(compute_complexity_findings(&input).is_empty());
    }

    #[test]
    fn relative_threshold_uses_median_when_enough_siblings() {
        // 3 siblings each with 1 shallow function → median nesting = 1.
        // Subject has nesting 4 → ratio 4 > default ratio threshold 3.0.
        let shallow = "function s(){ return 1; }\n";
        let deep_body = "function d(){ if (a) { if (b) { if (c) { return 1; } } } }\n";
        let entries = vec![
            (PathBuf::from("dir/s1.ts"), shallow.to_owned()),
            (PathBuf::from("dir/s2.ts"), shallow.to_owned()),
            (PathBuf::from("dir/s3.ts"), shallow.to_owned()),
            (PathBuf::from("dir/subject.ts"), deep_body.to_owned()),
        ];
        let bodies = make_bodies(&entries);
        let siblings: Vec<PathBuf> = entries.iter().map(|(p, _)| p.clone()).collect();
        let cfg = cfg();
        let input = ComplexityInput {
            path: Path::new("dir/subject.ts"),
            siblings: &siblings,
            bodies: &bodies,
            cfg: &cfg,
        };
        let findings = compute_complexity_findings(&input);
        assert!(
            findings
                .iter()
                .any(|f| f.kind == ComplexityFindingKind::Nesting && f.directory_median == Some(1)),
            "expected relative-median nesting finding; got: {findings:?}"
        );
    }

    #[test]
    fn skips_relative_when_too_few_siblings() {
        // 1 sibling — below default min_directory_siblings=3. With
        // a deep-nesting subject ratio that *would* exceed the
        // ratio threshold but stays under the absolute cap of 6.
        let shallow = "function s(){ return 1; }\n";
        let medium = "function m(){ if (a){ if (b){ if (c){ if (d){ return 1; } } } } }\n";
        let entries = vec![
            (PathBuf::from("dir/s1.ts"), shallow.to_owned()),
            (PathBuf::from("dir/subject.ts"), medium.to_owned()),
        ];
        let bodies = make_bodies(&entries);
        let siblings: Vec<PathBuf> = entries.iter().map(|(p, _)| p.clone()).collect();
        let cfg = cfg();
        let input = ComplexityInput {
            path: Path::new("dir/subject.ts"),
            siblings: &siblings,
            bodies: &bodies,
            cfg: &cfg,
        };
        let findings = compute_complexity_findings(&input);
        // medium has nesting 5; absolute cap is 6 → no finding.
        assert!(
            findings.is_empty(),
            "ratio threshold must NOT apply with too few siblings; got: {findings:?}"
        );
    }
}
