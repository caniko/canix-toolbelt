"""Disposable network/provider fixtures; no Proton account or production network."""

import http.server
import json
import pathlib
import socket
import struct
import subprocess
import sys
import threading
import time


def run(*args):
    return subprocess.check_output(args, text=True).strip()


def ip(namespace, *args):
    return run('ip', '-n', namespace, *args) if namespace else run('ip', *args)


def wg(namespace, *args):
    return run('ip', 'netns', 'exec', namespace, 'wg', *args) if namespace else run('wg', *args)


def keys(name):
    private = pathlib.Path('/run') / f'{name}.key'
    private.write_text(run('wg', 'genkey') + '\n')
    private.chmod(0o600)
    public = subprocess.check_output(['wg', 'pubkey'], input=private.read_bytes(), text=False).decode().strip()
    pathlib.Path(f'/run/{name}.pub').write_text(public)
    return str(private), public


def tunnel(host, peer, namespace, host_ip, peer_ip, endpoint, allowed, mark):
    host_key, host_public = keys(host + '-host')
    peer_key, peer_public = keys(host + '-peer')
    for ns, name, address, key in ((None, host, host_ip, host_key), (namespace, peer, peer_ip, peer_key)):
        ip(ns, 'link', 'add', name, 'type', 'wireguard')
        ip(ns, 'address', 'add', address, 'dev', name)
        port = 51820 if ns else (51819 if name == 'wg-home' else 51818)
        wg(ns, 'set', name, 'private-key', key, 'listen-port', str(port))
        ip(ns, 'link', 'set', name, 'up')
    wg(None, 'set', host, 'fwmark', str(mark), 'peer', peer_public, 'endpoint', endpoint, 'allowed-ips', allowed)
    wg(namespace, 'set', peer, 'peer', host_public, 'allowed-ips', host_ip)
    return peer_public


