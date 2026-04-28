# Performance

`mmk` is called by humans and by LLM agents mid-task. Latency between
the tool call and the ranking is user-visible. Read this file before
optimising anything; it's the starting point for the next pass.

## Fixture conventions

Numbers below are measured against five reference repositories, kept
out of the repo by name to avoid implying endorsement. They span a
useful range of scales:

| label | window commits | notes |
| ----- | -------------: | ----- |
| A     |        ~5 k    | small TS/JS app |
| B     |       ~10 k    | small-medium TS app |
| C     |       ~16 k    | medium-large TS monorepo |
| D     |       ~19 k    | large TS monorepo |
| E     |      ~156 k    | very large monorepo |

Cold = first call with all caches cleared. Warm = median-of-N with
caches populated. Round 3 re-baselined the fixture set; pre-round-3
tables below referenced an older A-D set sized at ~1.4-15 k commits,
left in place for historical lineage.

## Where the time goes — cache miss

When all three caches are cold, gix-LCS dominates. Median of 3 cold
runs on fixture D (the slowest, 14 654 in-window commits):

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
   ~30 ms savings on a 5 s D run. Hygiene, not perf.

## Where the time goes — cache hit (round 2, three caches)

Calls 2-N for the same repo skip the parallel diff entirely *and*
skip revwalk + HEAD-tree enumeration. The work collapses to: cache
loads + LOC of touched blobs + aggregation. Every call to
`analyze()` shares this fast path — `mmk analyze`, `mmk review`,
`mmk pre-edit`, and the per-snapshot calls inside `mmk drift`.

| fixture | cold (s) | warm (s) | speedup |
| ------- | -------: | -------: | ------: |
| A       |    0.25  |    0.01  |   25×  |
| B       |    0.78  |    0.01  |   78×  |
| C       |    1.69  |    0.04  |   42×  |
| D       |    4.81  |    0.08  |   60×  |

Hook latencies on D (the cost of each PreToolUse/PostToolUse fire),
warm:

| command         | warm (s) |
| --------------- | -------: |
| `mmk analyze`   |    0.08  |
| `mmk pre-edit`  |    0.07  |
| `mmk review`    |    0.10  |

A PreToolUse + PostToolUse pair on D is now ~170 ms of mmk overhead,
down from ~600 ms in round 1.

After round 3 (per-blob LOC cache), warm wall on the round-3 fixture
set:

| fixture | analyze | review | pre-edit | session-summary | drift (5) |
| ------- | ------: | -----: | -------: | --------------: | --------: |
| A       |  0.00s  |  0.01s |   0.01s  |          0.00s  |    0.01s  |
| B       |  0.01s  |  0.02s |   0.02s  |          0.02s  |    0.12s  |
| C       |  0.01s  |  0.06s |   0.06s  |          0.04s  |    0.15s  |
| D       |  0.03s  |  0.11s |   0.11s  |          0.07s  |    0.16s  |
| E       |  0.05s  |  0.12s |   0.12s  |          0.20s  |    0.26s  |

Stage 2 (per-blob LOC cache) accounts for most of the round-3 warm
saving: the `loc (touched only)` phase on E_156k drops from 28 ms to
4 ms on every cache hit, and drift's 5 snapshots inherit the saving
multiplied through.

JSON output is byte-identical to the cold-path baseline; the caches
only change timing.

## `mmk drift` — boundary-walk short-circuit

Drift takes K snapshots, each calling `analyze_at(anchor_oid)`. Even
warm, drift was paying ~1.1 s on D regardless of K because
`walker::find_session_boundaries` ran a full HEAD revwalk with no
committer-time cutoff — ~150 k commits walked just to grab the K most
recent merges. The fix breaks the loop as soon as `merges.len() >= k`
(the linear-chunk fallback only fires when fewer than K merges exist,
in which case `all` is still complete because we never break early).

| fixture | drift cold | drift warm |
| ------- | ---------: | ---------: |
| A       |      1.04s |      0.13s |
| B       |      1.12s |      0.17s |
| C       |      3.07s |      0.21s |
| D       |      6.52s |      0.37s |

