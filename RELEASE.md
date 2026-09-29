# First Rust release handoff

Candidates: `fleetix 0.2.0`, then `canix-toolbelt 0.1.0`. This is preparation
evidence, not a published release. On 2026-09-30 the operator authorized uploads
and downstream synchronization, selecting **library/release changes only**.
Concurrent topology/health and cloud-host/public-edge work is outside this release.

## Release execution

- Fleetix's release-only tree is committed as `cce1be1` and pushed to GitHub.
  Its archive passes 66 all-feature tests, library-only tests, both Clippy feature
  configurations, warning-free rustdoc, treefmt, and the publication dry-run.
  GitHub CI must validate the Nix integration and named MSRV shell before tagging.
- Both GitHub repositories have the declared `CRATES_IO_API_TOKEN`. Full target
  resynchronization was retried twice but stopped at FIDO2 decryption with
  `libfido2 error 47`; neither retry wrote a remote credential. The earlier
  successful GitHub sync remains in place.
- Align toolbelt's registry dependency after Fleetix `0.2.0` appears on crates.io,
  rerun package/feature gates, then publish through the signed-tag workflow.
- Toolbelt's release-only archive passes all-feature and library-only tests,
  both Clippy feature configurations, warning-free rustdoc, and the publication
  dry-run. The named MSRV shell and all 36 x86_64 Nix checks evaluate through
  Canix's guard. Full treefmt exposed one existing helper normalization in
  `nix/treefmt.nix`, now included in the release gate repair.

The following sections retain the preparation evidence and outstanding producer
follow-ups; the execution record above supersedes the earlier handoff decision.

## Implemented and locally verified

- Public library with explicit-path sync/async runtime-manifest loading and
  typed, source-preserving errors. Registry-only dependencies, including
  published `fleetix 0.1.0`; no Canix or flake source dependency.
- Optional CLI using the same loader. A subprocess test succeeds with an empty
  executable search path, proving that manifest loading needs no external tool.
- The all-feature suite covers seven integration tests and one compiled
  documentation example. Five integration tests exercise the library without
  CLI features, including consumer-supplied Pkl schema defaults and typed entries.
- Clippy with all targets/features and denied warnings, warning-free rustdoc,
  treefmt, the release metadata audit, and `cargo audit` pass. Library-only
  Clippy also passes.
- Rust 1.88.0 `cargo check --all-targets --all-features --locked` passes with
  the existing local MSRV toolchain. Local stable/MSRV checks cleared inherited
  nightly-only `RUSTFLAGS` and `CARGO_ENCODED_RUSTFLAGS`.
- `cargo publish --dry-run --allow-dirty --all-features --locked` verifies the
  packaged crate successfully. `--allow-dirty` was used for local inspection
  only; the eventual release must use the coordinated release tree.
- `simit release trust check` confirms `keys/maintainers.gpg` matches the
  operator's configured signing key. No signing identity was substituted.
- The authorized local Simit generator created CI and crates.io publishing
  workflows while retaining the existing GitHub Pages environment. Its drift
  check passes. CI also requests warning-free documentation through
  `ci.extra_env.RUSTDOCFLAGS`.
- Simit's Nix and split-runner Actions renderers now honor the requested test and
  Clippy features and include the library-only pass. CI and publishers both select
  `.#msrv`, check its compiler identity, and compile all targets with the configured
  feature policy. Cargo-runtime MSRV gates install their exact toolchain first.
- `flake-modules/rust.nix` supplies toolbelt's MSRV environment through Harbor.
  It derives the compiler from Cargo metadata and clears inherited nightly flags.
  The new `harbor-rs` input follows the existing Fleetix tooling lock; it supplies
  compiler tooling only, never Rust dependency source.
- Fleetix's workflows were regenerated with the same local Simit. Its default
  environment now supplies repository-owned treefmt, covering Rust and Nix; its
  MSRV/stable environments clear inherited nightly flags. Both projects' workflow
  drift checks pass against this generator snapshot.
- The latest Fleetix all-feature run passes 76 tests, including the concurrently
  developed health publisher and service-profile tests. This verifies the shared
  snapshot, not an isolated release commit.
- Fleetix's final full `treefmt --ci` passes with zero changes. Scoped formatter
  checks for the Simit changes and toolbelt's Nix changes also pass, and all three
  checkouts pass `git diff --check`.
- GitHub `CRATES_IO_API_TOKEN` was synchronized from the declared encrypted source
  and verified by remote name listings on both repositories. Toolbelt reports
  `2026-09-29T22:46:52Z`, Fleetix `2026-09-29T22:46:53Z` as update times. No
  plaintext credential was printed or written to a checkout.

The declared MSRV is 1.88, matching Fleetix. Preserve the actual MSRV and
dependency-audit gates in CI before publication.
The final registry probes returned HTTP 404 for both candidate versions; registry
publication and Canix's dependency migration remain pending.

## Blockers and recovery

1. **Release ownership:** establish a stable release scope with concurrent
   contributors before committing/tagging. Canix, Fleetix, toolbelt, and Simit
   have concurrent work. No release commit/tag/push was made for this candidate.
2. **Simit integration:** the coverage fixes are uncommitted in
   `src/render/ci.rs`, `src/render/flake.rs`, and `tests/ci_runtime_gates.rs`.
   Six new regression tests pass, including real split-job generation and
   executing the generated MSRV script against matching/mismatched fake compilers.
   All 44 `init_flake` integration tests and 16 flake-renderer unit tests pass.
   The broader `init_ci` suite passes 90 of 91: the existing
   `check_failure_hint_includes_effective_generation_flags` test expects explicit
   recovery flags, but the generator emits only the shorter saved-policy command.
   Coordinate that failure and the six dead-helper warnings in concurrently
   edited `src/commands/init_ci.rs` with its owner before releasing Simit.
