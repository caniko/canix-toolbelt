# canix-toolbelt vision

Reusable, host-agnostic building blocks extracted from Canix, plus one Cargo
library and CLI that other Rust projects consume as crates rather than through
Nix source injection. Two output families: `nixosModules.*` / `flakeModules.*` /
`homeModules.*` and `lib.mk*Site` helpers for Nix users, and the
`canix-toolbelt` crate for runtime-manifest loading, PR review/merge, campaign
orchestration, qualification evidence and build-train coordination. Users are
other NixOS fleets and Rust projects - including Canix itself - that want the
stable parts without the host and secret coupling.

## Problem

Good Nix modules and Rust helpers accrete inside one fleet repository until they
are impossible to reuse: they read host names, secrets, or a private topology,
and consumers copy them. Separately, a review/merge or qualification workflow
implemented as shell glue cannot be tested or reused. The toolbelt keeps the
low-churn pieces here with explicit inputs, and publishes the operational logic
as a typed library with a thin CLI.

## Principles

- Extraction means low churn, no host/secret coupling, and usefulness outside
  Canix. If a piece needs the fleet's identity, it stays in the fleet.
- Reuse libraries, not source: consumers depend on the crate and its features;
  no toolbelt subprocess or Canix dependency is required for the library paths.
- Shared types are re-exported, not re-implemented. `build-train` preserves
  Fleetix's `Connection`, `Service`, `serve`, `rollover` and `policy_identity`
  as re-exports of the exact public types over `nix-manager-core`.
- Policy is explicit and consumer-owned. Review policy comes from `--policy`,
  `CANIX_REVIEW_POLICY`, or a user-owned `review.pkl`; a missing policy is an
  error and `Policy::default()` fails validation. PR source branches never
  supply policy, and credentials never enter receipts.
- Honest, resumable state. Repeated `review ensure` resumes the same durable
  request; an ambiguous submission is reconciled and never replayed; only
  `ready` exits zero.
- Findings block until fixed in a freshly reviewed revision or dispositioned
  with source evidence; provider-addressed metadata cannot suppress them.
- No silent fallbacks at boundaries: unsupported forges/transports fail
  explicitly, credentials stay in declared environment or the existing `gh`
  owner, and native modes fail closed rather than degrade.
- Modules state their recovery boundary: the cloud-host profile keeps `/nix`,
  `/var/lib`, `/var/log`, `/etc/ssh` and snapshots on separate subvolumes, and
  count limits are documented as limits, not backups.

## Scope

- NixOS modules: CPU microcode, GPU vendor stacks and GPU routing, EFI/btrfs,
  pipewire, fwupd, boot assessment, hardware watchdog, cloud-host profile,
  build-train service.
- Home Manager modules: pinentry routing, OpenCode Claude subscriptions,
  modde/mpv GPU adapters.
- flake-parts modules for `treefmt-nix`, git-hooks, host-output selection and a
  ripgrep-based structure/boundary check; `lib.mk*Site` site plumbing.
- Rust library/CLI: runtime manifest, `review` (Forge/Provider contracts,
  GitHub/Roborev adapters), `orchestration` (journal, admission, recovery),
  `qualification` (inventories, receipts, Cargo graph comparison, sealed
  handoff), `build-train`, `direct-network`, execution protocol.
- Schemas and generated wire contracts (`runtime/ReviewPolicy.pkl`,
  `EXECUTION_PROTOCOL.md`) with generation scripts kept in the package.

## Non-goals

- A second host registry. The cloud-host module composes with the consumer's
  existing `nixosSystem` builder and leaves NixOS options as the interface.
- Starting workers, fetching credentials or conferring merge authority.
  Orchestration keeps journals and admission policy; the consumer owns hosts,
  routes, models and permissions.
- Hiding an unattempted provider behind a fallback. Retired adapters (Greptile)
  are removed, not kept as implicit defaults.
- A global GPU switch such as `DRI_PRIME`; roles stay independent and explicit
  settings win.

## Direction

Unreleased work is the qualification surface: source-bound inventories, bounded
execution receipts, legacy evidence verification, deterministic source export
and composition-bound handoff sealing, with directory symlinks preserved
byte-for-byte. Orchestration keeps adding bounded recovery and complete forge
history. The trend is replacing scratch evidence helpers with typed Rust
operations, while the 0.5.0 line moved shared construction onto registry Fleetix.

## Place in the fleet

Canix consumes it as a flake input (`flake.nix`) and through
`homeModules`/`nixosModules` in `root/profiles/*`, `root/user/can/*` and
`home/user/shared/skillnet.nix`; `pink-raven` pins its own canix-toolbelt input
for the published execution-contract API.
`docs/src/operations/repository-review.md` points operators at the
`canix-toolbelt` review and merge commands.
