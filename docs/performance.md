# Performance

`mmk` is called by humans and by LLM agents mid-task. Latency between
the tool call and the ranking is user-visible. Read this file before
optimising anything; it's the starting point for the next pass.

## Where the time goes — cache miss

When the per-commit cache is cold (first call on a repo, or after
`mmk cache clear`), gix-LCS dominates. Median of 3 vscode runs
(13 700 commits, the slowest fixture):

```
[mmk] open+head:           ~3.5 ms     0.07 %
[mmk] revwalk:             ~210  ms    4.3  %
[mmk] head path enum:      ~12   ms    0.25 %
[mmk] per-commit diff:     ~4900 ms    97.7 %  ← dominates
[mmk]   blob loads:        1696  / sum   30 ms
[mmk]   lcs diffs:        44841  / sum 7900 ms
[mmk]   bstr→PathBuf:     46760  / sum    5 ms
[mmk] loc (touched only):  ~35   ms    0.7  %
[mmk] aggregation total:   ~30   ms    0.6  %
```

Two facts to internalise about the cold path:

1. **gix LCS line-counting is ~95 % of wall time on every fixture.**
   `change.diff(resource_cache).line_counts()` inside the parallel
   per-commit loop. Anything else is bounded by the remaining ~5 %.
2. **Post-diff aggregation is 0.6 %.** Path interning, hasher swaps,
   pre-sized maps, fusing the three sequential passes — caps at
   ~30 ms savings on a 5 s vscode run. Hygiene, not perf.

## Where the time goes — cache hit

Calls 2-N for the same repo skip the parallel diff entirely. The work
collapses to: revwalk + cache load + aggregation.

| repo    | commits | cold (s) | warm (s) | speedup |
| ------- | ------: | -------: | -------: | ------: |
| immich  |   1 342 |     0.27 |     0.03 |   9.0×  |
| cal.diy |   1 749 |     0.85 |     0.05 |  17.0×  |
| n8n     |   2 613 |     1.90 |     0.10 |  19.0×  |
| vscode  |  13 700 |     5.88 |     0.29 |  20.2×  |

JSON output is byte-identical to the cold-path baseline; the cache
only changes timing.

## Measuring

| Tool         | Command                                                           |
| ------------ | ----------------------------------------------------------------- |
| Phase timing | `MMK_TRACE=1 mmk analyze ... > /dev/null`                         |
| Heap         | `cargo build --release --features dhat-heap` → writes `dhat-heap.json` |
| Sampling     | `samply record --save-only -o p.json.gz target/release/mmk ...`   |
| Fixtures     | `diff -ru --exclude=timing.txt /tmp/mmk-fixtures /tmp/mmk-fixtures-after` |

Notes:
- `MMK_TRACE` is zero-cost when unset.
- `dhat-heap` is a feature flag in `mmk-cli/Cargo.toml`; off by default
  the dep isn't even pulled in. Build runtime cost is significant when
  on — diagnostic only.
- samply on macOS needs the binary's symbol table; we deliberately
  don't `strip = "symbols"` in `[profile.release]` (only in
  `[profile.dist]`).
- `/tmp/mmk-fixtures` has deterministic JSON outputs (`duration_ms`
  stripped) for cal.diy / immich / n8n / vscode at pinned HEADs. **Any
  optimisation that produces a non-empty fixture diff outside
  `duration_ms` is a semantic change, not a perf win.** This is how
  the multiset-diff drift below was caught.

## Build configuration

`Cargo.toml`:

```toml
[profile.release]
lto = "thin"                  # 5-10% over codegen-only
codegen-units = 1             # whole-crate optimisation
debug = "line-tables-only"    # required for samply (~4 MB binary cost)

[profile.bench]
inherits = "release"

[profile.dist]
inherits = "release"
strip = "symbols"             # shipped artefacts only
```

Deliberately *not* set, none measured to pay off here yet:
- `lto = "fat"` — 5 % more codegen, 3-5× compile time. Move to dist
  if it ever lands real numbers.
- `panic = "abort"` — small win, breaks `catch_unwind`. Flip if
  benches show it.
- `target-cpu=native` — non-portable, useful only for local benching.
- `mimalloc` / `jemalloc` — DHAT shows the hot allocator is gix's
  blob cache; swap won't help until gix's path is bypassed.

## Persistent per-commit cache (kept)

Each commit's `(added, deleted)` deltas are immutable once the commit
exists. We persist them at
`~/Library/Caches/mmk/<repo-id>/cache.bincode.v<N>` (`MMK_CACHE_DIR`
overrides; `<repo-id>` = SHA-256 of the canonical `.git` path; `v<N>`
bumps on diff-implementation changes). On every `analyze`, we
partition `commit_infos` into cached / missing, run gix-LCS only on
the missing set, then merge. Atomic writes via tmp-rename; concurrent
invocations may race on save (last writer wins, lost entries
recomputed next time).

Code: `mmk-git/src/cache.rs`, wired in
`mmk-git/src/lib.rs::analyze`. Subcommands: `mmk cache info`,
`mmk cache clear`. The cache lives outside the working tree, no
`.gitignore` entry needed.

What this changes about the perf shape:
- The "where the time goes" picture above describes the *cold* path.
  Warm calls collapse to ~30 ms aggregation + cache deserialization.
- Cache miss happens only when a commit's SHA isn't yet known to the
  cache — first call on a repo, plus any new commits since.
- Cache invalidation is implicit: ignore-glob changes don't
  invalidate (filtering is applied at aggregation time, not stored
  in the cache).

## Tried and reverted — don't redo without new evidence

**Multiset-line-hash diff replacing gix LCS** *(2026-04)*. Algorithm
was ~2× faster; reverted because reorders register as `(0, 0)` and
the raw-byte path bypasses gix's CRLF/text-attribute filter, causing
ranking drift (cal.diy top-1 changed; n8n #2/#3 swapped; immich /
vscode shifted 1-5 %). Wall win was 3-7 % warm, not worth the
semantic cost. If a future pass wants the algorithmic win, use
`imara-diff` directly **inside** gix's filtered-blob pipeline.

**Per-thread `BlobCache`** *(2026-04)*. `AHashMap<ObjectId,
Arc<Vec<u8>>>` keyed by blob id. Reverted because rayon scatters a
file's history across workers; hit rate was effectively zero. Future
attempts must put the cache *outside* the parallel section
(content-addressed, shared via `DashMap` / `arc-swap`).

**bstr→PathBuf zero-copy.** Currently ~5 ms / 0.1 % wall on vscode.
Worth doing for cleanliness; won't move the needle.

**Path interning across aggregation maps.** Targets the 0.6 % slice;
~30 ms ceiling. Hygiene, not perf.

**Larger gix per-thread caches.** Already 64 MB object + 256 MB pack
per worker; the bottleneck is the LCS algorithm, not blob fetch
latency.

## Discipline

1. Measure before you optimise. Re-baseline `MMK_TRACE` against the
   four fixtures.
2. Diff fixture JSONs after every change. Clean diff is the
   "perf, not semantic" gate.
3. State cold vs warm, run-1 vs median-of-N. Don't conflate them.
4. Keep "tried and reverted" entries here. Negative results outlive
   the experiment that produced them.
