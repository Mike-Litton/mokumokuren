default: test

bootstrap:
    cargo install cargo-nextest cargo-dist cargo-binstall --locked

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

bench:
    cargo bench --workspace

build:
    cargo build --workspace

install:
    cargo install --path mmk-cli --locked

release-dry:
    dist build --artifacts=all

clean:
    cargo clean
    rm -rf dist/ .mokumokuren/
