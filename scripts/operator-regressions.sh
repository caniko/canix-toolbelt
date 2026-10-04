#!/usr/bin/env bash
set -euo pipefail
git show --no-patch --format='checkout=%H%ntree=%T%nparents=%P' HEAD
cargo test --locked --no-default-features --test operator
git fetch --no-tags --depth=1 origin \
  9832d14caa59106c31c68787869ba234beafe19e \
  7c6a21ffbf6dbc71fd42ba3d78f99fd50ab9e37d
fixture=$(mktemp -d)
trap 'rm -rf "$fixture"' EXIT
cp Cargo.toml Cargo.lock "$fixture/"
cp -R src tests examples runtime "$fixture/"
export CARGO_TARGET_DIR="$PWD/target/operator-regressions"
old=9832d14caa59106c31c68787869ba234beafe19e
git show "$old:src/operator.rs" >"$fixture/src/operator.rs"
# The older API lacked a callback; reproduce its CLI's readiness-before-run
# ordering with this compatibility shim so failures are assertions, not builds.
cat >>"$fixture/src/operator.rs" <<'RS'
pub fn run_with_ready(
    config: &OperatorConfig,
    services: &mut impl ServiceManager,
    cancelled: &AtomicBool,
    ready: impl FnOnce() -> io::Result<()>,
) -> io::Result<Outcome> {
    ready()?;
    run(config, services, cancelled)
}
RS
cargo test --manifest-path "$fixture/Cargo.toml" --locked --no-default-features --test operator --no-run
for regression in \
  readiness_follows_worker_quiescence_and_the_durable_fence \
  terminal_leftover_requests_do_not_create_another_run \
  service_manager_errors_exhaust_the_persisted_retry_budget \
  unconfirmed_stage_stop_retains_fences_and_does_not_restore_workers \
  partial_restoration_never_requiesces_recovered_workers; do
  if cargo test --manifest-path "$fixture/Cargo.toml" --locked --no-default-features --test operator "$regression" -- --exact; then
    echo "Faulty implementation unexpectedly passed: $regression" >&2
    exit 1
  fi
  echo "RED against $old: $regression"
done
old=7c6a21ffbf6dbc71fd42ba3d78f99fd50ab9e37d
git show "$old:src/operator.rs" >"$fixture/src/operator.rs"
cargo test --manifest-path "$fixture/Cargo.toml" --locked --no-default-features --test operator --no-run
for regression in fresh_request_after_terminal_run_starts_new_run stale_cancelled_request_cannot_replay_stages; do
  if cargo test --manifest-path "$fixture/Cargo.toml" --locked --no-default-features --test operator "$regression" -- --exact; then
    echo "Faulty implementation unexpectedly passed: $regression" >&2
    exit 1
  fi
  echo "RED against $old: $regression"
done
