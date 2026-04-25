# What each number tells you

`mmk` deliberately emits more than one metric. The output is shaped
so that each number answers a question the others can't. Read this
doc when you want to know *which* number to look at for *which*
decision — and when each one starts lying to you.

The metrics group into four layers. Within a layer, fields
correlate; across layers, they decouple.

| Layer                  | Fields                                                                |
| ---------------------- | --------------------------------------------------------------------- |
| **Magnitude**          | `weighted_churn`, `commits_touching`, `relative_churn`, `hotspot_score` |
| **Time**               | `last_modified`                                                       |
| **Topology**           | `top_couples`, `blast_radius.nodes`                                   |
| **Distribution / drift** | `commit_entropy`, `churn_of_churn`, `entered_top_n`, `rank_climbs`  |

## Magnitude layer

Three views of "how much has this file moved." Pairwise correlated;
each surfaces a different mode of "a lot."

### `weighted_churn`

**Definition.** `sum(added_lines + deleted_lines)` across commits in
the window, with each commit's contribution multiplied by
`exp(-age / tau)`. `tau_days` (default 90) is the 1/e point.

**Answers.** *How much recent edit volume has this file absorbed?*

**Decouples from `commits_touching`.** A single 500-line bulk rewrite
and 50 ten-line commits both produce `weighted_churn = 500`; their
`commits_touching` differ by 50×. The contrast tells you whether the
load is *sustained activity* or *one rewrite event*.

**Watch for.** A single commit that survived the `bulk.max_files /
bulk.max_lines` filter but is still pathologically large can dominate
the window. If a file's `weighted_churn` is high and its
`commits_touching` is 1, suspect this and `git log -p` the one commit.

### `commits_touching`

**Definition.** Distinct commits in the window that modified this
file (after rename folding).

**Answers.** *How often is this file touched?* Frequency, not volume.

**Decouples from `weighted_churn`.** Many tiny commits (high
frequency, low volume per commit) and one huge commit (low frequency,
high volume) read very differently. Auto-formatting passes that touch
many files for trivial reasons inflate `commits_touching` without
meaning much; this is the metric most contaminated by mechanical
changes.

**Watch for.** Lint-fix bots, codemod sweeps, and license-header
updates that touch every file. The `bulk.max_files` filter catches
the worst of these but not all.

### `relative_churn`

**Definition.** `weighted_churn / loc`. (For `files[]` the LOC is
HEAD-LOC; for `session_files[]` it's session-base LOC. See `loc` and
the LOC epoch contract in [`schema.md`](schema.md).)

**Answers.** *How densely is this file being changed, relative to
its size?*

**Decouples from `weighted_churn`.** Ranking by raw `weighted_churn`
buries small files that get rewritten constantly under the giant
files where any change is statistically big. `relative_churn`
recovers them. A 50-line file with `relative_churn = 4.0` is being
rewritten roughly four times per window — a much louder signal than
a 5000-line file at the same value, where the same churn is barely a
percent of the file.

**Watch for.** Tiny files with `loc` near zero — division blows up.
The pipeline excludes `loc == 0`, but `loc == 1` can still produce
absurd ratios.

### `hotspot_score`

**Definition.** `log(1 + weighted_churn) × log(1 + loc)`. The
ranking key.

**Answers.** *Where should I look first?* Files that are both big and
churning rank highest. The double `log(1 + …)` is what keeps a
single mega-commit on a small file from dominating, and what keeps
huge but-quiet files from dominating either.

**Decouples from `weighted_churn` and `relative_churn`.** It doesn't,
mathematically — it's a function of `weighted_churn` and `loc`. It's
shipped as a separate field because it's the *ranking* used in
`hotspot_rank`, and exposing the score lets a consumer see the
distance between rank-1 and rank-2 (often dramatic, sometimes
narrow). Two files with `rank` 1 and 2 might be tightly tied or far
apart; only the score tells you which.

**Watch for.** Tied scores. The tie-breaker is alphabetic on path,
which is stable but arbitrary. Don't read meaning into rank-3 vs.
rank-4 if their scores are within rounding error.

## Time layer

### `last_modified`

**Definition.** RFC 3339 timestamp of the most recent in-window
commit that touched this file.

**Answers.** *When was this last touched?*

**Decouples from everything in the magnitude layer.** A file can be
hot but cold (high `weighted_churn` from edits months ago, dormant
since) or quiet but warm (rare touches, but one was yesterday). The
recency-weighted churn already factors age in — but not visibly.
`last_modified` is the visible recency signal.

**Watch for.** A repo with a recent bulk-rewrite event will have
many files showing the same `last_modified`. That doesn't tell you
they all genuinely changed semantically.

## Topology layer

### `top_couples` (per-file array)

**Definition.** For each ranked file, its top-K co-changing
partners by Jaccard similarity:
`co_change(A, B) / |commits touching A or B|`.

**Answers.** *If I touch this file, what historically changes
alongside it?*

**Decouples from magnitude entirely.** A small, quiet file can be
tightly coupled to a big hot one. The magnitude layer ranks files
by individual load; coupling tells you which loads are *linked*.

**Watch for.** "Popular files" that co-change with everything because
of merge commits or bulk refactors. The `bulk.max_files` filter
helps, but project metadata (CHANGELOG, READMEs) can still leak in
on busy repos.

### `blast_radius.nodes` (when `--blast-radius <PATH>` is set)

**Definition.** 1-hop co-change neighborhood of `PATH` filtered by
`threshold` (default 0.10 Jaccard). Same data as `top_couples`,
selected by user query rather than ranked output.

**Answers.** *I'm about to edit this specific file. What else
should I re-read?*

