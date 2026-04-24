use criterion::{criterion_group, criterion_main, Criterion};
use mmk_core::churn::weighted_churn;
use mmk_core::types::{Commit, CommitInfo, FileDelta};
use std::path::PathBuf;

fn make_commits(n: usize) -> Vec<Commit> {
    (0..n)
        .map(|i| Commit {
            info: CommitInfo {
                sha: format!("{i:040x}"),
                parent_sha: None,
                timestamp: 1_700_000_000 + i as i64 * 3600,
                author_email: "bench@example.com".into(),
            },
            deltas: vec![FileDelta {
                path: PathBuf::from(format!("file_{}.rs", i % 32)),
                added: 10,
                deleted: 4,
            }],
        })
        .collect()
}

fn bench_weighted_churn(c: &mut Criterion) {
    let commits = make_commits(1024);
    let now = 1_700_000_000 + 1024 * 3600;
    let tau = 90.0 * 86_400.0;
    c.bench_function("weighted_churn_1024", |b| {
        b.iter(|| weighted_churn(&commits, now, tau));
    });
}

criterion_group!(benches, bench_weighted_churn);
criterion_main!(benches);
