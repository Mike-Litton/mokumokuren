//! Coupling-stage perf budget. The v0.2.0 plan calls for the
//! coupling pass to stay under ~50ms on a 2k-commit synthetic
//! fixture; this is the guard.

use ahash::AHashSet;
use criterion::{criterion_group, criterion_main, Criterion};
use mmk_core::coupling::top_couples_for;
use mmk_core::types::{Commit, CommitInfo, FileDelta};
use std::path::PathBuf;

/// Synthetic fixture sized to mirror git.git's filtered-commit shape:
/// ~2000 commits, ~8 files per commit, drawn from a pool of 500
/// distinct files. The 50-target subset matches the default top-N.
fn make_commits(n_commits: usize, files_per_commit: usize, n_files: usize) -> Vec<Commit> {
    (0..n_commits)
        .map(|i| {
            let deltas = (0..files_per_commit)
                .map(|j| FileDelta {
                    path: PathBuf::from(format!("file_{:05}.rs", (i * 7 + j * 13) % n_files)),
                    added: 10,
                    deleted: 4,
                })
                .collect();
            Commit {
                info: CommitInfo {
                    sha: format!("{i:040x}"),
                    parent_sha: None,
                    timestamp: 1_700_000_000 + i as i64 * 3600,
                    author_email: "bench@example.com".into(),
                },
                deltas,
            }
        })
        .collect()
}

fn bench_top_couples_for(c: &mut Criterion) {
    let commits = make_commits(2000, 8, 500);
    let targets: AHashSet<PathBuf> = (0..50)
        .map(|i| PathBuf::from(format!("file_{:05}.rs", i)))
        .collect();
    c.bench_function("top_couples_for_2k_50targets", |b| {
        b.iter(|| top_couples_for(&commits, &targets, 5));
    });
}

criterion_group!(benches, bench_top_couples_for);
criterion_main!(benches);
