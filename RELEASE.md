# Rust release execution

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
