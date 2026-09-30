"""Offline aioquic 1.3 WebTransport fixture; no production service code.

Exercises Extended CONNECT, HTTP datagrams and WebTransport streams, following
aioquic's pinned examples/http3_server.py and examples/demo.py contracts.
"""

import argparse
import asyncio
import os
import subprocess

from aioquic.asyncio import QuicConnectionProtocol, connect, serve
from aioquic.h3.connection import H3_ALPN, H3Connection
from aioquic.h3.events import (
    DatagramReceived,
    HeadersReceived,
    WebTransportStreamDataReceived,
)
from aioquic.quic.configuration import QuicConfiguration
from aioquic.quic.events import ProtocolNegotiated


class Protocol(QuicConnectionProtocol):
    def __init__(self, *args, **kwargs):
        super().__init__(*args, **kwargs)
        self.http = None
        self.session = None
        self.status = asyncio.Queue()
        self.datagrams = asyncio.Queue()
        self.streams = asyncio.Queue()
        self.buffers = {}
        self.replies = {}

    def quic_event_received(self, event):
        if isinstance(event, ProtocolNegotiated):
            assert event.alpn_protocol in H3_ALPN
            self.http = H3Connection(self._quic, enable_webtransport=True)
        if self.http is None:
            return
        for received in self.http.handle_event(event):
            if self._quic.configuration.is_client:
                if isinstance(received, HeadersReceived):
                    self.status.put_nowait(dict(received.headers)[b":status"])
                elif isinstance(received, DatagramReceived):
                    self.datagrams.put_nowait(received.data)
                elif isinstance(received, WebTransportStreamDataReceived):
                    assert received.session_id == self.session
                    data = self.buffers.setdefault(received.stream_id, bytearray())
                    data.extend(received.data)
                    if received.stream_ended:
                        self.streams.put_nowait(bytes(self.buffers.pop(received.stream_id)))
            else:
                self.receive_server(received)
        self.transmit()

    def receive_server(self, event):
        if isinstance(event, HeadersReceived):
            headers = dict(event.headers)
            accepted = (
                headers.get(b":method") == b"CONNECT"
                and headers.get(b":protocol") == b"webtransport"
                and headers.get(b":path") == b"/wt"
            )
            if accepted:
                self.session = event.stream_id
            self.http.send_headers(
                event.stream_id,
                [(b":status", b"200" if accepted else b"400")],
                end_stream=not accepted,
            )
        elif isinstance(event, DatagramReceived) and event.stream_id == self.session:
            self.http.send_datagram(event.stream_id, event.data)
        elif isinstance(event, WebTransportStreamDataReceived):
            assert event.session_id == self.session
            if event.stream_id not in self.replies:
                self.replies[event.stream_id] = self.http.create_webtransport_stream(
                    self.session, is_unidirectional=True
                )
            self._quic.send_stream_data(
                self.replies[event.stream_id], event.data, end_stream=event.stream_ended
            )

    async def open_session(self):
        self.session = self._quic.get_next_available_stream_id()
        self.http.send_headers(
            self.session,
            [
                (b":method", b"CONNECT"),
                (b":scheme", b"https"),
                (b":authority", b"app.example.test:8443"),
                (b":path", b"/wt"),
                (b":protocol", b"webtransport"),
            ],
        )
        self.transmit()
        assert await asyncio.wait_for(self.status.get(), 20) == b"200"

    async def roundtrip(self, size):
        payload = os.urandom(size)
        stream = self.http.create_webtransport_stream(
            self.session, is_unidirectional=True
        )
        self._quic.send_stream_data(stream, payload, end_stream=True)
        self.transmit()
        assert await asyncio.wait_for(self.streams.get(), 60) == payload
        # Datagrams are unreliable by design. Require one matching round trip
        # under loss, with a bounded retry, rather than pretending they are TCP.
        datagram = os.urandom(1000)
        for _ in range(10):
            self.http.send_datagram(self.session, datagram)
            self.transmit()
            try:
                if await asyncio.wait_for(self.datagrams.get(), 1) == datagram:
                    return
            except TimeoutError:
                pass
        raise AssertionError("WebTransport datagram round trip timed out")


async def main(args):
    config = QuicConfiguration(
        is_client=args.mode == "client",
        alpn_protocols=H3_ALPN,
        max_datagram_frame_size=65536,
        idle_timeout=60,
    )
    if args.mode == "server":
        config.load_cert_chain(args.certificate, args.key)
        server = await serve(
            args.host, args.port, configuration=config, create_protocol=Protocol
        )
        try:
            await asyncio.Future()
        finally:
            server.close()
    else:
        config.server_name = "app.example.test"
        config.load_verify_locations(args.ca)
        async with connect(
            args.host, args.port, configuration=config, create_protocol=Protocol
        ) as client:
            await client.open_session()
            await client.roundtrip(args.size)
            if args.rebind:
                # Test-VM-only NAT change on the same QUIC connection/socket.
                # The port becomes observable at the public edge after conntrack
                # expiry; no application-level reconnect is made here.
                await asyncio.to_thread(
                    subprocess.run,
                    ["nft", "-f", "-"],
                    input=f"""table ip wt_rebind {{
                        chain postrouting {{
                            type nat hook postrouting priority srcnat;
                            ip daddr {args.host} udp dport {args.port} snat to :40000
                        }}
                    }}""",
                    text=True,
                    check=True,
                )
                await asyncio.to_thread(
                    subprocess.run,
                    ["conntrack", "-D", "-p", "udp", "--dport", str(args.port)],
                    check=True,
                )
                try:
                    await client.roundtrip(args.size)
                finally:
                    await asyncio.to_thread(
                        subprocess.run,
                        ["nft", "delete", "table", "ip", "wt_rebind"],
                        check=True,
                    )
            print("WebTransport CONNECT, stream and datagram round trips passed")


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("mode", choices=["server", "client"])
    parser.add_argument("--host", required=True)
    parser.add_argument("--port", type=int, default=8443)
    parser.add_argument("--certificate")
    parser.add_argument("--key")
    parser.add_argument("--ca")
    parser.add_argument("--size", type=int, default=4 * 1024 * 1024)
    parser.add_argument("--rebind", action="store_true")
    asyncio.run(main(parser.parse_args()))
