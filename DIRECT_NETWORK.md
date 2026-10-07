# Background direct-network protection

`nixosModules.direct-network` keeps explicitly enrolled background cgroups on
their current physical/fleet routes while a desktop VPN owns the host default.
It uses `canix-toolbelt-direct-network`, built with the `direct-network` feature.
The Rust library has no fleet paths, account credentials or host inventory.

```nix
canix-toolbelt.networking.directNetwork = {
  enable = true;
  users.operator = {
    services.backend = "app-amc.slice";
    services.sync = "app.slice";
    slices = ["app-amc.slice" "agent-tools.slice"];
  };
  systemServices.backup = "system.slice";
  directInterfaces = ["wg-home"];
  dnsZones."vpn.example.test" = "10.123.0.1";
};
```

Use actual enabled units and explicit UIDs. `services` maps units to their
existing resource parent. Already-enrolled dedicated slices remain unchanged;
otherwise an additional `*-direct.slice` child preserves every ancestor and
unit-local limit. Persistent anchor units retain cgroup identity and order
service startup after routing/DNS enrollment. Independent transient workers
need their dedicated slice enrolled too. Never enroll `app.slice`, an entire
UID, or the user manager. System-owned `systemSlices` supports existing native
builder boundaries without relocating them.

## Routing and DNS contract

- nftables' socket-cgroup matching runs on each output packet, including sockets
  opened before connection. Marks use the reserved `0xca010000` value; explicit
  socket marks from other networking engines are preserved.
- Routing priorities 49/50 and tables 51821/51822 are reserved. Conflicting rules
  fail readiness. The daemon copies live main-table physical and explicitly
  enrolled fleet-interface routes, excluding dummy, tunnel and container links.
  It fills an inactive table before switching rules, including across restarts
  and interrupted IPv4/IPv6 updates. Missing direct defaults terminate in an
  unreachable route rather than falling back into the desktop VPN.
- Connected physical/fleet destinations remain reachable from desktop apps.
  Internet output remains under the desktop VPN's routing and kill-switch policy.
  Source NAT normalizes new sockets initially given a VPN/dummy source address;
  existing conntrack mappings and connections remain stable.
- A dedicated dnsmasq listens on `127.0.0.54:5354` and `[::1]:5354`. Only enrolled
  ordinary TCP/UDP DNS requests are redirected to it. Its own upstream requests
  bypass redirection and retain direct routing. Upstreams come from current
  physical NetworkManager devices, explicit DNS servers and fleet DNS zones.
  No resolver is hard-coded as a public fallback.
- NSS uses `files mymachines myhostname dns`. This is necessary because the
  `resolve` NSS module delegates over AF_UNIX, discarding the caller's routing
  identity. Desktop DNS still uses the normal host stub and VPN resolver policy.
  Explicit D-Bus/Varlink clients of systemd-resolved are outside this contract;
  applications using encrypted DNS retain their own endpoints and direct egress.
  The NixOS nsncd/nscd proxy also leaves hostname lookups to the caller, while
  account/group lookups retain their existing proxy policy. nsncd uses its
  documented `NSNCD_IGNORE_HOSTS=true` switch; glibc nscd disables the hosts
  database in its final configuration. This preserves the originating cgroup
  for glibc's files/DNS lookup path.

Gate desktop VPN launch with `canix-toolbelt-direct-network launch --systemctl
/absolute/systemctl --unit direct-network-<slice>.service -- /absolute/client`.
The launcher starts the selected user anchors, requires readiness and execs the
client. Readiness retries initial socket creation and route/DNS initialization
for up to 15 seconds. `canix-toolbelt-direct-network ready` is available for
inspection. The root
daemon accepts only fixed readiness requests from configured UIDs; clients
cannot enroll additional cgroups or supply commands. Root-only `ready
--routes-only` bootstraps the independent DNS listener. Readiness synchronously
refreshes current cgroup identities and requires the direct DNS TCP listener.
Kernel protection is retained if the daemon exits; stopping the unit does not
delete routing rules or create a fail-open window.

Configuration activation may migrate/restart enrolled units once. Connecting,
changing servers, losing a tunnel, reconnecting and disconnecting do not restart
them. A physical uplink/address change can naturally invalidate internet TCP
sessions and is separate from a VPN-only transition.

## Qualification

Native tests cover route selection, IPv6 gateways/source addresses and invalid
policy data. `tests/direct_network_probe.py` exercises the production daemon in
disposable rootless network namespaces, including existing/new IPv4/IPv6 TCP,
UDP DNS redirection, kill-switch-shaped routes and retained kernel protection.
The probe also checks external IPv4/IPv6 listener handshakes with strict
reverse-path filtering, retained conntrack routing and guarded client launch.
Run with the project's approved environment:

```sh
cargo build --features cli,direct-network --bin canix-toolbelt-direct-network
unshare --user --map-root-user --net python3 tests/direct_network_probe.py \
  target/debug/canix-toolbelt-direct-network /path/to/approved/scratch
```

`checks.x86_64-linux.direct-network-transitions` adds actual systemd user/system
cgroups, same-UID desktop applications, timer work, transient workers, NSS DNS,
a real full-tunnel WireGuard peer and an independent fleet WireGuard link.
It checks persistent connections, new requests, listener replies and preserved
resource limits through connection, server change, tunnel loss, reconnect and
disconnect. These provider-independent fixtures do not establish compatibility
with future VPN client firewall implementations; consuming fleets must qualify
the pinned client and perform a real credentialed transition test before live
acceptance.
