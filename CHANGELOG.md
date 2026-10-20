# Changelog

All notable Rust library and CLI changes are documented here. Nix module history
is recorded in Git. This file follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Fixed

- Keep the source-owned custody policy assertion aligned with the qualified
  complete-job-log observer revision.
- Verify complete original job logs when GitHub omits or restores redundant
  numeric step-file copies in run-log ZIPs, preserving exact job identity,
  aggregate coverage, shared content and historical failed custody packets.
- Retain original hosted producers through transient GitHub API failures with
  Simit's qualified native observer, complete job enrollment and independent
  uploaded-artifact verification; preserve historical failed custody packets.
- Use published Pklx 0.2 for embedded manifest and review-policy evaluation,
  removing the vulnerable ZIP 8 dependency while preserving typed loading and
  rejection of synchronous loading inside a Tokio runtime.
- Preserve Git-tracked directory symlinks in source inventories and deterministic
  exports. Version 2 binds link text and executable mode while retaining version 1
  evidence verification and rejecting omitted or recursive archive link targets.
- Resolve rustls 0.23.46 to address the TLS advisories disclosed on 2026-10-14.
- Reject explicit TTL-propagation attributes in eligible direct-route snapshots
  before effects; preserve omission of ineligible paths. This defensive boundary
  is tested with controlled snapshots, not unsupported native IP-route fixtures.
- Reject unsupported route/nexthop destination rewrites and authored `pervasive`
  flags before direct replay can drop forwarding policy. Preserve kernel status
  flags, authored `onlink`, and omission of VPN/container-only and mixed paths.
- Reject expiration on eligible direct routes before replay can make them
  permanent or extend their lifetime during reconciliation. Preserve omission
  of VPN/container-only and mixed routes.
- Require prepared immutable service images with readiness bind targets for
  inline admission. `RootDirectoryStartOnly=true` remains an authored host-control
  opt-in, not an automatic fallback for unprepared images.

### Added

- Reusable source-bound Rust qualification operations and an independent
  `qualification` CLI: inventories, bounded execution receipts, legacy evidence
  verification, Cargo graph comparison, deterministic source export and
  composition-bound handoff sealing.
## [0.5.0] - 2026-10-11

### Changed

- Consume registry Fleetix 0.6.0 for native shared construction. Preserve the
  Toolbelt library paths and frontend return type through compatibility adapters
  while sharing Fleetix's exact service, connection and command types. Native
  execution and root ownership remain in Fleetix over nix-manager-core 0.3.0.
- Alias the existing NixOS option paths to Fleetix's service module, including
  retained-deployment staging, operator-bound policy, configurable private paths
  and preserved runtime-directory ownership through offline rollover.

### Fixed

- Keep trusted publisher verification on the admitted signed source rather than
  the workflow's default-branch revision. Permit signed-tag and trusted-publisher
  recovery while retaining original-attempt-only branch/PR qualification.
- Run the direct resolver under a dedicated unprivileged account with no
  capabilities; bind readiness and reload receipts to that account's complete
  credential evidence as well as MainPID, cgroup and all four owned listeners.
- Explicitly install root-owned readable, non-resolver-writable servers files,
  including replacement of restrictive stale temporary files before DNS reload.
- Compose rooted admission drop-ins through the native systemd package tree and
  retain completed oneshot payload invocation identities in kernel VM evidence.
- Mount the immutable ext4 readiness fixture read-only without journal replay,
  and retain bounded native image/dissection diagnostics on startup refusal.
- Keep disposable foreign DNS peers alive through both service-start refusal
  checks, with bounded lifetime and active-peer assertions before cleanup.
- Retry malformed pinentry pane observations within the original deadline,
  retaining decode diagnostics and refusing to treat unreadable state as cleanup.
- Reject ECMP groups above the bounded iproute2 replay capacity and unsupported
  cross-family gateways before any forwarding, firewall or DNS changes.
- Keep rooted-service readiness in the payload's pre-start lifecycle, with
  read-only client-closure and live socket-directory binds. Preserve existing
  chrooted hooks, credentials, resource policy and automatic retries through
  admission denial and recovery.
