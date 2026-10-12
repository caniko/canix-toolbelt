import json
import time
from collections.abc import Callable
from typing import TYPE_CHECKING, cast

if TYPE_CHECKING:
    from test_driver.machine import QemuMachine

# The NixOS driver injects these names when executing this script.
machine = cast('QemuMachine', globals()['machine'])
start_guests = cast(Callable[[], None], globals()['start_all'])

start_guests()
machine.wait_for_unit('multi-user.target')
machine.wait_for_unit('nscd.service')
machine.succeed('getent passwd operator')
machine.succeed('systemctl start network-fixture.service')
machine.wait_for_file('/run/network-fixture-ready', timeout=60)
machine.wait_for_unit('user@1000.service')
try:
    machine.wait_until_succeeds('canix-toolbelt-direct-network ready')
except Exception:
    # Retain causal route and native resolver evidence before the driver tears
    # down the VM; client transport timeouts alone hide admission failures.
    for command in [
        'journalctl -b -u direct-network.service -u direct-network-dns.service --no-pager -n 100',
        'ip -6 -N -j route show table all',
        'systemctl show direct-network-dns.service -p ActiveState,SubState,MainPID,ControlGroup',
        'ss -lnptu',
    ]:
        status, output = machine.execute(command)
        print(command, status, output)
    raise
user = 'runuser -u operator -- env XDG_RUNTIME_DIR=/run/user/1000 '
userctl = user + 'systemctl --user '
userrun = user + 'systemd-run --user --quiet --wait --pipe '
probe = '@PYTHON@ @FIXTURE@ probe'
machine.succeed(userctl + 'start backend.service listener.service')
machine.wait_for_file('/home/operator/state.json')
before = json.loads(machine.succeed('cat /home/operator/state.json'))['counter']
pid = machine.succeed(userctl + 'show backend.service -p MainPID --value').strip()


def check_direct() -> None:
    machine.wait_until_succeeds(userctl + 'is-active backend.service')
    result = json.loads(machine.succeed(userrun + '--slice=agent-tools.slice ' + probe))
    assert result['v4'] == result['v6'] == result['fleet'] == 'direct', result
    assert result['dns'] == ['2001:db8:20::20', '203.0.113.20'], result
    assert result['fleetDns'] == '10.123.0.1', result
    for family, prefix in [('-4', '198.18.37.0/24'), ('-6', '2001:db8:37::/64')]:
        rules = json.loads(machine.succeed(f'ip {family} -j rule show'))
        table = next(str(rule['table']) for rule in rules if rule['priority'] == 50)
        routes = json.loads(machine.succeed(f'ip {family} -j route show table {table}'))
        assert any(route.get('dst') == prefix and route.get('dev') == 'uplink.37' for route in routes), routes
    assert machine.succeed(userrun + '--slice=agent-tools.slice curl --fail --max-time 5 http://198.18.37.2:8080/').strip() == 'direct'
    machine.succeed('curl --fail --max-time 5 http://127.0.0.1:4096/')
    # New external listener connections must pass reverse-path filtering and
    # reply directly while the VPN owns the internet default route.
    machine.succeed('ip netns exec directpeer curl --interface 203.0.113.20 --fail --max-time 5 http://198.18.0.1:4096/')
    machine.succeed("ip netns exec directpeer curl --interface 2001:db8:20::20 --fail --max-time 5 'http://[2001:db8:1::1]:4096/'")
    assert machine.succeed(userctl + 'show backend.service -p MainPID --value').strip() == pid
    assert machine.succeed(userctl + 'show backend.service -p NRestarts --value').strip() == '0'


def pause_dns_admission_probes() -> int:
    counter = json.loads(machine.succeed('cat /home/operator/state.json'))['counter']
    machine.succeed('touch /run/direct-network-dns-admission-negative')
    machine.wait_until_succeeds('@PYTHON@ -c \'import json; assert json.load(open("/home/operator/state.json"))["dnsAdmissionNegative"]\'')
    return counter


def resume_dns_admission_probes(counter: int) -> None:
    machine.succeed('rm /run/direct-network-dns-admission-negative')
    machine.wait_until_succeeds('@PYTHON@ -c \'import json; s=json.load(open("/home/operator/state.json")); assert not s["dnsAdmissionNegative"] and s["counter"] > ' + str(counter + 2) + "'")


