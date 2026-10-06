# Changelog

All notable Rust library and CLI changes are documented here. Nix module history
is recorded in Git. This file follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added

- Purpose-specific Fleetix GPU routes for game rendering, browser media and mpv,
  with explicit consumer overrides and integrated Home Manager inheritance.
- Batched, deduplicated nix-direnv input GC roots whose successful retention
  precedes cache renewal.
- Opt-in OpenCode Muse Code subscription adapters for V1 and V2, with shared
  protocol fixtures and preserved legacy compatibility patches.
- A durable Rust systemd-stage operator with interruption checkpoints, bounded
  retry accounting and restoration of quiesced workers.
- A build-train construction adapter candidate composing Fleetix's coordinator
  with the native Nix backend. Registry dependency and locked-source
  qualification remain pending; this candidate is not a released interface.
- Provider-neutral, revision-bound PR review contracts and durable request
  accounting, with an explicit Greptile GitHub-comment adapter.
- `review ensure`, read-only `review gate`, evidence-backed finding dispositions,
  and a separate protected, expected-head-bound `merge --apply` command.
- Complete parent-review findings, trusted writer request/disposition markers,
  bounded HTTP/polling, rate-limit checkpoints, and ambiguous-submission recovery.

### Fixed

- Supply a CA bundle to sandboxed Zola site builds so local-only site fixtures
  can initialize Zola's HTTP client.

## [0.2.1] - 2026-10-06

### Fixed

- Retain CI qualification when branch protection requires only the review-policy
  check; failed, pending, cancelled, skipped or missing CI still blocks merging.
- Match App-bound required checks by both context and GitHub App identity, so
  unrelated advisory checks and legacy statuses cannot block satisfied requirements.
- Preserve comparison and policy history across review retries, and keep unknown
  submissions fenced when their journal is missing.
- Bind receipt publication to the original forge request so edited markers or
  changed evidence cannot open another publication lifetime.

## [0.1.0] - 2026-09-30

### Added

- Cargo-native runtime-manifest types and synchronous/asynchronous Pkl loading,
  extracted from Canix with explicit consumer-owned paths and typed errors.
- Optional `canix-toolbelt runtime show --path PATH` CLI using the same library.
- Simit-generated GitHub verification and crates.io publication workflows,
  release configuration, and exported maintainer trust root.
- Registry Fleetix `0.2` integration, allowing downstream consumers to share
  one supported Fleetix series with the runtime loader.
