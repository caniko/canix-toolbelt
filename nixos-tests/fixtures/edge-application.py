"""Small HTTP/WebSocket and TCP metadata targets for the isolated edge VM test."""

import asyncio
import hashlib

from aiohttp import WSMsgType, web


async def headers(request):
    return web.json_response(dict(request.headers))


async def upload(request):
    digest = hashlib.sha256()
    async for chunk in request.content.iter_chunked(65536):
        digest.update(chunk)
    return web.Response(text=digest.hexdigest())


async def download(request):
    response = web.StreamResponse()
    await response.prepare(request)
    for _ in range(128):
        await response.write(b"x" * 65536)
    await response.write_eof()
    return response


async def websocket(request):
    response = web.WebSocketResponse()
    await response.prepare(request)
    async for message in response:
        if message.type == WSMsgType.TEXT:
            await response.send_str(message.data)
    return response


async def git(reader, writer):
    writer.write(b"SSH-2.0-edge-fixture\r\n")
    await writer.drain()
    writer.close()
    await writer.wait_closed()


async def proxy(reader, writer):
    header = await asyncio.wait_for(reader.readline(), 5)
    writer.write(header)
    await writer.drain()
    writer.close()
    await writer.wait_closed()


async def main():
    app = web.Application()
    app.add_routes(
        [
            web.get("/", headers),
            web.post("/upload", upload),
            web.get("/download", download),
            web.get("/ws", websocket),
        ]
    )
    runner = web.AppRunner(app)
    await runner.setup()
    await web.TCPSite(runner, "127.0.0.1", 8082).start()
    async with (
        await asyncio.start_server(git, "10.77.0.2", 2222),
        await asyncio.start_server(proxy, "10.77.0.2", 2525),
    ):
        await asyncio.Future()


if __name__ == "__main__":
    asyncio.run(main())
