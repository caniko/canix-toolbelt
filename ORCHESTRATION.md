# Native campaign coordination

The optional Unix `orchestration` library feature supplies restart-safe,
host-neutral primitives for existing agent campaigns. Consumers own the host
inventory, routes, credentials, model/permission policy, operator goals and
native adapters. Canix owns its fleet frontend. These APIs do not start workers,
fetch credentials or confer merge authority.

## Journal compatibility

`Manifest::validate` verifies complete assignment counts, distinct existing
owners, linked repairs, linked releases and producer references. A refresh accepts
additive registered linked work and rejects changed ownership, assignment
classification or lost assignments. `linkedReleasePRs` and `linkedReleasePRCount`
remain separate from baseline and supported-repair scope. `Manifest::coverage`
reports each class's counts and native state totals, including UNKNOWN evidence.
Unknown manifest fields and journal fields remain available to the consumer.
Declared producer IDs enter the event fingerprint before their first handoff, so
an additive producer declaration generates a useful new delivery immediately.

`refresh` works transactionally against the existing JSON journal. It preserves
pending request IDs/bodies, audit checkpoints, admission history and unknown
fields. Source/review/CI event hashes exclude poll timestamps and derived
progress. Terminal comparison hashes ignore unrelated later target/check
movement; edited historical feedback still invalidates the audit. Fingerprints
retain sorted ASCII JSON and legacy finite-number spelling.
The journal retains the validated manifest as `retainedManifest`; refreshes compare
against it across restarts. Initial imports reject missing already-journaled scope.
Worker progress and terminal audits require the delivered event and its retained
observation fingerprint to match current source, feedback and dependency evidence.
Legacy deliveries without an observation binding require a fresh turn before
certifying progress. A new source-bound audit may supersede stale coverage only
when its feedback fingerprint matches the current history.

Native adapters must retain the original journal bytes for import, supply
source-bound dependency digests and worker reports, and report unsuccessful
observations independently. Neither an idle session nor a waiting report proves
completion. `finished` requires complete idle host observations and exact audited
terminal coverage, including linked work and pending admissions.
An audited terminal PR with a `ready_for_work` checkpoint remains unfinished until
the resulting progress version has a durable delivery receipt. Pending inputs
retain `progressVersion`; acknowledgment records `deliveredProgressVersion`.

## Admission and recovery

`Policy` declares host capacities, producer priority, explicit first-goal
expedites and ordering. `select` rejects incomplete/wrong-host activity and
chooses only original owners, preferring first current-goal turns before repeat
turns and oldest admissions within the repeat class, ordered by parsed RFC 3339
instants rather than their spelling.

`prepare` retains an existing pending body verbatim. Persist its result through
`atomic_json` before submitting it. On ambiguity, reconcile the exact ID against
both inbox and transcript; a transcript 404 can coexist with a queued input.
`acknowledge` requires the exact persisted ID and records delivery, not successful
execution or acceptance. OpenCode V2 models belong to registered sessions; a
prompt body cannot override the model. Verify owner identity/model/permissions
before native submission.
Fresh inputs advance a durable `admissionGeneration`, so a reverted event cannot
reuse an already-delivered request ID. Existing pending IDs and bodies stay exact.

`record_idle_recovery` allows at most three bounded continuations for a source
without a usable checkpoint. `schedule_recheck` permits one bounded follow-up
per unchanged waiting report. Both use an injected clock. New source/evidence or
a new report can release further useful work.
Each POST attempt persists `pending.submissionAttemptAt` before submission, and
its receipt retains `deliveredSubmissionAttemptAt`. Idle recovery compares the
full instant against that actual attempt, falling back conservatively to
`deliveredAt` for historical receipts. Preparation time alone is insufficient.

`Lease` acquires the existing persistent coordinator lock without replacing or
unlinking its anchor. `atomic_json` uses private, fsynced, create-new staging and
an atomic rename. Collision-resistant staging tolerates abandoned legacy temporary
files without overwriting them. A consumer must scope its state directory and
serialize writers through the same lease, including the legacy coordinator during cutover.
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
Owner and mirror inboxes are revalidated before using a capacity observation;
duplicate receipt IDs reject observation. Reconciliation validates every pending
receipt before committing its candidate journal. Session selectors are checked
again immediately before prompt or compaction submission.

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
Assistant and compaction pages share an identity set; a message cannot alias both
kinds. Unknown pending-compaction statuses report malformed evidence while
retaining the pending request and preventing dispatch.
Use the cycle's `occupied` observation, including all queued inputs, in the final
`finished` check. Refresh again after mutations before claiming completion.

