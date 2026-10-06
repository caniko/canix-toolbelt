# Shared construction

The build-train adapter composes Fleetix's builder-local coordinator with the
specialist `nix-manager-core` native frontier. Toolbelt's `0.4.0` release candidate
adds this optional interface while preserving the published `0.3.0` review APIs.

The Cargo feature is `build-train`; the service executable additionally requires
`cli`. Fleetix `0.5.1` and nix-manager-core `0.3.0` are published registry
dependencies. Toolbelt's candidate has passed all-feature and library-only tests,
warnings-denied Clippy and rustdoc, and compilation of the packaged archive using
those registry dependencies. Local path-patched lifecycle fixtures separately
qualify their captured candidate sources.

Fleetix `0.5.1` preserves request-local terminal failures before importing store
evidence during worker completion and restart. Materialized outputs do not clear
a failed request; its own explicit retry and live admission remain required.

## Service and policy

`canix-toolbelt.services.buildTrain` receives an existing operator account, a
qualified feature-enabled package, a builder identity, and the consumer's resource
admission contract. It provisions a private same-UID socket, private persistent
state and preparation directories, and request-scoped direct GC roots. The
coordinator memory ceiling does not replace Nix daemon build-resource admission.

Build workers and graph planners have independent capacity. Native queries use a
short deadline; realization has a separate worker deadline. The planning reply
deadline must cover two native queries and their termination grace periods.

The immutable policy digest uses versioned positional JSON. It binds backend
tools, system, store-root namespace, substitution policy, worker and planner
limits, queue capacity, aging, deadlines, socket/state locations, memory ceiling,
and admission identity. Nix/Rust parity is covered by the service fixture and is
independent of `serde_json/preserve_order` feature unification.

The service retains its original policy during a host activation. A changed
connection or persisted journal is rejected rather than altering queued work.
Drain old requests under their original coordinator before replacing its policy.
Protocol/journal version 2 rejects version 1 state without rewriting it.

## Request lifecycle

Library callers evaluate and freeze their source, then `Register` exact derivation
and named-output roots before their live preparation checks. Intake is durable and
held until `Admit`. Each caller retains its own attempt, checkpoints, cache
publication and activation stages. Automatic connection discovery uses a path
supplied by the caller and checks the selected builder identity. A missing default
permits standalone operation; malformed, explicit or mismatched contracts fail.

Planning does not block worker completion, status, cancellation or activation
fences. A deadline fails that request but retains a tardy planner's capacity and
roots until it exits. Explicit retry restores held admission; it does not bypass
the caller's safety checks. Disconnect detaches, while cancellation removes only
that request's pending interests.

After a request is terminal, explicit `Retire` archives its identity, outcome and
graph before removing train membership and releasing its private root namespace.
Status remains available from the archive. Retirement refuses live planners,
running interests and fence owners; root-release failures remain retryable across
restart. Other requests and caller-owned attempt roots retain independent pins.

Retention and retirement share the exact archived evidence closure: source
without a selector, requested derivations, and reachable derivations and outputs.
Unused sibling outputs present in native derivation JSON are excluded. Dependencies
of restored parents remain retained as evidence even when restoration prunes dispatch.
Unrelated entries, including correctly shaped store symlinks, preserve the whole
namespace for inspection. A dangling default connection symlink is an invalid
deployed contract and fails closed.
Retiring a newer activation never authorizes an older superseded request.

## Qualification status

The native service policy and private-root lifecycle have focused evaluation and
Rust regressions. A bounded real Nix fixture has also qualified cold dispatch,
late joining, shared-dependency priority, independent completion, cancellation and
held retry, restart, fence recovery, and archived request-local root release.
A signed local cache fixture also qualified actual NAR restoration, content
verification, dependency pruning, and cache-loss rejection with compilation
disabled for every realization.
Production resource admission, activation, Atlas/Murph behavior and measured
performance remain consumer qualification gates. Routine fleet enablement depends
on qualified upstream publication and registry-only consumption.
