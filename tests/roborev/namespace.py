"""Process accounting and cleanup confined to an offline fixture namespace."""

import json
import os
import signal
import subprocess
import sys
import tempfile
import time
from contextlib import contextmanager
from pathlib import Path


def require_isolation():
    assert os.getpid() == 1, "use unshare --user --map-root-user --net --pid --fork --mount-proc"
    assert Path("/proc/self/ns/pid").readlink() == Path("/proc/1/ns/pid").readlink()
    interfaces = {line.split(":", 1)[0].strip()
                  for line in Path("/proc/net/dev").read_text().splitlines()[2:]}
    assert interfaces == {"lo"}, "fixture needs its own offline network namespace"


def processes():
    return sorted(int(path.name) for path in Path("/proc").iterdir()
                  if path.name.isdigit() and int(path.name) > 1)


def reap(exclude=()):
    # Popen still owns the exit statuses of the excluded foreground handles.
    for pid in processes():
        if pid in exclude:
            continue
        try:
            os.waitpid(pid, os.WNOHANG)
        except ChildProcessError:
            pass


def wait_for(predicate, description, timeout=30):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        value = predicate()
        if value:
            return value
        time.sleep(0.1)
    raise AssertionError(f"timed out: {description}")


def communicate(command, timeout, exclude=()):
    deadline = time.monotonic() + timeout
    while True:
        try:
            return command.communicate(timeout=0.1)
        except subprocess.TimeoutExpired:
            # Detached daemons become PID 1's children. A stop client waits
            # for them to disappear, including their zombie process entries.
            reap(exclude=(command.pid, *exclude))
            if time.monotonic() >= deadline:
                command.kill()
                stdout, stderr = command.communicate(timeout=10)
                raise subprocess.TimeoutExpired(command.args, timeout, stdout, stderr) from None


def cleanup():
    require_isolation()
    deadline = time.monotonic() + 10
    while True:
        # Repeat the sweep to include children forked during an earlier sweep.
        for pid in processes():
            try:
                os.kill(pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
        reap()
        remaining = processes()
        if not remaining:
            return []
        if time.monotonic() >= deadline:
            return [f"namespace cleanup left processes: {remaining}"]
        time.sleep(0.1)


@contextmanager
def fixture(receipt_path, receipt, prefix):
    require_isolation()
    assert not processes(), "fixture namespace already contains child processes"
    # Keep daemon sockets below a short root; receipt directories may be deep.
    temporary = tempfile.TemporaryDirectory(prefix=prefix, dir=os.environ.get("ROBOREV_TEST_TMPDIR"))
    try:
        yield Path(temporary.name)
    finally:
        error = sys.exc_info()[1]
        errors = cleanup()
        try:
            temporary.cleanup()
        except OSError as cleanup_error:
            errors.append(str(cleanup_error))
        receipt["cleanup"] = "verified" if not errors else "failed"
        receipt["cleanup_errors"] = errors
        if error is not None:
            receipt["error"] = f"{type(error).__name__}: {error}"
        receipt["exit_code"] = int(error is not None or bool(errors))
        receipt_path.write_text(json.dumps(receipt, indent=2) + "\n")
        if error is None:
            assert not errors, errors
