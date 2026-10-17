# Source-bound qualification

The optional `qualification` feature exposes reusable Rust operations for source
inventories, command receipts, portable verification, Cargo graph comparison,
source export and handoff sealing. Enable `cli,qualification` for the independent
`canix-toolbelt qualification` frontend. Frontends call the library directly and
own their output envelopes and exit handling.

## Hosted original-producer custody

The source-owned `.github/original-producers.json` enrollment names every job in
the six-job CI workflow and the full 34-installable matrix. CI's full-flake gate
still realizes all 71 x86 Linux checks. Custody uses Simit's reusable native
`ci observe-original` and `ci verify-original-retention` contracts, independently
binding branch push and PR originals; repository-owned historical Python tests
and helper bytes remain historical evidence.

The workflow first checks out the protected event revision with both PR merge
parents, then selects the exact event head without discarding those objects.
It provisions an independent full-revision-pinned Simit CLI without evaluating
the observed source's Nix code. Credentials are scoped to the two native observer
steps. Those steps clear loader and shell startup variables before launching the
shell and engine; the engine path is not supplied through a preceding step's
`GITHUB_ENV` writes.
Native capture checks repository/run/head/event/workflow/attempt one,
the exact branch or unambiguous PR target, complete required job inventories and
successful native step outcomes. Transient read-only GETs retry within one
aggregate deadline; watch exit status cannot replace terminal producer state.
Failed, cancelled, skipped, incomplete or identity-changed results never establish
acceptance. Interrupted responses, original archives and explicit failed receipts
remain available to the ordinary `always()` artifact upload.

The 360-minute hosted job bounds every phase: checkout 2 minutes, source selection
1, Nix setup 5, native engine provisioning 20, capture/finalization 318, upload 3,
independent retention verification 6 and diagnostics upload 1. Capture's
18,900-second budget leaves its step three minutes for the source probe, bounded
native finalization and process teardown. These limits total 356 minutes, leaving
four minutes of job-level reserve. Deadline exhaustion remains failure; original
producers are never rerun to repair a custody packet.

After capture, native retention verification compares the independent event and
enrollment, local member inventory, actual provider ZIP digest and member hashes,
original observer identity, fresh successful producer outcomes and at least
thirty days of artifact retention. Revalidation GET evidence is retained in a
separate diagnostic artifact, along with always-incomplete sibling preflight
receipts (`producer-evidence.preflight-*` and
`producer-evidence.retention.preflight-*`) even when no packet directory exists.
Diagnostics are never included in or promoted to the acceptance packet.
New receipts use `simit.original-custody.v1`;
historical `producer-custody-v1` receipts are not rewritten or upgraded.

Native custody receipts remain `qualified:false`. Source signatures, immutable
workflow/source binding, complete full34/full71 semantic evidence, exact-head
review, merge policy and release publication require their separate
repository-owned acceptance binders.

The immutable engine is Simit #66's accepted actual merge
`22c235944807b57badea57515365b3fc4c9f5b9e` (Rust 1.88). Its independent
337-member acceptance manifest has SHA-256
`ec560ea9df9ab7bd3e6bab78675de475b1f0968448c24feed86a876431ddc71f`.
The preserved consumer draft was integrated against that accepted API; consumer
acceptance requires new original push and PR custody for the final Toolbelt head.
The repository policy regression checks enrollment, credential scope, deadlines,
diagnostic separation and the exact engine selection:

```sh
direnv exec . cargo test --locked --test original_producer_enrollment
```

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
    "executor": "/approved-toolbelt/bin/canix-toolbelt-qualification-executor",
    "setpriv": "/approved-util-linux/bin/setpriv"
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
`unshare` supporting `--forward-signals`, and the packaged native executor. All
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

New execution receipts use schema version 5. Earlier execution receipts lacked
one or more of the current descriptor, status, host-file and control-channel
protections and cannot satisfy `verify-run` or a new sealed gate.
Historical bytes remain evidence; do not rewrite them to the newer version.

The supervisor constructs a minimal filesystem with selected sources, inputs,
programs and runtime dependencies, public tools and libraries, the immutable Nix
store and a private bounded work tmpfs. Host home and control directories are
absent, and a private network namespace isolates host abstract sockets. It uses
kernel mount IDs to identify visible mounts, and remounts them read-only
without removing existing locked mount flags. It re-enters the working directory
through that protected tree. Source/input mutations, including writes through
hardlink aliases and temporary changes later restored, are denied. The producer
and version probes run through the byte-bound `setpriv` with all capabilities
removed and `no_new_privs`; the supervisor is non-dumpable. A missing mount-ID or
read-only-mount capability fails closed before producer execution.
See [the v5 guide](qualification-v5.md) for dependency selection, quotas and work
retention. Selecting a containing source directory preserves symlink semantics;
individually selected symlink mounts fail closed instead of being flattened.

Before any files or signal registrations are created, the freshly exec'd
single-threaded supervisor invokes Linux `close_range(3, UINT_MAX, 0)`. This
closes inherited non-CLOEXEC handles regardless of their number; unsupported or
denied descriptor closing fails closed. An inherited writable descriptor cannot
bypass the new read-only mounts. The syscall has one locally audited unsafe
boundary because the safe Rust APIs cannot close arbitrary inherited handles.