**Decouples from `top_couples`.** Same metric, different framing.
`top_couples` is "for the most loaded files, here's their
neighborhood"; `blast_radius` is "for *this* file regardless of
load, here's its neighborhood." Use `blast_radius` when you've
already chosen what to edit.

**Watch for.** The threshold materially changes results. The output
echoes the effective threshold so consumers can see what filter was
applied — an LLM agent with one threshold and a CI report with
another should not be compared without normalizing.

## Distribution / drift layer

These metrics are about *patterns* across commits, not magnitudes
per file. They move independently of everything above.

### `commit_entropy`

**Definition.** Shannon entropy of files-touched-per-commit in the
session, normalized to `[0, 1]` by `ln(n)` where `n` is the number
of session commits.

**Answers.** *How spread out is the session's work across commits?*

**Decouples from all magnitudes.** A session with 100 commits each
touching one file gets `commit_entropy ≈ 1.0`. A session with 99
trivial commits and one bulk commit gets `commit_entropy` near 0.
Both can have identical total `weighted_churn`.

**Watch for.** What it does *not* measure. It's entropy over
*per-commit file counts*, not over *which files were touched*. A
session that hits the same one file in every commit looks "uniform"
from this metric's perspective even though the work is concentrated
on a single target. If you want "how concentrated is the work by
file," look at the distribution of `weighted_churn` across
`session_files[]`, not at `commit_entropy`.

### `churn_of_churn`

**Definition.** Per file, in the session: `min(added, deleted) × 2 /
(added + deleted)`. 0 when changes are pure additions or pure
deletions; 1 when adds and deletes are perfectly balanced.

**Answers.** *Is this file being thrashed (written, then
unwritten)?*

**Decouples from `weighted_churn`.** A file with high
`weighted_churn` because the agent kept adding code reads as
"productive activity" — `churn_of_churn` is near 0. A file with the
same `weighted_churn` because the agent kept oscillating reads as
"thrash" — `churn_of_churn` near 1. Identical magnitude, opposite
meaning.

**Watch for.** A file that grew from 100 lines to 500 with constant
small revisions can have `churn_of_churn` near 1 even though the net
change is purely additive — because each individual edit replaced
old lines with new ones. Read this together with the *net* line
count change (HEAD-LOC vs. base-LOC), not in isolation.

### `entered_top_n`, `rank_climbs`

**Definition.** Files that are in the session's top-N ranking but
not in the window's; and rank deltas (window rank minus session
rank, when both are defined).

**Answers.** *What shifted in importance because of this session?*

**Decouples from absolute rankings.** A file at rank 50 in the
window and rank 5 in the session shows up in `rank_climbs` with
`delta = 45`. It might never make the top-N of either ranking on
its own — but the *change* is what flags it as session-driven.

**Watch for.** `top` setting matters. A small `--top` (e.g. 3)
makes nearly every session-edited file enter `entered_top_n`; a
large `--top` (e.g. 100) hides real shifts. The signal is calibrated
to whatever `--top` you ran.

## How to use them together

The framing changes by *who's reading the output* and *what they're
deciding*.

### Agent inner loop, before an edit

The decision is "which file should I edit, and what cascades?"

1. Look at the top of `files[]` — `hotspot_score` ranks them.
2. For the chosen target, read its `top_couples` — those are the
   files an edit will likely touch alongside.
3. For the agent's known target, read `--blast-radius` instead — it
   surfaces the neighborhood without needing the file to be in the
   ranked output.

Skip the distribution / drift layer at this stage. It's about
sessions in retrospect, not edit choice.

### CI/CD review of an in-flight session

The decision is "should a human look at this PR?"

1. Read `session.base_resolved_via` first. If it's `head_minus_one`
   or anything beginning with `merge_base_`, the comparison is
   automatic; if `explicit` or `since_commit`, the agent picked the
   base on purpose.
2. Read `session.commit_entropy`. Below ~0.3 means one commit did
   most of the work — worth looking at directly.
3. Read `session.churn_of_churn` per file. Anything close to 1.0
   that also appears in `entered_top_n` is a thrash candidate; flag
   for human review.
4. Read `entered_top_n` for files newly hot — these are the
   session's center of mass.

Skip raw `weighted_churn` at this stage. It's noise here; the
session-vs-window deltas are what matters.

### Triage of an existing repository

The decision is "where do I start cleaning up?"

1. Top-N by `hotspot_score`.
2. For each, cross-check `relative_churn`. A high
   `hotspot_score` with high `relative_churn` is rot; a high
   `hotspot_score` with low `relative_churn` is sheer size, which
   may not need refactoring at all.
3. Read `top_couples` for the worst offenders — that's the
   refactoring scope, not just the file.

`commit_entropy` and the session block don't apply here; they
require a `--base` reference.

## When the metrics start lying

A few failure modes worth naming:

- **Squashed PRs and rebases.** Squash workflows collapse a session
  of small commits into one big one. `commits_touching` drops,
  `commit_entropy` becomes meaningless, `churn_of_churn` may
  cancel out adds against deletes that never coexisted in HEAD's
  reachable history.
- **Renames the diff engine misses.** The default rename-similarity
  threshold catches most, but a heavy refactor that splits one file
  into three can register as one deletion + three additions, all
  with no co-change linkage.
- **Bulk machine-generated commits inside the window.** The
  `bulk.max_files` filter catches the obvious ones; check `analysis.commits_filtered.bulk` to see how many were dropped.
  If it's high, the visible window may not be representative.
- **Short windows on quiet files.** A file touched once in a 30-day
  window has `commits_touching = 1` and a large `relative_churn`
  if that one touch was substantive — the metric isn't *wrong*, but
  there's no way to tell single-touch from sustained activity at
  this resolution.

If a number ever surprises you, the right next move is `git log
--follow` on the offending path, not changing the metric.
