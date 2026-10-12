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

Multipath routes retain every eligible nexthop, its weight, order and on-link
flag. A route containing an ineligible nexthop is omitted as a whole. The daemon
does not silently rewrite a producer's forwarding policy by removing members.
NetworkManager IPv6 link-local DNS addresses retain their uplink scope. An
authored scope must match that uplink; the same address on two uplinks remains
two distinct upstreams.

## Kernel ownership and upgrades

The module reserves mark `0xca010000`, rule priorities 49/50, routing tables
51821/51822 and route protocol 242 **within those two tables**. Other routing
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

Dnsmasq runs as root with an explicitly empty group override and no capabilities
on its unprivileged port. The module emits an explicit empty
`CapabilityBoundingSet=` directive; an empty Nix list would omit it and retain
systemd's default capabilities. This permits same-UID descriptor inspection from the
routing daemon without giving it `CAP_SYS_PTRACE`. Silent peers outside the
allowed UID list are rejected before reading their requests.

Servers-file installation invalidates the reload-dispatch receipt before any
write. Failed dispatches remain pending and are retried even when the committed
file is unchanged. A receipt binds exact bytes and the authenticated resolver
process/socket generation; a resolver restart cannot inherit it. A successful
HUP dispatch establishes signal delivery, **not asynchronous reload completion
or upstream reachability**. The hosted VM gate separately verifies effective
DNS answers and persistent application connections through transitions.

The root-only `ready --routes-only` barrier allows DNS bootstrap after route and
cgroup reconciliation. Normal readiness still requires authenticated listeners
and a current reload-dispatch receipt. Anchors want and order after DNS but use
their readiness precheck for admission, preserving running workloads across
resolver lifetime changes.
