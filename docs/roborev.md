# Roborev ownership and consumption

Toolbelt owns the reusable Roborev package recipe, Home Manager program/service
modules, daemon transport, durable dispatch/publication contracts and Linux
worker. The standalone `opencode-repo-review-pr` client owns OpenCode commands and
presentation. Consumers supply credentials, trusted policy, executable paths,
resource values, fleet placement and qualified dependency pins.

## Nix interfaces

- `homeModules.roborev` and `homeManagerModules.roborev` export the generic module.
  Set `programs.roborev.package` and approved `agentCommands` explicitly. Both
  the program and service default to disabled. The module has no `osConfig`
  dependency and does not enroll repositories, install hooks or migrate state.
- `lib.mkRoborevPackage { pkgs; harborGo; harborJs; buildPkgs ? pkgs.buildPackages; }`
  builds stock Roborev v0.71.0 with the selected Go/Bun helpers. Consumer package
  sets remain explicit, including cross-build native tools.
- `lib.mkRoborevFlakeModule { nixpkgs; harborGo; harborJs; harborMeta; homeManager; }`
  composes the package, frontend/dependency artifacts, development shell and
  module/unit regression checks. `flakeModules.roborev` selects those arguments
  from conventional consumer inputs. Toolbelt qualifies its own composition too.
- `lib.roborevTests` exposes the reusable evaluation/activation/unit/switch fixtures.
  Runtime scripts under `tests/roborev/` use disposable homes and isolated
  namespaces. Set `ROBOREV_TEST_TMPDIR` when a short fixture root is required.

## Rust interfaces

Enable `canix-toolbelt`'s `review` feature. `RoborevUnix` implements the bounded
daemon transport with a mandatory controller checkout verifier.
`RoborevHttp` shares that transport with `RoborevLocal`, which preserves the
standalone client's direct-daemon journals and captured working-tree recovery.
Local committed/working-tree scopes remain advisory; their actual compared base
is retained and cannot qualify a different PR through dedicated-App publication.
`dispatch_roborev_once` records UNKNOWN before enqueue and reconciles the original
request-exclusive job without replay. Persisted receipts must attest the exact
full `target-tip..head` comparison and complete findings; a Markdown verdict is
insufficient. `RoborevGitHub` authenticates receipts against the dedicated policy
App and fresh forge facts. Policy loading is explicit and never falls back to an
unselected provider.

`RoborevDispatch.expected_files` is the independently verified full comparison
census. The preparation callback returns `(checkout, agent, expected_files, runner)`;
the runner verifies the checkout and census on every dispatch. A complete receipt
requires persisted coverage with no excluded files and exactly that reviewed count.
`RoborevRunner::enqueue` returns `RoborevJobIdentity { id, uuid }`; dispatch journals
retain both identities and correlate every listing and saved review against them.
`RoborevReceipt::from_saved_review` accepts the frozen dispatch, policy, original
job identity and persisted review. Dispatch journal version 2 preserves UNKNOWN
recovery; older numeric-ID-only journals remain intact and require reconciliation
with their original implementation rather than acquiring UUID authority implicitly.

Version 0.3 adds fields to `Review` and binds `GitHub::publish_check` to an explicit
policy. Downstream struct literals must initialize `provider_job_id`,
`provider_review_id` and `reviewed_base`; older serialized records still decode.
Review history upgrades in place, retaining lock inodes and earlier request IDs.
An intact legacy attempt may resume, but incomplete historical state cannot grant
a new comparison's dispatch.

The independent `canix-toolbelt-roborev-worker` crate owns Linux preparation,
direct adapter authentication, admission, execution fencing, offline isolation and
live verification. Its `roborev-worker` executable implements the legacy private
helper arguments. See its packaged README for persistent compatibility and native
qualification inputs. The portable library retains `forbid(unsafe_code)`.
Production reservations require original `Admission` custody; standalone fence
registration/loading/reservation are available only to native fixture features.

## Qualification boundary

Package/module tests, native Linux worker fixtures and real-daemon protocol tests
are separate evidence. Offline fixtures do not qualify a production credential
broker, root-owned custody or live provider/forge execution. A consumer adopts
registry APIs only after verifying the released artifact, and Nix interfaces only
after exact-source producer CI passes. Existing UNKNOWN attempts are never
reset or replayed during migration.

The required native-worker CI job prepares namespace admission on its disposable
GitHub-hosted Ubuntu runner before running Nix-store Bubblewrap. This prerequisite
does not change the worker's confinement policy or skip isolation tests.
