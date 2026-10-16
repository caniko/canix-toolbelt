# Qualification receipt v5

Version 5 closes the host-file, host-control-socket and pre-containment loader
environment gaps in earlier execution receipts. Earlier receipts are retained
historical evidence and cannot satisfy new execution gates.

The supervisor builds a private tmpfs root and exposes private copies of selected
source paths, inputs, canonical programs and explicit runtime dependencies, plus public system tool
and library trees, the immutable Nix store, four standard null/random devices,
and its fresh PID-namespace `/proc`. Host home directories, host `/etc` and host
control directories under `/run` are absent. Additional directory dependencies
reject visible sockets and FIFOs. A separate network namespace also isolates
abstract Unix sockets and the host network. Commands must use pre-provisioned
offline dependencies; hosted checkout and dependency provisioning happen before
execution.

Every probe and main execution copies only inventoried files into its private
tmpfs, checks the copied bytes against the boundary's independently selected
digests, recreates recorded relative links, and mounts that snapshot read-only.
In-place writes, path replacement and transient extra host files cannot affect
the producer's view. A mismatch during copying prevents producer launch. The
snapshot root is capped at 1 GiB and 131,072 inodes; exceeding either bound fails
closed, independently of the work-tree quota. Public system tool/library trees
remain platform-owned runtime dependencies, not a claim to capture the operating
system. They are the only permitted host directory binds.
Inventory collection also admits at most 131,072 file entries and conservatively
reserves a 64 MiB serialized-evidence budget before inserting each entry. The
combined named sources share those bounds. Fingerprints stream through the same
bounded JSON writer instead of materializing unbounded serialization buffers.
A source selected through a symlink is rejected before spawn; selecting
its containing directory preserves the recorded symlink semantics instead.
Inventory v1 cannot represent empty directories: recursive selections containing
one fail closed, and source archives reject extra directories not implied by
inventoried files or link resolution. Select explicit files when empty siblings
are not part of the source contract.
On Unix, selected files with multiple hard links also fail closed, including
symlink inodes, symlink targets and links outside the selected tree. Inventory v1
and its tar export do not represent inode aliases; select independent source copies instead.
Selected and structural source directories must have mode `0755` on Unix;
inventory v1 cannot bind any other directory mode, and archive verification rejects
noncanonical directory headers rather than silently normalizing their semantics.
Regular archive files must use mode `0644` or `0755`; special or otherwise
unrepresented permission bits cannot be hidden behind the executable flag.
The redundant POSIX regular-file type bits emitted by Rust's tar encoder are allowed.
Synthesized snapshot parents use canonical modes independently of the launcher's
umask; the producer retains its originally inherited umask.
Private copied files, symlinks, selected directories and synthesized parents
use access and modification timestamp zero, matching source archives. This
normalization completes before any public host tree is attached.
Execution and portable run/seal verification require every symlink target to be
explicitly selected in the same source inventory, so hashing a target cannot
admit an unmounted dangling link.
Link chains must terminate without cycles and every hop must agree with the
terminal file's digest and executable flag.
Only the fresh work tree is writable. It is backed by a separate tmpfs capped
at 256 MiB and 16,384 inodes. Capacity exhaustion cannot yield passing evidence.
The private supervisor observes kernel syscall results for the producer and its
fork/clone descendants. `ENOSPC`, `EDQUOT`, `EFBIG`, and allocation/file-limit signals are
latched even when a producer handles the failure, cleans up, and exits zero.
Execution requires native x86_64/aarch64 Linux with `PTRACE_SEIZE`,
`PTRACE_GET_SYSCALL_INFO`, and seccomp filtering available inside the namespace;
missing observation fails closed. An inherited filter rejects `CLONE_UNTRACED`
and makes `clone3` return `ENOSYS` so ordinary `clone` fallback stays observable.
Linux AIO and io_uring setup also return `ENOSYS`: completion-queue errors are
not syscall return values, so commands must use synchronous I/O fallback.
Compat-architecture execution is not supported. Job-control stops use
`PTRACE_LISTEN`; ordinary signals are forwarded rather than swallowed.
After descendant drainage, the supervisor retains bounded regular files,
directories and symlinks without following links. Work files are visible to
the caller after completion; use the bounded log streams for live progress.
The launcher and privilege limiter
use sanitized environments; recorded producer settings, including loader
variables, are applied by the final exec adapter after containment and capability
removal. Before parsing any policy or mounting its paths, PID 1 checks the opened
policy bytes against the host caller's in-memory `boundarySha256` selection.
Those admitted bytes are copied into the private tmpfs, never bind-mounted from
the externally writable host inode. The final exec adapter checks the same
identity again before applying the environment. Both version probes and the main
producer use this policy identity. Retained `boundary.json` is mandatory in runs
and seals; portable verification checks its digest, environment and exact snapshot
file/directory/mount selections against the inventories, inputs and executable
identities. Each retained source selection is nonempty, uses root-relative paths
and covers exactly its inventory entries; unrelated or missing selections fail
portable verification too. Earlier schema-5 packets without these snapshot selections or the
policy identity remain historical, not qualifying evidence.
Bootstrap runs before that private root exists. Mutable launcher/executor bytes
are copied into anonymous executable files, sealed against writes and resizing,
then checked against the selected digest before launch. The host retains those
descriptors until all probes and the main execution finish; PID 1 closes inherited
descriptors before starting a producer. Each bootstrap copy is capped at 256 MiB
and uses the request's cancellation/deadline checks. This route requires Linux
executable memfd support. A non-root caller may instead execute a canonical
root-owned program beneath entirely root-owned, non-writable ancestors; this
preserves platform executable attachments such as AppArmor's unshare policy.
The executor's later privilege-dropped re-entry uses its digest-checked private
snapshot, not the bootstrap descriptor's deleted-file pseudo-path.
The producer's home is a fresh `work/home`. The default Cargo home is
`work/cargo-home`. A supplied Cargo dependency cache needs an explicit recursive
source selection (or file input selections), binding its contents before and
after execution. Do not expose a cache containing credentials or control channels.

