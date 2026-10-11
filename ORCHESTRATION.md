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
Legacy imports with retained packet entries also require an explicit complete
assignment-to-packet mapping and matching `sessionID` / `ownerHost` in those
entries. If the legacy journal lacks that evidence, the consumer must retain its
independently verified historical manifest as `retainedManifest` under the shared
writer lease before refresh. Never derive historical ownership from the proposed
replacement manifest. Rejected imports leave every pending body and checkpoint
unchanged; matching imports preserve unknown fields and exact pending inputs.
Assignment and producer sets are sorted by stable identity before event hashing;
reordering the manifest alone does not create a turn or expire dependency audits.
Previously persisted request IDs and bodies still retain their original hashes.
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
Dependency-bearing completion additionally requires a refreshed matching manifest,
the current retained dependency fingerprint and per-PR dependency-audit coverage.
A newly added producer cannot certify completion against a pre-refresh journal.
An audited terminal PR with a `ready_for_work` checkpoint remains unfinished until
the resulting progress version has a durable delivery receipt. Pending inputs
retain `progressVersion`; acknowledgment records `deliveredProgressVersion`.
A waiting checkpoint retains its follow-up obligation across refreshes, restarts
and delivery of the bounded recheck. An old non-waiting report cannot release it;
only a supported non-waiting report bound to the current delivery and source can
permit completion. Existing source-bound audits remain retained.
Unsupported or missing report statuses reject refresh transactionally before
adopting any terminal audit evidence.
Stale reports cannot replace the retained current waiting checkpoint. When a
waiting report omits its own timestamp, its bounded recheck delay starts at the
delivery receipt, so a long-delayed pending submission does not shorten the wait.

## Admission and recovery

`Policy` declares host capacities, producer priority, explicit first-goal
expedites and ordering. `select` rejects incomplete/wrong-host activity and
chooses only original owners, preferring first current-goal turns before repeat
turns and oldest admissions within the repeat class, ordered by parsed RFC 3339
instants rather than their spelling.
Unknown admission times sort after valid repeat admissions. A malformed preparation
timestamp does not hide a valid delivery timestamp used as the fallback.

`prepare` retains an existing pending body verbatim. Persist its result through
`atomic_json` before submitting it. On ambiguity, reconcile the exact ID against
both inbox and transcript; a transcript 404 can coexist with a queued input.
Retained pending prompts require nonempty event version, goal-policy and preparation
metadata before any observation, receipt reconciliation or dispatch. Optional legacy
observation/progress bindings remain nullable; present event metadata in the body
must match the retained version. Malformed imports preserve the original journal
and fail before provider effects rather than losing the event on acknowledgment.
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
ceiling. New compaction IDs hash a versioned tuple of session and physical message
IDs, so distinct field boundaries cannot alias. Retained pending IDs and bodies,
including legacy IDs, are retried verbatim. Requests are persisted before
submission. Queued, running, completed and failed compactions
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
Queued compactions remain healthy waits. Native queued/running compactions observed
without a journaled request also reserve owner capacity and block prompt or fresh
compaction dispatch, even when a newer physical assistant message appears. Their
exact message observations are exposed in `inProgressCompactions` cycle reports;
completion releases the reservation without inventing a pending request.
Reservation discovery exhausts the bounded filtered compaction history independently
of context reset. Newer completed/failed records cannot hide older queued/running
work; incomplete pagination or an exhausted search bound rejects observation.
Every non-null retained compaction must
have an object body, valid durable message and physical IDs, steer delivery and
a retained preparation marker before observation, publication or capacity selection.
Use the cycle's `occupied` observation, including all queued inputs, in the final
`finished` check. Refresh again after mutations before claiming completion.