def serve(label, port=8080):
    class Handler(http.server.BaseHTTPRequestHandler):
        def do_GET(self):
            body = (label + '\n').encode()
            self.send_response(200)
            self.send_header('Content-Length', str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def log_message(self, *args):
            pass

    class Server(http.server.ThreadingHTTPServer):
        address_family = socket.AF_INET6

    listener = Server(('::', port), Handler)
    threading.Thread(target=listener.serve_forever, daemon=True).start()
    echo = socket.socket(socket.AF_INET6)
    echo.bind(('::', port + 1))
    echo.listen()

    def connection(client):
        with client:
            while data := client.recv(1024):
                client.sendall(label.encode() + b':' + data)

    print('ready', flush=True)
    while True:
        client, _ = echo.accept()
        threading.Thread(target=connection, args=(client,), daemon=True).start()


def setup(dnsmasq):
    for namespace, local, remote, subnet in (('directpeer', 'uplink', 'directpeer0', '198.18.0'), ('vpnpeer', 'outer', 'vpnpeer0', '198.19.0')):
        ip(None, 'netns', 'add', namespace)
        ip(None, 'link', 'add', local, 'type', 'veth', 'peer', 'name', remote)
        ip(None, 'link', 'set', remote, 'netns', namespace)
        ip(None, 'address', 'add', f'{subnet}.1/24', 'dev', local)
        ip(None, 'link', 'set', local, 'up')
        ip(namespace, 'address', 'add', f'{subnet}.2/24', 'dev', remote)
        ip(namespace, 'link', 'set', remote, 'up')
        ip(namespace, 'link', 'set', 'lo', 'up')
        ip(namespace, 'route', 'add', 'default', 'via', f'{subnet}.1')
        ip(namespace, 'address', 'add', '203.0.113.20/32', 'dev', 'lo')
        ip(namespace, '-6', 'address', 'add', '2001:db8:20::20/128', 'dev', 'lo', 'nodad')
    ip(None, '-6', 'address', 'add', '2001:db8:1::1/64', 'dev', 'uplink', 'nodad')
    ip('directpeer', '-6', 'address', 'add', '2001:db8:1::2/64', 'dev', 'directpeer0', 'nodad')
    ip(None, 'link', 'add', 'link', 'uplink', 'name', 'uplink.37', 'type', 'vlan', 'id', '37')
    ip(None, 'address', 'add', '198.18.37.1/24', 'dev', 'uplink.37')
    ip(None, '-6', 'address', 'add', '2001:db8:37::1/64', 'dev', 'uplink.37', 'nodad')
    ip(None, 'link', 'set', 'uplink.37', 'up')
    ip('directpeer', 'link', 'add', 'link', 'directpeer0', 'name', 'peer.37', 'type', 'vlan', 'id', '37')
    ip('directpeer', 'address', 'add', '198.18.37.2/24', 'dev', 'peer.37')
    ip('directpeer', '-6', 'address', 'add', '2001:db8:37::2/64', 'dev', 'peer.37', 'nodad')
    ip('directpeer', 'link', 'set', 'peer.37', 'up')
    ip('directpeer', '-6', 'route', 'add', 'default', 'via', '2001:db8:1::1')
    ip(None, 'route', 'add', 'default', 'via', '198.18.0.2', 'metric', '100')
    ip(None, '-6', 'route', 'add', 'default', 'via', '2001:db8:1::2', 'metric', '100')
    tunnel('wg-home', 'wg-home', 'directpeer', '10.123.0.2/32', '10.123.0.1/32', '198.18.0.2:51820', '10.123.0.0/24', 0xca010000)
    ip(None, 'route', 'add', '10.123.0.0/24', 'dev', 'wg-home')
    ip('directpeer', 'route', 'add', '10.123.0.2/32', 'dev', 'wg-home')
    tunnel('wg-proton', 'wg-proton', 'vpnpeer', '172.31.0.1/32', '172.31.0.2/32', '198.19.0.2:51820', '0.0.0.0/0,::/0', 51820)
    ip(None, '-6', 'address', 'add', '2001:db8:31::1/128', 'dev', 'wg-proton', 'nodad')
    ip('vpnpeer', '-6', 'address', 'add', '2001:db8:31::2/128', 'dev', 'wg-proton', 'nodad')
    wg('vpnpeer', 'set', 'wg-proton', 'peer', pathlib.Path('/run/wg-proton-host.pub').read_text(), 'allowed-ips', '172.31.0.1/32,2001:db8:31::1/128')
    ip('vpnpeer', 'route', 'add', '172.31.0.1/32', 'dev', 'wg-proton')
    ip('vpnpeer', '-6', 'route', 'add', '2001:db8:31::1/128', 'dev', 'wg-proton')
    ip('vpnpeer', 'address', 'add', '198.51.100.20/32', 'dev', 'lo')
    ip('vpnpeer', '-6', 'address', 'add', '2001:db8:30::20/128', 'dev', 'lo', 'nodad')
    dns_processes = []
    for namespace, label, ipv4, ipv6 in (('directpeer', 'direct', '203.0.113.20', '2001:db8:20::20'), ('vpnpeer', 'vpn', '198.51.100.20', '2001:db8:30::20')):
        addresses = '198.18.0.2,10.123.0.1' if namespace == 'directpeer' else '198.19.0.2,172.31.0.2'
        dns_process = subprocess.Popen(['ip', 'netns', 'exec', namespace, dnsmasq, '--keep-in-foreground', '--conf-file=/dev/null', '--no-resolv', '--no-hosts', '--bind-interfaces', f'--listen-address={addresses}', '--pid-file=', f'--host-record=public.example.test,{ipv4},{ipv6}', '--host-record=fleet.vpn.example.test,10.123.0.1', '--host-record=reload.example.test,192.0.2.10'])
        dns_processes.append((namespace, dns_process, addresses.split(','), ipv4, ipv6))
        process = subprocess.Popen(['ip', 'netns', 'exec', namespace, sys.executable, '-u', __file__, 'server', label], stdout=subprocess.PIPE, text=True)
        assert process.stdout.readline().strip() == 'ready'
    ip('directpeer', 'address', 'add', '198.18.0.3/24', 'dev', 'directpeer0')
    alternate_dns = subprocess.Popen(['ip', 'netns', 'exec', 'directpeer', dnsmasq, '--keep-in-foreground', '--conf-file=/dev/null', '--no-resolv', '--no-hosts', '--bind-interfaces', '--listen-address=198.18.0.3', '--pid-file=', '--host-record=public.example.test,203.0.113.20,2001:db8:20::20', '--host-record=fleet.vpn.example.test,10.123.0.1', '--host-record=reload.example.test,192.0.2.11'])
    dns_processes.append(('directpeer', alternate_dns, ['198.18.0.3'], '203.0.113.20', '2001:db8:20::20'))
    # HTTP readiness does not establish DNS startup. Publish the fixture marker
    # only after each real DNS process answers at all its declared addresses.
    for namespace, process, addresses, ipv4, ipv6 in dns_processes:
        deadline = time.monotonic() + 10
        for address in addresses:
            while True:
                assert process.poll() is None, (namespace, address, process.returncode)
                result = subprocess.run(['ip', 'netns', 'exec', namespace, sys.executable, __file__, 'dns-query', '4', '0', ipv4, ipv6, '0', address], capture_output=True, text=True, timeout=5, check=False)
                if result.returncode == 0:
                    print(result.stdout, flush=True)
                    break
                assert time.monotonic() < deadline, (namespace, address, result.stderr)
                time.sleep(0.05)
    pathlib.Path('/run/network-fixture-ready').touch()
    while True:
        time.sleep(60)


def probe(include_dns=True):
    import urllib.request
    results = {}
    for key, address in (('v4', '203.0.113.20'), ('v6', '[2001:db8:20::20]'), ('fleet', '10.123.0.1')):
        results[key] = urllib.request.urlopen(f'http://{address}:8080', timeout=5).read().decode().strip()
    if include_dns:
        results['dns'] = sorted({entry[4][0] for entry in socket.getaddrinfo('public.example.test', 8080, type=socket.SOCK_STREAM)})
        results['fleetDns'] = socket.gethostbyname('fleet.vpn.example.test')
    return results


def dns_query(version, source_port, ipv4, ipv6, mark=0, address=None):
    family = socket.AF_INET if version == 4 else socket.AF_INET6
    address = address or ('127.0.0.53' if version == 4 else '::1')
    question_name = b''.join(bytes([len(label)]) + label for label in b'public.example.test'.split(b'.')) + b'\0'
    observations = []
    for record_type, expected, answer_family in [(1, ipv4, socket.AF_INET), (28, ipv6, socket.AF_INET6)]:
        transaction = 0xa100 + record_type
        question = question_name + struct.pack('!HH', record_type, 1)
        request = struct.pack('!HHHHHH', transaction, 0x0100, 1, 0, 0, 0) + question
        with socket.socket(family, socket.SOCK_DGRAM) as query:
            query.settimeout(2)
            if mark:
                query.setsockopt(socket.SOL_SOCKET, socket.SO_MARK, struct.pack('I', mark))
            if source_port:
                query.bind(('127.0.0.1' if version == 4 else '::1', source_port))
            query.sendto(request, (address, 53))
            response, peer = query.recvfrom(4096)
        assert peer[:2] == (address, 53), peer
        identity, flags, questions, answers, _, _ = struct.unpack('!HHHHHH', response[:12])
        assert identity == transaction and flags & 0x8000 and not flags & 0x020f, (identity, flags)
        assert questions == 1 and response[12:12 + len(question)] == question, response.hex()
        offset = 12 + len(question)
        values = []
        for _ in range(answers):
            # Skip the answer owner, accepting both literal and compressed names.
            while response[offset]:
                if response[offset] & 0xc0 == 0xc0:
                    offset += 2
                    break
                offset += 1 + response[offset]
            else:
                offset += 1
            kind, record_class, _, length = struct.unpack('!HHIH', response[offset:offset + 10])
            offset += 10
            data = response[offset:offset + length]
            offset += length
            if kind == record_type and record_class == 1:
                values.append(socket.inet_ntop(answer_family, data))
        assert values == [expected], (version, source_port, mark, record_type, values, expected)
        observations.append({'transaction': transaction, 'type': record_type, 'peer': peer, 'answers': values})
    return {'version': version, 'sourcePort': source_port, 'mark': mark, 'queries': observations}


def client(state):
    # These sockets are opened before VPN activation and are never recreated.
    connections = [socket.create_connection((ip, 8081), timeout=5) for ip in ('203.0.113.20', '2001:db8:20::20')]
    counter = 0
    while True:
        for connection in connections:
            connection.sendall(b'alive')
            assert connection.recv(1024) == b'direct:alive'
        counter += 1
        # Deliberate non-answering DNS admission negatives are orthogonal to
        # established TCP continuity. Heartbeats and fresh HTTP probes continue;
        # the VM checks NSS before and after this explicitly bounded fault phase.
        dns_negative = pathlib.Path('/run/direct-network-dns-admission-negative').exists()
        results = probe(include_dns=not dns_negative)
        assert results['v4'] == results['v6'] == results['fleet'] == 'direct'
        if not dns_negative:
            assert results['dns'] == ['2001:db8:20::20', '203.0.113.20']
        results['dnsAdmissionNegative'] = dns_negative
        results['counter'] = counter
        temporary = pathlib.Path(state + '.new')
        temporary.write_text(json.dumps(results))
        temporary.replace(state)
        time.sleep(0.5)


if sys.argv[1] == 'setup':
    setup(sys.argv[2])
elif sys.argv[1] == 'server':
    serve(sys.argv[2], int(sys.argv[3]) if len(sys.argv) > 3 else 8080)
elif sys.argv[1] == 'client':
    client(sys.argv[2])
elif sys.argv[1] == 'probe':
    result = json.dumps(probe())
    if len(sys.argv) > 2:
        pathlib.Path(sys.argv[2]).write_text(result)
    print(result)
elif sys.argv[1] == 'dns-query':
    print(json.dumps(dns_query(int(sys.argv[2]), int(sys.argv[3]), sys.argv[4], sys.argv[5], int(sys.argv[6]) if len(sys.argv) > 6 else 0, sys.argv[7] if len(sys.argv) > 7 else None)), flush=True)
