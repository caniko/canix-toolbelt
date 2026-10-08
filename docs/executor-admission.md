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

`checks.<system>.executor-admission-eval` exercises adjacent and detached NixOS
graphs and verifies that worker configuration, restart triggers, dependencies
and ownership remain stable while the admission document changes.
