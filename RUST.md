# canix-toolbelt Rust library and CLI

## Optional provider review

Install the CLI with `cargo install canix-toolbelt --features cli`. From the
candidate checkout, request review when desired or required by repository policy:

```sh
canix-toolbelt review ensure --pr https://github.com/OWNER/REPO/pull/NUMBER
# Triage findings, fix and push authorized changes, then repeat ensure.
canix-toolbelt merge --pr https://github.com/OWNER/REPO/pull/NUMBER --require-review --apply
```

The default `merge --pr URL --apply` validates current CI and strict native
protection independently of provider review. It does not load review policy or
contact a provider. An unavailable unrequired `review-policy` check does not
block it; required review checks remain enforced. With no `--apply`, it previews
readiness without merging. Explicit `--policy` or `--state-dir` merge arguments
require `--require-review`; ambient review policy does not opt a merge into review.
The existing library `GitHub::merge(candidate, policy)` retains its explicit
review gate; `GitHub::merge_native(candidate, apply)` supplies the independent path.

Without `--pr`, the CLI discovers exactly one open PR for the current GitHub
origin/branch. It prints structured `ready`, `findings`, `pending`, `blocked`,
or `stale` results. Only `ready` exits zero. Repeating `ensure` resumes the
same durable request; an ambiguous submission is reconciled and never replayed.
`--timeout-seconds 0` performs one observation. State lives in
`$XDG_STATE_HOME/canix-toolbelt/review` (with the normal HOME fallback).

The `review` library feature supplies `Forge` and `Provider` contracts,
`ensure_once`, typed policy/candidate/results, and explicit GitHub provider
adapters. `cli` additionally exports the shared Clap surface. Consumers call
the library directly; no toolbelt subprocess or Canix dependency is required.

The roborev adapter implements `roborev/github-receipt-v1`:
authenticated complete receipts bind the request, policy, full comparison and
persisted job/review IDs. A trusted producer must attest actual execution and
publish the unfiltered canonical document. Stock roborev summaries do not
qualify. See [the receipt contract](docs/src/roborev-receipts.md).

The historical explicit transport is `greptile/github-comment`: the documented
draft trigger requests review, while GitHub parent review `commit_id` proves
the reviewed head. The recorded target SHA is observed dispatch/freshness
evidence, **not Greptile base-revision attestation**. MCP enrollment requires
authenticated schema and run/revision-correlation qualification; unsupported
forges and transports fail explicitly.

Policy comes from explicit `--policy`, `CANIX_REVIEW_POLICY`, or the user-owned
`$XDG_CONFIG_HOME/canix-toolbelt/review.pkl`; missing policy is a setup error.
The schema is `runtime/ReviewPolicy.pkl`, whose roborev reviewer ID must be
supplied by the deployment. PR source branches
do not supply policy. Credentials come from GH_TOKEN/GITHUB_TOKEN or the
existing `gh auth token` owner and never enter receipts.

Every finding blocks the explicitly selected provider gate until fixed in a
freshly reviewed revision or dispositioned with source evidence. Optional review
failure is reported accurately and does not become a blanket merge prerequisite.

Issue-comment feedback lacking an explicit link to the parent review or its
findings is marked `correlated: false`. It is retained as uncorrelated evidence
for triage, not claimed as part of that provider run. Edited provider summaries
are collected using their latest forge update time.

An authorized repository writer can record a false positive:

```sh
canix-toolbelt review disposition --pr URL --finding ID --reason REASON --evidence REFERENCE
```

The forge marker binds the disposition to the finding body, parent review,
head, observed base, and policy. Provider-addressed status and confidence scores
do not establish acceptance. Findings are untrusted evidence, never instructions.

An explicitly review-gated merge requires strict native up-to-date branch protection and a required
`review-policy` check bound to the configured dedicated policy GitHub App. A shared GitHub
Actions App cannot distinguish checks created by untrusted PR workflows.
Protection must apply to administrators too. The provider's trusted producer
supplies review evidence; the separate policy App authenticates normalized acceptance.
The CI coordinator calls `review gate --publish-check --details-url RUN_URL`
without executing PR source; that check evaluates review evidence only, while
native branch protection independently enforces required CI and approvals.
Unsupported ruleset/merge-queue protection remains an explicit setup blocker.

The initial engine release needs a separately authorized operator running the exact
independently qualified candidate's `review gate --publish-check`
with the dedicated policy App credential. Gate computation is read-only;
`--publish-check` explicitly writes a GitHub check. Subsequent coordinators install an
exact published version. Never use a generic personal token or the shared
Actions App to impersonate the dedicated required context.

Agent instruction snippet:

