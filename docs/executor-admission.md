# Live executor admission

Import `nixosModules.executor-admission` in a NixOS configuration with Home
Manager integration to connect an independently enabled worker to toolbelt's
live profiles and activation artifacts. The established profile module mirrors
its state into Home Manager:

```nix
canix-toolbelt.services.executorAdmission.secondary = {
  enable = true;
  profile = "near-builder";
  contract = "secondary-worker";
};
```

Declare the profile and the enabled owning activation contract separately.
The module generates a root-owned, read-only JSON file at
`/etc/executor-secondary-admission.json` containing
`{ "version": 1, "accepting": true }`. Its `accepting` value follows the live
profile, including specialisation overrides. Use `fileName` to select a stable
basename, and use the read-only `file` option in the worker's configuration.
An explicit `accepting = false` can close admission independently of the profile.
An explicit `accepting = true` cannot reopen a worker whose profile is disabled:
the generated policy composes profile eligibility and the manual switch with AND.

The file is an inspectable generated artifact of the named activation contract.
The module does not alter worker enablement, unit dependencies or restart triggers.
Switching the profile replaces the `/etc` policy while the worker continues to
serve existing work. The executor must reread this file before each **new**
reservation, fail closed on missing/invalid policy, and keep idempotent replay,
Stop and recovery available for already admitted ownership.

Fleetix owns endpoint/topology data and deployment. Consumers own host placement,
credentials, firewall scope, workload limits and rollout gates. This module owns
the reusable profile-to-admission/activation-artifact composition. It contains no
host names, endpoint inventory, credentials or application package dependencies.

`checks.<system>.executor-admission-eval` exercises all six combinations of
adjacent/detached and default/true/false manual admission in NixOS
graphs and verifies that worker configuration, restart triggers, dependencies
and ownership remain stable while the admission document changes.

Hosted qualification binds every required gate to the selected PR head (or
the event revision for push and release jobs) before evaluating or realizing
the source. Simit required gates own their setup inside `run`; they do not
inherit `[ci].extra_setup`. Keep that binding in each declared gate command.
Installable setup preserves the runner's initial checkout revision separately,
then asserts and records the exact executing HEAD and tree immediately before
realization. The retained `revision` and `tree` describe the selected source.

When changing `simit.toml`, import the complete generated patch from the
declared-generator preparation job, including README badges and workflows.
The preparation artifact contains the exact source, generator and maintainer
trust hashes, generated files and a member-hashed receipt. Preparation alone
does not qualify the successor; its own attempt-1 required gates must pass.

The declared generator is Simit
`0d917f8cbbf99bf271006a0cf6f590c34aa828bc` (`simit 0.19.0`, merged in #43).
Its original push/PR checks and exact-head production/regeneration run
`38065064067` passed. Independent provider-archive verification bound all 276
tracked source files, tree `318fbe76c8c9a4f646c3a77cd3be1cf19e110cd3`, and
generator SHA-256 `8acc30d1d8f1d291568b1634e4a029608176f0cc59cd9f005e994e73a5c8063d`.
The matrix explicitly selects `separate_events = true`: push and PR producers
must both succeed independently. Source preparation is followed by complete
generated-patch import and successor qualification; no preparation receipt or
cancelled predecessor grants acceptance.
