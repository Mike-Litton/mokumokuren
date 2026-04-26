// Optional heap profiler. Active only with `--features dhat-heap`;
// see `docs/performance.md` § Measuring.
#[cfg(feature = "dhat-heap")]
#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

fn main() -> std::process::ExitCode {
    #[cfg(feature = "dhat-heap")]
    let _profiler = dhat::Profiler::new_heap();

    mokumokuren::run()
}
