"""Disposable uplink-DNS provider for testing changes without daemon restarts."""

import pathlib
import sys

if sys.argv[1:9] != ['--wait', '2', '--escape', 'no', '--get-values', 'IP4.DNS,IP6.DNS', 'device', 'show'] or len(sys.argv) != 10:
    sys.exit(3)
if sys.argv[9] == 'uplink':
    print(pathlib.Path('/run/direct-network-dns-upstream').read_text(), end='')