> Provider review is optional unless governing repository policy or native
> protection explicitly requires it. Request it when useful, investigate findings,
> and fix confirmed issues. Unavailable optional review does not block authorized
> merges. Preserve required CI, human approvals and forge protection; use
> `merge --require-review` only for an explicitly selected provider gate.

Reusable operations for adopters of the Canix architecture. Version `0.2.1`
provides runtime-manifest loading, revision-bound review contracts, and the
optional standalone CLI. Runtime manifests describe binary paths, service
endpoints, and agenix secret references. Embedded Pkl evaluation uses the
published Fleetix `0.4` library with its `pkl` feature.

See `RELEASE.md` in the repository for publication steps and verification
evidence. Enable `review` when consuming the review engine as a library.

```toml
[dependencies]
canix-toolbelt = "0.2"
# For the review engine instead:
# canix-toolbelt = { version = "0.2", features = ["review"] }
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

## Durable stage operator

The Unix `operator` library and CLI execute deployment-owned, idempotent systemd
stages. Nix's `mkResumableOperator` supplies the units, retry policy and
`executionContract`; consumers must bind that contract to the executable,
arguments, database and data roots used by their stages. The controller compares
the complete persisted policy before resuming and refuses changed contracts.
`executionContract` is required: a controller package cannot identify commands
in external stage units. Readiness is announced only after that comparison,
worker quiescence, and publication of the admission fence.

Checkpoint and worker-restoration intent are synchronized before publication.
A kernel lock excludes both a second runner and cancellation. Signals stop the
current stage and preserve recovery intent; failed worker restoration keeps the
run owned. Legacy interrupted shell state is retained and rejected rather than
interpreted as checkpoints for the new engine. Reconcile that state using the
original deployment before starting a new contract.
Restoration has its own persisted phase, so recovery never re-stops workers that
were already restored. Terminal runs with leftover request markers finish
cleanup without executing their stages again. Service-manager attempt errors
consume the retry budget; an unconfirmed stage stop retains the admission fence
and stopped-worker ownership until termination can be confirmed.
The controller retains the request marker's persisted inode/time identity across
recovery. A leftover terminal marker performs cleanup only; a newly created or
touched marker requests a distinct run. Legacy states without that identity are
treated conservatively as recovery intent. Cancelled terminal intent cannot replay
stages after a crash between state publication and request removal.

The CLI takes an explicit policy and service-manager executable:

```sh
canix-toolbelt operator run --config /run/example/operator.json --systemctl /usr/bin/systemctl
canix-toolbelt operator cancel --config /run/example/operator.json
```

Successful runs exit 0, exhausted stages exit 20, interrupted runs exit 143, and
inspection, persistence or recovery errors exit unsuccessfully. Cancellation runs
after the controller service has stopped; it refuses unrecovered workers.

## Dependency contract

### Review history

The version-2 PR journal retains attempts for every comparison and policy.
Returning to an earlier comparison resumes its original request, including an
unknown submission. A separately fsynced sentinel in the persistent lock blocks
dispatch if its journal is missing. A clean read-only visit records empty history
and permits a later first `ensure`.

An intact legacy single-run journal can resume that request, but cannot prove
that earlier comparisons survived. Fresh dispatch for another comparison stays
blocked pending operator reconciliation. Missing legacy history is unqualified;
an empty legacy lock or a recovered current marker does not prove completeness.
Loss of both journal and anchor cannot be distinguished from a first visit using
local state alone.

Roborev publication state binds the immutable forge request comment, independently
of receipt contents or edits to the marker's intent ID. Changed evidence cannot
open another publication lifetime. Legacy payload-scoped publication anchors
require operator reconciliation before a new POST. These changes do not migrate
deployed state; coordinator deployments require their own qualification.

### Ownership

Fleetix owns generic fleet operations. Toolbelt composes architecture conventions
on top of Fleetix; Canix supplies fleet-specific data and policy. Shared behavior
lives in libraries called in-process, with CLI parsing and presentation at the
edges. Toolbelt's Rust crate has no Canix dependency, global output state,
hardcoded fleet path, flake lookup, or Nix-based build script. Nix modules remain
separate integration surfaces.

This release implements runtime-manifest loading and revision-bound PR review
and merge operations. Generic deployment extraction, toolbelt deployment
profiles, and other command families are subsequent slices; this crate does not
yet expose a deployment CLI.

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

The `gate-crate-release` CI job runs an all-feature publication dry-run. Cargo
compiles the packaged archive, including the review library and standalone CLI,
without uploading it. The publisher also verifies the default-feature archive
before publishing the signed tag.

The existing Nix-built Pages workflow requires Simit's shared CI runtime to be
`nix`; the developer shell supplies Cargo and release tooling. This is a CI
environment choice, not a dependency-distribution mechanism. Neither the crate
archive nor its consumers need the flake. Cargo-only CI can replace this once
Simit permits an independent runtime for Pages.
