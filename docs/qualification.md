# Source-bound qualification

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
  "timeoutSeconds": 600,
  "containment": {
    "unshare": "/approved-util-linux/bin/unshare",
    "executor": "/approved-toolbelt/bin/canix-toolbelt-qualification-executor"
  }
}
```

```text
canix-toolbelt qualification run --request gate.json --out /state/runs/test-1
canix-toolbelt qualification verify-run /state/runs/test-1 --expected composition.json
```

Run inside the already approved project environment. The library does not load
direnv, approve shells, acquire Nix evaluation/activation leases or select a
toolchain. Nix operations must be guarded Canix commands. Supply all dependency
source roots and environment configuration read by the command. Execution clears
the inherited environment and supplies only the build/locale/home allowlist plus
explicit overrides. The receipt records the complete supplied environment.
Settings such as `CC`, `RUSTUP_TOOLCHAIN`, and target-specific linker variables
must be explicit overrides. Overrides must not contain credentials; inherited
credentials are neither supplied nor serialized.

Execution requires Linux user/PID/mount namespaces, a caller-selected util-linux
`unshare` supporting `--forward-signals`, and the packaged native executor. Both
containment binaries are hashed before execution and rechecked afterward. The
executor is PID 1 with a namespace-local `/proc`; after reaping the command
leader, it checks kernel child accounting with `waitpid(-1, WNOHANG)`. Any live or
zombie child invalidates completion, even if a privileged producer remounts
`/proc`. Namespace init's exit causes the kernel to kill
and drain all remaining processes, including `setsid`/`setpgid` descendants,
before the launcher is reaped. Version probes use the same boundary and retain
both stdout and stderr in the tool-version evidence. Cancellation/timeout asks
init to exit and allows at most five seconds per cleanup attempt; unconfirmed
cleanup leaves pending evidence and returns an error without final hashes or a
completed receipt. Unsupported platforms fail before spawning a command. Missing
namespace permissions or launcher capabilities fail closed rather than falling
back to process groups. Inventory/archive/portable verification remain portable.

New execution receipts use schema version 2. Version-1 execution receipts lacked
this containment guarantee and cannot satisfy `verify-run` or a new sealed gate.
Historical bytes remain evidence; do not rewrite them to the newer version.

The retained request records canonical command, tool, containment and working
directory selections, plus the complete effective environment. Portable
verification parses that hash-bound request and checks tool/source/input labels,
executable paths, containment paths and exact environment equality against the
receipt, without resolving original host paths.
Inventory keys and relative symlink targets use `/` between path components on
every platform; Unix filenames containing literal backslashes are rejected as
ambiguous in this portable format.

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
