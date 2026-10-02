# canix-toolbelt Rust library and CLI

Reusable operations for adopters of the Canix architecture. The first Rust
release extracts runtime-manifest loading from Canix: binary paths, service
endpoints, and agenix secret references. It depends on the published Fleetix
library (`0.2`) for embedded Pkl evaluation.

The `0.1.0` release provides the library and optional CLI shown below. See
`RELEASE.md` in the repository for publication and verification evidence.

```toml
[dependencies]
canix-toolbelt = "0.1.0"
```

```rust,no_run
use canix_toolbelt::runtime::RuntimeManifest;
use std::path::Path;

let manifest = RuntimeManifest::load_from(Path::new("/run/example/runtime.pkl"))?;
println!("{} binaries", manifest.bins.len());
# Ok::<(), canix_toolbelt::runtime::LoadError>(())
```

Async callers use `RuntimeManifest::load_from_async(path).await`. The synchronous
loader returns an error inside an existing Tokio runtime. Errors retain the
manifest path and underlying evaluator diagnostic.

Install the optional standalone CLI with Cargo:

```sh
cargo install canix-toolbelt --features cli
canix-toolbelt runtime show --path /run/example/runtime.pkl
```

`runtime show` emits the typed manifest as JSON. A missing or invalid manifest
exits unsuccessfully. It reads only the manifest and its Pkl dependencies;
referenced binaries and secrets are not executed or read. Producers supply the
three mappings `bins`, `endpoints`, and `secrets` and any schema defaults; see
[`examples/runtime.pkl`](examples/runtime.pkl). The existing camelCase wire
format, including `agenixPath`, is preserved.

## Durable stage operator (unreleased)

The Unix `operator` library and CLI execute deployment-owned, idempotent systemd
stages. Nix's `mkResumableOperator` supplies the units, retry policy and
`executionContract`; consumers must bind that contract to the executable,
arguments, database and data roots used by their stages. The controller compares
the complete persisted policy before resuming and refuses changed contracts.

Checkpoint and worker-restoration intent are synchronized before publication.
A kernel lock excludes both a second runner and cancellation. Signals stop the
current stage and preserve recovery intent; failed worker restoration keeps the
run owned. Legacy interrupted shell state is retained and rejected rather than
interpreted as checkpoints for the new engine. Reconcile that state using the
original deployment before starting a new contract.

The CLI takes an explicit policy and service-manager executable:

```sh
canix-toolbelt operator run --config /run/example/operator.json --systemctl /usr/bin/systemctl
canix-toolbelt operator cancel --config /run/example/operator.json
```

Successful runs exit 0, exhausted stages exit 20, interrupted runs exit 143, and
inspection, persistence or recovery errors exit unsuccessfully. Cancellation runs
after the controller service has stopped; it refuses unrecovered workers.

## Dependency contract

Fleetix owns generic fleet operations. Toolbelt composes architecture conventions
on top of Fleetix; Canix supplies fleet-specific data and policy. Shared behavior
lives in libraries called in-process, with CLI parsing and presentation at the
edges. Toolbelt's Rust crate has no Canix dependency, global output state,
hardcoded fleet path, flake lookup, or Nix-based build script. Nix modules remain
separate integration surfaces.

This release implements runtime-manifest loading. Generic deployment extraction,
toolbelt deployment profiles, and other command families are subsequent slices;
this crate does not yet expose a deployment CLI.

## Development and releases

Run `cargo test --all-features`, `cargo test --no-default-features`,
`cargo clippy --all-targets --all-features -- -D warnings`, and
`cargo doc --no-deps --all-features`. Format through the repository's `treefmt`.
The library and CLI build with Cargo alone; Nix is optional developer tooling.

Simit generates the GitHub verification and crates.io publication workflows.
The saved policy in `simit.toml` requests MSRV, all-feature, audit, and docs
coverage. Regenerate with `simit init ci --platform github` and verify with
`simit init ci --platform github --check --diff`, using the current generator
described in `RELEASE.md`. That handoff also records the Nix-runtime coverage
fixes and remaining release gates. CI and publication both test all features and
the library-only configuration. The MSRV step selects `.#msrv`, verifies Rust
1.88.0, and checks all targets with all features; `flake-modules/rust.nix` derives
that compiler from this crate's `rust-version`.
Signed exact-version tags trigger publication. Before tagging, check the
maintainer trust root, versioned changelog, registry dependencies, package
contents, dry-run publication, and GitHub publishing credential. Confirm the
version on crates.io before switching downstream dependencies.

The existing Nix-built Pages workflow requires Simit's shared CI runtime to be
`nix`; the developer shell supplies Cargo and release tooling. This is a CI
environment choice, not a dependency-distribution mechanism. Neither the crate
archive nor its consumers need the flake. Cargo-only CI can replace this once
Simit permits an independent runtime for Pages.
