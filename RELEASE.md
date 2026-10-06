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
- Toolbelt `0.1.0` is published and not yanked, verified through the crates.io
  version API on 2026-09-30. Its signed tag points to
  `b44a83f5766d08b94bbe4619916806db97599c38`, consuming registry Fleetix `0.2.0`.
  [Full CI](https://github.com/caniko/canix-toolbelt/actions/runs/36675514328)
  and [publication](https://github.com/caniko/canix-toolbelt/actions/runs/36676168725)
  passed, including signature verification and the actual upload. Its isolated
  archive also passes both test/Clippy feature modes, rustdoc, package listing,
  publication dry-run, audit, and release-metadata checks.
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
- A standalone consumer compiled using only registry Fleetix `0.2.0` and
  toolbelt `0.1.0`. It loaded a consumer-owned Pkl manifest and called Fleetix
  string rendering with an empty `PATH`, confirming runtime independence from
  Nix, Canix, and the upstream frontends.
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

Generator integration and remaining operational follow-ups:

- Simit's renderer/flake fixes are committed in `96acee7`, with regeneration-hint
  coverage completed in `e1b7fbe`. The six release-gate regressions and all 93
  current `init_ci` integration tests pass. The earlier 44 flake-generator and
  16 renderer tests also passed. Concurrent Simit integration superseded the
  original uncommitted-generator handoff.
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

Both required registry versions are verified. Canix's authorized consumer slice
now uses registry Fleetix `0.2.0` and toolbelt `0.1.0`, inherited by
`canix-foundation` with toolbelt's default features disabled. The duplicated
runtime model/loader is replaced by a transparent default-path facade over
toolbelt; `/run/canix/runtime.pkl` remains Canix-owned.

Canix's migration is committed in `3dcccb5d2`. All 81 foundation and 84 command
tests pass, including the amended-schema and missing-file compatibility tests;
foundation Clippy is warning-free. A pinned release-only Canix snapshot preserves
both personal/admin command schemas, help, runtime JSON, and human/JSON missing-file
diagnostics exactly. Cargo resolves one Fleetix instance. The source-pin generator
removes the obsolete Fleetix Git hash.

Both actual production packages build through guarded Canix evaluation and
`cache binary realize --drv-output out --no-push`, with GC roots under
`.nix-results/`. Their packaged binaries pass the same pre-migration output
comparisons. The immutable source is
`/nix/store/lcajwmgmp81b70w7w0fysw7q32zvpmcp-source` (committed Canix `9ae4760a2`
plus the five migration files); realized outputs are:

- Admin: `/nix/store/0d96br91hp92zhx1y714jbihg57dv426-canix-admin-0.1.0`.
- Personal: `/nix/store/sbj04i420mc74y1q75zamwwyd7vj1r5a-canix-0.1.0`.

Concurrent Canix health registration depends on unpublished `fleetix::health`
APIs and blocks the shared-checkout admin build. The operator selected
release-only snapshot validation; that health work requires its own release.
The broad Canix library run also found a Git fixture rejected by the operator's
identity hook (`lfs_pointer_scan_uses_expected_commits_without_git_lfs`). Its test
and hooks are preserved; this is separate from runtime compatibility coverage.

Canix's `docs/src/design/cli-products.md` defines the three-product contract.
Generic deployment extraction is subsequent work; this release provides
runtime-manifest loading rather than a deployment CLI.