check_direct()
# The systemd-backed harness owns authenticated guarded-launch qualification.
machine.succeed(user + 'canix-toolbelt-direct-network launch --systemctl /run/current-system/sw/bin/systemctl --unit direct-network-agent-tools.service -- /run/current-system/sw/bin/echo guarded-client | grep guarded-client')
# Foreign selectors must fail closed even when priority/table/mark resemble a
# retained guard rule. A second daemon must reject them before touching state.
# Deliberately invalid rules must not interrupt the live continuity workload.
machine.succeed('ip netns add rule-conflicts')
for family, source in [('-4', '198.18.0.0/24'), ('-6', '2001:db8:1::/64')]:
    ip = f'ip -n rule-conflicts {family}'
    machine.succeed(f'{ip} rule add priority 49 to {source} lookup 51822')
    machine.succeed(f'{ip} rule add priority 50 fwmark 3389063168 lookup 51822')
    original_rules = machine.succeed(f'{ip} -j rule show')
    used_tables = {str(rule['table']) for rule in json.loads(original_rules)}
    # The kernel treats an omitted destination as a wildcard during duplicate
    # detection. Use the inactive table so the foreign rule can be installed.
    table = next(table for table in ['51821', '51822'] if table not in used_tables)
    for conflict in [
        f'priority 49 lookup {table}',
        f'priority 50 fwmark 3389063168/0xffff0000 lookup {table}',
        f'priority 50 from {source} fwmark 3389063168 lookup {table}',
        f'priority 50 iif lo fwmark 3389063168 lookup {table}',
        f'not priority 50 fwmark 3389063168 lookup {table}',
    ]:
        machine.succeed(f'{ip} rule add {conflict}')
        with_conflict = machine.succeed(f'{ip} -j rule show')
        status, diagnostics = machine.execute('ip netns exec rule-conflicts canix-toolbelt-direct-network daemon --policy /etc/direct-network-policy.json 2>&1')
        assert status != 0 and 'conflict' in diagnostics, (conflict, diagnostics)
        assert machine.succeed(f'{ip} -j rule show') == with_conflict
        machine.succeed(f'{ip} rule del {conflict}')
    assert machine.succeed(f'{ip} -j rule show') == original_rules
machine.succeed('ip netns del rule-conflicts')
check_direct()

# Table ownership is independent of policy rules. Untagged/static leftovers
# must survive a failed daemon start byte-for-byte, including both families.
machine.succeed('ip netns add table-conflicts')
for family, destination in [('-4', '192.0.2.0/24'), ('-6', '2001:db8:99::/64')]:
    ip = f'ip -n table-conflicts {family}'
    for table in [51821, 51822]:
        machine.succeed(f'{ip} route add unreachable {destination} table {table} proto static')
        before_routes = machine.succeed(f'{ip} -N -j route show table all')
        status, diagnostics = machine.execute('ip netns exec table-conflicts canix-toolbelt-direct-network daemon --policy /etc/direct-network-policy.json 2>&1')
        assert status != 0 and 'unowned route' in diagnostics, diagnostics
        assert machine.succeed(f'{ip} -N -j route show table all') == before_routes
        machine.succeed(f'{ip} route del unreachable {destination} table {table}')
machine.succeed('ip netns del table-conflicts')

# A container-only bridge must never create a desktop exemption or background
# main-table route. The declared virtual uplink may back an eligible bridge.
machine.succeed('ip link add br-container type bridge')
machine.succeed('ip link add container-a type veth peer name container-b')
machine.succeed('ip link set container-a master br-container')
machine.succeed('ip link set br-container up')
machine.succeed('ip route add 192.0.2.0/24 dev br-container')
machine.wait_until_succeeds('canix-toolbelt-direct-network ready')
for table in [51821, 51822]:
    routes = json.loads(machine.succeed(f'ip -j route show table {table}'))
    assert not any(route.get('dev') == 'br-container' for route in routes), routes
machine.succeed('ip link del container-a')
machine.succeed('ip link del br-container')

def wait_for_weighted_route(family: str, destination: str, metric: int, second_device: str) -> None:
    # The inactive table intentionally retains an older complete snapshot.
    # Assert the path selected by the live rule, not every retained table.
    code = ('import json,subprocess; '
            f'rules=json.loads(subprocess.check_output(["ip","{family}","-j","rule","show"])); '
            'table=next(str(r["table"]) for r in rules if r["priority"]==50); '
            f'routes=json.loads(subprocess.check_output(["ip","{family}","-j","route","show","table",table])); '
            f'r=next(r for r in routes if r.get("dst")=="{destination}" and r.get("metric")=={metric}); '
            f'assert [(h["dev"],h["weight"]) for h in r["nexthops"]]==[("uplink",2),("{second_device}",3)], r; '
            'assert all("onlink" in h["flags"] for h in r["nexthops"]), r')
    machine.wait_until_succeeds(f"@PYTHON@ -c '{code}'", timeout=60)


