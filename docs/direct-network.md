# Direct-network routing and readiness

`canix-toolbelt.networking.directNetwork` enrolls explicit background cgroups.
Desktop applications keep the host's VPN policy. Existing application sockets
and resource slice limits survive route, resolver and daemon transitions.

## Uplink selection

Ordinary Ethernet links are eligible. Bridges and bonds require an eligible
member; VLANs require an eligible lower link. The traversal is bounded and
container-only bridges are omitted. `directInterfaces` explicitly enrolls
additional uplinks, including fleet WireGuard interfaces. An explicit virtual
interface can back an eligible bridge or VLAN.
Same-namespace VLAN parents come from iproute2's named `link` field. A foreign
namespace's `link_index` cannot establish eligibility through a matching local
interface index.
Non-array or oversized link snapshots (more than 4096 interfaces) reject
reconciliation and readiness before any route, nftables or DNS update.

Multipath routes retain every eligible nexthop, its weight, order and on-link
flag. A route containing an ineligible nexthop is omitted as a whole. The daemon
does not silently rewrite a producer's forwarding policy by removing members.
Kernel-derived nexthop status flags, including `linkdown`, are accepted without
being replayed as authored flags. The kernel derives current reachability and
offload state from the actual devices; authored `onlink` remains preserved.
Encapsulation on an eligible route or any of its nexthops rejects reconciliation
before route, nftables or DNS updates. Unsupported forwarding semantics must not
be silently stripped from a replayed route.
Cross-family `via` attributes on eligible routes or nexthops also reject the
snapshot, including null or malformed attributes. ECMP replay supports at most
60 nexthops per route. The maximal supported replay has eight arguments and
28 netlink bytes per hop, leaving space for route attributes under both
iproute2's 512-argument batch array and its 4096-byte netlink request. Larger groups reject before either
routing table, policy rules, firewall or DNS state is changed; they are never
truncated or partially appended. This replay limit applies only after complete
direct-interface eligibility is established. Larger VPN/container-only or mixed
groups remain omitted as a whole and do not deny direct readiness.
Route-level `metrics` (such as path MTU, advertised MSS and congestion-control
options) also reject an eligible snapshot before effects, for both single-path
and ECMP routes. The ordinary route preference `metric` remains supported;
unsupported path options are never silently dropped during replay.
Route or nexthop `flow` attributes (realms used by traffic-control classification)
also reject an eligible snapshot before effects rather than losing that policy.
An eligible route with a `tos` selector rejects before effects, preventing replay
from widening the route's traffic match. IPv6's canonical `pref = "medium"`
remains supported through the kernel default; non-default or malformed preference
values reject instead of changing direct-uplink selection.
Source-selector `from`/`src` fields also reject on eligible routes; the supported
preferred-source field is `prefsrc`, which is distinct from those selectors.
Referenced nexthop IDs (`nhid`) reject even when compatibility output expands
their members: inline ECMP cannot preserve a resilient group's bucket policy.
ECMP's canonical global scope remains supported; other explicit ECMP scopes
reject rather than being rewritten to the inferred universe scope. Ordinary
single-path scopes are retained and emitted explicitly. This includes global
scope omitted by iproute2's JSON printer, which otherwise becomes inferred link
scope during device-only IPv4 replay. Canonical global/link/host scopes are
supported; unknown or malformed scopes reject before effects.

The selected `Route` now carries a typed `RouteScope` and serializes its canonical
name, so a scope-only change invalidates the installed snapshot. Rust consumers
that construct `Route` literals must supply this field when adopting 0.5.0.

The projection retains destinations, eligible interfaces, gateway addresses,
preferred sources, canonical scopes, ordinary metrics and complete inline hop order/weights/onlink
policy. Route protocol and table identity belong to the owned target namespace;
kernel reachability/offload flags are observed rather than authored replay
policy. Lifetime countdowns and cache/statistics do not form the installed
projection: current main-table presence is polled again at each reconciliation.
NetworkManager IPv6 link-local DNS addresses retain their uplink scope. An
authored scope must match that uplink; the same address on two uplinks remains
two distinct upstreams.

## Kernel ownership and upgrades

The module reserves mark `0xca010000`, rule priorities 49/50, routing tables
51821/51822, UDP DNS conntrack zone `0xca01`, and route protocol 242 **within those two tables**. Other routing
owners must not use that namespace. Each table is inspected independently of
policy-rule presence, for both address families. Untagged or differently tagged
routes cause reconciliation to fail before any table flush. Flushes select only
protocol 242. The observed owned IPv6 unreachable fallback is explicitly deleted
from the inactive table first; iproute2 refuses to flush a default of that type
at the maximum metric, even with a type selector. IPv4 routes use
`add`; IPv6 routes use non-replacing `append` to
retain same-prefix, same-metric siblings on distinct interfaces, including
link-local routes. Neither operation replaces a concurrently inserted foreign
route. The unreachable fallback uses `add`, and observed conflicts block the
switch after installation too.
The inactive complete snapshot is installed before moving the live rules.

