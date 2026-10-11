"""Bounded, disposable negative peers for the direct-network VM gate."""

import argparse
import pathlib
import select
import socket
import time


def silent_peers(path: str, marker: pathlib.Path, seconds: float) -> None:
    clients: list[socket.socket] = []
    connected = 0
    deadline = time.monotonic() + seconds
    try:
        while time.monotonic() < deadline:
            for client in list(clients):
                readable, _, _ = select.select([client], [], [], 0)
                if readable and not client.recv(1, socket.MSG_PEEK):
                    clients.remove(client)
                    client.close()
            while len(clients) < 16 and time.monotonic() < deadline:
                client = socket.socket(socket.AF_UNIX)
                client.settimeout(0.2)
                try:
                    client.connect(path)
                except (TimeoutError, OSError):
                    client.close()
                    break
                clients.append(client)
                connected += 1
            if connected >= 16:
                marker.touch()
            time.sleep(0.01)
    finally:
        for client in clients:
            client.close()


def foreign_listener(mode: str, marker: pathlib.Path, seconds: float) -> None:
    listeners: list[socket.socket] = []
    try:
        for family, address in [(socket.AF_INET, '127.0.0.54'), (socket.AF_INET6, '::1')]:
            if mode == 'missing-ipv6' and family == socket.AF_INET6:
                continue
            if mode == 'connected-v4' and family != socket.AF_INET:
                continue
            if mode == 'connected-v6' and family != socket.AF_INET6:
                continue
            if mode == 'mapped-connected' and family != socket.AF_INET6:
                continue
            if mode.startswith('connected-') or mode == 'mapped-connected':
                listener = socket.socket(family, socket.SOCK_DGRAM)
                listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
                if mode == 'mapped-connected':
                    address = '::ffff:127.0.0.54'
                    listener.setsockopt(socket.IPPROTO_IPV6, socket.IPV6_V6ONLY, 0)
                listener.bind((address, 5354))
                peer = '::ffff:127.0.0.1' if mode == 'mapped-connected' else '::1' if family == socket.AF_INET6 else '127.0.0.1'
                listener.connect((peer, 40001))
                listeners.append(listener)
                continue
            protocols = [socket.SOCK_STREAM] if mode == 'tcp' else [socket.SOCK_STREAM, socket.SOCK_DGRAM]
            for protocol in protocols:
                listener = socket.socket(family, protocol)
                if family == socket.AF_INET6:
                    listener.setsockopt(socket.IPPROTO_IPV6, socket.IPV6_V6ONLY, 1)
                listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
                listener.bind((address, 5354))
                if protocol == socket.SOCK_STREAM:
                    listener.listen(128)
                listeners.append(listener)
        marker.touch()
        if mode == 'mapped-connected':
            listener.settimeout(0.2)
            deadline = time.monotonic() + seconds
            while time.monotonic() < deadline:
                try:
                    data = listener.recv(1024)
                except TimeoutError:
                    continue
                if data == b'mapped-competition':
                    pathlib.Path(str(marker) + '.received').write_text(data.decode() + '\n')
        else:
            time.sleep(seconds)
    finally:
        for listener in listeners:
            listener.close()


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('mode', choices=['silent-peers', 'tcp', 'both', 'missing-ipv6', 'connected-v4', 'connected-v6', 'mapped-connected'])
    parser.add_argument('--marker', required=True, type=pathlib.Path)
    parser.add_argument('--socket', default='/run/direct-network/ready.sock')
    parser.add_argument('--seconds', type=float, default=30)
    args = parser.parse_args()
    if not 0 < args.seconds <= 120:
        parser.error('seconds must be in (0, 120]')
    if args.mode == 'silent-peers':
        silent_peers(args.socket, args.marker, args.seconds)
    else:
        foreign_listener(args.mode, args.marker, args.seconds)