# Weighted ECMP must retain the complete eligible path set in each family.
for family, destination, first, second in [
    ('-4', '192.0.2.0/24', '198.18.0.2', '10.123.0.1'),
    ('-6', '2001:db8:99::/64', 'fe80::1', 'fe80::2'),
]:
    machine.succeed(f'ip {family} route add {destination} metric 42 nexthop via {first} dev uplink weight 2 onlink nexthop via {second} dev wg-home weight 3 onlink')
    wait_for_weighted_route(family, destination, 42, 'wg-home')
    machine.succeed(f'ip {family} route del {destination} metric 42')
machine.wait_until_succeeds('canix-toolbelt-direct-network ready')
check_direct()

# Kernel ECMP link-state flags must reconcile while the usable path and all
# established application traffic stay intact.
machine.succeed('ip link add ecmp-status type veth peer name ecmp-peer')
machine.succeed('ip link set ecmp-status up')
machine.succeed('ip link set ecmp-peer up')
machine.succeed('ip address add 198.18.43.1/24 dev ecmp-status')
machine.succeed('ip -6 address add 2001:db8:43::1/64 dev ecmp-status nodad')
for family, destination, gateway in [('-4', '192.0.2.0/24', '198.18.0.2'),
                                     ('-6', '2001:db8:99::/64', 'fe80::99')]:
    machine.succeed(f'ip {family} route add {destination} metric 43 nexthop via {gateway} dev uplink weight 2 onlink nexthop via {gateway} dev ecmp-status weight 3 onlink')
# Drop carrier while retaining IFF_UP. Administrative down makes Linux reject
# reinstalling authored onlink hops with "Nexthop device is not up".
machine.succeed('ip link set ecmp-peer down')
for family, destination in [('-4', '192.0.2.0/24'), ('-6', '2001:db8:99::/64')]:
    machine.succeed(f'ip {family} -j route show table main | @PYTHON@ -c \'import json,sys; r=next(r for r in json.load(sys.stdin) if r.get("dst")=="{destination}"); assert "linkdown" in r["nexthops"][1]["flags"]\'')
    wait_for_weighted_route(family, destination, 43, 'ecmp-status')
machine.wait_until_succeeds('canix-toolbelt-direct-network ready')
check_direct()
machine.succeed('ip link del ecmp-status')
machine.wait_until_succeeds('canix-toolbelt-direct-network ready')

# Unsupported encapsulation must deny admission without stripping forwarding
# semantics or changing the retained route, nftables or resolver snapshot.
encap_destination = '2001:db8:100::/64'
for route in [
    f'{encap_destination} encap seg6 mode encap segs 2001:db8:20::20 via fe80::99 dev uplink onlink metric 44',
    f'{encap_destination} metric 44 nexthop encap seg6 mode encap segs 2001:db8:20::20 via fe80::99 dev uplink weight 2 onlink nexthop via fe80::2 dev wg-home weight 3 onlink',
]:
    snapshot_commands = [
        f'ip {family} -N -j route show table {table}'
        for family in ['-4', '-6'] for table in [51821, 51822]
    ] + ['ip -4 -j rule show', 'ip -6 -j rule show',
         'nft --stateless list table inet toolbelt_direct_network',
         'cat /run/direct-network/servers']
    retained = [machine.succeed(command) for command in snapshot_commands]
    machine.succeed(f'ip -6 route add {route}')
    main = json.loads(machine.succeed('ip -6 -j route show table main'))
    encapsulated = next(r for r in main if r.get('dst') == encap_destination)
    assert 'encap' in encapsulated or any('encap' in h for h in encapsulated.get('nexthops', [])), encapsulated
    status, diagnostics = machine.execute('timeout 20 canix-toolbelt-direct-network ready 2>&1')
    assert status == 1 and diagnostics.startswith('direct-network: '), (status, diagnostics)
    assert [machine.succeed(command) for command in snapshot_commands] == retained
    check_direct()
    machine.succeed(f'ip -6 route del {encap_destination} metric 44')
    machine.wait_until_succeeds('canix-toolbelt-direct-network ready')
    check_direct()

# Silent, unauthorized sockets cannot hold the privileged routing loop hostage.
machine.succeed('systemd-run --unit=silent-peers --property=User=nobody @PYTHON@ @NEGATIVE@ silent-peers --seconds 30 --marker /tmp/silent-peers-ready')
machine.wait_for_file('/tmp/silent-peers-ready')
machine.succeed('timeout 8 canix-toolbelt-direct-network ready')
machine.succeed('systemctl stop silent-peers.service')

