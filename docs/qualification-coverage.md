# Sealed obligation coverage

`qualification verify-coverage SEAL --expected requirements.json --receipts
receipts.json --report-sha256 HASH` verifies a sealed `artifacts/coverage.json`
against independently selected obligations and provider-bound report/receipt
hashes. The library exposes the same `verify_coverage` operation.

The provider observer first establishes repository, actual checkout revision,
workflow/generator identity, exact run/attempt, required terminal jobs and artifact
custody. Obtain the hashes from that authenticated observation; taking them from
the downloaded bundle itself establishes only self-consistency. This operation
does not collect forge evidence, authenticate a provider or authorize deployment.
Its result explicitly reports `coverage-integrity-and-applicability`.

`CoverageRequirements` selects a full `Composition` and a nonempty map of stable
obligation IDs. Each obligation selects an exact `level`, sealed `gate` label and
nonempty set of original `methods`. `CoverageReport` uses the same composition
and obligation keys, with each method mapped to its execution status. The gate
receipts must pass `verify_seal`; every required method must be present and
`passed`. Failed, skipped, pending, cancelled, unknown, missing, extra and
duplicate method results are rejected.

Levels are explicit: `producer` and `consumer-conformance` are distinct and never
implicitly promoted. Consumer conformance additionally requires the immutable
consumer source/artifact under `Composition.sources["consumer"]` and its contract
digest under `Composition.inputs["consumer-contract"]`. Callers select the
complete cohort, including patches, features, tools and archived dependency inputs
that matter to the gate. The verifier checks exact cohort equality rather than
inferring equivalence from similar job names.

There is no hosted `live-acceptance` level. Host configuration/runtime equality,
production restore/signature verification, recovery equality, explicit thaw and
application health remain separate observations of the actual deployment.

The regression fixture uses 174 synthetic selectors to prove completeness
enforcement against real version-4 namespace-contained receipts. It does not
execute or qualify the original PostgreSQL methods. A PostgreSQL conformance lane
must report those exact original selectors after executing them and retain their
unchanged frozen oracle bytes as composition inputs.

## Invocation identity in version-4 receipts

`RunRequest.argumentZero` is optional for new requests. Execution retains the
original command name there and records the canonical executable path in
`command[0]`, so multicall binaries execute the selected applet while the receipt
hashes the actual canonical bytes. Explicit overrides are also supported and
bound by the request hash. Older receipts without the field remain verifiable.
Rust callers constructing a request set `argument_zero: None` for the default.

The namespace supervisor still owns PID 1, the read-only source mounts, private
writable work tree, report descriptor and descendant accounting. After setpriv
drops capabilities and sets no-new-privs, a private exec-only role of that same
byte-bound supervisor replaces itself with the canonical producer and requested
argv[0]. It neither creates namespaces nor modifies mounts and cannot replace
namespace init. Portable receipt verification does not consult the original
host's PATH or aliases.
