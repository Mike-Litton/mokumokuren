# Performance

`mmk` is called by humans and by LLM agents mid-task. Latency between
the tool call and the ranking is user-visible. Read this file before
optimising anything; it's the starting point for the next pass.

## Where the time goes — cache miss

When all three caches are cold (first call on a repo, or after
`mmk cache clear`), gix-LCS dominates. Median of 3 vscode runs
(14 654 commits, the slowest fixture):

```
[mmk] open+head:           ~3.3 ms     0.07 %
[mmk] revwalk:             ~209 ms     4.3  %
[mmk] head path enum:      ~13  ms     0.27 %
[mmk] per-commit diff:     ~4708 ms    97   %  ← dominates
[mmk] cache save (×3):     ~6  ms      0.12 %
[mmk] loc (touched only):  ~30  ms     0.6  %
[mmk] aggregation total:   ~30  ms     0.6  %
```

Two facts to internalise about the cold path:

1. **gix LCS line-counting is ~95 % of wall time on every fixture.**
   `change.diff(resource_cache).line_counts()` inside the parallel
   per-commit loop. Anything else is bounded by the remaining ~5 %.
2. **Post-diff aggregation is 0.6 %.** Path interning, hasher swaps,
   pre-sized maps, fusing the three sequential passes — caps at
   ~30 ms savings on a 5 s vscode run. Hygiene, not perf.

## Where the time goes — cache hit (round 2, three caches)

Calls 2-N for the same repo skip the parallel diff entirely *and*
skip revwalk + HEAD-tree enumeration. The work collapses to: cache
loads + LOC of touched blobs + aggregation. Every call to
`analyze()` shares this fast path — `mmk analyze`, `mmk review`,
`mmk pre-edit`, and the per-snapshot calls inside `mmk drift`.

| repo    | commits | cold (s) | warm (s) | speedup |
| ------- | ------: | -------: | -------: | ------: |
| immich  |   1 425 |     0.25 |     0.01 |    25×  |
| cal.diy |   1 887 |     0.78 |     0.01 |    78×  |
| n8n     |   3 123 |     1.69 |     0.04 |    42×  |
| vscode  |  14 654 |     4.81 |     0.08 |    60×  |

Hook latencies on vscode (the cost of each PreToolUse/PostToolUse
fire), warm:

| command         | warm (s) |
| --------------- | -------: |
| `mmk analyze`   |    0.08  |
| `mmk pre-edit`  |    0.07  |
| `mmk review`    |    0.10  |

A PreToolUse + PostToolUse pair on the largest fixture is now
~170 ms of mmk overhead, down from ~600 ms in round 1.

JSON output is byte-identical to the cold-path baseline; the caches
only change timing.

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
  `duration_ms` and the version strings is a semantic change, not a
  perf win.** This is how the multiset-diff drift below was caught.

## Build configuration

`Cargo.toml`:

```toml
[profile.release]
lto = "thin"                  # 5-10 % over codegen-only
codegen-units = 1             # whole-crate optimisation
debug = "line-tables-only"    # required for samply (~4 MB binary cost)
panic = "abort"               # mmk has no catch_unwind anywhere

[profile.bench]
inherits = "release"

[profile.dist]
inherits = "release"
lto = "fat"                   # shipped binary only — adds 30-60 s
                              # to dist build, unlocks cross-crate
                              # inlining beyond `thin` reach
strip = "symbols"
```

Round 2 added `panic = "abort"` to `[profile.release]` (audit
confirms no `catch_unwind`/`std::panic` anywhere) and bumped
`[profile.dist]` from `lto = "thin"` to `lto = "fat"`. Dev iteration
keeps `thin` LTO for the local `cargo build --release` loop; only
the cross-compiled release artefacts pay the fat-LTO build cost.

Deliberately *not* set:
- `target-cpu=native` — non-portable, useful only for local benching.
  Whether to add a `target-cpu=x86-64-v3` matrix to `[profile.dist]`
  is a Compiler-Explorer / measurement question, not a default flip.
- `mimalloc` / `jemalloc` — DHAT shows the hot allocator is gix's
  blob cache; swap won't help until gix's path is bypassed.

