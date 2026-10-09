# Native campaign coordination

The optional Unix `orchestration` library feature supplies restart-safe,
host-neutral primitives for existing agent campaigns. Consumers own the host
inventory, routes, credentials, model/permission policy, operator goals and
native adapters. Canix owns its fleet frontend. These APIs do not start workers,
fetch credentials or confer merge authority.

## Journal compatibility

`Manifest::validate` verifies complete assignment counts, distinct existing
owners, linked repairs and producer references. A refresh accepts additive
registered repairs and rejects changed ownership or lost assignments. Unknown
manifest fields and journal fields remain available to the consumer.

`refresh` works transactionally against the existing JSON journal. It preserves
pending request IDs/bodies, audit checkpoints, admission history and unknown
fields. Source/review/CI event hashes exclude poll timestamps and derived
progress. Terminal comparison hashes ignore unrelated later target/check
movement; edited historical feedback still invalidates the audit. Fingerprints
retain sorted ASCII JSON and legacy finite-number spelling.

Native adapters must retain the original journal bytes for import, supply
source-bound dependency digests and worker reports, and report unsuccessful
observations independently. Neither an idle session nor a waiting report proves
completion. `finished` requires complete idle host observations and exact audited
terminal coverage, including linked work and pending admissions.

## Admission and recovery

`Policy` declares host capacities, producer priority, explicit first-goal
expedites and ordering. `select` rejects incomplete/wrong-host activity and
chooses only original owners, preferring first current-goal turns before repeat
turns and oldest admissions within the repeat class.

`prepare` retains an existing pending body verbatim. Persist its result through
`atomic_json` before submitting it. On ambiguity, reconcile the exact ID against
both inbox and transcript; a transcript 404 can coexist with a queued input.
`acknowledge` requires the exact persisted ID and records delivery, not successful
execution or acceptance. OpenCode V2 models belong to registered sessions; a
prompt body cannot override the model. Verify owner identity/model/permissions
before native submission.

`record_idle_recovery` allows at most three bounded continuations for a source
without a usable checkpoint. `schedule_recheck` permits one bounded follow-up
per unchanged waiting report. Both use an injected clock. New source/evidence or
a new report can release further useful work.

`Lease` acquires the existing persistent coordinator lock without replacing or
unlinking its anchor. `atomic_json` uses private, fsynced, create-new staging and
an atomic rename. A consumer must scope its state directory and serialize
writers through the same lease, including the legacy coordinator during cutover.
Evaluation, activation and campaign leases remain distinct resources.

## Read-only shadow qualification

```sh
cargo run --locked --features orchestration --example orchestration_shadow -- \
  MANIFEST STATE CONTROL HANDOFFS
```

The example reads an existing manifest/journal and reports event-hash parity,
pending-body preservation and input digests. It neither acquires admission nor
submits inputs or writes journals. Supply a retained snapshot for a repeatable
qualification; live dependency handoff movement may legitimately change an
event. This is compatibility evidence, not a runtime cutover receipt.

Before a consumer enables the feature, qualify and publish the library through
the repository's normal release workflow and verify its registry artifact.
Host-aware transport, complete forge collection, evidence exchange and a
single-writer idle-boundary cutover need their own consumer qualification.

## Native driver and observation boundaries

`driver::cycle` checks all declared hosts, exact owner titles/models/permission
selectors, mirror execution and inboxes, queued-input capacity, and context
pressure before admission. Retain the original selectors in `Expectations`;
the adapter also verifies the effective policy of the selected agent. Prompt
receipts are checked against exact payloads in both inboxes and transcripts.
Journal publication precedes submission. Lost responses retain the same pending
ID and body. Read-only cycles neither publish nor submit.

Context compaction begins at 180k observed input/cache tokens, with a hard 300k
ceiling. Compaction IDs retain the legacy physical-message fingerprint and are
persisted before submission. Queued, running, completed and failed compactions
remain distinct; only a verified completed compaction releases continuation.
Physical usage is retrieved through V2's pre-pagination assistant/compaction
filters, bounded to four 64-message pages per type. Preserve message-list
`{data, cursor}` envelopes in `Reply.data` when pagination is possible. Missing,
malformed, overflowing or exhausted physical usage fails observation; it never
defaults to zero. Input, cache reads and cache writes all count. The actual
usage-bearing assistant ID is retained even when newer empty assistants or
completed compactions are present.
Use the cycle's `occupied` observation, including all queued inputs, in the final
`finished` check. Refresh again after mutations before claiming completion.

`snapshots` collects full GitHub/Forgejo history through explicit authenticated
transport traits. GitHub collection includes paginated issue comments, reviews,
outdated/resolved review threads, nested inline comments and check contexts.
Malformed/truncated pagination, repeated cursors, and comparison movement fail
collection. Consumers retain the last valid snapshot and record the error. These
observations provide scheduling evidence, never merge or closure authority.

`evidence` exchanges bounded byte-exact artifacts under explicitly declared owner
roots. It excludes symlinks, project checkouts and files over 8 MiB, and records
omitted/deferred artifacts separately. Scope and digests are validated before
application. Unexpected receiver edits remain conflicts; only acknowledged
digests advance the exchange journal. Replication targets are private mirrors of
the authoritative owner's paths, serialized by the shared exchange lease.
