"""Test-only loopback namespace retaining the caller's UID and dropping caps."""
import ctypes
import fcntl
import os
import socket
import struct
import sys

assert os.geteuid() != 0, "run this disposable fixture as an unprivileged user"
with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sock:
    flags = struct.unpack("16sH", fcntl.ioctl(sock, 0x8913, struct.pack("16sH", b"lo", 0))[:18])[1]
    fcntl.ioctl(sock, 0x8914, struct.pack("16sH", b"lo", flags | 1))


class Header(ctypes.Structure):
    _fields_ = [("version", ctypes.c_uint32), ("pid", ctypes.c_int)]


class Data(ctypes.Structure):
    _fields_ = [("effective", ctypes.c_uint32), ("permitted", ctypes.c_uint32), ("inheritable", ctypes.c_uint32)]


libc = ctypes.CDLL(None, use_errno=True)
assert libc.prctl(47, 4, 0, 0, 0) == 0, ctypes.get_errno()
header = Header(0x20080522, 0)
data = (Data * 2)()
assert libc.capset(ctypes.byref(header), ctypes.byref(data)) == 0, ctypes.get_errno()
with open("/proc/self/status") as stream:
    status = dict(line.split(":", 1) for line in stream if ":" in line)
assert all(int(status[key].strip(), 16) == 0 for key in ["CapInh", "CapPrm", "CapEff", "CapAmb"])
assert set(socket.if_nameindex()) == {(1, "lo")}
os.execvpe(sys.argv[1], sys.argv[1:], os.environ)