D warm drift dropped 4×. Per-snapshot `analyze_at` cost is now
~50 ms warm; ~28 ms of that is `loc (touched only)` (the same phase
that dominates `mmk analyze` warm — see "Round-3 candidates" below).

## `mmk session-summary` — ancestor-walk bound (v0.4)

Same pattern as drift: `analyze_session` called `walk_ancestors(base)`
with `rev_walk(start).all()` and no committer-time cutoff. On a
long-history repo the walk traversed every ancestor of `base`
(~150 k commits) just to build a filter set that's only checked
against window commits (~14 k, bounded by `since_ts`). Ancestors
older than `since_ts` cannot match a window commit's sha — they're
unreachable filter input. Fix: pass `since_ts` into `walk_ancestors`
and use `Sorting::ByCommitTimeCutoff`, mirroring `walk_commits_from`.

Warm `mmk session-summary` (median of 3, cache cleared per command):

| fixture | before | after | delta |
| ------- | -----: | ----: | ----: |
| A       |  0.07s | 0.03s |  -57% |
| B       |  0.10s | 0.05s |  -50% |
| C       |  0.15s | 0.09s |  -40% |
| D       |  0.74s | 0.24s |  **-68%** |

`session-summary --drift-sessions 5` benefits proportionally because
it calls `analyze_session` per snapshot. JSON output unchanged
(byte-identical pre/post).

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
- The local fixture cache holds deterministic JSON outputs
  (`duration_ms` stripped) for fixtures A-D at pinned HEADs. **Any
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
- `target-cpu=x86-64-v3` — inspected on the four kernels the round-2
  plan flagged (`is_binary`, `bytecount_nl`, `bstr_to_pathbuf`, the
  coupling inner loop). Findings:
  - `is_binary` calls `core::slice::memchr::memchr_aligned` OOL; std
    runtime-detects AVX2 already.
  - `bytecount_nl` autovectorizes to wide SIMD on baseline (NEON/SSE2,
    16-byte). AVX2 (256-bit) would 1.5-2× the inner loop, but total
    cost is ~30 ms warm on D for *all* `count_lines` calls combined.
  - `bstr_to_pathbuf` bottleneck is allocation + std's scalar
    `from_utf8`. AVX2 doesn't unlock anything; the real SIMD win
    requires the `simdutf8` crate (runtime-dispatched, no profile
    change needed).
  - Coupling inner loop is hashmap-bound (CLMUL is baseline x86-64
    since Westmere/2010) + `PathBuf` allocation. Branch- and
    pointer-chasing-bound, not autovectorizable.
  Estimated wall-time win on shipped binaries: ~10-15 ms warm on D
  best case, traded for dropping pre-Haswell (2013) CPU support. Not
  worth the portability cost. Don't redo without new evidence.
- `mimalloc` / `jemalloc` — DHAT shows the hot allocator is gix's
  blob cache; swap won't help until gix's path is bypassed.

## Persistent caches (kept)

Four caches share `<cache-root>/<repo-id>/` (`MMK_CACHE_DIR`
overrides; `<repo-id>` = SHA-256 of the canonical `.git` path):

| file                       | shape                                  | invariant |
| -------------------------- | -------------------------------------- | --------- |
| `cache.bincode.v<N>`       | per-commit `(added, deleted)` deltas, keyed by SHA | a commit's deltas are immutable once the commit exists |
| `revwalk.bincode.v<N>`     | revwalk results, keyed by `(anchor_sha, since_ts)`  | the set of commits reachable from a fixed anchor with committer-time ≥ a fixed cutoff is immutable |
| `head_tree.bincode.v<N>`   | tree-walk entries + `head_paths_ignored`, keyed by `(commit_sha, ignores_hash)` | a tree's blob list is immutable; ignore globs filter deterministically |
| `loc.bincode.v<N>`         | per-blob `(line_count or binary)`, keyed by 20-byte blob OID | a blob's line count is a property of its bytes; `find_blob` returns raw inflated bytes (no CRLF/text-attribute filter), so same OID = same count |