3. **Runtime coupling:** Simit currently requires `ci.runtime = "nix"` when
   Pages is configured. Preserve Pages and Nix gates. This only selects a CI
   toolchain environment; Cargo consumers and crate archives are Nix-independent.
4. **Credential tooling follow-up:** Canix's `lib/secrets/PersonalPcEnv.pkl` declares
   GitHub destinations `caniko/fleetix` and `caniko/canix-toolbelt` on the
   existing `modde-crates-io-api-token` target; its sidecar was regenerated.
   Both GitHub destinations have now been applied and verified.
   `canix secret sync --check --target modde-crates-io-api-token` still panics
   on incompatible global/local Clap `verbose` argument types, before collecting
   the plan or contacting a forge. The successful route was guarded
   `canix repo eval .#secretSyncTargets`, followed by `secret-manager sync` with
   the evaluated JSON and the two declared GitHub destinations. The target's
   pre-existing Codeberg user destination has expired OAuth credentials and
   aborted the first full-target attempt before any GitHub write. The successful
   GitHub-only run kept the canonical declaration and existing managed entries;
   it did not prune. Codeberg credential renewal is a separate follow-up. The installed
   `canix workspace release secrets check` also rejects GitHub origins as
   non-Codeberg URLs; it is not evidence of GitHub readiness.
5. **Nix integration:** the touched flake parses, but full Nix validation has
   not run. The final guarded toolbelt MSRV-shell evaluation exhausted a 300-second
   wait on `/run/lock/canix/nix-eval.lock`, held by an unrelated
   `cache binary build .#e2e-browser-bin` (PID 1182981 at observation time).
   The named Nix MSRV shell has therefore not yet been executed, despite the
   earlier direct Rust 1.88 check passing. Retry through the documented
   Canix evaluation/build routes after that operation finishes. Do not bypass
   the shared evaluation lock.

From this checkout, the authorized current-generator invocation is:

```sh
direnv exec . cargo run --manifest-path ../simit/Cargo.toml -- init ci --platform github --check --diff
simit release trust check
```

The earlier permission/toolchain obstacle to running the local generator was
resolved. The installed older Simit would remove the Pages environment; keep
using the current source or an updated binary. Generate changes by omitting
`--check --diff`, then repeat the check. The Simit checkout itself is receiving
concurrent changes, so refresh its state and coordinate any source fixes.

Inspect hook output before installing: the current hooks-only template expects a
`pkgs.rust-bin` overlay which this flake's ordinary package set does not declare
(the MSRV module applies it to its own package set). Preserve treefmt as
the formatting route and wire a real MSRV toolchain rather than installing an
unusable hook. Verify library-only and CLI-enabled configurations in generated
CI; plain default-feature tests do not exercise the optional CLI.

### Validation/release-owner handoff

The task-specific Simit changes are the feature/MSRV renderer fixes, generated
MSRV-shell support, their new regression file, adjusted expectations in
`tests/init_ci.rs`, and the MSRV contract in
`docs/src/getting-started/ci-adoption.md` plus its changelog entries. Other Simit refactors are concurrent
work and need separate ownership review. Existing public `cross_template` callers
retain their original signature.

For Fleetix, review the candidate metadata/docs, generated workflows,
`nix/treefmt.nix`, formatter/dev-shell wiring, and the follows-only lock addition.
The first full treefmt pass also exposed pre-existing formatting drift; include
only reviewed formatter deltas with the formatter rollout. Coordinate the new
topology/health implementation before choosing the exact release snapshot.

For toolbelt, review the Rust candidate, trust root, generated workflows, and
MSRV module/input. The lockfile's existing Disko changes and the cloud-host,
public-edge, and Gatus modules belong to concurrent work. Intent-to-add was used
only to expose this task's new Cargo/MSRV/formatter files to Git-backed Nix;
there are no task-authored commits, tags, pushes, or crate uploads.

For the coordinated Canix migration, release the required Fleetix APIs first,
then align this candidate's Fleetix version and lockfile to that supported series
and rerun the package/feature gates. The current `0.1.0` dependency proves the
loader works from an already published registry crate; do not introduce two
Fleetix series into Canix just to bypass the prerequisite release.

Run the release quality gates, inspect the exact package listing, and publish via
the Simit-generated crates.io workflow using a signed `0.1.0` tag. Verify the
actual version on crates.io before migrating Canix. Broader multi-channel release
secret checks may request minisign inputs; that does not substitute for checking
the actual crates.io publisher contract.

## Downstream migration

Canix still owns its current runtime loader pending registry publication. Then:

1. Add a versioned `canix-toolbelt` workspace dependency with default features
   disabled, inherited by `canix-foundation`.
2. Replace its runtime types/loading with this library, preserving the
   `/run/canix/runtime.pkl` default and existing `RuntimeManifest::load()` facade,
   JSON shape, error behavior, and role gates. Keep the amended-Pkl schema test.
3. Replace Canix's Fleetix Git pin with the registry version aligned during the
   prerequisite release above; verify a single compatible Fleetix series in the
   consumer graph.
4. Update Cargo.lock and obsolete Git hashes together; do not inject source
   through Nix or commit path patches to bridge an unpublished crate.

The full three-product contract lives in Canix's
`docs/src/design/cli-products.md`; generic deployment extraction is subsequent
work and is not implemented by this runtime-manifest slice.
