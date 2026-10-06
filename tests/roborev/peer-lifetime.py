"""Deterministic stale-socket PID reuse, confined to a disposable PID namespace."""

import hashlib
import json
import os
import socket
import subprocess
import sys
from pathlib import Path

from namespace import fixture, require_isolation


def main():
    require_isolation()
    binary = str(Path(sys.argv[1]).resolve())
    destination = Path(sys.argv[2]).resolve()
    destination.mkdir(parents=True, exist_ok=True)
    receipt = {"tier": "offline-kernel-peer-lifetime", "binary": binary,
               "binary_sha256": hashlib.sha256(Path(binary).read_bytes()).hexdigest()}
    with fixture(destination / "receipt.json", receipt, "rr-peer-life-") as root:
        endpoint = root / "peer.sock"
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as listener:
            listener.bind(str(endpoint))
            endpoint.chmod(0o600)
            listener.listen(2)
            listener.settimeout(5)
            old = subprocess.Popen([binary, "peer-stale-client", str(endpoint)],
                                   stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
            assert old.wait(timeout=5) == 0, old.stderr.read()
            old.stderr.close()
            # ns_last_pid belongs to THIS isolated PID namespace. It deliberately
            # assigns the released PID to the next approved direct child; no host
            # PID, production sysctl, daemon or hardware setting is changed.
            Path("/proc/sys/kernel/ns_last_pid").write_text(str(old.pid - 1))
            replacement = subprocess.Popen([binary, "peer-client", str(endpoint)],
                                           stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
            try:
                assert replacement.pid == old.pid, (old.pid, replacement.pid)
                with listener.accept()[0] as stale:
                    assert stale.recv(1) == b"x"
                    observed = subprocess.run([binary, "authenticate-fd", str(os.getpid()), binary, str(stale.fileno())],
                                              pass_fds=(stale.fileno(),), capture_output=True, timeout=5, check=False)
                    assert observed.returncode != 0, "stale socket authenticated the replacement PID"
                with listener.accept()[0] as current:
                    assert current.recv(1) == b"x"
                    observed = subprocess.run([binary, "authenticate-fd", str(os.getpid()), binary, str(current.fileno())],
                                              pass_fds=(current.fileno(),), capture_output=True, timeout=5, check=False)
                    assert observed.returncode == 0, observed.stderr
                    identity = json.loads(observed.stdout)
                    assert identity["pid"] == replacement.pid
                    current.sendall(b"x")
                assert replacement.wait(timeout=5) == 0, replacement.stderr.read()
                receipt.update({"reused_pid": replacement.pid, "replacement_identity": identity,
                                "stale_connection_rejected": True, "current_connection_authenticated": True})
            finally:
                if replacement.poll() is None:
                    replacement.terminate()
                replacement.wait(timeout=5)
                replacement.stderr.close()
    print(json.dumps(receipt))


if __name__ == "__main__":
    main()