The revwalk and head-tree caches are bounded by a least-recently-
*inserted* eviction policy (default cap: 32 entries each); the LOC
cache uses the same eviction with a 16k-entry cap (sized to cover the
full HEAD blob set of the largest fixture in the perf baseline). LRU
on hit (instead of LRU on insert) was rejected because touching on
hit forces a save on every warm call, which defeats the "skip
rewrites when nothing changed" optimisation.

`mmk cache info` reports all four caches. `mmk cache clear` takes
`--scope all|deltas|revwalk|head-tree|loc` (default `all`). The
`loc` scope was previously aliased to head-tree; round 3 split them
because the per-blob LOC cache is the actual line-count cache.

What this changes about the perf shape:
- The "where the time goes" picture above describes the *cold* path.
  Warm calls collapse to ~80 ms total on D (~30 ms of which is
  `loc (touched only)` blob inflation; that phase is not yet cached).
- Cache miss happens only when a key isn't yet known to the cache.
  For revwalk/head-tree, a new HEAD or a different `--since` produces
  a new key — old entries stay valid, just unused, until the LRU
  cap evicts them.
- Cache invalidation is implicit. Ignore-glob changes flow through
  the head-tree key automatically (via `ignores_hash`); they don't
  invalidate the per-commit delta cache (filtering is applied at
  aggregation time, not stored in the cache).

## Round 3 outcomes

Re-baselined against five fixtures sized by in-window commit count:
A (~5k), B (~10k), C (~16k), D (~19k), E (~156k). Numbers below are
warm-2 wall on E unless noted; cold path on E remains gix-LCS-bound
and was not the round-3 target.

**Per-blob LOC cache (Stage 2). Landed.** Sibling cache to the
existing three, keyed by 20-byte blob OID, capped at 16k entries with
LRI eviction. Hot path on calls 2-N: `count_loc` is now hash-lookup
instead of zlib-inflate per touched blob.

| phase                         | baseline (E warm) | after | saving |
| ----------------------------- | ----------------: | ----: | -----: |
| `loc (touched only)` analyze  |          28.4 ms  | 4.5 ms |  84% |
| `loc (touched only)` review   |          28.8 ms  | 4.6 ms |  84% |
| `mmk drift --sessions 5` wall |           390 ms  | 260 ms |  33% |
| `mmk session-summary` wall    |           230 ms  | 200 ms |  13% |

JSON output byte-identical (modulo `duration_ms`) on all five
fixtures × five commands.

## Tried and reverted — don't redo without new evidence

**Multiset-line-hash diff replacing gix LCS** *(2026-04)*. Algorithm
was ~2× faster; reverted because reorders register as `(0, 0)` and
the raw-byte path bypasses gix's CRLF/text-attribute filter, causing
ranking drift across all four fixtures (top-1 changes, top-3 swaps,
1-5 % shifts in the ranked output). Wall win was 3-7 % warm, not
worth the semantic cost. If a future pass wants the algorithmic win,
use `imara-diff` directly **inside** gix's filtered-blob pipeline.

**Per-thread `BlobCache`** *(2026-04)*. `AHashMap<ObjectId,
Arc<Vec<u8>>>` keyed by blob id. Reverted because rayon scatters a
file's history across workers; hit rate was effectively zero. Future
attempts must put the cache *outside* the parallel section
(content-addressed, shared via `DashMap` / `arc-swap`).

**bstr→PathBuf zero-copy.** Currently ~5 ms / 0.1 % wall on D.
Worth doing for cleanliness; won't move the needle.

**Path interning across aggregation maps.** Targets the 0.6 % slice;
~30 ms ceiling. Hygiene, not perf.

**Larger gix per-thread caches.** Already 64 MB object + 256 MB pack
per worker; the bottleneck is the LCS algorithm, not blob fetch
latency.

**`bytecount` for newline counting** *(2026-04, round 3)*. Drop-in
replacement for `bytes.iter().filter(==b'\n').count()` in
`mmk-git/src/binary.rs::count_lines`. Measured warm `loc (touched
only)` on E_156k: 28.4 → 26.2 ms (-2.2 ms, ~8% — below the round-3
≥3 ms / ≥30 % verification floor). Reverted. Worth retrying only if
a future change makes `count_lines` itself the dominant cost.

