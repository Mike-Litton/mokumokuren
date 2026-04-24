use criterion::{criterion_group, criterion_main, Criterion};

fn bench_noop(c: &mut Criterion) {
    c.bench_function("noop", |b| b.iter(mmk_core::noop));
}

criterion_group!(benches, bench_noop);
criterion_main!(benches);
