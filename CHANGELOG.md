# Changelog

All notable Rust library and CLI changes are documented here. Nix module history
is recorded in Git. This file follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added

- Durable systemd-stage operator with an optional Unix CLI, execution-contract
  binding, kernel locking, synchronized checkpoints, bounded retries and worker
  restoration. Interrupted runs reject a changed package/argument contract and
  preserve legacy shell state for explicit reconciliation.

### Fixed

- Start cold systemd stage units without resetting nonexistent failed state,
  retaining bounded retries for missing units and service-manager errors.

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

[Unreleased]: https://github.com/caniko/canix-toolbelt/compare/0.2.0...HEAD
[0.2.0]: https://github.com/caniko/canix-toolbelt/compare/0.1.1...0.2.0
