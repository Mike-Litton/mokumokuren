# Coupling and blast radius

mmk's topology layer answers "if I touch this file, what else is
likely to need attention?" It comes in two surfaces:

- `--couples-of <PATH>` and the `top_couples[]` array on every ranked
  entry — the historical co-change list.
- `--blast-radius <PATH>` — a 1-hop neighborhood graph filtered by a
  Jaccard threshold.

The metric is **Jaccard similarity** of the co-changing commit sets,
capped at 1.0:

```
jaccard(a, b) = |commits touching both| / |commits touching either|
```

Symmetric, scale-free, and robust to file-size differences.

## Coupling: what historically co-changes

`--couples-of <PATH>` answers "if I touch this file, what historically
changes alongside it?":

```shell
mmk analyze --couples-of a.rs
```

Each ranked entry in JSON output also carries a `top_couples` array —
the same data, attached per file. In text mode the co-change blocks
are off by default (one-line-per-file table stays grep-friendly);
pass `--couples` to render them inline:

```shell
mmk analyze --couples
```

## Blast radius: 1-hop co-change neighborhood

`--blast-radius <PATH>` emits an explicit graph of partners with
Jaccard ≥ a threshold. Useful when the agent is about to edit a file
and wants to know what *else* it should re-read:

```shell
mmk analyze --blast-radius a.rs
mmk analyze --blast-radius a.rs --blast-radius-threshold 0.05
```

The threshold defaults to `0.10` — loose enough to surface real
coupling on young repos. Override per call with
`--blast-radius-threshold`, or pin per-repo:

```toml
[blast_radius]
threshold = 0.10
```

The effective threshold is echoed in the JSON output
(`blast_radius.threshold`) so consumers can see what filter produced
the listed nodes.

## COUPLING findings: the review/pre-edit threshold

`mmk review` and `mmk pre-edit` emit COUPLING findings that flag
historical partners not touched in the current edit. These have a
**separate threshold** from blast-radius, governed by
`[coupling] threshold` (default `0.30`):

```toml
[coupling]
threshold = 0.30
ignore_partners = []
```

The split exists because the two surfaces optimize for different
things. Blast-radius is the exploratory query — surfacing faint
signal is the point. COUPLING findings flow into the agent's edit
loop, where a sub-0.30 partner is noise that costs context tokens
and can drive wrong work. The four-repo eval showed 67 % of
COUPLING findings under the old single-threshold default sat in the
0.10–0.30 range — borderline signal at best, false positives at
worst.

## `ignore_partners`: pruning the missed-partner list

Some files legitimately co-change but should never be flagged as a
"missed partner." Examples from the eval:

- Sibling-workspace `package.json` files in JS/TS monorepos —
  renovatebot dep bumps move them in lockstep, but a hand edit to
  one workspace doesn't imply you should also edit the other.
- Generated artifacts (`*.generated.*`, `*.pb.go`, openapi blobs).
- Build / release config (`Fastfile`, `.yarnrc*`).

`[coupling] ignore_partners` is a glob list of paths that are
filtered out of the *missed-partner* slot in COUPLING findings. They
still contribute to history (they're still legitimate co-changes
inside the analysis); they just don't fire as warnings.

```toml
[coupling]
ignore_partners = [
    "**/package.json",
    "**/*-lock.json",
    "**/*.generated.*",
]
```

This is distinct from the top-level `ignore` list, which excludes
paths from the analyze ranking entirely.

## Tuning for your repo

The defaults are calibrated against the v0.3 eval (cal.diy, immich,
n8n, vscode). Real repos vary. To measure the noise floor on yours:

```shell
mmk eval --sample 50
```

emits a noise-floor report covering firing rate, layer mix, and the
jaccard distribution of COUPLING findings. If a majority of findings
sit in the 0.10–0.30 bucket, raise `[coupling] threshold`. If
specific partner paths dominate the noisy-partner list, add them to
`[coupling] ignore_partners`.

For the JS/TS ecosystem, `mmk init --profile js-ts` ships defaults
derived directly from the eval — workspace `package.json`,
lockfiles, generated artifacts, build config.

## Why empirical, not architectural

mmk's coupling is the *historical* co-change cone, not a
counterfactual model. A file pair that *should* co-change but
historically hasn't will not appear; one that has but shouldn't
will. The signal is "this is the pattern your team has actually
followed," which is the right question for catching agent-introduced
coupling regressions — but the wrong question for "what's the ideal
module boundary."