# Authenticate the actual hardened DNS unit, not unrelated TCP reachability.
# Stopping DNS must preserve the established application process/connections.
dns_pid = machine.succeed('systemctl show direct-network-dns.service -p MainPID --value').strip()
dns_status = dict(line.split(':', 1) for line in machine.succeed(f'cat /proc/{dns_pid}/status').splitlines())
resolver_uid = int(machine.succeed('id -u direct-network-dns').strip())
assert resolver_uid != 0 and list(map(int, dns_status['Uid'].split())) == [resolver_uid] * 4
for capability in ['CapInh', 'CapEff', 'CapPrm', 'CapBnd', 'CapAmb']:
    machine.succeed(f'grep "^{capability}:[[:space:]]*0000000000000000$" /proc/{dns_pid}/status')
assert dns_status['NoNewPrivs'].strip() == '1'
assert machine.succeed(f'stat -Lc %d:%i /proc/{dns_pid}/ns/net').strip() == machine.succeed('stat -Lc %d:%i /proc/1/ns/net').strip()
machine.succeed('canix-toolbelt-direct-network ready')
machine.succeed('systemd-run --unit=resolver-root-peer sleep 60')
root_peer = machine.succeed('systemctl show resolver-root-peer.service -p MainPID --value').strip()
machine.succeed(f'systemd-run --wait --pipe --collect --unit=resolver-privilege-proof --property=User=direct-network-dns --property=Group=direct-network-dns --property=CapabilityBoundingSet= --property=NoNewPrivileges=yes @PYTHON@ @NEGATIVE@ privilege-boundary --root-peer {root_peer}')
machine.succeed('systemctl stop resolver-root-peer.service')
negative_counter = pause_dns_admission_probes()
# Repeated starts must rerun admission even though the slice anchor stays live.
# Existing prechecks and payloads still execute inside RootDirectory/RootImage.
for name in ['rooted-start', 'imaged-start']:
    previous = ''
    for _ in range(2):
        machine.succeed(f'systemctl start {name}.service')
        # Inactive oneshots clear the manager's live InvocationID. Retain the
        # actual payload environment instead, without changing its lifecycle.
        current = machine.succeed(f'cat /run/direct-network-root-state/{name}-invocation').strip()
        assert len(current) == 32 and all(c in '0123456789abcdef' for c in current), (name, current)
        assert current != previous, (name, current, previous)
        previous = current
        machine.succeed(f'grep -Fx pre /run/direct-network-root-state/{name}-pre')
        machine.succeed(f'grep -Fx payload /run/direct-network-root-state/{name}-payload')
        machine.succeed(f'grep -Fx post /run/direct-network-root-state/{name}-post')
        machine.succeed(f'test "$(systemctl show {name}.service -p MemoryMax --value)" = 67108864')
        machine.succeed(f'rm /run/direct-network-root-state/{name}-pre /run/direct-network-root-state/{name}-payload /run/direct-network-root-state/{name}-post /run/direct-network-root-state/{name}-invocation')
for _ in range(2):
    machine.succeed(userctl + 'start rooted-user-start.service')
    for phase in ['pre', 'payload', 'post']:
        machine.succeed(f'grep -Fx {phase} /run/direct-network-root-state/rooted-user-start-{phase}')
        machine.succeed(f'rm /run/direct-network-root-state/rooted-user-start-{phase}')
    machine.succeed(userctl + 'show rooted-user-start.service -p MemoryMax --value | grep -Fx 67108864')
machine.succeed('systemctl start rooted-retry.service')
machine.succeed('test -e /run/direct-network-root-state/rooted-retry-payload')
# The workload disables start limits; each automatic restart must admit anew.
retry_admissions = {machine.succeed('systemctl show rooted-retry.service -p InvocationID --value').strip()}
retry_started = time.monotonic()
for _ in range(6):
    machine.succeed('rm /run/direct-network-root-state/rooted-retry-pre /run/direct-network-root-state/rooted-retry-payload /run/direct-network-root-state/rooted-retry-post')
    machine.succeed('systemctl kill --kill-whom=main --signal=TERM rooted-retry.service')
    machine.wait_until_succeeds('test -e /run/direct-network-root-state/rooted-retry-payload', timeout=10)
    invocation = machine.succeed('systemctl show rooted-retry.service -p InvocationID --value').strip()
    assert invocation and invocation not in retry_admissions, (invocation, retry_admissions)
    retry_admissions.add(invocation)
    machine.succeed('grep -Fx pre /run/direct-network-root-state/rooted-retry-pre')
    machine.succeed('grep -Fx payload /run/direct-network-root-state/rooted-retry-payload')
    machine.wait_until_succeeds('grep -Fx post /run/direct-network-root-state/rooted-retry-post')
