use ahash::AHashMap;
use mmk_core::hotspot::{rank, RankInputs};
use std::path::PathBuf;

fn path(s: &str) -> PathBuf {
    PathBuf::from(s)
}

#[test]
fn empty_inputs_yield_empty_output() {
    let empty_w = AHashMap::new();
    let empty_r = AHashMap::new();
    let empty_l = AHashMap::new();
    let empty_c = AHashMap::new();
    let empty_lm = AHashMap::new();
    let out = rank(
        RankInputs {
            weighted: &empty_w,
            relative: &empty_r,
            loc: &empty_l,
            commits_touching: &empty_c,
            last_modified: &empty_lm,
        },
        10,
    );
    assert!(out.is_empty());
}

#[test]
fn ranking_orders_by_score_desc() {
    let mut weighted: AHashMap<PathBuf, f64> = AHashMap::new();
    weighted.insert(path("low.rs"), 5.0);
    weighted.insert(path("high.rs"), 500.0);
    weighted.insert(path("mid.rs"), 50.0);

    let mut loc: AHashMap<PathBuf, u32> = AHashMap::new();
    loc.insert(path("low.rs"), 100);
    loc.insert(path("high.rs"), 100);
    loc.insert(path("mid.rs"), 100);

    let relative = AHashMap::new();
    let cts = AHashMap::new();
    let lm = AHashMap::new();

    let out = rank(
        RankInputs {
            weighted: &weighted,
            relative: &relative,
            loc: &loc,
            commits_touching: &cts,
            last_modified: &lm,
        },
        0,
    );
    assert_eq!(out.len(), 3);
    assert_eq!(out[0].path, path("high.rs"));
    assert_eq!(out[0].hotspot_rank, 1);
    assert_eq!(out[1].path, path("mid.rs"));
    assert_eq!(out[1].hotspot_rank, 2);
    assert_eq!(out[2].path, path("low.rs"));
    assert_eq!(out[2].hotspot_rank, 3);
}

#[test]
fn files_missing_from_head_loc_are_excluded() {
    let mut weighted: AHashMap<PathBuf, f64> = AHashMap::new();
    weighted.insert(path("alive.rs"), 100.0);
    weighted.insert(path("deleted.rs"), 999.0);

    let mut loc: AHashMap<PathBuf, u32> = AHashMap::new();
    loc.insert(path("alive.rs"), 50);

    let relative = AHashMap::new();
    let cts = AHashMap::new();
    let lm = AHashMap::new();
    let out = rank(
        RankInputs {
            weighted: &weighted,
            relative: &relative,
            loc: &loc,
            commits_touching: &cts,
            last_modified: &lm,
        },
        10,
    );
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].path, path("alive.rs"));
}

#[test]
fn top_n_truncates_after_sort() {
    let mut weighted: AHashMap<PathBuf, f64> = AHashMap::new();
    let mut loc: AHashMap<PathBuf, u32> = AHashMap::new();
    for i in 0..10 {
        let p = path(&format!("f{i}.rs"));
        weighted.insert(p.clone(), f64::from(i + 1) * 10.0);
        loc.insert(p, 100);
    }
    let relative = AHashMap::new();
    let cts = AHashMap::new();
    let lm = AHashMap::new();
    let out = rank(
        RankInputs {
            weighted: &weighted,
            relative: &relative,
            loc: &loc,
            commits_touching: &cts,
            last_modified: &lm,
        },
        3,
    );
    assert_eq!(out.len(), 3);
    assert_eq!(out[0].path, path("f9.rs"));
    assert_eq!(out[2].path, path("f7.rs"));
}

#[test]
fn score_formula_matches_spec() {
    let mut weighted: AHashMap<PathBuf, f64> = AHashMap::new();
    weighted.insert(path("a.rs"), 99.0);
    let mut loc: AHashMap<PathBuf, u32> = AHashMap::new();
    loc.insert(path("a.rs"), 9);

    let relative = AHashMap::new();
    let cts = AHashMap::new();
    let lm = AHashMap::new();
    let out = rank(
        RankInputs {
            weighted: &weighted,
            relative: &relative,
            loc: &loc,
            commits_touching: &cts,
            last_modified: &lm,
        },
        1,
    );
    let expected = 100.0_f64.ln() * 10.0_f64.ln();
    assert!((out[0].hotspot_score - expected).abs() < 1e-9);
}
