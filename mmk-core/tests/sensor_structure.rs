//! Integration tests for the STRUCTURE sensor — shape detection,
//! aggregation edge cases, divergence detection.

use mmk_config::StructureCfg;
use mmk_core::sensors::{
    compute_structure_finding, FilesMap, StructureFindingKind, StructureInput, StructureMode,
};
use std::path::{Path, PathBuf};

fn cfg() -> StructureCfg {
    StructureCfg::default()
}

fn bodies(entries: &[(&str, &str)]) -> (FilesMap, Vec<PathBuf>) {
    let mut m = FilesMap::default();
    let mut v = Vec::new();
    for (p, b) in entries {
        let path = PathBuf::from(p);
        m.insert(path.clone(), (*b).to_owned());
        v.push(path);
    }
    (m, v)
}

#[test]
fn shape_index_does_not_aggregate_with_plain_ts() {
    // index.ts uses synthetic suffix `__index__`; siblings with
    // a plain `.ts` don't match its shape.
    let (bodies_map, siblings) = bodies(&[
        ("dir/a.ts", "import { z } from 'zod';\nexport const A=1;\n"),
        ("dir/b.ts", "import { z } from 'zod';\nexport const B=2;\n"),
        ("dir/c.ts", "import { z } from 'zod';\nexport const C=3;\n"),
    ]);
    let cfg = cfg();
    let input = StructureInput {
        path: Path::new("dir/index.ts"),
        siblings: &siblings,
        bodies: &bodies_map,
        subject_body: None,
        mode: StructureMode::PreEditNew,
        cfg: &cfg,
    };
    assert!(
        compute_structure_finding(&input).is_none(),
        "index shape must not aggregate with plain .ts siblings"
    );
}

#[test]
fn test_files_aggregate_separately() {
    // 3 .test.ts siblings + 3 plain .ts siblings. A new .test.ts
    // should aggregate against the .test.ts ones only.
    let (bodies_map, siblings) = bodies(&[
        (
            "dir/a.test.ts",
            "import { describe } from 'vitest';\nexport const A=1;\n",
        ),
        (
            "dir/b.test.ts",
            "import { describe } from 'vitest';\nexport const B=2;\n",
        ),
        (
            "dir/c.test.ts",
            "import { describe } from 'vitest';\nexport const C=3;\n",
        ),
        (
            "dir/a.ts",
            "import { thing } from 'lib';\nexport const A=1;\n",
        ),
        (
            "dir/b.ts",
            "import { thing } from 'lib';\nexport const B=2;\n",
        ),
        (
            "dir/c.ts",
            "import { thing } from 'lib';\nexport const C=3;\n",
        ),
    ]);
    let cfg = cfg();
    let input = StructureInput {
        path: Path::new("dir/d.test.ts"),
        siblings: &siblings,
        bodies: &bodies_map,
        subject_body: None,
        mode: StructureMode::PreEditNew,
        cfg: &cfg,
    };
    let f = compute_structure_finding(&input).expect("test-shape aggregation");
    let sources: Vec<&str> = f
        .convention
        .common_imports
        .iter()
        .map(|i| i.source.as_str())
        .collect();
    assert!(sources.contains(&"vitest"));
    assert!(!sources.contains(&"lib"));
}

#[test]
fn aggregation_3_of_4_passes_default_majority() {
    // 4 siblings; 3 import zod. Default import_majority=0.66 →
    // floor=ceil(0.66*3) = 2 (after the subject is filtered out
    // there are 4 siblings; floor=ceil(0.66*4)=3). At 3-of-4 the
    // import survives.
    let (bodies_map, siblings) = bodies(&[
        (
            "dlg/award.tsx",
            "import { z } from 'zod';\nexport const A=1;\n",
        ),
        (
            "dlg/goal.tsx",
            "import { z } from 'zod';\nexport const B=2;\n",
        ),
        (
            "dlg/job.tsx",
            "import { z } from 'zod';\nexport const C=3;\n",
        ),
        ("dlg/empty.tsx", "export const D=4;\n"),
    ]);
    let cfg = cfg();
    let input = StructureInput {
        path: Path::new("dlg/new.tsx"),
        siblings: &siblings,
        bodies: &bodies_map,
        subject_body: None,
        mode: StructureMode::PreEditNew,
        cfg: &cfg,
    };
    let f = compute_structure_finding(&input).expect("3-of-4 aggregation");
    assert!(f
        .convention
        .common_imports
        .iter()
        .any(|i| i.source == "zod"));
}

#[test]
fn aggregation_2_of_3_below_majority_drops_import() {
    // 3 siblings; 2 import 'rare'. floor=ceil(0.66*3)=2 — so it
    // *just* survives. Knock it down to 1-of-3 to ensure dropping.
    let (bodies_map, siblings) = bodies(&[
        ("dlg/a.tsx", "import { z } from 'zod';\nimport { x } from 'rare';\nexport function CreateADialog(){}\n"),
        ("dlg/b.tsx", "import { z } from 'zod';\nexport function CreateBDialog(){}\n"),
        ("dlg/c.tsx", "import { z } from 'zod';\nexport function CreateCDialog(){}\n"),
    ]);
    let cfg = cfg();
    let input = StructureInput {
        path: Path::new("dlg/new.tsx"),
        siblings: &siblings,
        bodies: &bodies_map,
        subject_body: None,
        mode: StructureMode::PreEditNew,
        cfg: &cfg,
    };
    let f = compute_structure_finding(&input).expect("3 zod siblings");
    let sources: Vec<&str> = f
        .convention
        .common_imports
        .iter()
        .map(|i| i.source.as_str())
        .collect();
    assert!(sources.contains(&"zod"));
    assert!(
        !sources.contains(&"rare"),
        "1-of-3 must not survive 0.66 majority; got: {sources:?}"
    );
}

#[test]
fn divergence_fires_when_template_missing() {
    let (bodies_map, siblings) = bodies(&[
        ("dlg/award.tsx", "export function CreateAwardDialog(){}\n"),
        ("dlg/goal.tsx", "export function CreateGoalDialog(){}\n"),
        ("dlg/job.tsx", "export function CreateJobDialog(){}\n"),
    ]);
    let subject_body = "export const x = 1;\n";
    let cfg = cfg();
    let input = StructureInput {
        path: Path::new("dlg/divergent.tsx"),
        siblings: &siblings,
        bodies: &bodies_map,
        subject_body: Some(subject_body),
        mode: StructureMode::Review,
        cfg: &cfg,
    };
    let f = compute_structure_finding(&input).expect("divergence");
    let StructureFindingKind::ReviewDivergent {
        missing_templates, ..
    } = &f.kind
    else {
        panic!("expected divergence; got {:?}", f.kind);
    };
    assert!(missing_templates.contains(&"Create*Dialog".to_owned()));
}
