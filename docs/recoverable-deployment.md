# Recoverable deployment units

`nixosModules.recoverable-deployment` and
`homeModules.recoverable-deployment` install the fixed owner-side units for the
Fleetix deployment worker. Fleetix owns the Rust worker and its transaction
state machine; toolbelt owns the service, timer, path and policy paths. Fleet
inventory, allowed effects, trusted verifiers, health checks and application
recovery remain consumer policy.

The worker is opt-in. Every deployment is disabled by default and adds no
service, timer, path unit, state directory or assertion until `enable = true`.
`package` has no default: until Fleetix publishes a package revision containing
the `deployment-cli` feature, consumers must select an explicit package. There
is no silent fallback to an older Fleetix package.

## CLI contract

The installed policy is fixed at build time and passed as the only privileged
input:

```text
${package}/bin/fleetix-deploy run --policy POLICY --request REQUEST
${package}/bin/fleetix-deploy recover --policy POLICY
```

The request path is writable and untrusted. It can nominate a candidate, but it
cannot supply commands, privilege, health checks, an admission verifier or an
approval flag. The admission verifier and health command are mandatory and come
only from the installed policy. A request that is unknown, malformed or not
admitted fails closed.

## Policy schema

The rendered policy uses Fleetix's camelCase JSON schema:

| Key | Value |
| --- | --- |
| `version` | `1` |
| `target` | Deployment target slug |
| `backend` | `nixos` or `home-manager` |
| `stateRoot` | Absolute private persistent state directory |
| `lockPath` | Absolute shared activation-lock anchor |
| `profilePath` | `/nix/var/nix/profiles/system`, or the Home Manager profile |
| `currentSystemPath` | `/run/current-system`, or the Home Manager profile |
| `nixStore` | Absolute `nix-store` executable |
| `nixEnv` | Absolute `nix-env` executable |
| `admissionCommand` | Absolute trusted verifier argv |
| `healthCommand` | Absolute trusted health argv |
| `commandTimeoutSeconds` | `1..3600` |
| `healthTimeoutSeconds` | `1..3600` |
| `healthIntervalSeconds` | `1..3600` |

The policy contains no self-declared approval field. Admission is a trusted
verifier receipt bound to the plan, target, backend, candidate, observed
previous generations, request digest and expiry. The policy is a world-readable
store object: keep credentials in verifier- or health-owned files, never in
command argv.

NixOS fixes `profilePath = /nix/var/nix/profiles/system` and
`currentSystemPath = /run/current-system`. Home Manager fixes both to
`~/.local/state/nix/profiles/home-manager`, because the worker must observe the
same profile it restores.

## NixOS

```nix
services.recoverable-deployment.deployments.atlas = {
  enable = true;
  package = inputs.fleetix.packages.${pkgs.system}.fleetix-deploy;
  requestPath = "/var/lib/fleetix/requests/atlas.json";
  admissionCommand = ["${myAdmissionPackage}/bin/admit"];
  healthCommand = ["${myHealthPackage}/bin/check"];
};
```

`myAdmissionPackage` and `myHealthPackage` are consumer-supplied, qualified
packages, not permissive verifier stubs. Until Fleetix
publishes a revision with `fleetix-deploy` and the `deployment-cli` feature,
supply an explicitly built package; there is no old-package fallback.

Defaults:

- state root: `/var/lib/fleetix/deployment/<name>`, created `0700` by
  the owner-checking worker; the module never runs garbage collection, prunes old
  policy/worker roots or removes the persistent state directory;
- shared lock: `/run/lock/fleetix-deployment.lock`; set `lockPath` to the
  Canix legacy lock when other activation frontends already use it;
- command timeout: 30 seconds; health timeout: 30 seconds; health interval:
  5 seconds.

The run service is a target-owned `Type=oneshot` process. It is not a terminal
session and it is not restarted automatically. `KillMode=control-group` bounds
the whole activation process tree. The service keeps the ordinary root
environment needed for NixOS activation and Home Manager writes; this module
does not apply sandboxing that would block `/etc`, `/run`, `/nix` or profile
writes.

`TimeoutStartSec` is

```text
max(7200, 16 * commandTimeoutSeconds + 3 * healthTimeoutSeconds + 180)
```

so the worker has room for trial activation, health checks, commit and several
restoration command invocations. The floor preserves the original two-hour
activation budget for default policies.

Deployment attribute names are lowercase alphanumeric/hyphen slugs, without a
leading/trailing hyphen, and must not end in `-recover`, which is reserved for
the recovery unit. The unit set for an enabled deployment named `atlas` is:

- `recoverable-deployment-atlas.service` — run request;
- `recoverable-deployment-atlas-recover.service` — recover unsettled
  transactions;
- `recoverable-deployment-atlas.path` — optional `PathChanged` watch;
- `recoverable-deployment-atlas-recover.timer` — bounded periodic recovery.

The path unit is installed by default and can be disabled with
`watchRequest = false`. It observes changes rather than continuously restarting
while a request exists. Queued requests are also considered at startup, gated by
`ConditionPathExists`. Write requests atomically with a new attempt ID. The run
service remains available for an explicit start; requests are never generated
in the Nix store.

Only activation-lock contention (worker exit 75) schedules an automatic run
retry after 30 seconds. Recovery units accept 75 as a deferred scan, so expected
contention during activation does not mark a service failed; the periodic timer
retries recovery. Deferral does not certify restoration. Other recovery failures
still fail the unit. Denied, invalid, unsafe or restored requests do not loop.