`snapshots` collects full GitHub/Forgejo history through explicit authenticated
transport traits. GitHub collection includes paginated issue comments, reviews,
outdated/resolved review threads, nested inline comments and check contexts.
All GitHub connections require stable declared counts; the single checked commit
must match the PR head. Forgejo inline counts and nonempty unique history IDs are
validated too. Both forges re-fetch the complete check rollup after collection.
GitHub's returned PR URL must match the requested coordinate; its URL and opaque
ID must remain stable through that final re-fetch.
GitHub revisions must be full 40-digit hexadecimal object IDs, and its update
identity must parse as RFC 3339 before history is fetched. Malformed check rollups
return errors. Forgejo accepts exactly 2,000 rows only after an empty sentinel
page; a nonempty or malformed sentinel fails the bounded observation.
Forgejo requires a valid retained update timestamp, and inline-comment identities
must remain unique across the combined review history.
Malformed/truncated pagination, repeated cursors, and comparison movement fail
collection. Consumers retain the last valid snapshot and record the error. These
observations provide scheduling evidence, never merge or closure authority.

`evidence` exchanges bounded byte-exact artifacts under explicitly declared owner
roots. It excludes symlinks, project checkouts and files over 8 MiB, and records
omitted/deferred artifacts separately. Scope and digests are validated before
application. Unexpected receiver edits remain conflicts; only acknowledged
digests advance the exchange journal. Replication targets are private mirrors of
the authoritative owner's paths, serialized by the shared exchange lease.
`evidence::apply` acquires the lease before receiver validation or mutation.
`evidence::Mirror::acquire` pins a mode-0700 root and locks that directory inode
exclusively, without waiting. Retain this guard around every authorized local
mutation and use `Mirror::apply` for exchanges within the same lease.
`Mirror::apply` requires an exclusive mutable borrow so sharing one guard cannot
admit simultaneous exchanges. Use a caller-owned mutex when sharing the retained
guard between threads. Legacy adapters must lock the same root inode before
editing, renaming or deleting its
contents. The active root itself must not be replaced. This contract serializes
cooperative writers; it supplies no inode-CAS guarantee against lease-bypassing
processes. Conflicts observed under the lease preserve receiver bytes.
Overlapping roots are normalized before traversal. Unchanged acknowledged paths
remain in `Batch.verify`, so receiver edits or deletions cannot evade comparison.
Source deletions enter `Batch.delete` and remove only the acknowledged receiver
bytes; replays are idempotent. Persist `Receipt.removed` and remove those exact
path/digest bindings from the sender's `known` map. A source file becoming a
directory uses two exchanges: defer descendants, remove the acknowledged receiver
file, retain that removal receipt, then transfer descendants once its old identity
is removed from `known`. A conflicting receiver file continues to block descendants.
Application independently
enforces 10,000 file mutations, 20,000 verification entries, 20,000 omitted/deferred
entries and the
48 MiB encoded-payload estimate. Receiver reads, directory creation, staging,
rename and deletion use pinned no-follow directory handles.
Allowlist roots must be nonempty scoped relative paths. Ancestor links are
fsynced before descending or writing, including existing links on a retry after
an interrupted exchange. Fsync failure prevents an acceptance receipt.
Source enumeration, artifact reads and deletion discovery also use pinned
no-follow handles; an ancestor replacement cannot redirect collection outside
the already-open authoritative directory.

Worker report URLs must be unique across `prs` and `linkedReleasePRs`; duplicate
entries reject the entire refresh without adopting partial progress or audits.
Terminal historical coverage retains its producer dependency fingerprint in
`auditedDependencies`. Changed dependencies invalidate completion and require a
fresh delivered report. Imported historical audits for packets with producers
must carry `dependencyVersion`, the canonical digest of
`{"producers": packet.dependencies, "evidence": current_dependency_map}`; old
feedback alone cannot absorb a new dependency event. Both forge collectors also
require globally unique inline-comment IDs; Forgejo requires nonempty base SHA
and branch identities before fetching history.
