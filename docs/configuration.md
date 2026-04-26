# Configuring mmk

A repo-local `mokumokuren.toml` at the Git root is auto-discovered.
Without one, every tracked text file is included in the ranking —
which on most real repos surfaces noise (translations, vendored
dependencies, lockfiles) at the top.

## Bundled profiles

`mmk init` writes a starter file. Pass `--profile <NAME>` to use an
opinionated, ecosystem-tuned starting point instead of the generic
template:

| Profile  | When to use                                                      |
| -------- | ---------------------------------------------------------------- |
| (none)   | Generic template with commented-out examples per ecosystem.      |
| `js-ts`  | Node / npm / yarn / pnpm projects, esp. monorepos.               |
| `rust`   | `cargo` workspaces.                                              |
| `python` | Poetry / pip / uv projects.                                      |
| `go`     | Go modules.                                                      |

```shell
mmk init --profile js-ts
```

Profiles are deliberately conservative — they ship the ignore globs
and `[coupling]` defaults derived from the v0.3 four-repo eval, but
nothing more. If a profile doesn't fit, edit the resulting
`mokumokuren.toml` directly; it's a checked-in file you own.

## Top-level `ignore`

A list of glob patterns. Files matching any pattern are excluded
from the hotspot ranking and from coupling history. Patterns are
unioned with `--ignore` flags on the command line.

```toml
ignore = [
    "po/**",
    "Cargo.lock",
    "vendor/**",
]
```

Some patterns that often pay off, by ecosystem:

- **Rust:** `Cargo.lock`, `target/**`
- **JavaScript / Node:** `node_modules/**`, `package-lock.json`,
  `yarn.lock`, `pnpm-lock.yaml`, `dist/**`, `.next/**`, `**/*.d.ts`
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
team, hand-authored for another). The point of `mokumokuren.toml`
is that the call belongs to the repo's maintainers, not the tool.

## `[coupling]`

Controls COUPLING findings emitted by `mmk review` and
`mmk pre-edit`. See [`coupling.md`](coupling.md) for the design
rationale.

```toml
[coupling]
threshold = 0.30
ignore_partners = [
    "**/package.json",
    "**/*-lock.json",
]
```

| Field             | Default | Notes                                                                  |
| ----------------- | ------- | ---------------------------------------------------------------------- |
| `threshold`       | `0.30`  | Minimum Jaccard a partner must reach to fire a COUPLING finding.       |
| `ignore_partners` | `[]`    | Glob list — paths that never fire as the *missed partner*.             |

CLI overrides: `--coupling-threshold <FLOAT>` on review / pre-edit /
session-summary.

## `[blast_radius]`

Controls the 1-hop co-change neighborhood emitted by
`--blast-radius <PATH>`. **Distinct knob** from `[coupling]` —
blast-radius is the exploratory query where a low threshold is
right.

```toml
[blast_radius]
threshold = 0.10
```

CLI override: `--blast-radius-threshold <FLOAT>`.

## CLI flags vs file config

| Source              | When it wins                                          |
| ------------------- | ----------------------------------------------------- |
| `--coupling-threshold` etc. | Always wins for the call.                     |
| `mokumokuren.toml`  | Loaded if discovered at the repo root (or via `--config <PATH>`). |
| Built-in defaults   | Apply when neither of the above sets the field.       |

The effective `Config` is echoed in JSON output under `config`, so a
consumer can see exactly what produced the result.