assert len(retry_admissions) == 7 and time.monotonic() - retry_started < 60, retry_admissions
machine.succeed('systemctl stop direct-network-dns.service')
# Admission denial remains a payload failure: Restart= keeps retrying and later
# recovers without start/restart/reset-failed. Observe retries before starting
# bounded foreign peers so these waits do not consume their fixture lifetime.
machine.succeed('rm /run/direct-network-root-state/rooted-retry-pre /run/direct-network-root-state/rooted-retry-payload /run/direct-network-root-state/rooted-retry-post')
denied_restart_before = int(machine.succeed('systemctl show rooted-retry.service -p NRestarts --value'))
machine.succeed('systemctl kill --kill-whom=main --signal=TERM rooted-retry.service')
machine.wait_until_succeeds('test "$(systemctl show rooted-retry.service -p NRestarts --value)" -ge ' + str(denied_restart_before + 3), timeout=60)
for phase in ['pre', 'payload', 'post']:
    machine.succeed(f'test ! -e /run/direct-network-root-state/rooted-retry-{phase}')
for mode, peer_user in [('tcp', 'root'), ('both', 'root'), ('missing-ipv6', 'root'), ('same-account', 'direct-network-dns')]:
    marker = f'/tmp/foreign-{mode}-ready' if mode == 'same-account' else f'/run/foreign-{mode}-ready'
    # Keep the impostor alive through both 15-second service-start barriers.
    # The explicit lifetime stays within the negative peer's 120-second bound.
    peer_mode = 'both' if mode == 'same-account' else mode
    machine.succeed(f'systemd-run --unit=foreign-dns --property=User={peer_user} @PYTHON@ @NEGATIVE@ {peer_mode} --seconds 120 --marker {marker}')
    machine.wait_for_file(marker)
    machine.succeed('timeout 8 canix-toolbelt-direct-network ready --routes-only')
    machine.fail('timeout 4 canix-toolbelt-direct-network ready')
    machine.fail(user + 'timeout 4 canix-toolbelt-direct-network ready --routes-only')
    machine.succeed(user + '@PYTHON@ -c \'import socket; s=socket.socket(socket.AF_UNIX); s.settimeout(8); s.connect("/run/direct-network/ready.sock"); s.sendall(b"ready\\n"); assert s.recv(32) == b"not-ready\\n"\'')
    # execute returns captured stdout; the launcher's denial diagnostic is stderr.
    status, output = machine.execute(user + 'canix-toolbelt-direct-network launch --systemctl /run/current-system/sw/bin/systemctl --unit direct-network-agent-tools.service -- /run/current-system/sw/bin/echo must-not-launch 2>&1')
    assert status == 1 and 'direct-network: ' in output and 'direct-network user anchors failed' not in output and 'must-not-launch' not in output, output
    machine.succeed(userctl + 'is-active direct-network-agent-tools.service')
    if mode == 'tcp':
        # Persistent anchors stay alive, but cannot admit a new service payload
        # after the native resolver disappears. Cover both managers and a
        # system service UID that has no direct readiness authorization.
        machine.succeed('systemctl is-active direct-network-canix-background-direct.service')
        machine.succeed(userctl + 'is-active direct-network-app-amc-direct.service')
        machine.succeed('systemctl is-active foreign-dns.service')
        rooted_status, rooted_output = machine.execute('systemctl start rooted-start.service')
        assert rooted_status == 1, (rooted_status, rooted_output)
        machine.succeed('test "$(systemctl show rooted-start.service -p Result --value)" = exit-code')
        machine.succeed('test ! -e /run/direct-network-root-state/rooted-start-pre')
        machine.succeed('test ! -e /run/direct-network-root-state/rooted-start-payload')
        machine.succeed('test ! -e /run/direct-network-root-state/rooted-start-invocation')
        system_status, system_output = machine.execute('systemctl start late-start.service')
        assert system_status == 1, (system_status, system_output)
        machine.succeed('systemctl is-active foreign-dns.service')
        user_status, user_output = machine.execute(userctl + 'start late-start.service')
        assert user_status == 1, (user_status, user_output)
        machine.succeed('systemctl is-active foreign-dns.service')
        machine.succeed('test ! -e /tmp/direct-network-late-system')
        machine.succeed('test ! -e /tmp/direct-network-late-system-pre')
        machine.succeed('test ! -e /home/operator/direct-network-late-user')
        machine.succeed('test ! -e /home/operator/direct-network-late-user-pre')
        machine.succeed('systemctl reset-failed late-start.service')
        machine.succeed(userctl + 'reset-failed late-start.service')
    if mode == 'both':
        image_status, image_output = machine.execute('systemctl start imaged-start.service')
        assert image_status == 1, (image_status, image_output)
        machine.succeed('test "$(systemctl show imaged-start.service -p Result --value)" = exit-code')
        machine.succeed('test ! -e /run/direct-network-root-state/imaged-start-pre')
        machine.succeed('test ! -e /run/direct-network-root-state/imaged-start-payload')
        machine.succeed('test ! -e /run/direct-network-root-state/imaged-start-invocation')
        rooted_user_status, rooted_user_output = machine.execute(userctl + 'start rooted-user-start.service')
        assert rooted_user_status == 1, (rooted_user_status, rooted_user_output)
        for phase in ['pre', 'payload', 'post']:
            machine.succeed(f'test ! -e /run/direct-network-root-state/rooted-user-start-{phase}')
        machine.succeed(userctl + 'reset-failed rooted-user-start.service')
    machine.succeed('systemctl is-active foreign-dns.service')
    machine.succeed('systemctl stop foreign-dns.service')
    # Successful transient services are unloaded after stop. Wait for their
    # removal before reusing the name; reset-failed requires a loaded unit.
    machine.wait_until_succeeds('test "$(systemctl show foreign-dns.service -p LoadState --value)" = not-found')
    assert machine.succeed(userctl + 'show backend.service -p MainPID --value').strip() == pid
