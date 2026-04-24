# Mokumokuren (`mmk`)

Evidence-based Git health metrics for humans and LLM agents.

> Status: **v0.0.0 — scaffold only.** No metrics implemented yet. This repo is
> the substrate the implementation will grow into. See the implementation plan
> for the roadmap.

## Install

Once releases are cut, these install paths are available:

| Audience | Command |
|---|---|
| Any shell user | `curl -LsSf https://github.com/mlitton/mmk/releases/latest/download/mokumokuren-installer.sh \| sh` |
| Windows | `iwr https://github.com/mlitton/mmk/releases/latest/download/mokumokuren-installer.ps1 \| iex` |
| Rust devs (from source) | `cargo install mokumokuren` |
| Rust devs (prebuilt) | `cargo binstall mokumokuren` |

Both `mokumokuren` and `mmk` land on `$PATH`.

## Quickstart

```shell
mmk --help            # not yet — stub prints version
mokumokuren --help    # same stub binary
```

## Development

Prerequisites: Rust stable (pinned via `rust-toolchain.toml`) and
[`just`](https://github.com/casey/just).

```shell
just bootstrap        # install cargo-nextest, cargo-dist, cargo-binstall
just fmt              # format
just lint             # clippy -D warnings
just test             # nextest workspace
just build            # cargo build --workspace
just install          # cargo install --path mmk-cli --locked
just release-dry      # dist build --artifacts=all
```

## Workspace layout

```
mmk-core/    metric engine
mmk-git/     Git history walker
mmk-config/  config loading
mmk-cli/     the `mokumokuren` / `mmk` binary
mmk-web/     frontend (deferred to v0.4)
```

## License

Dual-licensed under either:

- [MIT License](LICENSE-MIT)
- [Apache License, Version 2.0](LICENSE-APACHE)

at your option.
