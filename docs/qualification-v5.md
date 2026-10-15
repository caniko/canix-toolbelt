# Qualification receipt v5

Version 5 closes the host-file, host-control-socket and pre-containment loader
environment gaps in earlier execution receipts. Earlier receipts are retained
historical evidence and cannot satisfy new execution gates.

The supervisor builds a private tmpfs root and exposes selected source paths,
inputs, canonical programs, explicit runtime dependencies, public system tool
and library trees, the immutable Nix store, four standard null/random devices,
and its fresh PID-namespace `/proc`. Host home directories, host `/etc` and host
control directories under `/run` are absent. Additional directory dependencies
reject visible sockets and FIFOs. A separate network namespace also isolates
abstract Unix sockets and the host network. Commands must use pre-provisioned
offline dependencies; hosted checkout and dependency provisioning happen before
execution.

Source and dependency mounts remain read-only, including inherited `nosymfollow`
flags. A source selected through a symlink is rejected before spawn; selecting
its containing directory preserves the recorded symlink semantics instead.
Inventory v1 cannot represent empty directories: recursive selections containing
one fail closed, and source archives reject extra directories not implied by
inventoried files or link resolution. Select explicit files when empty siblings
are not part of the source contract.
Only the fresh work tree is writable. It is backed by a separate tmpfs capped
at 256 MiB and 16,384 inodes. Capacity exhaustion cannot yield passing evidence.
After descendant drainage, the supervisor retains bounded regular files,
directories and symlinks without following links. Work files are visible to
the caller after completion; use the bounded log streams for live progress.
The launcher and privilege limiter
use sanitized environments; recorded producer settings, including loader
variables, are applied by the final exec adapter after containment and capability
removal. The producer's home is a fresh `work/home`. The default Cargo home is
`work/cargo-home`. A supplied Cargo dependency cache needs an explicit recursive
source selection (or file input selections), binding its contents before and
after execution. Do not expose a cache containing credentials or control channels.

`RunRequest.runtimePaths` optionally names absolute read-only runtime dependencies.
Every path must equal a selected source path or input file, so runtime bytes
participate in the composition instead of merely exposing a mutable host path.
Unbound runtime selections fail before spawn; portable verification enforces
the same rule without accessing the original host. Changed dependencies produce
a different composition, and detected mid-run changes cannot pass.
`RunRequest.toolArgumentZeros` optionally preserves tool
aliases separately from their canonical byte-bound executable paths. Effective
requests bind every selected alias. Rust callers initialize the new fields with
empty collections when no extra selection is needed.

Producer stdout/stderr each have a hard 512 MiB file-size limit (version probes
use 1 MiB). Reaching the main-stream limit is non-acceptance even if a shell
masks a failed child. Evidence JSON is limited to 64 MiB. Receipt publication
checks cancellation and deadlines during serialization, between writes and
after durable sync, then atomically exposes the completed receipt. Interrupted
publication discards the staged passing document and may retain a failed,
cancelled or timed-out disposition within the bounded cleanup budget. Elapsed
time includes the inventory serialization and durable bulk write.

Portable evidence rejects symlink hops, ambiguous backslashes and drive-prefix
artifact names. Original source inventories still support contained relative
source links. `ArtifactIndex` is a transparent newtype over a sorted map and
rejects duplicate JSON keys in standalone indexes and embedded requirements.
Source/input/tool/environment selections, inventory paths, receipt maps and seal
gate/artifact selections also reject duplicate keys. Portable receipt verification
requires every executable, tool and containment digest to be 64 hexadecimal digits.
Rust callers can use `collect`, array conversion, or `.into()` on an existing
`BTreeMap`; serialized bytes remain unchanged. Gate, request, execution-report and coverage JSON are parsed from
the same opened bytes whose selected hashes are checked. Artifact copying reads
only the opened descriptor's initial size and rejects changes. Git inventory
clears inherited repository-selection variables. Coverage requirements reject
duplicate obligation IDs instead of silently replacing an original method set.
All public evidence JSON reads share the descriptor-bound 64 MiB limit. Retaining
a completed work tree replaces prior children, preserving producer deletions and
directory/file transitions between probes and the main command.

Portable verification establishes integrity and applicability. Provider custody,
exact-source hosted qualification and live acceptance remain independent gates.
