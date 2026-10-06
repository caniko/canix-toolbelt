"""Prove fixture cleanup against a real detached process in a private namespace."""

import json
import subprocess
import sys
import tempfile
from pathlib import Path

from namespace import cleanup, fixture, processes, require_isolation, wait_for


def detached_child(ready):
    parent = subprocess.Popen([sys.executable, "-c", """
import os, signal, sys, time
from pathlib import Path
if os.fork() == 0:
    os.setsid()
    signal.signal(signal.SIGTERM, signal.SIG_IGN)
    Path(sys.argv[1]).write_text(str(os.getpid()))
    while True:
        time.sleep(1)
""", str(ready)])
    assert parent.wait(timeout=10) == 0
    wait_for(ready.exists, "detached fixture child")
    assert int(ready.read_text()) in processes(), "fixture did not create the detached child"


def main():
    require_isolation()
    with tempfile.TemporaryDirectory(prefix="rr-cleanup-") as directory:
        detached_child(Path(directory) / "ready")
        assert cleanup() == [], "cleanup failed"
        assert not processes(), "cleanup left a descendant or zombie"
        receipt_path = Path(directory) / "failure.json"
        try:
            with fixture(receipt_path, {}, "rr-cleanup-error-") as root:
                detached_child(root / "ready")
                raise AssertionError("intentional fixture failure")
        except AssertionError as error:
            assert str(error) == "intentional fixture failure", error
        receipt = json.loads(receipt_path.read_text())
        assert receipt["exit_code"] == 1 and receipt["cleanup"] == "verified", receipt
        assert "intentional fixture failure" in receipt["error"], receipt
        assert not root.exists() and not processes(), "failure cleanup did not finish before the receipt"
    print("private namespace detached-child cleanup passed")


if __name__ == "__main__":
    main()
