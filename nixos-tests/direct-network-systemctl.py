"""Disposable single-reload-denial fixture; all other unit operations are native."""

import os
import pathlib
import sys

if sys.argv[1:2] == ['kill']:
    count = pathlib.Path('/run/direct-network/reload-attempts')
    attempts = int(count.read_text()) if count.exists() else 0
    count.write_text(str(attempts + 1))
    denial = pathlib.Path('/run/direct-network/deny-reload')
    if denial.exists():
        denial.unlink()
        sys.exit(1)
os.execv('@SYSTEMCTL@', ['@SYSTEMCTL@', *sys.argv[1:]])
