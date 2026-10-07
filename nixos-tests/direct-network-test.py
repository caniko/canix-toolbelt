import json
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
machine.wait_until_succeeds('canix-toolbelt-direct-network ready')
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
    machine.succeed('curl --fail --max-time 5 http://127.0.0.1:4096/')
    # New external listener connections must pass reverse-path filtering and
    # reply directly while the VPN owns the internet default route.
    machine.succeed('ip netns exec directpeer curl --interface 203.0.113.20 --fail --max-time 5 http://198.18.0.1:4096/')
    machine.succeed("ip netns exec directpeer curl --interface 2001:db8:20::20 --fail --max-time 5 'http://[2001:db8:1::1]:4096/'")
    assert machine.succeed(userctl + 'show backend.service -p MainPID --value').strip() == pid
    assert machine.succeed(userctl + 'show backend.service -p NRestarts --value').strip() == '0'


check_direct()
# Foreign selectors must fail closed even when priority/table/mark resemble a
# retained guard rule. A second daemon must reject them before touching state.
for family, source in [('-4', '198.18.0.0/24'), ('-6', '2001:db8:1::/64')]:
    original_rules = machine.succeed(f'ip {family} -j rule show')
    for conflict in [
        'priority 49 lookup 51822',
        'priority 50 fwmark 3389063168/0xffff0000 lookup 51822',
        f'priority 50 from {source} fwmark 3389063168 lookup 51822',
        'priority 50 iif lo fwmark 3389063168 lookup 51822',
        'not priority 50 fwmark 3389063168 lookup 51822',
    ]:
        machine.succeed(f'ip {family} rule add {conflict}')
        with_conflict = machine.succeed(f'ip {family} -j rule show')
        status, diagnostics = machine.execute('canix-toolbelt-direct-network daemon --policy /etc/direct-network-policy.json')
        assert status != 0 and 'conflict' in diagnostics, (conflict, diagnostics)
        assert machine.succeed(f'ip {family} -j rule show') == with_conflict
        machine.succeed(f'ip {family} rule del {conflict}')
    assert machine.succeed(f'ip {family} -j rule show') == original_rules
check_direct()

for family in ['-4', '-6']:
    machine.succeed(f'ip {family} route add default dev wg-proton table 51820')
    machine.succeed(f'ip {family} rule add priority 32764 lookup main suppress_prefixlength 0')
    machine.succeed(f'ip {family} rule add priority 32765 not fwmark 51820 lookup 51820')
machine.succeed('resolvectl dns wg-proton 172.31.0.2')
machine.succeed("resolvectl domain wg-proton '~.'")
machine.succeed('resolvectl flush-caches')
desktop = json.loads(machine.succeed(userrun + '--slice=app.slice ' + probe))
assert desktop['v4'] == desktop['v6'] == 'vpn', desktop
assert desktop['fleet'] == 'direct', desktop
assert desktop['dns'] == ['198.51.100.20', '2001:db8:30::20'], desktop
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
