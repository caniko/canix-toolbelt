"""Run the production daemon in disposable, unprivileged network namespaces.

This verifies routing/DNS mechanics and socket continuity. The VM check supplies
separate desktop/service cgroups and real systemd startup/resource boundaries.
"""

import json
import pathlib
import socket
import subprocess
import sys
import tempfile
import threading
import time

binary = str(pathlib.Path(sys.argv[1]).resolve())


def run(*args):
    result = subprocess.run(args, text=True, check=False, capture_output=True, timeout=10)
    if result.returncode:
        raise RuntimeError(f'{args!r}: {result.stderr.strip()}')
    return result.stdout


server = subprocess.Popen([
    "unshare", "--net", "python3", "-u", "-c", """
import socket, threading
def serve(family):
    s = socket.socket(family)
    if family == socket.AF_INET6: s.setsockopt(socket.IPPROTO_IPV6, socket.IPV6_V6ONLY, 1)
    s.bind(('::' if family == socket.AF_INET6 else '0.0.0.0', 8080))
    s.listen()
    def echo(c):
        with c:
            while data := c.recv(1024): c.sendall(data)
    while True:
        c, _ = s.accept()
        threading.Thread(target=echo, args=(c,), daemon=True).start()
threading.Thread(target=serve, args=(socket.AF_INET6,), daemon=True).start()
print('ready', flush=True)
serve(socket.AF_INET)
"""], stdout=subprocess.PIPE, text=True)
assert server.stdout.readline().strip() == "ready"
daemon = None
try:
    run("ip", "link", "set", "lo", "up")
    run("ip", "link", "add", "uplink", "type", "veth", "peer", "name", "peer")
    run("ip", "link", "set", "peer", "netns", str(server.pid))
    run("ip", "addr", "add", "198.18.0.1/24", "dev", "uplink")
    run("ip", "-6", "addr", "add", "2001:db8:1::1/64", "dev", "uplink", "nodad")
    run("ip", "link", "set", "uplink", "up")

    def remote(*args):
        return run("nsenter", "--net=" + str(pathlib.Path('/proc') / str(server.pid) / 'ns/net'), "ip", *args)

    remote("link", "set", "lo", "up")
    remote("addr", "add", "198.18.0.2/24", "dev", "peer")
    remote("addr", "add", "203.0.113.20/32", "dev", "lo")
    remote("-6", "addr", "add", "2001:db8:1::2/64", "dev", "peer", "nodad")
    remote("-6", "addr", "add", "2001:db8:20::20/128", "dev", "lo", "nodad")
    remote("link", "set", "peer", "up")
    remote("route", "add", "default", "via", "198.18.0.1")
    remote("-6", "route", "add", "default", "via", "2001:db8:1::1")
    run("ip", "route", "add", "default", "via", "198.18.0.2", "metric", "100")
    run("ip", "-6", "route", "add", "default", "via", "2001:db8:1::2", "metric", "100")
    # Strict reverse-path filtering should consult the direct routing mark.
    for option, value in (('rp_filter', '1'), ('src_valid_mark', '1')):
        pathlib.Path(f'/proc/sys/net/ipv4/conf/all/{option}').write_text(value)

    listener = socket.socket(socket.AF_INET6)
    listener.setsockopt(socket.IPPROTO_IPV6, socket.IPV6_V6ONLY, 0)
    listener.bind(('::', 9090))
    listener.listen()

    def listener_replies():
        while True:
            connection, _ = listener.accept()
            with connection:
                connection.sendall(b'direct-listener')

    threading.Thread(target=listener_replies, daemon=True).start()

    # Both direct DNS address families have their own listener, rather than
    # forwarding a service query through the desktop's resolved policy.
    def dns(family):
        listener = socket.socket(family, socket.SOCK_DGRAM)
        listener.bind(('::1' if family == socket.AF_INET6 else '127.0.0.54', 5354))
        while True:
            data, peer = listener.recvfrom(4096)
            listener.sendto(b'direct-dns:' + data, peer)

    for family in (socket.AF_INET, socket.AF_INET6):
        threading.Thread(target=dns, args=(family,), daemon=True).start()
    readiness = socket.socket()
    readiness.bind(('127.0.0.54', 5354))
    readiness.listen()

    with tempfile.TemporaryDirectory(prefix='direct-network-', dir=sys.argv[2]) as temp:
        directory = pathlib.Path(temp)
        policy = directory / 'policy.json'
        cgroup = pathlib.Path('/proc/self/cgroup').read_text().strip().split(':', 2)[2].strip('/')
        policy.write_text(json.dumps({
            'socket': str(directory / 'ready.sock'), 'serversFile': str(directory / 'servers'),
            'ip': run('which', 'ip').strip(), 'nft': run('which', 'nft').strip(),
            'nmcli': '/bin/true', 'systemctl': '/bin/true',
            'cgroupPaths': [cgroup], 'allowedUsers': [0], 'directInterfaces': ['uplink'],
            'dnsServers': [], 'dnsZones': {}, 'networkManagerDns': False,
        }))
        log = (directory / 'daemon.log').open('w+')
        daemon = subprocess.Popen([binary, 'daemon', '--policy', str(policy)], stdout=log, stderr=log)
        for _ in range(5):
            if daemon.poll() is not None:
                log.seek(0)
                raise AssertionError(log.read())
            try:
                run(binary, 'ready', '--socket', str(directory / 'ready.sock'), '--routes-only')
                break
            except RuntimeError:
                time.sleep(0.1)
        else:
            log.seek(0)
            raise AssertionError(log.read())
        run(binary, 'ready', '--socket', str(directory / 'ready.sock'))
        true = run('which', 'true').strip()
        echo = run('which', 'echo').strip()
        assert run(binary, 'launch', '--socket', str(directory / 'ready.sock'), '--systemctl', true,
                   '--unit', 'fixture.service', '--', echo, 'guarded-client') == 'guarded-client\n'
        assert run(binary, 'launch', '--socket', str(directory / 'ready.sock'), '--systemctl', true,
                   '--', echo, 'desktop-only-user') == 'desktop-only-user\n'
        failed_launch = subprocess.run([binary, 'launch', '--socket', str(directory / 'missing.sock'),
                                       '--systemctl', true, '--unit', 'fixture.service', '--',
                                       echo, 'must-not-launch'], check=False, capture_output=True, text=True, timeout=20)
        assert failed_launch.returncode != 0 and 'must-not-launch' not in failed_launch.stdout
        old = [socket.create_connection((ip, 8080), timeout=3)
               for ip in ('203.0.113.20', '2001:db8:20::20')]

        def verify(label):
            for connection in old:
                connection.sendall(label.encode())
                assert connection.recv(1024) == label.encode()
            for ip in ('203.0.113.20', '2001:db8:20::20'):
                with socket.create_connection((ip, 8080), timeout=3) as connection:
                    connection.sendall(label.encode())
                    assert connection.recv(1024) == label.encode()
            for family, ip in ((socket.AF_INET, '127.0.0.53'), (socket.AF_INET6, '::1')):
                with socket.socket(family, socket.SOCK_DGRAM) as query:
                    query.settimeout(3)
                    query.sendto(b'query', (ip, 53))
                    assert query.recv(1024) == b'direct-dns:query'
            try:
                for family, source, destination in (('AF_INET', '203.0.113.20', '198.18.0.1'),
                                                    ('AF_INET6', '2001:db8:20::20', '2001:db8:1::1')):
                    reply = run('nsenter', '--net=' + str(pathlib.Path('/proc') / str(server.pid) / 'ns/net'),
                                'python3', '-c', f"import socket; c=socket.socket(socket.{family}); c.settimeout(3); c.bind(('{source}',0)); c.connect(('{destination}',9090)); print(c.recv(1024).decode())")
                    assert reply.strip() == 'direct-listener'
            except RuntimeError:
                print(run('nft', 'list', 'table', 'inet', 'toolbelt_direct_network'))
                raise

        verify('before')
        for transition in ('kill-switch', 'reconnect', 'server-change'):
            run('ip', 'link', 'add', 'pvpnksintrf0', 'type', 'dummy')
            run('ip', 'addr', 'add', '100.85.0.1/24', 'dev', 'pvpnksintrf0')
            run('ip', '-6', 'addr', 'add', 'fd85::1/64', 'dev', 'pvpnksintrf0', 'nodad')
            run('ip', 'link', 'set', 'pvpnksintrf0', 'up')
            run('ip', 'route', 'add', 'default', 'dev', 'pvpnksintrf0', 'metric', '1')
            run('ip', '-6', 'route', 'add', 'default', 'dev', 'pvpnksintrf0', 'metric', '1')
            verify(transition)
            # A separately marked control socket is outside the exemption and
            # must obey the simulated kill switch, not leak onto the direct path.
            with socket.socket() as desktop:
                desktop.setsockopt(socket.SOL_SOCKET, socket.SO_MARK, 123)
                desktop.settimeout(0.2)
                try:
                    desktop.connect(('203.0.113.20', 8080))
                except (OSError, TimeoutError):
                    pass
                else:
                    raise AssertionError('unprotected traffic bypassed kill switch')
            run('ip', 'link', 'del', 'pvpnksintrf0')
            verify('disconnect')
        daemon.terminate()
        daemon.wait(timeout=5)
        daemon = None
        verify('daemon-stopped-kernel-protection-retained')
        # Priority/table/mark alone do not establish ownership. A foreign rule
        # may additionally restrict the source, mask, interface or invert match.
        (directory / 'ready.sock').unlink()
        for family, source in (('-4', '198.18.0.0/24'), ('-6', '2001:db8:1::/64')):
            before_rules = run('ip', family, '-j', 'rule', 'show')
            used_tables = {str(rule['table']) for rule in json.loads(before_rules)}
            table = next(table for table in ['51821', '51822'] if table not in used_tables)
            conflicts = [
                ['priority', '49', 'lookup', '123'],
                ['priority', '49', 'lookup', table],
                ['priority', '50', 'fwmark', '3389063168/0xffff0000', 'lookup', table],
                ['priority', '50', 'from', source, 'fwmark', '3389063168', 'lookup', table],
                ['priority', '50', 'iif', 'lo', 'fwmark', '3389063168', 'lookup', table],
                ['not', 'priority', '50', 'fwmark', '3389063168', 'lookup', table],
            ]
            for conflict in conflicts:
                run('ip', family, 'rule', 'add', *conflict)
                with_conflict = run('ip', family, '-j', 'rule', 'show')
                rejected = subprocess.run([binary, 'daemon', '--policy', str(policy)], capture_output=True, text=True, timeout=5, check=False)
                assert rejected.returncode != 0 and 'conflict' in rejected.stderr, conflict
                assert run('ip', family, '-j', 'rule', 'show') == with_conflict, conflict
                run('ip', family, 'rule', 'del', *conflict)
            assert run('ip', family, '-j', 'rule', 'show') == before_rules
        # Restart with the retained tables and sockets: no active-table flush.
        daemon = subprocess.Popen([binary, 'daemon', '--policy', str(policy)], stdout=log, stderr=log)
        for _ in range(5):
            try:
                run(binary, 'ready', '--socket', str(directory / 'ready.sock'))
                break
            except RuntimeError:
                time.sleep(0.1)
        else:
            log.seek(0)
            raise AssertionError(log.read())
        verify('daemon-restarted-existing-connections-survive')
        print('PASS: production daemon; existing/new IPv4/IPv6 TCP, UDP DNS, repeated VPN-shaped transitions and retained protection')
finally:
    if daemon is not None:
        daemon.terminate()
        daemon.wait(timeout=5)
    server.terminate()
    server.wait(timeout=5)
