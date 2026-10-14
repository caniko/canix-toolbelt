# Source-bound qualification

## Hosted coverage applicability

`qualification verify-coverage SEAL --expected requirements.json --receipts
receipts.json --report-sha256 HASH` verifies a sealed `artifacts/coverage.json`
against independently selected obligations and provider-bound report/receipt
hashes. The library exposes the same `verify_coverage` operation.

The provider observer must first establish repository, actual checkout revision,
workflow/generator identity, exact run/attempt, required terminal jobs, and artifact
custody. Obtain the hashes from that authenticated observation; taking them from
the downloaded bundle itself establishes only self-consistency. This operation
does not collect forge evidence, authenticate a provider or authorize deployment.
Its result explicitly reports `coverage-integrity-and-applicability`.

`CoverageRequirements` selects a full `Composition` and a nonempty map of stable
obligation IDs. Each obligation selects an exact `level`, sealed `gate` label and
nonempty set of original `methods`. `CoverageReport` uses the same composition
and obligation keys, with each method mapped to a terminal execution status.
The gate receipts must pass `verify_seal`; every required method must be present
and `passed`. Failed, skipped, pending, cancelled, unknown, missing or extra method
results are rejected. A consumer may bind all 174 original PostgreSQL selectors
without replacing their inventory or frozen oracle bytes.

Levels are explicit: `producer` and `consumer-conformance` are distinct and never
implicitly promoted. Consumer conformance additionally requires the immutable
consumer artifact/source under `Composition.sources["consumer"]` and its contract
digest under
`Composition.inputs["consumer-contract"]`. Callers select the complete cohort,
including patches, features, tools and archived dependency inputs that matter to
the gate. The verifier checks exact cohort equality rather than inferring that
two jobs with similar names exercise equivalent behavior.

There is no hosted `live-acceptance` level. Host configuration/runtime equality,
production restore/signature verification, recovery equality, explicit thaw and
application health remain separate observations of the actual deployment.

The optional `qualification` feature exposes reusable Rust operations for source
inventories, command receipts, portable verification, Cargo graph comparison,
source export and handoff sealing. Enable `cli,qualification` for the independent
`canix-toolbelt qualification` frontend. Frontends call the library directly and
own their output envelopes and exit handling.

## Run a gate

Write a `RunRequest` JSON document with explicit roots and inputs:

```json
{
  "cwd": "/checkout",
  "sources": {
    "consumer": { "root": "/checkout", "paths": ["src", "Cargo.toml"] }
  },
  "inputs": {
    "lock": "/checkout/Cargo.lock",
    "cargoConfig": "/approved-cargo-home/config.toml"
  },
  "command": ["cargo", "test", "--locked", "--jobs", "2"],
  "environment": { "CARGO_BUILD_JOBS": "2" },
  "tools": { "rustc": "/approved-toolchain/bin/rustc" },
  "timeoutSeconds": 600
}
```

```text
canix-toolbelt qualification run --request gate.json --out /state/runs/test-1
canix-toolbelt qualification verify-run /state/runs/test-1 --expected composition.json
```

Run inside the already approved project environment. The library does not load
direnv, approve shells, acquire Nix evaluation/activation leases or select a
toolchain. Nix operations must be guarded Canix commands. Supply all dependency
source roots and environment configuration read by the command. Relevant inherited
Cargo/Rust environment is retained; explicit overrides are recorded verbatim and
must not contain credentials. Inherited credentials are not serialized.

`inventory --root ROOT --git` selects tracked and unignored untracked files via
NUL-delimited Git output. Gitlinks require separate source selections. Explicit
inventory paths recursively include everything beneath selected directories;
there are no inferred build-directory exclusions. Source identities bind content,
relative link text and executable bits. Escaping links, non-UTF-8 names, special
files and empty inventories are rejected. Selection of a deleted tracked file
fails rather than silently dropping that file.

