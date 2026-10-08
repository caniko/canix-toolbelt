"""Disposable age protocol fixture; never use its plaintext key stanzas outside tests."""

import base64
import os
import sys


def trace(event):
    path = os.environ.get("PINENTRY_FIXTURE_TRACE")
    if path:
        with open(path, "a", encoding="utf-8") as output:
            # State names only: never record the file key or secret response.
            output.write(event + "\n")


def receive():
    header = sys.stdin.buffer.readline().decode().strip().split()
    assert header and header[0] == "->", header
    chunks = []
    while True:
        line = sys.stdin.buffer.readline().strip()
        chunks.append(line)
        if len(line) < 64:
            break
    encoded = b"".join(chunks)
    return header[1], header[2:], base64.b64decode(encoded + b"=" * (-len(encoded) % 4))


def send(command, args=(), body=b""):
    sys.stdout.buffer.write(("-> " + " ".join([command, *args]) + "\n").encode())
    encoded = base64.b64encode(body).rstrip(b"=")
    for offset in range(0, len(encoded), 64):
        sys.stdout.buffer.write(encoded[offset:offset + 64] + b"\n")
    if len(encoded) % 64 == 0:
        sys.stdout.buffer.write(b"\n")
    sys.stdout.buffer.flush()


key = None
while True:
    command, args, body = receive()
    # rage also sends grease stanzas. Only our own recipient stanza contains
    # the disposable file key; later grease must not replace it.
    if command == "wrap-file-key" or (command == "recipient-stanza" and args[1:] == ["fixture"]):
        key = body
    if command == "done":
        break

assert key is not None and len(key) == 16
if sys.argv[1] == "--age-plugin=recipient-v1":
    send("recipient-stanza", ["0", "fixture"], key)
    assert receive()[0] == "ok"
else:
    trace("confirmation requested")
    send("confirm", ["eWVz", "bm8"], b"Fixture_confirmation")
    command, args, _ = receive()
    trace("confirmation accepted" if command == "ok" and args == ["yes"] else "confirmation rejected")
    if command != "ok" or args != ["yes"]:
        send("done")
        sys.exit(0)
    trace("PIN requested")
    send("request-secret", body=b"Fixture_age_PIN")
    command, _, pin = receive()
    if command != "ok":
        trace("PIN unavailable")
    else:
        trace("PIN accepted" if pin == b"age-fixture-pin" else "PIN unexpected response")
    if command != "ok" or pin != b"age-fixture-pin":
        send("done")
        sys.exit(0)
    send("file-key", ["0"], key)
    assert receive()[0] == "ok"
send("done")
