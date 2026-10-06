# Changelog

All notable Rust library and CLI changes are documented here. Nix module history
is recorded in Git. This file follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Fixed

- Require independently verified full file coverage before publishing a complete
  Roborev review receipt; unknown coverage and excluded files cannot qualify.
- Preserve the original daemon job ID/UUID across dispatch and receipt recovery;
  numeric-ID-only journals require explicit reconciliation without replay.
- Prepare namespace admission for the mandatory hosted Roborev worker tests.

### Changed

- Roborev dispatches carry `expected_files`, and runner enqueue replies return
  `RoborevJobIdentity`. Saved-review construction consumes both frozen contracts.

## [0.3.0] - 2026-10-06

### Added

- Roborev canonical receipt, request authorization, bounded Unix transport and
  write-ahead dispatch/publication APIs, with complete exact-comparison evidence.
- Direct-daemon local review provider shared with the OpenCode client, preserving
  captured working-tree diffs and legacy UNKNOWN submission journals.
- Independent Linux Roborev worker package for preparation, admission, execution,
  retained completion and mandatory offline-boundary verification.
- Reusable Roborev Nix package, Home Manager modules, flake composition and
  qualification fixtures, extracted from Canix.
- Durable systemd-stage operator with an optional Unix CLI, execution-contract
  binding, kernel locking, synchronized checkpoints, bounded retries and worker
  restoration. Interrupted runs reject a changed package/argument contract and
  preserve legacy shell state for explicit reconciliation.

### Fixed

- Reject truncated HTTP chunk terminators/trailers even when the received JSON
  is valid; enforce a single deadline across local daemon reads and writes.
- Retain earlier review attempts across comparison changes and preserve UNKNOWN
  outcomes during legacy journal migration instead of granting replay.
- Start cold systemd stage units without resetting nonexistent failed state,
  retaining bounded retries for missing units and service-manager errors.

### Changed

- `Review` gains optional provider job/review and actual compared-base identities;
  `GitHub::publish_check` requires an explicit dedicated-App policy.
- Native merge checks remain mandatory; provider review is selected explicitly
  with `merge --require-review`. Review commands require consumer-owned policy.

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
[0.3.0]: https://github.com/caniko/canix-toolbelt/compare/0.2.0...0.3.0
[0.2.0]: https://github.com/caniko/canix-toolbelt/compare/0.1.1...0.2.0
