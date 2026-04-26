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

## COUPLING findings: the v0.4 Wilson rule

`mmk review` and `mmk pre-edit` emit COUPLING findings that flag
historical partners not touched in the current edit. The decision
rule changed in v0.4 — see *Why conditional probability + Wilson*
below for the full reasoning. The knobs:

```toml
[coupling]
confidence_threshold = 0.20  # Wilson 95% lower bound on P(partner | subject)
min_sample_size      = 5     # don't infer from too few observations
ignore_partners      = []
```

A partner fires COUPLING when **both** conditions hold:

1. `commits_touching(subject) ≥ min_sample_size` — the subject has
   at least `n=5` commits in the window. Below that the binomial
   sample is too small for any inference; pre-edit falls through
   to a `Severity::Ok` "insufficient history" finding instead.
2. `wilson_lower_95(co_change(subject, partner), n) ≥ confidence_threshold`
   — the 95 % lower confidence bound on the conditional probability
   `P(partner | subject)` clears 0.20.

The legacy `threshold` field is silently mapped to
`confidence_threshold` for back-compat (`--verbose` surfaces a
deprecation note). Sub-0.30 partners on the v0.3 jaccard scale don't
translate cleanly to the new metric — a fresh `mmk eval` run is
worth a minute.

### Why conditional probability + Wilson

The v0.3 rule was `jaccard(A, B) ≥ threshold`. Jaccard is a
*symmetric similarity*. The agent's actual question is *asymmetric
and probabilistic*: "given I just edited A, what fraction of
historical edits to A also touched B?" That's
`P(B | A) = co_change(A, B) / commits_touching(A)`. v0.4's switch
to that quantity has three properties the jaccard rule lacked:

- **Statistically calibrated.** 95 % confidence is a standard, not a
  tuned hyperparameter. `confidence_threshold = 0.20` reads as "I
  want to know about partners with ≥ 20 % conditional probability,
  and I want statistical confidence in that estimate."
- **Frequency-invariant.** Hot files (54/203 ≈ 0.27) and quiet
  files (1/1 = 1.0) land on the same scale. The Wilson lower
  bound naturally penalizes low-N: a single 1/1 hit doesn't fire
  because the CI is wide.
- **Asymmetric, matching the agent's question.** "Given I edited
  A, what does A's history say about B?" — not "are A and B similar
  overall." Catches cases where A→B is strong but B→A is weak.

The v0.3 vscode validation found two failure modes the new rule
fixes:

| Subject (n)             | Partner (k)             | jaccard | Wilson lower 95% | v0.3 fired? | v0.4 fires? |
| ----------------------- | ----------------------- | ------: | ---------------: | :---------: | :---------: |
| `runInTerminalTool.ts` (203) | `*.test.ts` (54)   | 0.21    | 0.21             |   no   |   yes  |
| `chatWidget.ts` (133)   | `chatInputPart.ts` (27) | 0.08    | 0.14             |   no   |   no   |
| `breakpointsView.ts` (3)| `debugViewlet.css` (3)  | 0.23    | 0.44             |   no   |   no¹  |

¹ Wilson lower 0.44 *would* clear 0.20, but `n=3 < min_sample_size=5`
suppresses inference. Pre-edit emits an OK finding instead so the
agent can tell "no signal" from "mmk wasn't run."

The dual-condition gate (Wilson lower **AND** min_sample_size)
isn't curve-fitting — it's the standard "don't infer from too few
observations" practice. n=5 is the smallest sample where Wilson is
meaningfully informative; the same cutoff drives the chi-square
`expected_count ≥ 5` rule.

### Effective field on `top_couples[]`

Each entry in the JSON output now carries both views:

```json
{
  "partner": "src/foo.test.ts",
  "co_change_count": 54,
  "jaccard": 0.21,
  "conditional_probability": 0.266,
  "wilson_lower_95": 0.21
}
```

`jaccard` is preserved — it still drives `--blast-radius` (the
exploratory "what's near this file" surface, where symmetry is the
right question). `conditional_probability` and `wilson_lower_95`
drive review/pre-edit's COUPLING gate.

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
n8n, vscode) and re-validated under the v0.4 Wilson rule. Real repos
vary. To measure the noise floor on yours:

```shell
mmk eval --sample 50
```

emits a noise-floor report covering firing rate, layer mix, and the
Wilson-95 %-lower-bound distribution of COUPLING findings (buckets:
`0.00-0.20`, `0.20-0.40`, `0.40+`). If most findings sit in the
0.20-0.40 bucket and the firing rate feels noisy, raise
`[coupling] confidence_threshold` (e.g. to 0.30).

For high-breadth noise — the same `CHANGELOG.md` blamed across
many unrelated subjects — let `mmk eval --learn` synthesize a
suggested `ignore_partners` block:

```shell
mmk eval --sample 50 --learn
```

The suggestion uses the inverse conditional probability
`P(subject | partner)`: a partner that fires across many subjects
but no individual subject "owns" is the system-level-noise
signature. Paste the block into `mokumokuren.toml`.

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
