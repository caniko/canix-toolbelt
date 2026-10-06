"""Stall an actual preparation helper at its first source-object copying syscall.

This disposable parent tracer needs no production fault hooks or slow filesystem.
The controller's real process-group deadline kills the stopped copier and tracer.
"""

import ctypes
import json
import os
import signal
import struct
import sys
import time

helper = "__FIXTURE_HELPER__"
marker = "__FIXTURE_MARKER__"
wire = sys.stdin.buffer.read()
request = json.loads(wire)
input_fd, output_fd = os.pipe()
os.write(output_fd, wire)
os.close(output_fd)
libc = ctypes.CDLL(None, use_errno=True)
libc.ptrace.restype = ctypes.c_long
libc.ptrace.argtypes = [ctypes.c_uint, ctypes.c_int, ctypes.c_void_p, ctypes.c_void_p]


def ptrace(op, pid, address=0, data=0):
    result = libc.ptrace(op, pid, address, data)
    if result == -1:
        raise OSError(ctypes.get_errno(), "preparation stall fixture ptrace failed")
    return result


child = os.fork()
if child == 0:
    os.dup2(input_fd, 0)
    os.close(input_fd)
    ptrace(0, 0)  # PTRACE_TRACEME
    os.kill(os.getpid(), signal.SIGSTOP)
    os.execv(helper, [helper, "__canix-roborev-prepare"])
os.close(input_fd)
_, status = os.waitpid(child, 0)
assert os.WIFSTOPPED(status), "preparation stall fixture did not establish tracing"
ptrace(0x4200, child, data=1 | (1 << 20))  # TRACESYSGOOD | EXITKILL

while True:
    ptrace(24, child)  # PTRACE_SYSCALL
    _, status = os.waitpid(child, 0)
    if os.WIFEXITED(status):
        sys.exit(os.WEXITSTATUS(status))
    if os.WIFSIGNALED(status):
        sys.exit(1)
    assert os.WIFSTOPPED(status)
    if os.WSTOPSIG(status) != signal.SIGTRAP | 0x80:
        continue
    info = ctypes.create_string_buffer(88)
    ptrace(0x420E, child, address=ctypes.sizeof(info), data=ctypes.addressof(info))
    if info.raw[0] != 1:  # syscall entry
        continue
    arch = struct.unpack_from("=I", info.raw, 4)[0]
    number, fd0, fd1 = struct.unpack_from("=QQQ", info.raw, 24)
    # read, readv, splice and copy_file_range; sendfile's input is arg1.
    table = {
        0xC000003E: ({0, 19, 275, 326}, 40),  # x86_64
        0xC00000B7: ({63, 65, 76, 285}, 71),  # aarch64
    }
    assert arch in table, "unsupported native stall-fixture syscall ABI"
    input_calls, sendfile = table[arch]
    if number not in input_calls and number != sendfile:
        continue
    fd = fd1 if number == sendfile else fd0
    try:
        path = os.readlink(f"/proc/{child}/fd/{fd}")
    except OSError:
        continue
    if path.startswith(request["objects"] + "/"):
        with open(marker, "w") as stream:
            json.dump({"phase": "source-object-copy", "pid": child}, stream)
        # The real Rust helper is stopped immediately before source copying;
        # wait for the operation-wide controller deadline, preserving all state.
        time.sleep(60)
        raise AssertionError("preparation supervisor did not terminate stalled I/O")