Execution creates a new directory and a pending marker before spawning. Existing
evidence is never reused. stdout/stderr stream to files. A finite deadline is
required. Cancellation/deadline stops the Unix process group and reaps its leader.
The receipt rejects a producer that exits while descendants are still running,
so background children cannot turn a partial command into accepted evidence.
It distinguishes passed, failed, cancelled, timed-out and source-changed;
it retains the actual program, explicit tool bytes/versions, request identity,
before/after composition and logs. Missing final receipts do not pass. A zero
child exit cannot pass if source, inputs or tool bytes changed.

## Verify and seal

`verify-run` checks completed status, source inventories, request and log hashes,
and optionally the externally supplied composition. `verify-index` checks the
original Python artifact-index format (a relative-path/SHA-256 map). It rejects
path traversal and symlink escapes. Extra unindexed files are not evidence.

`verify-legacy-run` verifies historical Python `source.json`, separate or combined
logs and an optional retained lock. Its exit is nonzero for non-acceptable
executions. Legacy verification does not upgrade historical environment/tool/input
provenance to the new contract; external input and signature obligations still
need their recorded evidence. Original receipts and frozen oracle bytes remain
unchanged.

`verify-legacy-archive source.tar --index qualified-files.json` compares all tar
file contents with the original Python path/hash map. Git-exported directory
entries are supported; link targets are resolved within the archive without
extraction. The result explicitly reports `content-only`: historical maps do not
bind executable bits or link text. Qualification adapters that intentionally
change source bytes must be separately applied and checked by their owning engine.

`export-source --request inventory-spec.json --out source.tar` exports deterministic
selected bytes. `verify-archive source.tar --inventory inventory.json` verifies
without extraction. Archives preserve executable bits and relative symlinks;
every link target must be included. Duplicate entries, hard links, escaping/cyclic
links and special archive members fail closed. The exporter rechecks source after
writing. Existing archive paths are never overwritten.

`seal --request seal-request.json --out /state/handoff` takes an explicit
`composition`, nonempty `gates` map of labels to execution directories, independently
selected `expectedReceipts` label-to-receipt-hash map and optional
`artifacts` map of names to files. Every gate must pass on that composition. It
copies receipts, requests, logs and selected artifacts, with a hash index.
`verify-seal /state/handoff --expected composition.json --requirements gates.json`
requires the exact externally specified gate set and selected attempt hashes,
so omission or substitution of an older passing attempt cannot turn a failed
required check into acceptance. Seals are integrity records, not signatures or
deployment authorizations. Archive-to-source verification remains an explicit
operation; arbitrary artifacts do not gain source identity merely by being sealed.

## Cargo graph evidence

`parents metadata.json syn --version 2.0.119` preserves immediate parents,
features, dependency kinds, target expressions and shortest workspace paths in
the retained Python parent-report format. Unreachable resolved parents have an
empty path. `graph-delta before.json after.json` compares full name/version/source
identities, including a source change at an unchanged version. Missing resolve
graphs, duplicate IDs and unknown references fail closed.

Cargo target expressions are graph facts, not target compilation or runtime
acceptance. None of these commands changes dependency policy, selects releases,
collects a passing subset of required gates or asserts full Atlas acceptance.

## Python migration coverage

| Historical helper capability | Permanent Rust operation |
| --- | --- |
| `qualify.py`, `qualified-run.py`, `graph_run.py`: run, log, bind source/lock/tool inputs | `execute`, `verify_run` |
| `verify_handoff.py`: artifact hashes and portable run integrity | `verify_index`, `verify_legacy_run`, `verify_source_archive` |
| `graph.py`, `seal_join.py`: parents, shortest paths, distribution changes | `graph::Metadata::parents`, `graph::Metadata::delta` |
| `seal.py`, `seal_join.py`: source export and gate-bound handoff | `export_source`, `seal`, `verify_seal` |

Application-specific patch selection, Git signature/bundle qualification and
platform/UI gates stay with their existing owners. Retain the old evidence as
fixtures; do not rerun or replace the Python helpers to use the Rust operations.
