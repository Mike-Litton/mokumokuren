# Mokumokuren (`mmk`)

![Mokumokuren](docs/img/mokumokuren.png)

Ranks the files in your repository by how much maintenance effort they
consume, using only Git history. The same ranking, computed across an
LLM agent's commits, is a thrashing detector — when one corner of the
codebase dominates the list across sessions, the agent is rewriting it
instead of making progress.

The premise: files that get touched a lot are files that are hard to
get right. Bugs cluster where code keeps changing, and the cost of the
next change is highest where the last ten changes already landed.
Mokumokuren makes that "where" visible.

The metric is recency-weighted churn × file size (lines of code).
Ignore patterns are configured per-repo via `mokumokuren.toml` — the
tool ships with no ecosystem-specific defaults because there is no
ecosystem-neutral right answer.

## What you see

```
$ mmk analyze --top 10
rank  path                              loc  weighted_churn   commits       hotspot
----  ---------------------------  --------  --------------  --------  ------------
   1  mmk-git/tests/analyze.rs          462          369.00         1         36.30
   2  mmk-git/src/diff.rs               269          275.00         1         31.47
   3  mmk-git/src/lib.rs                206          131.00         1         26.04
   4  mmk-git/src/loc.rs                 82           70.00         1         18.84
   ...
```

- `weighted_churn` — added + deleted lines across the analysis window,
  exponentially decayed by commit age (more recent edits weigh more).
- `commits` — number of commits that touched the file in the window.
- `hotspot` — `weighted_churn × log10(loc)`. Larger files with sustained
  churn rank highest; one-line tweaks to a 10k-LOC file beat 100-line
  rewrites to a 50-LOC stub.

The top of the list is where to look first.

## How to read it

**Top of the list is where defects cluster.** Hotspots concentrate
bugs disproportionately. If a file's been near the top for a while,
it's a refactoring candidate.

**PR touches a top-N hotspot? Tighten review.** A change to a known
hotspot deserves more eyes than a change to a quiet file.

**Same file on top across multiple agent sessions? Agent is thrashing.**
If you're using an LLM agent and the same file dominates the ranking
across runs, the agent is rewriting one corner instead of making
forward progress. Either review the changes carefully or reset the
agent's context.

## Quickstart

```shell
mmk init                      # scaffold mokumokuren.toml in CWD
# edit mokumokuren.toml — uncomment the ignore patterns that apply
mmk analyze --top 20          # default 180-day window
mmk analyze --since 90days
mmk analyze --format json | jq '.files[] | .path'
mmk analyze --ignore 'docs/**'  # extra ignore on top of the config file
mmk analyze --config path/to/other.toml
```

## Configuring ignores

A repo-local `mokumokuren.toml` at the Git root is auto-discovered:

```toml
ignore = [
    "po/**",
    "Cargo.lock",
    "vendor/**",
]
```

Without a config file, every tracked text file is included — which on
most real repos surfaces noise (translation files, vendored
dependencies, lockfiles) at the top of the ranking. Run `mmk init` to
scaffold a starter template with commented-out examples covering the
common cases (translations, vendored, lockfiles, generated, engine
assets); uncomment what applies.

Some patterns that often pay off, by ecosystem:

- **Rust:** `Cargo.lock`, `target/**`
- **JavaScript / Node:** `node_modules/**`, `package-lock.json`,
  `yarn.lock`, `pnpm-lock.yaml`, `dist/**`, `.next/**`
- **Python:** `__pycache__/**`, `*.pyc`, `poetry.lock`,
  `**/migrations/**` (Django, often)
- **Ruby / Rails:** `Gemfile.lock`, `vendor/bundle/**`, `tmp/**`,
  `log/**`
- **Go:** `go.sum`, `vendor/**`
- **iOS / Swift:** `Pods/**`, `*.pbxproj`
- **Game engines:** `*.tscn`, `*.tres` (Godot); `*.unity` (Unity);
  `*.uasset`, `*.umap` (Unreal)
- **Translations (any ecosystem):** `**/*.po`, `**/*.pot`,
  `**/locale/**`, `**/Localization/**`

Ignore patterns are not portable across repos — what's noise in one
project is signal in another (`migrations/` is mechanical for one
team, hand-authored for another). The point of `mokumokuren.toml` is
that the call belongs to the repo's maintainers, not the tool.

## Install

| Audience | Command |
|---|---|
| macOS / Linux | `curl -LsSf https://github.com/mlitton/mmk/releases/latest/download/mokumokuren-installer.sh \| sh` |
| Windows | `iwr https://github.com/mlitton/mmk/releases/latest/download/mokumokuren-installer.ps1 \| iex` |
| From source | clone the repo, then `cargo install --path mmk-cli --locked` |

Both `mokumokuren` and `mmk` land on `$PATH`.

## Known limitations

- **Single snapshot per run.** No before/after comparison, no persistence
  between runs, no caching.
- **Shallow clones are warned but not rejected.** History before the
  shallow boundary isn't analyzed; the JSON output flags it.
- **Non-UTF8 paths are lossy.** A repository with non-UTF-8 path bytes
  may silently undercount churn for those paths. Vanishingly rare in
  practice; the failure mode is a missing entry, not a wrong ranking.
- **Config file schema is minimal.** Only `ignore = [...]` is supported.

## Development

Prerequisites: Rust stable (pinned via `rust-toolchain.toml`) and
[`just`](https://github.com/casey/just).

```shell
just bootstrap          # install cargo-nextest, cargo-dist, cargo-binstall
just fmt                # format
just lint               # clippy -D warnings
just test               # nextest workspace
just build              # cargo build --workspace
just install            # cargo install --path mmk-cli --locked
just release-plan       # show what the release pipeline would produce
just release-build-host # smoke-build the host-platform tarball locally
```

## Workspace layout

```
mmk-core/    metric engine
mmk-git/     Git history walker
mmk-config/  config + mokumokuren.toml loader
mmk-cli/     the `mokumokuren` / `mmk` binary
```

## License

Dual-licensed under either:

- [MIT License](LICENSE-MIT)
- [Apache License, Version 2.0](LICENSE-APACHE)

at your option.