machine.succeed('systemctl start direct-network-dns.service')
machine.wait_until_succeeds('canix-toolbelt-direct-network ready')
resume_dns_admission_probes(negative_counter)
check_direct()
# The long-running rooted payload recovers on its own from the denied retries.
machine.wait_until_succeeds('test -e /run/direct-network-root-state/rooted-retry-payload', timeout=45)
machine.wait_until_succeeds('grep -Fx post /run/direct-network-root-state/rooted-retry-post')
machine.succeed('systemctl is-active rooted-retry.service')
machine.succeed('systemctl reset-failed rooted-start.service imaged-start.service')
machine.succeed('systemctl start rooted-start.service imaged-start.service')
for name in ['rooted-start', 'imaged-start', 'rooted-retry']:
    machine.succeed(f'grep -Fx pre /run/direct-network-root-state/{name}-pre')
    machine.succeed(f'grep -Fx payload /run/direct-network-root-state/{name}-payload')
    machine.succeed(f'grep -Fx post /run/direct-network-root-state/{name}-post')
machine.succeed('systemctl start late-start.service')
machine.succeed(userctl + 'start late-start.service')
machine.succeed('test -e /tmp/direct-network-late-system')
machine.succeed('test -e /tmp/direct-network-late-system-pre')
machine.succeed('test -e /home/operator/direct-network-late-user')
machine.succeed('test -e /home/operator/direct-network-late-user-pre')
machine.succeed(userctl + 'start rooted-user-start.service')
for phase in ['pre', 'payload', 'post']:
    machine.succeed(f'grep -Fx {phase} /run/direct-network-root-state/rooted-user-start-{phase}')

# Even a fully enrolled live resolver cannot hide a competing connected UDP
# socket at an exact endpoint. Both address families must deny normal admission.
for mode in ['connected-v4', 'connected-v6', 'mapped-connected']:
    marker = f'/run/foreign-{mode}-ready'
    machine.succeed(f'systemd-run --unit=foreign-dns @PYTHON@ @NEGATIVE@ {mode} --marker {marker}')
    machine.wait_for_file(marker)
    if mode == 'mapped-connected':
        # The connected dual-stack alias must actually receive matching IPv4
        # datagrams while all four legitimate dnsmasq listeners remain alive.
        machine.succeed('@PYTHON@ -c \'import socket; s=socket.socket(socket.AF_INET,socket.SOCK_DGRAM); s.bind(("127.0.0.1",40001)); s.sendto(b"mapped-competition",("127.0.0.54",5354))\'')
        machine.wait_for_file(marker + '.received')
        machine.succeed('grep -Fx mapped-competition ' + marker + '.received')
    machine.succeed('timeout 8 canix-toolbelt-direct-network ready --routes-only')
    machine.fail('timeout 4 canix-toolbelt-direct-network ready')
    machine.succeed('systemctl stop foreign-dns.service')
    machine.wait_until_succeeds('test "$(systemctl show foreign-dns.service -p LoadState --value)" = not-found')
    machine.wait_until_succeeds('canix-toolbelt-direct-network ready')
    check_direct()

