# Changelog

All notable Rust library and CLI changes are documented here. Nix module history
is recorded in Git. This file follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Fixed

- Retain empty provider-log archives from cancelled original producers with
  their terminal status and byte hashes; require actual log files for success
  and reject duplicate archive members for every conclusion.
- Bind hosted required CI gates to the selected source revision before running
  them, and retain generated README badges alongside the workflow preparation
  patch for complete declared-generator imports.

## [0.5.0] - 2026-10-07

### Changed

- Consume registry Fleetix 0.6.0 for native shared construction. Preserve the
  Toolbelt library paths and frontend return type through compatibility adapters
  while sharing Fleetix's exact service, connection and command types. Native
  execution and root ownership remain in Fleetix over nix-manager-core 0.3.0.
- Alias the existing NixOS option paths to Fleetix's service module, including
  retained-deployment staging, operator-bound policy, configurable private paths
  and preserved runtime-directory ownership through offline rollover.

### Fixed

- Match the Claude release manifest to nixpkgs' raw or Zstandard download
  recipe, keeping both the declared checksum and installation format compatible.

### Added

- Expose held registration, admission, bounded completion waiting, drain and
  activation-readiness commands through the existing `build-train` frontend.
- Reusable OpenCode V2 Claude subscription and Jev Home Manager integration,
  with a pinned Meridian backend, per-user Fleetix loopback endpoints and
  runtime credentials. Preserve per-session project context and external
  service ownership through the native plugin lifecycle.

## [0.4.1] - 2026-10-07

- Expose Fleetix's guarded offline policy rollover through the library and
  `build-train rollover`, validating both immutable service contracts and keeping
  builder and request-root ownership intact. Retain the old fence, request
  archives and historical cleanup without admitting old-policy work.
- Publish the immutable service configuration at `/etc/fleetix-train/service.json`
  so operators can retain the exact pre-activation contract for policy recovery.

## [0.4.0] - 2026-10-06

### Added

- Optional dependency-aware build-train adapter over Fleetix's coordinator and
  nix-manager-core's exact named-output frontier, with a builder-local CLI/service
  and deployment-generated discovery and immutable policy contracts.
- Request-reachable GC-root retention and archived retirement, sharing the exact
  evidence closure while excluding unused native sibling outputs and preserving
  dependency evidence when cached-parent restoration prunes source dispatch.
- Nix/Rust policy parity and service configuration fixtures, plus source-qualified
  real Nix lifecycle and signed-cache restore-only evidence. Production activation,
  resource admission and Atlas/Murph performance remain consumer gates.

## [0.3.0] - 2026-10-06

### Fixed

- Require independently verified full file coverage before publishing a complete
  Roborev review receipt; unknown coverage and excluded files cannot qualify.
- Preserve the original daemon job ID/UUID across dispatch and receipt recovery;
  numeric-ID-only journals require explicit reconciliation without replay.
- Prepare namespace admission for the mandatory hosted Roborev worker tests.
- Preserve the client comparison contract: advisory committed reviews still
  require a full actual base and the selected head; working-tree reviews require
  the captured `dirty` scope.
- Run frontend qualification scripts with native Node instead of tracing their
  processes through Bun's FHS runner, retaining every generation and asset check.
- Reject truncated HTTP chunk terminators/trailers even when the received JSON
  is valid; enforce a single deadline across local daemon reads and writes.
- Retain earlier review attempts across comparison changes and preserve UNKNOWN
  outcomes during legacy journal migration instead of granting replay.

### Changed

- Roborev dispatches carry `expected_files`, and runner enqueue replies return
  `RoborevJobIdentity`. Saved-review construction consumes both frozen contracts.
- `Review` gains optional provider job/review and actual compared-base identities;
  `GitHub::publish_check` requires an explicit dedicated-App policy.
- Review commands require consumer-owned policy.

### Added

- Roborev canonical receipt, request authorization, bounded Unix transport and
  write-ahead dispatch/publication APIs, with complete exact-comparison evidence.
- Direct-daemon local review provider shared with the OpenCode client, preserving
  captured working-tree diffs and legacy UNKNOWN submission journals.
- Independent Linux Roborev worker package for preparation, admission, execution,
  retained completion and mandatory offline-boundary verification.
- Reusable Roborev Nix package, Home Manager modules, flake composition and
  qualification fixtures, extracted from Canix.

## [0.2.1] - 2026-10-06

### Added

- Explicit roborev receipt collection, request-bound publication and durable
  dispatch through a provisioned Unix socket, with complete canonical findings.
- Durable systemd-stage operator with an optional Unix CLI, execution-contract
  binding, kernel locking, synchronized checkpoints, bounded retries and worker
  restoration. Interrupted runs reject a changed package/argument contract and
  preserve legacy shell state for explicit reconciliation.

### Fixed

- Start cold systemd stage units without resetting nonexistent failed state,
  retaining bounded retries for missing units and service-manager errors.
- Retain CI qualification when branch protection requires only the review-policy
  check; failed, pending, cancelled, skipped or missing CI still blocks merging.
- Match App-bound required checks by both context and GitHub App identity, so
  unrelated advisory checks and legacy statuses cannot block satisfied requirements.
- Preserve comparison and policy history across review retries, and keep unknown
  submissions fenced when their journal is missing.
- Bind receipt publication to the original forge request so edited markers or
  changed evidence cannot open another publication lifetime.
- Provide Git for Nix-packaged review comparison tests.

### Changed

- Validate native CI and forge protection independently of optional provider
  review; `merge --require-review` retains the explicit provider acceptance gate.

## [0.2.0] - 2026-10-04

### Added

- Provider-neutral, revision-bound PR review contracts and durable request
  accounting, with an explicit Greptile GitHub-comment adapter.
- `review ensure`, read-only `review gate`, evidence-backed finding dispositions,
  and a separate protected, expected-head-bound `merge --apply` command.
- Complete parent-review findings, trusted writer request/disposition markers,
  bounded HTTP/polling, rate-limit checkpoints, and ambiguous-submission recovery.

### Fixed

- Replace the yanked `yoke-derive 0.8.3` lockfile entry with compatible `0.8.4`
  so locked crate verification and installation use a non-yanked dependency.

## [0.1.1] - 2026-10-02

### Changed

- Use published Fleetix `0.4` with explicit `pkl` support for the runtime loader,
  allowing consumers to share one Fleetix series with topology and GPU APIs.

### Fixed

- Supply a CA bundle to sandboxed Zola site builds so local-only site fixtures
  can initialize Zola's HTTP client.

## [0.1.0] - 2026-09-30

### Added

- Cargo-native runtime-manifest types and synchronous/asynchronous Pkl loading,
  extracted from Canix with explicit consumer-owned paths and typed errors.
- Optional `canix-toolbelt runtime show --path PATH` CLI using the same library.
- Simit-generated GitHub verification and crates.io publication workflows,
  release configuration, and exported maintainer trust root.
- Registry Fleetix `0.2` integration, allowing downstream consumers to share
  one supported Fleetix series with the runtime loader.

[Unreleased]: https://github.com/caniko/canix-toolbelt/compare/0.3.0...HEAD
[0.3.0]: https://github.com/caniko/canix-toolbelt/compare/0.2.1...0.3.0
[0.2.1]: https://github.com/caniko/canix-toolbelt/compare/0.2.0...0.2.1
[0.2.0]: https://github.com/caniko/canix-toolbelt/compare/0.1.1...0.2.0
