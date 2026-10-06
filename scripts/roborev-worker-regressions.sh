#!/usr/bin/env bash
# Native qualification only; no host service or provider credentials are used.
set -euo pipefail
manifest=crates/roborev-worker/Cargo.toml
export CANIX_TEST_GIT="$(command -v git)"
export CANIX_TEST_BWRAP="$(command -v bwrap)"
export CANIX_TEST_PYTHON="$(command -v python3)"
export CANIX_TEST_CC="$(command -v cc)"
cargo test --locked --manifest-path "$manifest" --jobs 2
cargo test --locked --manifest-path "$manifest" --jobs 2 --features roborev-preparation-tests,roborev-execution-tests
cargo clippy --locked --manifest-path "$manifest" --jobs 2 --all-targets --all-features -- -D warnings
RUSTDOCFLAGS='-D warnings' cargo doc --locked --manifest-path "$manifest" --jobs 2 --no-deps
cargo audit --file crates/roborev-worker/Cargo.lock
nix develop --max-jobs 1 --cores 2 .#msrv -c cargo check --locked --manifest-path "$manifest" --jobs 2 --all-targets --all-features
cargo publish --dry-run --locked --manifest-path "$manifest" --jobs 2
