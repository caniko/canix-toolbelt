"""Disposable network/provider fixtures; no Proton account or production network."""

import http.server
import json
import pathlib
import socket
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


def serve(label):
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

    listener = Server(('::', 8080), Handler)
    threading.Thread(target=listener.serve_forever, daemon=True).start()
    echo = socket.socket(socket.AF_INET6)
    echo.bind(('::', 8081))
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
    for namespace, label, ipv4, ipv6 in (('directpeer', 'direct', '203.0.113.20', '2001:db8:20::20'), ('vpnpeer', 'vpn', '198.51.100.20', '2001:db8:30::20')):
        subprocess.Popen(['ip', 'netns', 'exec', namespace, dnsmasq, '--keep-in-foreground', '--conf-file=/dev/null', '--no-resolv', '--no-hosts', '--pid-file=', f'--host-record=public.example.test,{ipv4},{ipv6}', '--host-record=fleet.vpn.example.test,10.123.0.1'])
        process = subprocess.Popen(['ip', 'netns', 'exec', namespace, sys.executable, '-u', __file__, 'server', label], stdout=subprocess.PIPE, text=True)
        assert process.stdout.readline().strip() == 'ready'
    pathlib.Path('/run/network-fixture-ready').touch()
    while True:
        time.sleep(60)


def probe():
    import urllib.request
    results = {}
    for key, address in (('v4', '203.0.113.20'), ('v6', '[2001:db8:20::20]'), ('fleet', '10.123.0.1')):
        results[key] = urllib.request.urlopen(f'http://{address}:8080', timeout=5).read().decode().strip()
    results['dns'] = sorted({entry[4][0] for entry in socket.getaddrinfo('public.example.test', 8080, type=socket.SOCK_STREAM)})
    results['fleetDns'] = socket.gethostbyname('fleet.vpn.example.test')
    return results


def client(state):
    # These sockets are opened before VPN activation and are never recreated.
    connections = [socket.create_connection((ip, 8081), timeout=5) for ip in ('203.0.113.20', '2001:db8:20::20')]
    counter = 0
    while True:
        for connection in connections:
            connection.sendall(b'alive')
            assert connection.recv(1024) == b'direct:alive'
        counter += 1
        results = probe()
        assert results['v4'] == results['v6'] == results['fleet'] == 'direct'
        assert results['dns'] == ['2001:db8:20::20', '203.0.113.20']
        results['counter'] = counter
        temporary = pathlib.Path(state + '.new')
        temporary.write_text(json.dumps(results))
        temporary.replace(state)
        time.sleep(0.5)


if sys.argv[1] == 'setup':
    setup(sys.argv[2])
elif sys.argv[1] == 'server':
    serve(sys.argv[2])
elif sys.argv[1] == 'client':
    client(sys.argv[2])
elif sys.argv[1] == 'probe':
    result = json.dumps(probe())
    if len(sys.argv) > 2:
        pathlib.Path(sys.argv[2]).write_text(result)
    print(result)
