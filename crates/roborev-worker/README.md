# Roborev Linux worker

`canix-toolbelt-roborev-worker` owns the reusable Linux preparation, direct-adapter
authentication, single-job admission, execution fence and offline worker verifier.
It is a separate package from the portable `canix-toolbelt` review library. Its
public interface is safe Rust; narrowly scoped Linux process/namespace setup is
private. It requires no Canix checkout, binary, runtime manifest or fleet data.
Direct adapter authentication requires Linux 6.5+ socket-associated peer pidfds;
older kernels fail closed rather than authenticating a recycled numeric PID.

Controllers supply full authorized request/comparison digests, private state
roots, immutable executable paths, resource policy and actual custody. Preparation
does not authorize dispatch. The offline runner has no live provider or forge
credential interface and does not qualify production headless execution.

The `roborev-worker` executable implements the existing
`__canix-roborev-prepare` and `__canix-roborev-worker` helper arguments. These names,
hash domains, journal schemas and persistent lock anchors are compatibility
contracts, not an upward dependency on Canix. Retain original state for UNKNOWN
outcomes; do not rename journals, delete lock anchors or replay ambiguous work.
Existing journals bind the original helper executable, so changing that path is
rejected rather than silently transferring authority. Finish or diagnose an old
request with its original helper; use this executable for new requests.

## Qualification

Run native tests in the approved project environment:

```sh
cargo test --manifest-path crates/roborev-worker/Cargo.toml
cargo test --manifest-path crates/roborev-worker/Cargo.toml --features roborev-preparation-tests,roborev-execution-tests
cargo clippy --manifest-path crates/roborev-worker/Cargo.toml --all-targets --all-features -- -D warnings
```

Preparation fixtures require `CANIX_TEST_GIT`, `CANIX_TEST_BWRAP` and
`CANIX_TEST_PYTHON`; durability fixtures also require `CANIX_TEST_CC`. Offline
worker fixtures additionally require explicit `CANIX_TEST_SYSTEMD_RUN`,
`CANIX_TEST_SYSTEMCTL`, `CANIX_TEST_PYTHON` and `CANIX_TEST_STORE_PATHS` inputs,
an unprivileged user namespace and a delegated systemd user manager. The legacy
environment variable names remain supported. This tier must run natively; a Nix
package sandbox or mock manager is not an equivalent admission test.

The execution-feature durability test also requires `CANIX_TEST_CC`, the absolute
Nix-store C compiler from the approved shell. It compiles a test-only fsync
interposer and loads it only in disposable controller children. Injected file or
directory `EIO` must prevent effect grants; failures after journal rename retain
UNKNOWN and deny replay. For a shell with `cc` on PATH:

```sh
CANIX_TEST_CC="$(command -v cc)" cargo test --manifest-path crates/roborev-worker/Cargo.toml --features roborev-execution-tests --test roborev_admission_durability
```

Direct-adapter authentication requires Linux 6.5 or newer with `SO_PEERPIDFD`.
Unsupported kernels fail closed. `tests/roborev/peer-lifetime.py` verifies a stale
socket against a deterministically reused PID in its own PID/network/user
namespace. Run it with the feature-built `roborev-admission-fixture` and a fresh
scratch directory. It checks the replacement's current connection as a positive
control and retains a cleanup receipt. The real-daemon admission fixture also
needs a short scratch path so upstream's Unix socket remains within its path limit.

Format with the repository's `treefmt`. Release this crate through the owning
repository's qualified CI, verify its registry artifact, then consume its exact
version through Cargo. Nix packaging and Home Manager modules have a separate
revision qualification; no consumer source injection is supported.
