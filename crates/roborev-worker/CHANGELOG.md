# Changelog

## [Unreleased]

- Pin the stalled-copy fixture's child before termination and require bounded
  pidfd exit readiness, avoiding a race between tracer reaping and child exit
  while retaining the overall deadline and incomplete-request no-replay checks.

## [0.1.0] - 2026-10-06

- Extract Linux preparation, socket-associated peer-pidfd authentication,
  admission, execution fencing, retained completion and offline verification
  from Canix into an independent Toolbelt crate and `roborev-worker` helper.
- Preserve helper arguments, hash domains, journal schemas, persistent lock
  anchors and UNKNOWN no-replay outcomes. Existing attempts remain bound to
  their original immutable helper executable.
- Require original `Admission` custody for production execution reservations;
  expose standalone execution-fence mutation only to native fixture features.
- Retain preparation/resource, completion/durability, peer-lifetime and
  systemd/namespace regression fixtures as independently selectable features.
