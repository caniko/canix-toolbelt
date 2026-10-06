# Rust release execution

## 0.4.0 shared construction

The optional Unix `build-train` feature uses the published Fleetix `0.5.1` and
nix-manager-core `0.3.0` crates directly. The release preserves the `0.3.0` Roborev
interfaces and qualification fixtures. The feature-enabled production output is
`packages.<system>.canix-toolbelt-build-train`; the default package remains a
library/CLI consumer without a coordinator service.

The native backend publication passed all 18 steps of
[run 37512848122](https://github.com/caniko/nix-manager-core/actions/runs/37512848122).
Its downloaded registry artifact has SHA-256
`927c3eb02e457b2a8f7b4c44d86b9d6ccf136234c87e977b13813f2e13ad1253`.
Canix provisioned its publication credential through targeted Secret Manager
sync; the source is `age/secrets/users/can/crates_io.age`.

Fleetix's recovery correction passed all 20 publication steps of
[run 37526212444](https://github.com/caniko/fleetix/actions/runs/37526212444).
Its unyanked `0.5.1` artifact has SHA-256
`430f1aa9a8a5f7a3581c0d6e9fa25a3067355fe560bb3a8c798c723906153987`.

Before tagging `0.4.0`, require full exact-candidate hosted CI, all declared Nix
installable builds, generator parity against Simit `afb7939`, maintainer trust,
and package-content inspection. After publication, verify the unyanked registry
version and checksum, then compile a registry-only consumer with `build-train`.
Production admission, host activation and Atlas/Murph behavior remain separate
consumer qualification gates documented in [BUILD_TRAIN.md](BUILD_TRAIN.md).

## 0.3.0 Roborev publication

The producer implementation was merged in [PR #9](https://github.com/caniko/canix-toolbelt/pull/9).
Toolbelt publishes `0.3.0`; the independent Linux worker publishes `0.1.0`.
Before either publication, qualify the release candidate through full CI,
all declared Nix installables, worker native regression/publication gates,
maintainer trust and package-content inspection. The final receipt coverage and
ID/UUID custody fixes belong to the `0.3.0` release notes.

Create a signed annotated numeric `0.3.0` tag for the Toolbelt publisher and a
signed annotated `roborev-worker-0.1.0` tag for the independent worker. Verify both
against the pinned maintainer keyring. The root `publish-crate.yaml` publishes
only Toolbelt; publish the independently qualified worker archive through Cargo
from the same tagged source. Verify both registry versions and archive checksums
before migrating consumers. Nix consumers separately pin the qualified producer
revision; registry availability alone does not qualify a flake revision.

## 0.2.0 publication

The release adds the provider-review library and guarded standalone CLI merged
in [PR #6](https://github.com/caniko/canix-toolbelt/pull/6). Cargo declares
`0.2.0`; the dated changelog and release validation are prepared in
[PR #8](https://github.com/caniko/canix-toolbelt/pull/8).

Before tagging the merged release revision:

1. Require successful full CI, including `gate-crate-release`, and Nix
   installable builds on that exact revision. The package gate runs
   `cargo publish --dry-run --locked --all-features` and compiles the archive
   with the review engine and CLI enabled.
2. Run `simit release trust check`, confirm the operator's configured identity
   and signing key, and verify that the exact `0.2.0` tag is unused locally and
   remotely. Generate and check workflows with qualified Simit revision
   `aab017d8bc5db17133d7730585b441cd0246a731`; older installed versions cannot
   parse the current `ci.nix_build` policy.
3. Create and verify the signed annotated `0.2.0` tag at the qualified merged
   revision, then push that exact tag. `publish-crate.yaml` accepts numeric
   semver tags, verifies the signature against `keys/maintainers.gpg`, reruns
   flake, tests, MSRV, audit, docs, Clippy, and default-feature publication
   dry-run checks, then publishes using `CRATES_IO_API_TOKEN`.
4. Wait for the publication workflow to succeed. Verify `0.2.0` is available
   and not yanked through the crates.io version API before installing it or
   changing downstream Cargo dependencies.

The repository's `CRATES_IO_API_TOKEN` slot exists; publication must establish
that it is still valid. GitHub Pages is not configured and its separate
workflow fails during Pages setup; this does not run in the crate publisher.
Provider-review App enrollment and native protection remain runtime rollout
prerequisites for `merge --apply`, rather than crate packaging prerequisites.

## Initial release record (2026-09-30)

On 2026-09-30 the operator authorized publication and downstream synchronization
of Fleetix `0.2.0` and canix-toolbelt `0.1.0`, selecting library/release changes
only. Concurrent topology/health and cloud-host/public-edge work has its own
release scope.

## Publication and credentials

- Fleetix `0.2.0` is published and not yanked, verified through the crates.io
  version API on 2026-09-30. Its signed tag points to
  `cce1be1ce91ba90f385aad18c21eacf0164e42fb`.
  [Full CI](https://github.com/caniko/fleetix/actions/runs/36644730641) and
  [publication](https://github.com/caniko/fleetix/actions/runs/36646242304) passed.
- Toolbelt's initial release-only commit is `90bd092`; `416364f` fixes the
  sandboxed Zola CA bundle. Its
  [full CI](https://github.com/caniko/canix-toolbelt/actions/runs/36646299699)
  passed. The candidate now consumes registry Fleetix `0.2.0`; its isolated
  archive passes both test/Clippy feature modes, rustdoc, package listing, and
  publication dry-run again. Audit and release-metadata checks also pass.
  The signed `0.1.0` publication tag follows CI on the aligned dependency.
- Full `modde-crates-io-api-token` sync succeeded on 2026-09-30 after hardware-key
  decryption and Codeberg OAuth renewal. The declared Codeberg user secret and
  both GitHub repository secrets were applied. Remote GitHub listings report
  `2026-09-30T05:50:12Z` (toolbelt) and `2026-09-30T05:50:13Z` (Fleetix).
  The unrelated stale `regicide-attic-token` managed entry was not pruned.

## Validation evidence

The isolated release trees were validated as Git-tree archives, preserving
concurrent edits in the canonical checkouts.

- Fleetix: 66 all-feature tests, library-only tests, both Clippy feature modes,
  warning-free rustdoc, treefmt, package listing, publication dry-run, and audit.
  GitHub CI also passed Nix checks and actual Rust 1.88 MSRV compilation.
- Toolbelt before Fleetix alignment: seven all-feature integration tests plus
  doctest, five library-only integration tests plus doctest, both Clippy modes,
  warning-free rustdoc, treefmt, package listing, publication dry-run, and audit.
  Named MSRV-shell and all 36 x86_64 Nix checks evaluated successfully. Full CI
  passed after the CA-bundle fix; the local site-helper check also built.
- Tests cover explicit consumer paths, schema defaults and typed entries,
  source-preserving errors, async loading, and a CLI subprocess with an empty
  executable search path. Manifest loading uses the embedded evaluator.
- Maintainer trust checks and signature verification passed using the operator's
  configured identity. No identity substitution or hook bypass was used.

## Generator and operational follow-ups

Simit owns the generated CI and publisher. The operator-authorized source
generator includes feature-policy and actual MSRV gates for Nix/split-runner
jobs, and named MSRV-shell support. Use this invocation from the checkout:

```sh
direnv exec . cargo run --manifest-path ../simit/Cargo.toml -- init ci --platform github --check --diff
simit release trust check
```

The installed older Simit would remove the required Pages environment. The
current source keeps Nix CI and Pages while distributing Rust dependencies
directly through Cargo. The Harbor input supplies toolchains, never crate source.
Both library-only and CLI-enabled configurations are covered; MSRV checks
select `.#msrv`, verify Rust 1.88.0, and clear inherited nightly flags.

Remaining producer follow-ups:

- Simit's task-specific renderer/flake fixes are uncommitted amid concurrent
  work. Six new release-gate regressions, 44 flake-generator integration tests,
  and 16 renderer unit tests pass. The broader `init_ci` suite previously passed
  90/91; `check_failure_hint_includes_effective_generation_flags` and six
  dead-helper warnings in concurrent `src/commands/init_ci.rs` need coordination.
- Toolbelt's separate Pages workflow receives HTTP 404 (`Get Pages site failed`)
  because GitHub Pages is not configured. This is separate from the green crate CI.
- `canix secret sync` currently panics on incompatible Clap `verbose` types.
  The successful operational route is guarded evaluation of
  `.#secretSyncTargets`, then `secret-manager sync --config <evaluated-json>
  --target modde-crates-io-api-token`. The older workspace release-secret checker
  rejects GitHub origins; actual publisher contracts and forge secret listings
  establish readiness.
- The hooks-only template assumes a `pkgs.rust-bin` overlay absent from the
  ordinary package set. Preserve repository treefmt and the working named MSRV
  environment when integrating hooks.

## Downstream migration

After toolbelt `0.1.0` is verified on crates.io:

1. Add a versioned `canix-toolbelt` dependency with default features disabled,
   inherited by `canix-foundation`.
2. Replace Canix's runtime types/loading with the library, preserving its
   `/run/canix/runtime.pkl` default facade, JSON shape, diagnostics, and role gates.
   Retain amended-schema and missing-file compatibility tests.
3. Replace Canix's Fleetix Git pin with registry `0.2`, sharing one Fleetix series.
4. Update Cargo.lock and obsolete Cargo Git hashes together.

Canix's `docs/src/design/cli-products.md` defines the three-product contract.
Generic deployment extraction is subsequent work; this release provides
runtime-manifest loading rather than a deployment CLI.