# A failed reload dispatch must be retried despite an unchanged committed file.
# The fixture denies just one dispatch and delegates subsequent effects natively.
machine.succeed('@PYTHON@ -c \'import json; p=json.load(open("/etc/direct-network-policy.json")); p.update(systemctl="@SYSTEMCTL_FIXTURE@",nmcli="@NMCLI_FIXTURE@",networkManagerDns=True,dnsServers=[]); json.dump(p,open("/run/direct-network-policy.json","w"))\'')
machine.succeed('printf "198.18.0.2\\n" >/run/direct-network-dns-upstream')
machine.succeed('mkdir -p /run/systemd/system/direct-network.service.d')
machine.succeed('printf "[Service]\\nExecStart=\\nExecStart=$(command -v canix-toolbelt-direct-network) daemon --policy /run/direct-network-policy.json\\n" >/run/systemd/system/direct-network.service.d/test-policy.conf')
machine.succeed('systemctl daemon-reload')
negative_counter = pause_dns_admission_probes()
machine.succeed('systemctl restart direct-network.service direct-network-dns.service')
machine.wait_until_succeeds('canix-toolbelt-direct-network ready')
machine.succeed('rm /run/direct-network-root-state/rooted-start-pre /run/direct-network-root-state/rooted-start-payload /run/direct-network-root-state/rooted-start-post')
machine.succeed('systemctl start rooted-start.service')
machine.succeed('grep -Fx post /run/direct-network-root-state/rooted-start-post')
for phase in ['pre', 'payload', 'post']:
    machine.succeed(f'rm /run/direct-network-root-state/rooted-user-start-{phase}')
machine.succeed(userctl + 'start rooted-user-start.service')
machine.succeed('grep -Fx post /run/direct-network-root-state/rooted-user-start-post')
machine.wait_until_succeeds('dig +short +time=2 +tries=1 @127.0.0.54 -p 5354 reload.example.test | grep -Fx 192.0.2.10')
resume_dns_admission_probes(negative_counter)
reload_pid = machine.succeed('systemctl show direct-network-dns.service -p MainPID --value').strip()
reload_before = int(machine.succeed('cat /run/direct-network/reload-attempts'))
machine.succeed('touch /run/direct-network/deny-reload')
machine.succeed('printf "198.18.0.3\\n" >/run/direct-network-dns-upstream')
machine.wait_until_succeeds('test "$(cat /run/direct-network/reload-attempts)" -ge ' + str(reload_before + 2))
machine.wait_until_succeeds('dig +short +time=2 +tries=1 @127.0.0.54 -p 5354 reload.example.test | grep -Fx 192.0.2.11')
assert machine.succeed('systemctl show direct-network-dns.service -p MainPID --value').strip() == reload_pid
machine.succeed('grep -Fx server=198.18.0.3 /run/direct-network/servers')
check_direct()
machine.succeed('rm /run/systemd/system/direct-network.service.d/test-policy.conf')
machine.succeed('systemctl daemon-reload')
negative_counter = pause_dns_admission_probes()
machine.succeed('systemctl restart direct-network.service direct-network-dns.service')
machine.wait_until_succeeds('canix-toolbelt-direct-network ready')
machine.wait_until_succeeds('dig +short +time=2 +tries=1 @127.0.0.54 -p 5354 reload.example.test | grep -Fx 192.0.2.10')
resume_dns_admission_probes(negative_counter)
check_direct()

for family in ['-4', '-6']:
    machine.succeed(f'ip {family} route add default dev wg-proton table 51820')
    machine.succeed(f'ip {family} rule add priority 32764 lookup main suppress_prefixlength 0')
    machine.succeed(f'ip {family} rule add priority 32765 not fwmark 51820 lookup 51820')