## Persistent caches (kept)

Three caches share `<cache-root>/<repo-id>/` (`MMK_CACHE_DIR`
overrides; `<repo-id>` = SHA-256 of the canonical `.git` path):

| file                       | shape                                  | invariant |
| -------------------------- | -------------------------------------- | --------- |
| `cache.bincode.v<N>`       | per-commit `(added, deleted)` deltas, keyed by SHA | a commit's deltas are immutable once the commit exists |
| `revwalk.bincode.v<N>`     | revwalk results, keyed by `(anchor_sha, since_ts)`  | the set of commits reachable from a fixed anchor with committer-time ≥ a fixed cutoff is immutable |
| `head_tree.bincode.v<N>`   | tree-walk entries + `head_paths_ignored`, keyed by `(commit_sha, ignores_hash)` | a tree's blob list is immutable; ignore globs filter deterministically |

The revwalk and head-tree caches are bounded by a least-recently-
*inserted* eviction policy (default cap: 32 entries each). LRU on
hit (instead of LRU on insert) was rejected because touching on hit
forces a save on every warm call, which defeats the "skip rewrites
when nothing changed" optimisation.

`mmk cache info` now reports all three caches. `mmk cache clear`
takes `--scope all|deltas|revwalk|loc` (default `all`).

What this changes about the perf shape:
- The "where the time goes" picture above describes the *cold* path.
  Warm calls collapse to ~80 ms total on vscode (~30 ms of which is
  `loc (touched only)` blob inflation; that phase is not yet cached).
- Cache miss happens only when a key isn't yet known to the cache.
  For revwalk/head-tree, a new HEAD or a different `--since` produces
  a new key — old entries stay valid, just unused, until the LRU
  cap evicts them.
- Cache invalidation is implicit. Ignore-glob changes flow through
  the head-tree key automatically (via `ignores_hash`); they don't
  invalidate the per-commit delta cache (filtering is applied at
  aggregation time, not stored in the cache).

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

## Deferred — round 3 candidates

**`imara-diff` direct, inside gix's filtered-blob pipeline.** The
warm path is now bounded by `loc (touched only)` (~28 ms vscode)
plus aggregation (~30 ms vscode). The cold-path bottleneck — gix
LCS — only fires on first-call-per-repo. Worth attacking only if
cold latency becomes a felt concern (e.g. CI, or a repo with a much
higher commit-churn rate than the current fixtures).

**Cache sharding by hash prefix** (Astral uv pattern). Premature
until 100k-commit-class repos appear in our fixture set. Today's
single-file caches deserialise in <5 ms even for vscode.

**LOC-at-HEAD cache for the touched-blob set.** The remaining ~28 ms
warm vscode is per-blob inflation in `loc::count_loc`. Caching this
is keyed by `(commit_sha, touched_paths_hash)` — but `touched_paths`
is the dynamic output of cache materialisation, so the key is
expensive to compute. Likely better to cache by individual
`(commit_sha, blob_oid) → loc` pairs, i.e. a content-addressed LOC
table.

**`bytecount` for `count_lines`** — burntsushi himself notes ~10–
20 ms cold against a 5 s LCS dominator; noise-tier. Reconsider once
LCS itself is no longer dominant.

**Salsa / turbo-tasks query engine.** matklad's *Against query-based
compilers* (2026-02) argues this exact case: linear non-reactive
pipelines don't benefit from a query engine. Hand-rolled per-phase
memoisation (the round-2 shape) is the right granularity.

**PGO / BOLT.** Not Cargo-native; complex pipeline; modest gain.
Reconsider only if everything above lands and the warm floor still
needs to come down.

## Discipline

1. Measure before you optimise. Re-baseline `MMK_TRACE` against the
   four fixtures.
2. Diff fixture JSONs after every change. Clean diff (modulo
   `analysis.duration_ms` and the version strings) is the
   "perf, not semantic" gate.
3. State cold vs warm, run-1 vs median-of-N. Don't conflate them.
4. Keep "tried and reverted" entries here. Negative results outlive
   the experiment that produced them.
