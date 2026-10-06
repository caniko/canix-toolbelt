# Roborev receipt transport

`roborev` with `github-receipt-v1` is a Toolbelt-owned coordinator protocol.
It is separate from stock roborev's human-facing GitHub comments. It requires
an explicit deployment policy with the review producer's immutable Bot user ID
and a `review-policy` check bound to the dedicated policy App's numeric ID.
Neither identity is inferred from a login or supplied by a pull-request branch.

Ensure posts the existing `toolbelt-review-request:v1` intent marker. A trusted
coordinator must observe that authorized writer's exact intent, dispatch its
frozen comparison to roborev, and publish a complete result as one issue comment:

```text
<!-- toolbelt-roborev-result:v1 {JSON receipt} -->
```

The receipt has `schemaVersion: 1`, `requestId`, `candidate` (the complete frozen
Candidate), `policyDigest`, `jobId` and `reviewId`, full `reviewedHead`
and `reviewedBase`, `status`, `completeFindings`, and `document`. The comparison
is the exact observed target SHA to source SHA; the coordinator must attest
that actual comparison, not merely echo requested coordinates. `document` is
roborev's canonical schema-version-2 structured document, including every
finding. A completed result requires positive persisted job and review IDs and
a non-null document. Failed/skipped results explicitly use null for any absent
job, review or document, preserving failures without inventing review content.
Version 0/1 legacy content, rendered Markdown, severity-filtered
summaries and synthesis-fallback comments cannot establish acceptance.

### Comparison semantics

The target tip, merge base and source head are distinct identities. For a PR,
`candidate.base` remains the freshly observed target tip even when its branch
advanced after the source branched. Dispatch uses that full `base..head` range;
neither replacing the base with the merge base nor labeling a merge-base job
with the requested target tip establishes this contract. The divergent-history
regression in `tests/roborev_comparison.rs` exercises different target-tip,
merge-base and head commits, with different changed-file sets for the two ranges.

Local committed and working-tree reviews may use separately declared advisory
input semantics. Those observations cannot be promoted to this PR receipt
transport without a new exact-comparison dispatch and its persisted evidence.

Collection requires the configured Bot user ID, all pages, a request from an
authorized repository writer, an exact request/comparison/policy match, a result
created after the request's latest edit, and a result outside the dispatch's
baseline IDs. The latest matching receipt controls the result. Failed, skipped,
cancelled, incomplete, malformed and oversized receipts block acceptance;
an older passing receipt cannot replace them. `unable_to_review` blocks too.
Every canonical finding blocks regardless of severity or provider verdict.
A `fail` verdict with no findings also blocks.

Finding identities include the forge receipt ID, ordinal and canonical body
digest. Dispositions require writer authorization and bind those identities,
the complete body, review, head, base and policy. Changed content invalidates a
disposition. Historical Greptile receipts retain their explicit provider; the
selector never substitutes them for roborev.

Submission remains non-idempotent. Toolbelt journals UNKNOWN before posting;
transport errors reconcile the original authorized marker without replay.
Deleting that marker blocks recovery. Dispatch and result publication use separate
durable ledgers and reconciliation, described below.

`RoborevReceipt::from_saved_review` binds the actual persisted range job from
v0.71.0's `show --json --job` output, not a single-commit review. It checks the
job/review IDs, checkout, agent, range job type, terminal state, prompt mode and
unfiltered canonical content. Panel, custom/prebuilt-prompt and severity-filtered
jobs cannot masquerade as that dispatched range.

`dispatch_roborev_once` supplies a request-scoped kernel lock and fsync/rename
write-ahead ledger. It records UNKNOWN before enqueue and never retries that
mutation. Recovery adopts exactly one matching persisted job in the exclusive
request checkout; zero jobs, multiple jobs, missing/corrupt state and reused
request IDs with different dispatch content block. Failed jobs return explicit
null review/document evidence. Queued/running jobs remain pending. Every tick
observes current daemon state, so an old pass cannot hide a later failed job.

On Unix, `RoborevUnix` implements `RoborevRunner` using a private local daemon
socket. It has no TCP, proxy, redirect or daemon-autostart fallback. It lists all
jobs/pages with panel and classification rows included, rejects duplicate/cyclic
or incomplete pages, submits the full ordinary unfiltered range once and reads
canonical evidence by persisted job ID. Requests and response sizes are bounded.
A mandatory consumer callback verifies immutable head/base and trusted worker
policy; transport permissions do not provide filesystem/credential containment.

`RoborevGitHub::authorized_request` obtains a current open comparison and marker
from an authorized writer, then rechecks the comparison. Its result is an
`AuthorizedRoborevRequest`, not a deserializable authorization token. Reobserve it
before dispatch when checkout preparation has intervened.
`publish_receipt_once` revalidates the request's full intent, comment and edit
identity immediately before publication. It records UNKNOWN before POST, verifies
the returned Bot ID/type and exact body, and reconciles an existing exact comment
without reposting after an ambiguous outcome. Loss of either ledger behind its
persistent request anchor blocks mutation replay. The consumer must preserve the
private state directory and derive receipts from current persisted daemon evidence.

The collector/request protocol and dispatch ledger do not install or qualify a coordinator.
The Canix worker bridge, approved harness, credentials and hostile-PR boundary
require their own execution evidence. A producer must refuse receipts above the
supported bound rather than truncate findings. No live enrollment is performed
by selecting this transport.