Recovery is installed independently of the deploying terminal. The run service
has `OnFailure=recoverable-deployment-atlas-recover.service`, the recover
service is wanted by `multi-user.target` at boot, and the timer runs the same
recover service after `OnBootSec=30s` and every `OnUnitInactiveSec=60s`. The
timer exists because a boot or `OnFailure` attempt can lose activation-lock
contention; the next inactive interval retries the same fixed-policy worker.
The recover worker scans only unsettled journals, treats completed transactions
as no-ops and does not perform root mutations beyond retaining its own log. The
timer is a bounded oneshot trigger, not a long-running daemon.

The worker creates missing private state and lock anchors, without chmodding
existing directories or replacing shared anchors. Provision custom lock
parents through their owning configuration. Never remove an anchor to resolve
contention; every deployment frontend on the host must share the same anchor.

## Activation without sudo

For NixOS, explicitly name existing users on the same deployment policy:

```nix
services.recoverable-deployment.deployments.atlas.requestUsers = ["can"];
```

With `requestUsers` set, omit a custom `requestPath`: the module provisions
`/var/lib/fleetix-requests/atlas/request.json` in a root-owned, group-writable
inbox for exactly those users. It requires the fixed request watcher. Group
membership changes take effect in a new login session.

An authorized user publishes a complete versioned Fleetix request atomically;
the watcher starts the root worker automatically. For an already prepared
`request.json`, these Nu-compatible commands need no sudo:

```nu
let inbox = "/var/lib/fleetix-requests/atlas"
let pending = (^mktemp $"($inbox)/request.XXXXXXXX")
^install -m 0600 request.json $pending
^mv -T $pending $"($inbox)/request.json"
```

This grants **nomination**, not approval. The same installed admission command
must approve the exact candidate and captured generations; the same health,
retention and recovery rules apply. Users cannot change the policy, worker argv
or private journal. Outsiders cannot write the inbox. No sudo/polkit rule or
general `systemctl` activation permission is added. An empty `requestUsers`
list grants nothing and preserves administrator-provisioned request paths.

Home Manager already runs as its owner without sudo: publish the same request
to its configured home-directory inbox and let its user path unit start the
fixed-policy user worker.

## Home Manager

```nix
services.recoverable-deployment.deployments.atlas = {
  enable = true;
  package = inputs.fleetix.packages.${pkgs.system}.fleetix-deploy;
  requestPath = "${config.home.homeDirectory}/.local/state/fleetix-deployment/requests/atlas.json";
  admissionCommand = ["${myAdmissionPackage}/bin/admit"];
  healthCommand = ["${myHealthPackage}/bin/check"];
};
```

State and lock paths default below
`~/.local/state/fleetix-deployment/`. The worker creates missing state directories
with mode `0700` and refuses unsafe existing ownership or modes. No Home Manager
activation hook chmods existing paths. The module asserts that state, lock and request paths
stay inside the user's home directory; it never writes a root-owned path from a
user activation.

User units use the systemd default target, not a graphical target:

- `recoverable-deployment-atlas.service` has
  `OnFailure=recoverable-deployment-atlas-recover.service`;
- `recoverable-deployment-atlas-recover.service` is wanted by `default.target`;
- `recoverable-deployment-atlas-recover.timer` is wanted by `timers.target` and
  uses the same 30-second boot and 60-second inactive interval;
- `recoverable-deployment-atlas.path` is wanted by `paths.target` when
  `watchRequest = true`.

The Home Manager backend is for standalone Home Manager profiles that observe
`~/.local/state/nix/profiles/home-manager`. Do not enable it for an integrated
Home Manager profile whose live profile is a NixOS-managed per-user profile; use
the NixOS backend or a separately reviewed policy instead.

## Authority and safe eligibility

This module installs units; it does not decide that a candidate is safe. The
consumer's trusted admission verifier must inspect the actual candidate and
approve only configuration-only effects that are compatible with the previous
generation. At minimum it must verify:

- the candidate's effects are configuration-only, with no acknowledged write
  loss and no unknown or consequential hooks;
- the schema and rollback path are compatible with the installed worker and
  previous generation;
- the previous version can still use any state the new version leaves behind;
- restoration targets are retained and observable, and the health command
  covers the deployed service and its dependencies.

Generation rollback restores managed configuration, not arbitrary data. A
backup is not permission to discard acknowledged writes. Database upgrades,
destructive migrations, secret rotation and changes to the recovery or
authorization substrate require a separate qualified maintenance path and must
be refused by the verifier. Unknown effects and migrations are refused by
default. If the observer or verifier is unavailable, admission stays closed.

The module deliberately adds no passwordless sudo rule, polkit action, setuid
helper, `User=`/`DynamicUser=` switch or caller-selected command. NixOS units run
as root because activation needs root; Home Manager units run as the user.
Privilege boundary changes are outside this module.

## Evaluation test

`nixos-tests/recoverable-deployment-eval.nix` is a pure evaluation test. It uses
dummy packages and shell stubs only to make command paths concrete; it never
starts the worker, activates a system or writes host state. It covers:

- disabled default and no units or directories;
- mandatory explicit package, admission verifier and health command;
- fixed policy arguments and backend/profile/current bindings;
- request path watch on and off;
- the fixed lock default, legacy lock customization and anchor preservation;
- root `OnFailure`, boot recover service and periodic timer linkage;
- the Home Manager activation, default-target user units and timer equivalents;
- absence of sudo or polkit escalation.

Wire it into the consumer's checks with the test's `{inputs, pkgs}` arguments.
It is not a live-worker or live-activation qualification.