The caller opens a private `execution.json` report on the host side and passes
its writable descriptor as the supervisor's stdin. Producer stdin is replaced
with `/dev/null`, and non-dumpability denies access through `/proc/1/fd`.
After kernel child accounting, the supervisor records the original producer
exit code or termination signal and whether descendants drained. The report is
hash-bound in the receipt and mandatory in direct/sealed portable verification;
launcher exit 125 cannot disguise a producer signal as an ordinary exit 125.

Only the newly created `OUT/work` tree and inherited stdout/stderr descriptors
are writable. `QUALIFICATION_WORK_DIR` and `TMPDIR` point to that tree; Cargo's
target directory defaults to `OUT/work/target` unless explicitly selected.
Explicit output paths must be inside the writable work tree. Dependency caches
and environment preparation happen before qualification; source-mutating gates
must be expressed as separate operations rather than passing immutable evidence.

Source traversal, input hashing and executable hashing check cancellation and
the shared execution deadline between reads. Expiry/cancellation during initial
capture returns an error before spawning; final capture cannot certify an expired
run. Tool/log finalization has one additional five-second budget. Successful tool
version streams are limited to 1 MiB each to bound memory retention.
Finalization also checks cancellation between reads and immediately before
receipt publication. Interrupted hashing retains a cancelled, non-accepting
receipt with available partial evidence; missing final hashes cannot verify.
Wall-clock and monotonic start times are sampled together before initial capture.

Direct `verify-run` requires the request, receipt, native execution report and
both logs to resolve inside the canonical run directory before reading them.
Archive verification rejects special files through descriptor-based regular-file
admission and normalizes leading `./` member prefixes while still rejecting
duplicate real members and retaining symlink target text.
The returned archive SHA-256 is calculated from the same opened byte stream used
for verification, including bytes after tar EOF; replacing the path cannot pair
a verified inventory with the digest of a replacement file. Changed descriptor
length or modification time invalidates verification. The seal and historical
receipt metadata themselves must also resolve inside their portable roots.

Source export retains real directories traversed by relative link targets,
including components cancelled by `..`, so `empty/../target` still resolves after
extraction. Verification rejects archives missing those directories or treating
a file/symlink as a required directory. Directory-symlink traversal is rejected
because lexical cancellation would not preserve its filesystem semantics.
Non-run CLI operations retain default SIGINT/SIGTERM termination; `run` alone
installs cooperative handlers to drain its PID namespace and retain cancellation.

Ubuntu 24.04 hosted qualification installs an ephemeral AppArmor `userns`
permission specifically for `/nix/store/*/bin/unshare` through Simit's
`extra_setup`. The global namespace restriction and AppArmor remain enabled.
The package's mandatory namespace probe fails closed and reports UID/GID maps
and the current AppArmor label when admission fails. A successful setup does not
replace the package tests or the full installable/flake matrix.
Native test concurrency is capped at two through Simit's `extra_env`, matching
the hosted build-core budget while retaining all tests and their finite deadlines.
Native CI also sets `CANIX_QUALIFICATION_REQUIRE_XATTRS=1`: all six real user-xattr
and POSIX-ACL fixtures must be created and rejected by work retention. Nix's
builder seccomp filter deliberately returns `ENOTSUP` for xattr creation.
Depending on the Nix version, enumeration returns either an empty list or
`ENOTSUP`. Inside that builder alone, the same test verifies blocked creation,
empty/unsupported enumeration and successful copying of metadata-free work.
The native requirement is not inherited by pure Nix derivations; unsupported
native fixtures fail.
Named ACL users use the current mapped UID so fixtures also work in single-UID
build namespaces.
The PID-1 binary shares only the minimal execution-report wire module with the
library, avoiding the full operator dependency graph in its repeatedly hashed
debug executable.

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
Regular source files, including symlink targets, must have exactly mode `0644`
or `0755`; other permissions cannot be represented by inventory v1. Linux source
capture also rejects xattrs and POSIX ACLs on selected files, symlinks, targets
and represented directories, including implicit parents. Metadata inspection
errors fail admission; a filesystem or Nix builder reporting unsupported xattrs
cannot expose that metadata through the native producer API either. Canonical
archive verification rejects local PAX metadata, including xattrs, ACLs and
ownership/timestamp overrides. The explicit legacy content-only route continues
to verify historical archives without claiming that metadata parity.
Portable execution verification requires the `/dev/null` public mount used for
producer stdin, in addition to rejecting mounts outside the runtime allowlist.

Execution creates a new directory and a pending marker before spawning. Existing
evidence is never reused. stdout/stderr stream to files. A finite deadline is
required. Cancellation/deadline stops the Unix process group and reaps its leader.
The receipt rejects a producer that exits while descendants are still running,
so background children cannot turn a partial command into accepted evidence.
It distinguishes passed, failed, cancelled, timed-out and source-changed;
it retains the actual program, explicit tool bytes/versions, request identity,
before/after composition and logs. Missing final receipts do not pass. A zero
child exit cannot pass if source, inputs or tool bytes changed.
A report received through the private native channel must remain present and
match that channel's selected digest through finalization. Removing its retained
pathname produces a failed receipt rather than a passing but unverifiable run.

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
links and special archive members fail closed. Canonical member paths preserve
their exact spelling: one leading `./` prefix and a directory's trailing `/` are
supported, while internal dot components, repeated separators and trailing
separators on files/symlinks are rejected. The exporter rechecks source after
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
