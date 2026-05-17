default: test

bootstrap:
    cargo install cargo-nextest cargo-dist cargo-binstall --locked

# Point git at the committed pre-commit hook. Run once after cloning;
# re-run after a re-clone or `git init` reset.
install-hooks:
    git config core.hooksPath .githooks
    @echo "pre-commit hook active — runs fmt-check + clippy on every commit."

fmt:
    cargo fmt --all

fmt-check:
    cargo fmt --all -- --check

lint:
    cargo clippy --workspace --all-targets -- -D warnings

test:
    cargo nextest run --workspace

test-ci:
    cargo nextest run --workspace --profile ci

# Discretionary slow-suite recipe; reserved for mutation/miri/fuzz runs.
# Default and CI profiles keep their tight timeout envelopes.
test-slow:
    cargo nextest run --workspace --profile slow

bench:
    cargo bench --workspace

build:
    cargo build --workspace

install:
    cargo install --path mmk-cli --locked

# Show the artifacts the release pipeline would produce on tag push.
# The actual cross-compile + publish runs in GitHub Actions; this is a
# local sanity check before tagging.
release-plan:
    dist plan

# Smoke test: build only the host-platform tarball locally. The full
# multi-target build runs in CI; cross-compiling locally needs rustup
# targets that aren't required for normal development.
release-build-host:
    dist build --artifacts=host

clean:
    cargo clean
    rm -rf dist/ .mokumokuren/