`RunRequest.runtimePaths` optionally names absolute read-only runtime dependencies.
Every path must equal a selected source path or input file, so runtime bytes
participate in the composition instead of merely exposing a mutable host path.
Unbound runtime selections fail before spawn; portable verification enforces
the same rule without accessing the original host. Changed dependencies produce
a different composition, and detected mid-run changes cannot pass. Even when
host bytes are restored before final capture, the producer consumes only the
digest-checked private copies, never the transient substitutions.
`RunRequest.toolArgumentZeros` optionally preserves tool
aliases separately from their canonical byte-bound executable paths. Effective
requests bind every selected alias. Rust callers initialize the new fields with
empty collections when no extra selection is needed.

Producer stdout/stderr each have a hard 512 MiB file-size limit (version probes
use independently captured 1 MiB streams). Probe capture uses private Unix
sockets and bounded readers that latch overflow even when a producer handles
`EPIPE`; work files retain their separate 256 MiB aggregate quota. The global
file-size ceiling remains 512 MiB, above that entire work quota, so a version
probe can initialize a cache larger than 1 MiB. A request admits at most 32 tool
probes, and their combined retained
version strings may not exceed 8 MiB. The probe that exceeds that aggregate budget
is retained as failed evidence (at most one additional 2 MiB stdout/stderr pair);
no later probes or main command run. Portable verification enforces the same
count and aggregate-version limits. Reaching the main-stream limit is
non-acceptance even if a shell masks a failed child. Main and probe log capture
retains the original opened descriptors; substituted public log paths cannot
become the selected producer output. Evidence JSON is limited to
64 MiB. Receipt publication
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
requires every input, executable, tool, containment and boundary-policy digest to
be 64 hexadecimal digits.
Retained executable, tool and containment paths must also be canonical and
absolute without consulting the original machine's filesystem.
Every retained source-file digest is subject to the same format check, even when
an externally supplied inventory and its enclosing composition were rehashed.
Rust callers can use `collect`, array conversion, or `.into()` on an existing
`BTreeMap`; serialized bytes remain unchanged. Gate, request, execution-report and coverage JSON are parsed from
the same opened bytes whose selected hashes are checked. Artifact copying reads
only the opened descriptor's initial size and rejects changes. Git inventory
clears inherited repository-selection variables. Coverage requirements reject
duplicate obligation IDs instead of silently replacing an original method set.
All public evidence JSON reads share the descriptor-bound 64 MiB limit. Retaining
a completed work tree replaces prior children, preserving producer deletions and
directory/file transitions and directory permission modes between probes and the
main command. Restrictive modes are restored after copying descendants. Retention
and startup restoration run under privileged PID 1 before producer capabilities
are dropped, so mode-`000` files and nested directories remain copyable; their
original modes are preserved for the next producer to inspect and change.
The first invocation starts with a fresh private work tree rather than trusting
pre-launch host files. Every later restoration must match the preceding private
tree's SHA-256 over names, types, modes, link text and file bytes. PID 1 supplies
that identity over a bounded private Unix socket; the caller does not select it
from the writable retained report. Replaced reports cannot redirect restoration.
Portable acceptance requires the final native report's work identity; older
schema-5 packets missing it remain historical evidence.
Retention rejects multiply linked work files and symlink inodes instead of silently changing inode-alias
semantics between commands. Startup restoration observes cancellation; cleanup
also kills and reaps a launcher that exceeds the graceful five-second budget.
Both Cargo-metadata CLI commands use the same bounded regular-file reader as
other evidence inputs, rejecting oversized files and FIFOs before parsing. Local
package graph identities retain Cargo's path-bearing package ID when `source`
is null, so switching same-version local dependencies remains visible.

Deadline regressions separate namespace startup from the post-execution hashing
witness. The end-to-end one-second deadline includes startup and may expire
before the producer is launched. A separate test grows an initially captured
input to 1 TiB and directly exercises the same capture/hash callback with a
one-second deadline, without treating an early startup timeout as hashing proof.

Portable verification establishes integrity and applicability. Provider custody,
exact-source hosted qualification and live acceptance remain independent gates.