- Preserve system and Home Manager readiness after the final pre-start command
  reset, and supply rooted Home Manager units with the same narrow read-only
  client-closure and live socket-directory binds as system services.
- Reject unsupported direct-route path metrics before replay instead of dropping
  MTU, MSS or congestion-control policy, including on ECMP routes.
- Stream resolver socket evidence with bounded rows and constant retained state,
  allowing busy host socket tables while preserving duplicate-listener rejection.
- Reject unsupported route and nexthop realms before replay, preserving producer
  traffic-control classification instead of silently dropping it.
- Apply the ECMP replay capacity only after complete direct eligibility, so large
  VPN/container-only and mixed groups remain omitted without denying readiness.
- Reject unsupported TOS selectors and non-default IPv6 route preferences before
  replay can widen traffic matching or change direct-uplink selection. Preserve
  ordinary canonical medium-preference admission.
- Reject source-specific selectors, compatibility-expanded nexthop IDs and
  non-default ECMP scopes before copying can discard source constraints, resilient
  group policy or authored scope. Keep preferred-source addresses and ordinary
  single-path connected scopes supported.
- Retain canonical single-path scope in the selected route and replay it explicitly,
  including global scope omitted by iproute2's JSON printer. Expose typed
  `RouteScope` in `Route` so scope-only changes also invalidate its snapshot.
- Match the Claude release manifest to nixpkgs' raw or Zstandard download
  recipe, keeping both the declared checksum and installation format compatible.
- Require native owner/host observation fencing through dispatch; unsupported
  adapters fail before reads or effects, and lost authority invalidates an idle
  context witness. Reject unbound imported pending evidence exchanges and keep
  source-disappearance races out of retained contradictory batches.
- Retain empty provider-log archives from cancelled original producers with
  their terminal status and byte hashes; require actual log files for success
  and reject duplicate archive members for every conclusion.
- Bind hosted required CI gates to the selected source revision before running
  them, and retain generated README badges alongside the workflow preparation
  patch for complete declared-generator imports.
- Keep direct-network routing ownership explicit in both reserved tables and
  address families, preserve foreign routes and complete eligible ECMP paths,
  and reject container-only implicit uplinks and noncanonical cgroup depths.
- Reject unsupported encapsulation on eligible routes and ECMP nexthops before
  routing, firewall or DNS updates instead of silently changing forwarding.
- Retain IPv6 same-prefix, same-metric routes across multiple uplinks without
  replacing foreign routes or blocking the readiness barrier.
- Explicitly delete retained owned IPv6 unreachable fallbacks before exclusive
  reinstallation, and emit the resolver unit's empty capability reset so its
  native namespace and descriptor ownership can be authenticated.
- Preserve conservative autoSafe/deny overlap rejection for store wrappers;
  backend wildcards can consume slashes and command arguments before another
  executable suffix, even when the declared executable names differ.
- Discover ordinary VLAN uplinks through iproute2's named parent field; reject
  foreign-namespace index collisions and retain VLAN route and DNS coverage.
- Make the unsafe-object preparation fixtures tolerate existing Git object
  fanout directories while preserving rejection and persistent tombstone checks.
- Authenticate all independent DNS listeners against the native resolver unit,
  process, network namespace and enrolled cgroup generation. Reject foreign,
  connected, wildcard and IPv4-mapped competing sockets. Scope NetworkManager
  link-local DNS and retry failed generation-bound reload dispatches without
  restarting enrolled workloads.
- Require authenticated readiness for anchored and unanchored guarded launches;
  retain routing-only coverage in the disposable namespace probe. Retry transient
  pinentry fixture observations within the existing deadline while retaining
  permanent failure diagnostics and separate concurrent prompt assertions.

### Added

- Provider-free execution wire types and pure validators generated for Rust and
  TypeScript from one versioned schema. Bind discovery to exact implementation,
  configuration and credential-reference identities; preserve u64 counters as
  strings and reject unavailable guarantees before execution. Ship shared
  canonical-byte/error-code fixtures and a dedicated hosted conformance check.
- Host-neutral native campaign journal, admission and bounded recovery primitives
  behind the `orchestration` feature, including native input reconciliation,
  complete forge-history collection, evidence replication, bounded context
  compaction and a read-only legacy shadow probe.
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