machine.succeed('resolvectl dns wg-proton 172.31.0.2')
machine.succeed("resolvectl domain wg-proton '~.'")
machine.succeed('resolvectl flush-caches')
try:
    desktop = json.loads(machine.succeed(userrun + '--slice=app.slice ' + probe))
    assert desktop['v4'] == desktop['v6'] == 'vpn', desktop
    assert desktop['fleet'] == 'direct', desktop
    assert desktop['dns'] == ['198.51.100.20', '2001:db8:30::20'], desktop
    # Real cgroups and wire-format DNS reuse the same UDP tuple in both policy
    # orders. Each query checks the transaction, answer and reverse-NAT peer;
    # no retries or conntrack flush may conceal a wrong-policy response.
    for version in [4, 6]:
        for port, slices in [(53001, ['agent-tools.slice', 'app.slice', 'agent-tools.slice']),
                             (53002, ['app.slice', 'agent-tools.slice', 'app.slice'])]:
            for slice_name in slices:
                ipv4, ipv6 = ('203.0.113.20', '2001:db8:20::20') if slice_name == 'agent-tools.slice' else ('198.51.100.20', '2001:db8:30::20')
                print(machine.succeed(userrun + f'--slice={slice_name} @PYTHON@ @FIXTURE@ dns-query {version} {port} {ipv4} {ipv6}'))
        # Socket marks owned by another engine override cgroup enrollment.
        print(machine.succeed(f'systemd-run --quiet --wait --pipe --slice=system-direct.slice @PYTHON@ @FIXTURE@ dns-query {version} 53003 198.51.100.20 2001:db8:30::20 123'))
        print(machine.succeed(f'systemd-run --quiet --wait --pipe --slice=app.slice @PYTHON@ @FIXTURE@ dns-query {version} 53004 203.0.113.20 2001:db8:20::20 3389063168'))
    # A listener reply to client port 53 still needs reverse DNAT; it must not
    # be mistaken for the resolver's own port-53 upstream query.
    print(machine.succeed('systemd-run --quiet --wait --pipe --slice=system-direct.slice @PYTHON@ @FIXTURE@ dns-query 4 53 203.0.113.20 2001:db8:20::20'))
    machine.succeed(userrun + "--slice=agent-tools.slice sh -c 'dig +tcp +short +time=2 +tries=1 @127.0.0.53 public.example.test | grep -Fx 203.0.113.20'")
except Exception:
    for command in [
        'resolvectl status',
        'nft list table inet toolbelt_direct_network',
        'conntrack -L -p udp -o extended',
        'journalctl -b -u systemd-resolved.service -u direct-network.service -u direct-network-dns.service --no-pager -n 150',
        'ss -lnptu',
    ]:
        status, output = machine.execute(command)
        print(command, status, output)
    raise
check_direct()

# A system-manager timer running as the same UID remains direct too.
machine.succeed('systemctl start backup.timer')
machine.wait_for_file('/home/operator/system-probe.json')
system = json.loads(machine.succeed('cat /home/operator/system-probe.json'))
assert system['v4'] == system['v6'] == 'direct', system
machine.succeed('ip netns exec vpnpeer wg set wg-proton listen-port 51821')
machine.succeed('wg set wg-proton peer $(cat /run/wg-proton-peer.pub) endpoint 198.19.0.2:51821')
check_direct()
machine.succeed('curl -4 --fail --max-time 5 http://203.0.113.20:8080/ | grep vpn')

# Lost tunnel + advanced-kill-switch-shaped IPv4/IPv6 dummy routes/DNS.
machine.succeed('ip link set wg-proton down')
for family in ['-4', '-6']:
    machine.succeed(f'ip {family} route flush table 51820')
machine.succeed('ip link add pvpnksintrf0 type dummy')
machine.succeed('ip address add 100.85.0.1/24 dev pvpnksintrf0')
machine.succeed('ip -6 address add fd85::1/64 dev pvpnksintrf0 nodad')
machine.succeed('ip link set pvpnksintrf0 up')
for family in ['-4', '-6']:
    machine.succeed(f'ip {family} route add default dev pvpnksintrf0 metric 1')
machine.succeed('resolvectl revert wg-proton')
machine.succeed('resolvectl dns pvpnksintrf0 100.85.0.2')
machine.succeed("resolvectl domain pvpnksintrf0 '~.'")
machine.fail('curl --fail --max-time 2 http://203.0.113.20:8080/')
check_direct()

machine.succeed('ip link del pvpnksintrf0')
machine.succeed('ip link set wg-proton up')
for family in ['-4', '-6']:
    machine.succeed(f'ip {family} route add default dev wg-proton table 51820')
machine.succeed('resolvectl dns wg-proton 172.31.0.2')
machine.succeed("resolvectl domain wg-proton '~.'")
machine.succeed('resolvectl flush-caches')
check_direct()
machine.succeed('curl --fail --max-time 5 http://203.0.113.20:8080/ | grep vpn')
for family in ['-4', '-6']:
    machine.succeed(f'ip {family} rule del priority 32764')
    machine.succeed(f'ip {family} rule del priority 32765')
machine.succeed('resolvectl revert wg-proton')
check_direct()
machine.succeed('curl --fail --max-time 5 http://203.0.113.20:8080/ | grep direct')
machine.wait_until_succeeds('@PYTHON@ -c \'import json; assert json.load(open("/home/operator/state.json"))["counter"] > ' + str(before + 3) + "'")
assert machine.succeed(userctl + 'show app-amc.slice -p MemoryMax --value').strip() == '536870912'
assert machine.succeed(userctl + 'show backend.service -p MemoryMax --value').strip() == '134217728'