**Path interning in `coupling::collect_couples_for`** *(2026-04, round
3)*. Replaced `(PathBuf, PathBuf)` keys with `(u32, u32)` interned
ids. Measured warm `coupling::top_couples_for` on E_156k: 10.1 →
7.5 ms (-2.6 ms, 26% — just under the 30% floor; ≥3 ms also missed).
Reverted. The interior added ~10 LOC and one `mut intern` closure for
no statistically meaningful win on the reference set; the perf shape
was hashmap-bound *behind* the path-clone cost rather than dominated
by it.

**`walk_commits_from` skip-time-decode** *(2026-04, round 3)*.
Replaced `commit.time()?.seconds` with `info.commit_time.unwrap_or(0)`
on the assumption that the sort populates `info.commit_time` and
`commit.time()` would re-decode. Cold revwalk on E_156k regressed
~12% (326 → 365 ms — `info.commit_time` is populated eagerly by
`Sorting::ByCommitTimeCutoff` while `commit.time()` is lazy and may
share cache state with later `commit.author()`). Reverted.

**`find_session_boundaries` `.sorting()` pin** *(2026-04, round 3)*.
The early `break` after observing K merges relies on the walk
surfacing commits in newest-first order; gix's default is BFS, which
isn't *guaranteed* to be newest-first on multi-parent histories.
Added an explicit `Sorting::ByCommitTimeCutoff { NewestFirst, 0 }`
plus a regression test. Reverted: on the five reference fixtures BFS
happens to produce newest-first, drift JSON was byte-identical
pre/post, and the synthesized adversarial topology I built didn't
trigger the misordering either. The change pinned gix internals for
no observed bug. Reintroduce only with a topology that actually
demonstrates the misordering. (The `info.parent_ids.len()` part of
the same commit — replacing `info.object()` + `commit.parent_ids().
count()` — was kept; it's a hygiene win, not a bugfix.)

**`Cow<'_, [FileDelta]>` at cache materialize (Stage 3a candidate).**
Predicted ~2 ms saving on E_156k warm; threading `Cow` through the
`Commit` type (in `mmk-core/types`) and every downstream consumer
(`weighted_churn`, `coupling`, etc.) is a real structural change.
Skipped per the round-3 default-to-drop stance for sub-floor wins.

**Fused churn aggregation (Stage 3b candidate).** `weighted_churn`,
`commits_touching`, and `last_modified` each walk commits
independently — together ~9 ms warm on E_156k. Fusing three of them
(`relative_churn` must come after `weighted_churn`) would save ~6 ms
but collapses three small pure functions with discrete unit tests
into one fused function with a struct return. Skipped per the
round-3 default-to-drop stance — the maintainability cost outweighs
the win after Stage 2 already moved the warm floor by 24 ms.

**`mmk review` gix in-process diff (Stage 4 candidate).** Targeted
the ~30 ms `git diff --numstat` subprocess on every review. Has
three known parity corners (`gix::status` vs stitched
`worktree_to_index + index_to_tree`; rename detection passing
`Rewrites { copies: None, percentage: None }` to match
git-numstat-without-`-M`; binary handling via the resource cache
since gix returns `(0, 0)` not the `- -` sentinel). Deferred: the
implementation cost is high, parity risk is real (any non-empty
fixture diff blocks the merge), and Stage 2 already brought review
warm on E_156k from 150 ms to 120 ms. The trait abstraction
(`ChangedFileSource`) was also dropped — a single-impl trait adds no
value on its own; reintroduce together with the gix impl when the
parity work is funded.

## Deferred — round 4 candidates

After round 3, E_156k warm `mmk analyze` is ~50 ms (was 70 ms);
`mmk review` is ~120 ms (was 150 ms). The remaining headroom on the
warm path is small and dominated by process startup + the
`git diff` subprocess.

| phase                            | warm E | candidate                                       | est. saving |
| -------------------------------- | -----: | ----------------------------------------------- | ----------: |
| `mmk review` git-diff subprocess |  30 ms | replace with gix in-process diff (Stage 4)      |     ~25 ms  |
| process startup                  |  18 ms | long-lived daemon (see "streaming" below)       |       all   |
| `gix::open` per call             |   4 ms | reuse `gix::Repository` across calls            |       all   |
| aggregation total                |  10 ms | hygiene; capped by Amdahl                       |      ~3 ms  |

**`mmk review` subprocess swap.** Detailed analysis lives in the
"Tried and reverted" section above — the parity work was scoped but
deferred. Reintroduce together with the daemon (below); the same
`gix::Repository` reuse that the daemon needs makes the in-process
diff effectively free.

**`imara-diff` direct, inside gix's filtered-blob pipeline.** Cold-
path attack: gix LCS is still 95 % of cold wall, ~4.7 s on D
cold. Worth doing only if cold latency becomes a felt concern (CI,
much-higher-churn repos than the current fixtures).

**Cache sharding by hash prefix** (Astral uv pattern). Premature
until 100k-commit-class repos appear in our fixture set. Today's
single-file caches deserialise in <5 ms even for D.

**`bytecount` / `simdutf8` runtime-dispatched SIMD.** Drop-in
replacements for `bytes.iter().filter(==b'\n').count()` and
`String::from_utf8_lossy` respectively. Both runtime-detect features.
Estimated ~5-10 ms warm on D combined; below the LOC cache and
coupling interning by an order of magnitude.

**Salsa / turbo-tasks query engine.** matklad's *Against query-based
compilers* (2026-02) argues this exact case: linear non-reactive
pipelines don't benefit from a query engine. Hand-rolled per-phase
memoisation (the round-2 shape) is the right granularity.

**PGO / BOLT.** Not Cargo-native; complex pipeline; modest gain.
Reconsider only if everything above lands and the warm floor still
needs to come down.

## Streaming on live edits — future work

The forward-looking ambition is sub-millisecond review fires on every
keystroke. Process startup is the floor: `mmk` spawns at ~18 ms even
on a fully-cached call, and `gix::open` adds 3-5 ms per invocation.
Below ~40 ms warm on E_156k is unreachable through any in-process
optimisation; only a long-lived process erases it.

The natural shape: `mmk-web` (currently empty) hosts a
sidecar daemon that holds:

- one `gix::Repository` per worktree (loaded once, reused across
  fires);
- the four persistent caches in memory (deltas, revwalk, head-tree,
  loc) — write-through to disk on the same triggers the current CLI
  uses;
- an IPC surface (Unix socket or named pipe) that accepts the same
  args shape as the CLI and returns the same JSON envelope.

What the round-3 work already prepared:
- The per-blob LOC cache means the only computational work per
  keystroke is the diff itself (~5 ms with gix in-process from
  Stage 4) plus a HEAD-tree-relative re-walk only when HEAD moves.
- The cache structures already use the `<cache-root>/<repo-id>/`
  layout, which a daemon can simply mmap or hold by reference.
- `gix::Repository` reuse is already safe: the analyze pipeline
  threads a `ThreadSafeRepository` and converts to thread-local
  copies inside parallel sections.

What was *not* scaffolded in round 3, deliberately:
- No IPC code, protocol, or socket plumbing.
- No `ChangedFileSource` trait — single-impl traits would have been
  dead weight; reintroduce together with the gix in-process diff
  when both land.
- No `mmk-web` content; the crate stays as a placeholder.

The Stage 4 `git diff` → gix work is a precondition: every fork+exec
in the warm path is a daemon-killer. Sequence Stage 4 before any
daemon work.

## Discipline

1. Measure before you optimise. Re-baseline `MMK_TRACE` against the
   four fixtures.
2. Diff fixture JSONs after every change. Clean diff (modulo
   `analysis.duration_ms` and the version strings) is the
   "perf, not semantic" gate.
3. State cold vs warm, run-1 vs median-of-N. Don't conflate them.
4. Keep "tried and reverted" entries here. Negative results outlive
   the experiment that produced them.