`snapshots` collects full GitHub/Forgejo history through explicit authenticated
transport traits. GitHub collection includes paginated issue comments, reviews,
outdated/resolved review threads, nested inline comments and check contexts.
All GitHub connections require stable declared counts; the single checked commit
must match the PR head. Forgejo inline counts and nonempty unique history IDs are
validated too. Both forges re-fetch the complete check rollup after collection.
GitHub also revalidates every published mergeability, review, draft and target
field; asynchronous changes reject the observation rather than publish stale status.
Required scheduling enums, booleans and branch names, plus the requested comment,
review, thread and check-context fields, are validated before publication. A
missing nullable field is truncated evidence; an explicit native null remains
valid for deleted authors, review commits/submission times and pending-check
conclusions, timestamps and URLs.
GitHub's returned PR URL must match the requested coordinate; its URL and opaque
ID must remain stable through that final re-fetch.
Both providers require exact canonical URL identity: GitHub uses `/pull/`, Forgejo
uses `/pulls/`, and PR numbers use positive canonical decimal spelling. Aliases
cannot register the same native pull under different assignment URLs.
GitHub repository `viewerPermission` is validated and compared at the final
observation. Missing/unsupported permissions reject the snapshot; the explicit
native null returned for GitHub App authentication remains valid and is retained.
GitHub revisions must be full 40-digit hexadecimal object IDs, and its update
identity must parse as RFC 3339 before history is fetched. Malformed check rollups
return errors. `ForgeRest::get` returns `ForgeResponse` with native JSON `body`
and an optional parsed `x-total-count` in `total_count`. Consumers must preserve
the response header, reject malformed or duplicated metadata and never infer
counts from returned rows. Every history page requires that authoritative total,
stable across pagination, and the exact expected page length. Forgejo accepts
exactly 2,000 rows only after an empty, count-bound sentinel page; oversized
declared totals reject before further requests. Migrating a bare-JSON adapter
requires retaining this transport metadata; there is no short-page fallback.
Forgejo validates review, issue/inline-comment and native `CommitStatus` rows
(the row field is `status`, while the combined rollup uses `state`). Native
nullable user pointers and empty string fields remain valid. Both published
draft and mergeability booleans must remain stable through the final observation.
Forgejo requires a valid retained update timestamp, and inline-comment identities
must remain unique across the combined review history.
Malformed/truncated pagination, repeated cursors, and comparison movement fail
collection. Consumers retain the last valid snapshot and record the error. These
observations provide scheduling evidence, never merge or closure authority.
Both collectors share a 128-request budget across all nested pagination and
reject more than 2,000 inline comments across the entire PR. Exhaustion is an
incomplete observation, never a partial snapshot. Forgejo binds both the returned
pull number and canonical URL before collecting its history.

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
A source directory becoming a file first defers the replacement and sends the
acknowledged descendant deletions with `Batch.prune` structural retirement roots.
Oversized transitions drain in bounded exchanges. Partial drains carry only the
outermost fully drained subtrees as retirement roots, retaining structural cleanup
authority until the final drain; the replacement remains deferred throughout.
`evidence::Exchange` retains `known`, the exact pending batch, its receipt and a
receipt-bound retry cursor. Conflicted paths and subtrees take one exchange off,
so an early blocked frontier cannot monopolize retries. This applies equally to
sibling subtrees, ordinary deletions and conflicting writes. The cursor resets
from each complete receipt; conflicts remain retained and are retried on later turns.
Persist the serialized checkpoint before sending and atomically after
`Exchange::acknowledge`. Lost responses must resend its retained pending batch.
Acknowledgment rejects unbound, incomplete, foreign or stale receipts without
changing the checkpoint. `Receipt.batch_sha256` binds the exact encoded batch.
Each new checkpointed batch has an OS-random `exchange_id`, retained across
retries, so an old receipt cannot certify an identical payload on a later turn.
New identities reserve available journal capacity during collection. Conditional
deletions never supply speculative space; acknowledged removals free capacity
for the next exchange, so a valid conflicted receipt remains adoptable.
Acknowledged files or ancestors that become excluded symlinks or project checkouts
remain omitted, retaining their receiver identities and bytes. This applies to
narrow allowlist roots too; unrelated eligible artifacts continue advancing.
The legacy stateless `collect` entrypoint rejects budget-deferred work and requires
the checkpointed API for multi-exchange draining; bounded structural transitions
retain their previous conditional-removal protocol.
Only empty, non-checkout directories are retired. Foreign descendants preserve
the directory and withhold its removal identities, so the sender retains `known`
and retries that same phase. Persist `Receipt.pruned` with `Receipt.removed`;
only after that durable receipt removes the old identities may the replacement
file be sent. Retrying a completed retirement resynchronizes surviving ancestors.
Receiver leaf-type changes, including sockets and oversized files, remain
per-path conflicts; unrelated paths can advance under the same lease.
Blocked receiver ancestors (files, symlinks or checkouts) likewise conflict only
the affected descendants, for writes, unchanged verification and deletion alike.
Application independently
enforces 10,000 file mutations, 20,000 verification entries, 20,000 omitted/deferred
entries and the
48 MiB encoded-payload bound. Receiver reads, directory creation, staging,
rename and deletion use pinned no-follow directory handles.
Allowlist roots must be nonempty scoped relative paths. Ancestor links are
fsynced before descending or writing, including existing links on a retry after
an interrupted exchange. Fsync failure prevents an acceptance receipt.
Successful verification and idempotent write/deletion replays also resync their
ancestor links and containing directory. Visible byte equality or absence alone
cannot acknowledge a previous rename/unlink whose final fsync failed.
Source enumeration, artifact reads and deletion discovery also use pinned
no-follow handles; an ancestor replacement cannot redirect collection outside
the already-open authoritative directory.
Source traversal also limits the aggregate directory-entry inventory to 20,000
and path depth to 64 components, including empty directories. Compact JSON
serialization is counted without buffering before accepting or applying a batch;
escaped path bytes therefore cannot exceed the 48 MiB encoded bound.

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
