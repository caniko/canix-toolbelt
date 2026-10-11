"""Process-local Codex routing using the pinned Headroom adapter.

No install/init path is used: user configuration, login and persisted provider
identities remain owned by Codex. CLI overrides select its embedded session mode.
"""

import os
import sys

from headroom.cli.wrap import _codex_session_launch_settings


def launch(binary, port, args, environ):
    arguments, environment, _display = _codex_session_launch_settings(
        port=port, codex_args=tuple(args), environ=dict(environ)
    )
    os.execve(binary, [binary, *arguments], environment)


if __name__ == "__main__":
    launch("@codex@", int("@port@"), sys.argv[1:], os.environ)