UDP DNS interception assigns the reserved zone from the current socket before
conntrack lookup. Resolver replies use that same zone only for the enrolled
listener's exact source endpoints and DNS response bit; its upstream queries
remain outside it. Replies to a client using source port 53 still enter the
reply zone before the resolver's upstream-query exemption is considered.
This separates retained direct DNS mappings from new desktop sockets reusing
the same tuple, in either policy order. A foreign nonzero socket mark remains
outside interception. Only TCP restores an output mark from conntrack, to retain
passive listener handshakes; UDP output enrollment comes from the current socket.
The DNS zone does not isolate every other UDP masquerade mapping. It is reserved
for this module and must not be assigned by another raw-output zone owner.

Earlier daemon versions installed untagged routes. They are not adopted or
automatically deleted during an upgrade. Inspect their route/rule origin and
perform an operator-authorized, namespace-scoped cleanup before starting this
version. The daemon does not remove a foreign route merely because no rule
currently refers to its table.

## Resolver admission

Ordinary readiness authenticates `direct-network-dns.service`'s native MainPID,
process start time, exact unified-cgroup membership, currently enrolled cgroup
inode and host network namespace. The main process must own unique listening
socket inodes for all four endpoints:

| Transport | Endpoint |
| --- | --- |
| TCP | `127.0.0.54:5354` |
| UDP | `127.0.0.54:5354` |
| TCP | `[::1]:5354` |
| UDP | `[::1]:5354` |

Wildcard bindings, IPv4-mapped competition, foreign or child-only ownership, missing sockets, duplicate
listeners and observed identity movement deny admission. Two bracketed
observations compare the required socket mapping; ordinary upstream socket
activity does not have to remain static. This is an observation-bound startup
barrier, not a promise that the resolver cannot exit after acknowledgement.
Socket tables are streamed through a 4096-byte per-row bound, retaining only
the required listener identity. Unrelated host sockets do not impose an aggregate
table-size limit. The scan still reaches EOF to detect competing listeners and
rejects malformed, oversized or unreadable evidence.

Systemd starts dnsmasq as the dedicated static `direct-network-dns` account and
group, with no capabilities on its unprivileged port. The module emits an explicit empty
`CapabilityBoundingSet=` directive; an empty Nix list would omit it and retain
systemd's default capabilities. Admission resolves the trusted account at runtime
and requires all four process UIDs to match its nonzero UID, all five capability
sets to be empty, and `NoNewPrivs` to be set. Missing, malformed or duplicate
credential fields deny admission. Credentials participate in both bracketed
identity checks and reload-receipt equality; sharing the resolver UID does not
substitute for MainPID-owned listener evidence.

Only the routing daemon has `CAP_SYS_PTRACE` and `CAP_DAC_READ_SEARCH`, alongside
`CAP_NET_ADMIN`, to inspect the cross-UID MainPID's FD links and host network
namespace. Ptrace admission and reading the resolver-owned `0500` FD directory
have separate checks, so both inspection capabilities are required. The resolver cannot
read root-only host files, signal unrelated root processes, or write its root-owned
`0755` runtime directory and `0644` servers file. Replacement-file permissions are
explicit even after a restrictive umask or stale temporary file. Silent peers
outside the allowed UID list are rejected before reading their requests.

Servers-file installation invalidates the reload-dispatch receipt before any
write. Failed dispatches remain pending and are retried even when the committed
file is unchanged. A receipt binds exact bytes and the authenticated resolver
process/socket generation; a resolver restart cannot inherit it. A successful
HUP dispatch establishes signal delivery, **not asynchronous reload completion
or upstream reachability**. The hosted VM gate separately verifies effective
DNS answers and persistent application connections through transitions.

The root-prefixed pre-start `ready --routes-only` barrier allows DNS bootstrap
after route and cgroup reconciliation, before the unprivileged payload starts.
Normal readiness still requires authenticated listeners
and a current reload-dispatch receipt. Anchors want and order after DNS but use
their readiness precheck for admission, preserving running workloads across
resolver lifetime changes.
Every configured system and Home Manager service also checks normal readiness
on each start, after the final empty `ExecStartPre` reset and before its remaining
pre-start commands. Authored resets still clear earlier hooks, but cannot erase
admission. An already-active anchor
cannot admit new workloads while DNS is unavailable. System-manager prechecks
run only the narrow readiness client with systemd's `+` prefix so service UIDs
need no additional user authorization. Established workloads remain running.
Units using `RootDirectory` or `RootImage` retain the same inline readiness
precheck. Admission denial is therefore a service execution failure governed by
the workload's existing `Restart`, `StartLimitIntervalSec` and `StartLimitBurst`
policy, including automatic recovery when DNS returns. Existing pre/start/post
commands retain their configured root, credentials and resource limits.
A generated systemd drop-in, installed for both system and Home Manager units,
appends read-only binds for the readiness package's
individual runtime-closure paths and the host `/run/direct-network` directory.
It preserves authored binds and does not expose the whole Nix store or change
`RootDirectoryStartOnly`. Closure enumeration happens at build time, without
import-from-derivation. Binding the directory rather than its socket inode lets
each new precheck observe a recreated daemon socket.
For immutable or read-only alternate roots, create the bind destinations with
the correct file/directory type when constructing the root. This includes
`/run/direct-network` and each store path in the readiness package's closure.
Writable roots may let systemd create missing destinations; a read-only root
cannot receive missing mount points at service start.
Rooted user-manager units retain systemd's native prerequisites: unprivileged
user namespaces for `RootDirectory`, and systemd's authorized mountfsd image
mounting support for `RootImage`. The module supplies the readiness binds without
changing those kernel, authorization or service namespace policies.
