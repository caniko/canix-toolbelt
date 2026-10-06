#!/usr/bin/env bash
# Native qualification only; no host service or provider credentials are used.
set -euo pipefail
manifest=crates/roborev-worker/Cargo.toml
export CANIX_TEST_GIT="$(command -v git)"
export CANIX_TEST_BWRAP="$(command -v bwrap)"
export CANIX_TEST_PYTHON="$(command -v python3)"
export CANIX_TEST_CC="$(command -v cc)"
# Diagnose namespace admission before the worker deliberately suppresses raw
# subprocess errors. Only an immutable tool's version runs in this preflight.
for knob in /proc/sys/kernel/unprivileged_userns_clone /proc/sys/kernel/apparmor_restrict_unprivileged_userns; do
  if [[ -r $knob ]]; then
    printf '%s=' "$knob"
    cat "$knob"
  fi
done
"$CANIX_TEST_BWRAP" \
  --unshare-all --unshare-user --unshare-cgroup --as-pid-1 --disable-userns \
  --die-with-parent --new-session --cap-drop ALL \
  --ro-bind /nix/store /nix/store --proc /proc --dev /dev --tmpfs /tmp \
  --clearenv -- "$CANIX_TEST_GIT" --version
cargo test --locked --manifest-path "$manifest" --jobs 2
cargo test --locked --manifest-path "$manifest" --jobs 2 --features roborev-preparation-tests,roborev-execution-tests
cargo clippy --locked --manifest-path "$manifest" --jobs 2 --all-targets --all-features -- -D warnings
RUSTDOCFLAGS='-D warnings' cargo doc --locked --manifest-path "$manifest" --jobs 2 --no-deps
cargo audit --file crates/roborev-worker/Cargo.lock
nix develop --max-jobs 1 --cores 2 .#msrv -c cargo check --locked --manifest-path "$manifest" --jobs 2 --all-targets --all-features
cargo publish --dry-run --locked --manifest-path "$manifest" --jobs 2
