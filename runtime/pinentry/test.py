"""Real GnuPG, rage, Qt and Zellij with disposable agents and fixture-only PINs."""

import fcntl
import json
import os
import pty
import select
import struct
import subprocess
import sys
import tempfile
import termios
import threading
import time
import unittest
from pathlib import Path

AGENT, ZELLIJ, CONNECT, GPGCONF, BASH, GPG, DIRECT, RAGE, XVFB, XDOTOOL = sys.argv[1:11]
sys.argv[1:11] = []
WRAPPED_RAGE = sys.argv[1:2] == ["--wrapped-rage"]
if WRAPPED_RAGE:
    del sys.argv[1]


class RequestContextIntegration(unittest.TestCase):
    """Exercise executable entrypoints with recording, non-secret backends."""

    @classmethod
    def setUpClass(cls):
        cls.temp = tempfile.TemporaryDirectory(prefix="pc-", dir=os.environ.get("TMPDIR", "/data/scratch/tmp/opencode"))
        cls.addClassCleanup(cls.temp.cleanup)
        root = Path(cls.temp.name)
        backend = root / "backend"
        backend.write_text(f"#!{sys.executable}\nimport json, os\n"
                           "print(json.dumps({key: os.environ.get(key) for key in "
                            "['DISPLAY', 'WAYLAND_DISPLAY', 'XAUTHORITY', 'XDG_SESSION_TYPE', 'XDG_RUNTIME_DIR', 'GPG_TTY', 'PINENTRY_USER_DATA']}))\n")
        backend.chmod(0o700)
        manager = root / "manager"
        manager.write_text(f"#!{sys.executable}\nprint('DISPLAY=:manager\\nWAYLAND_DISPLAY=manager\\nXAUTHORITY=manager\\nXDG_SESSION_TYPE=manager')\n")
        manager.chmod(0o700)
        binary = root / "router"
        compile_env = dict(os.environ, CANIX_PINENTRY_GPG=str(backend), CANIX_PINENTRY_QT=str(backend),
                           CANIX_PINENTRY_SYSTEMCTL=str(manager))
        subprocess.run(["rustc", "--edition=2024", "-D", "warnings", str(Path(__file__).with_name("main.rs")), "-o", str(binary)],
                       env=compile_env, capture_output=True, check=True, timeout=30)
        cls.gpg = root / "gpg"
        cls.gpg.symlink_to(binary)
        cls.gpg2 = root / "gpg2"
        cls.gpg2.symlink_to(binary)
        cls.agent = root / "canix-toolbelt-pinentry-agent"
        cls.agent.symlink_to(binary)
        cls.env = {key: value for key, value in os.environ.items()
                   if not key.startswith(("ZELLIJ", "GPG", "GNUPG", "SSH_", "PINENTRY"))}

    def test_agent_preserves_all_request_display_values(self):
        env = dict(self.env, PINENTRY_USER_DATA="canix-pinentry-v1:desktop", DISPLAY=":request",
                   WAYLAND_DISPLAY="request-wayland", XAUTHORITY="request-auth", XDG_SESSION_TYPE="request-type")
        result = subprocess.run([self.agent], env=env, text=True, capture_output=True, check=True, timeout=5)
        actual = json.loads(result.stdout)
        for key in ["DISPLAY", "WAYLAND_DISPLAY", "XAUTHORITY", "XDG_SESSION_TYPE"]:
            self.assertEqual(actual[key], env[key])

    def test_gpg_accepts_the_open_controlling_terminal_outside_devpts(self):
        master, slave = pty.openpty()
        self.addCleanup(os.close, master)
        self.addCleanup(os.close, slave)
        result = subprocess.run([BASH, "--noprofile", "--norc", "-c", 'exec < /dev/tty; exec "$@"', "fixture", str(self.gpg)],
                                env=self.env, stdin=slave, text=True, capture_output=True, check=True, timeout=5,
                                start_new_session=True, preexec_fn=lambda: fcntl.ioctl(0, termios.TIOCSCTTY, 0))
        self.assertEqual(json.loads(result.stdout)["GPG_TTY"], "/dev/tty")

    def test_piped_gpg_preserves_an_explicit_terminal(self):
        result = subprocess.run([self.gpg], env=dict(self.env, GPG_TTY="/dev/ttyS0"), input="", text=True,
                                capture_output=True, check=True, timeout=5)
        self.assertEqual(json.loads(result.stdout)["GPG_TTY"], "/dev/ttyS0")

    def test_piped_gpg_discovers_the_interactive_stderr_terminal(self):
        master, slave = pty.openpty()
        self.addCleanup(os.close, master)
        self.addCleanup(os.close, slave)
        result = subprocess.run([self.gpg], env=self.env, input="piped signing data", stderr=slave,
                                stdout=subprocess.PIPE, text=True, check=True, timeout=5)
        self.assertEqual(json.loads(result.stdout)["GPG_TTY"], os.ttyname(slave))

    def test_real_gpg_forwards_wayland_and_clears_stale_agent_display(self):
        with tempfile.TemporaryDirectory(prefix="pw-", dir=self.temp.name) as directory:
            root = Path(directory)
            home = root / "g"
            home.mkdir(mode=0o700)
            record = root / "context.json"
            backend = root / "pinentry"
            backend.write_text(f"#!{sys.executable}\nimport json, os, sys\nfrom pathlib import Path\n"
                               "Path(os.environ['PINENTRY_FIXTURE_RECORD']).write_text(json.dumps({key: os.environ.get(key) for key in "
                               "['DISPLAY', 'WAYLAND_DISPLAY', 'XAUTHORITY', 'XDG_SESSION_TYPE', 'XDG_RUNTIME_DIR']}))\n"
                               "print('OK', flush=True)\n"
                               "for line in sys.stdin:\n"
                               " if line.startswith('GETPIN'): print('D display-fixture-pin', flush=True)\n"
                               " print('OK', flush=True)\n"
                               " if line.startswith('BYE'): break\n")
            backend.chmod(0o700)
            binary = root / "router"
            subprocess.run(["rustc", "--edition=2024", "-D", "warnings", str(Path(__file__).with_name("main.rs")), "-o", str(binary)],
                           env=dict(self.env, CANIX_PINENTRY_GPG=GPG, CANIX_PINENTRY_QT=str(backend)),
                           capture_output=True, check=True, timeout=30)
            gpg = root / "gpg"
            gpg.symlink_to(binary)
            agent = root / "canix-toolbelt-pinentry-agent"
            agent.symlink_to(binary)
            (home / "gpg-agent.conf").write_text(f"pinentry-program {agent}\ndefault-cache-ttl 0\nno-allow-external-cache\n")
            startup = dict(self.env, GNUPGHOME=str(home), PINENTRY_FIXTURE_RECORD=str(record),
                           DISPLAY=":stale-agent", WAYLAND_DISPLAY="stale-wayland", XAUTHORITY="stale-auth",
                           XDG_SESSION_TYPE="stale-type", XDG_RUNTIME_DIR=str(root))
            subprocess.run([GPGCONF, "--launch", "gpg-agent"], env=startup, capture_output=True, check=True, timeout=10)
            try:
                subprocess.run([GPG, "--batch", "--pinentry-mode", "loopback", "--passphrase", "display-fixture-pin",
                                "--quick-generate-key", "Display fixture <display@example.invalid>", "ed25519", "sign", "0"],
                               env=startup, capture_output=True, check=True, timeout=15)
                challenge = root / "challenge"
                challenge.write_text("Request-local graphical context\n")
                for context in [
                    {"WAYLAND_DISPLAY": "wayland-request:% name", "XDG_SESSION_TYPE": "wayland", "XDG_RUNTIME_DIR": str(root / "request runtime")},
                    {"DISPLAY": ":request", "XAUTHORITY": str(root / "request auth"), "XDG_SESSION_TYPE": "x11", "XDG_RUNTIME_DIR": str(root)},
                ]:
                    with self.subTest(context=context):
                        client = dict(startup)
                        for key in ["DISPLAY", "WAYLAND_DISPLAY", "XAUTHORITY", "XDG_SESSION_TYPE", "XDG_RUNTIME_DIR"]:
                            client.pop(key, None)
                        client.update(context)
                        subprocess.run([gpg, "--batch", "--yes", "--output", str(root / "signature"), "--detach-sign", str(challenge)],
                                       env=client, input="", text=True, capture_output=True, check=True, timeout=15)
                        self.assertEqual(json.loads(record.read_text()), {key: context.get(key) for key in
                                         ["DISPLAY", "WAYLAND_DISPLAY", "XAUTHORITY", "XDG_SESSION_TYPE", "XDG_RUNTIME_DIR"]})
            finally:
                subprocess.run([GPGCONF, "--kill", "gpg-agent"], env=startup, capture_output=True, check=True, timeout=10)

    def test_both_gpg_entrypoints_refresh_request_context(self):
        env = dict(self.env, PINENTRY_USER_DATA="canix-pinentry-v1:desktop", ZELLIJ_PANE_ID="17", ZELLIJ_SESSION_NAME="request-session")
        for executable in [self.gpg, self.gpg2]:
            with self.subTest(executable=executable.name):
                result = subprocess.run([executable], env=env, input="", text=True, capture_output=True, check=True, timeout=5)
                self.assertEqual(json.loads(result.stdout)["PINENTRY_USER_DATA"], "canix-pinentry-v1:zellij:17:request-session")


class PinentryIntegration(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temp = tempfile.TemporaryDirectory(prefix="cp-", dir=os.environ.get("TMPDIR", "/data/scratch/tmp/opencode"))
        cls.root = Path(cls.temp.name)
        for name in ["r", "g", "home", "config", "bin"]:
            (cls.root / name).mkdir(mode=0o700)
        cls.env = {key: value for key, value in os.environ.items()
                   if not key.startswith(("ZELLIJ", "GPG", "GNUPG", "SSH_", "PINENTRY"))}
        cls.env.update(HOME=str(cls.root / "home"), XDG_CONFIG_HOME=str(cls.root / "config"),
                       XDG_RUNTIME_DIR=str(cls.root / "r"), GNUPGHOME=str(cls.root / "g"),
                       TERM="xterm-256color", DISPLAY=":987", WAYLAND_DISPLAY="unavailable-fixture-display", LC_ALL="C")
        (cls.root / "g/gpg-agent.conf").write_text(f"pinentry-program {AGENT}\ndefault-cache-ttl 0\nno-allow-external-cache\n")
        cls.config = cls.root / "zellij.kdl"
        cls.config.write_text(f'default_shell "{BASH}"\nshow_release_notes false\nshow_startup_tips false\nsession_serialization false\n')
        cls.session = f"pinentry-test-{os.getpid()}"
        cls.clients = []
        cls.queries = []
        cls.addClassCleanup(cls.cleanup)
        cls.zellij("attach", "--create-background", cls.session, target=False)
        cls.attach_client()
        cls.origin = cls.wait_for(lambda: next((p for p in cls.panes() if not p["is_plugin"]), None))
        cls.zellij("action", "new-tab", "--name", "other")
        cls.attach_client()
        cls.command([GPGCONF, "--launch", "gpg-agent"])
        cls.client = cls.root / "gpg"
        cls.client.symlink_to(DIRECT)
        cls.command([GPG, "--batch", "--pinentry-mode", "loopback", "--passphrase", "client-fixture-pin",
                     "--quick-generate-key", "Pinentry fixture <pinentry@example.invalid>", "ed25519", "sign", "0"])
        listing = cls.command([GPG, "--batch", "--with-colons", "--list-secret-keys"]).stdout
        cls.fingerprint = next(line.split(":")[9] for line in listing.splitlines() if line.startswith("fpr:"))
        plugin = cls.root / "bin/age-plugin-fido2-hmac"
        plugin.write_text(f"#!{sys.executable}\n" + Path(__file__).with_name("fixture_plugin.py").read_text())
        plugin.chmod(0o700)
        if not WRAPPED_RAGE:
            (cls.root / "bin/pinentry").symlink_to(DIRECT)
        cls.identity = cls.root / "identity"
        cls.identity.write_text("AGE-PLUGIN-FIDO2-HMAC-188VDVA\n")
        cls.age_env = dict(cls.env, PATH=f"{cls.root / 'bin'}:{cls.env['PATH']}")
        if not WRAPPED_RAGE:
            cls.age_env["PINENTRY_PROGRAM"] = DIRECT
        cls.encrypted = cls.root / "fixture.age"
        subprocess.run([RAGE, "--encrypt", "--identity", str(cls.identity), "--output", str(cls.encrypted)],
                       input=b"decrypted fixture\n", env=cls.age_env, check=True, timeout=15, capture_output=True)

    @classmethod
    def command(cls, args, **kwargs):
        return subprocess.run(args, env=cls.env, text=True, capture_output=True, check=True, timeout=15, **kwargs)

    @classmethod
    def zellij(cls, *args, target=True):
        command = [ZELLIJ, "--config", str(cls.config)]
        if target:
            command += ["--session", cls.session]
        return cls.command(command + list(args)).stdout

    @classmethod
    def attach_client(cls):
        master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 120, 0, 0))
        process = subprocess.Popen([ZELLIJ, "--config", str(cls.config), "attach", cls.session], env=cls.env,
                                   stdin=slave, stdout=slave, stderr=slave, start_new_session=True)
        os.close(slave)
        cls.clients.append((process, master))

        def drain():
            try:
                while os.read(master, 65536):
                    pass
            except OSError:
                pass

        threading.Thread(target=drain, daemon=True).start()
        cls.wait_for(lambda: len(cls.zellij("action", "list-clients").splitlines()) > len(cls.clients))

    @classmethod
    def panes(cls):
        return json.loads(cls.zellij("action", "list-panes", "--json"))

    @classmethod
    def wait_for(cls, condition):
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline:
            result = condition()
            if result:
                return result
            time.sleep(0.05)
        raise AssertionError("timed out waiting for fixture state")

    @classmethod
    def query(cls, context, tty="/dev/pts/99999999", env=None):
        process = subprocess.Popen([CONNECT], env=env if env is not None else cls.env, text=True, stdin=subprocess.PIPE,
                                   stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        cls.queries.append(process)
        context_option = f"OPTION pinentry-user-data={context}\n" if context is not None else ""
        process.stdin.write(context_option + f"OPTION ttyname={tty}\nGET_PASSPHRASE --data X X PIN Fixture_prompt\n/bye\n")
        process.stdin.flush()
        process.stdin.close()
        process.stdin = None
        return process

    def prompt_pane(self, prompt):
        def prompted():
            for candidate in self.panes():
                if candidate["title"] == "Hardware key PIN":
                    pane = f"terminal_{candidate['id']}"
                    if prompt in self.zellij("action", "dump-screen", "--pane-id", pane):
                        return candidate
            return None

        popup = self.wait_for(prompted)
        self.assertTrue(popup["is_floating"])
        self.assertEqual(popup["tab_id"], self.origin["tab_id"])
        pane = f"terminal_{popup['id']}"
        return pane

    def popup_query(self):
        process = self.query(f"canix-pinentry-v1:zellij:{self.origin['id']}:{self.session}")
        return process, self.prompt_pane("Fixture_prompt")

    def assert_closed(self):
        self.wait_for(lambda: all(p["title"] != "Hardware key PIN" for p in self.panes()))
        self.assertEqual(list((self.root / "r").glob("canix-pinentry-*")), [])

    def enter(self, pane, pin):
        self.zellij("action", "write-chars", "--pane-id", pane, pin)
        self.assertNotIn(pin, self.zellij("action", "dump-screen", "--pane-id", pane))
        self.zellij("action", "send-keys", "--pane-id", pane, "Enter")

    def test_floating_prompt_routes_to_origin_and_does_not_echo_pin(self):
        process, pane = self.popup_query()
        self.enter(pane, "fixture-pin")
        stdout, stderr = process.communicate(timeout=15)
        self.assertIn("D fixture-pin", stdout, stderr)
        self.assertNotIn("ERR ", stdout)
        self.assertNotIn("terminal_", stdout)
        self.assert_closed()

    def test_closing_popup_cancels_without_hanging_or_reprompting(self):
        process, pane = self.popup_query()
        self.zellij("action", "close-pane", "--pane-id", pane)
        stdout, _ = process.communicate(timeout=15)
        self.assertIn("ERR ", stdout)
        self.assertNotIn("D fixture-pin", stdout)
        self.assert_closed()

    def terminal_query(self, context):
        master, slave = pty.openpty()
        self.addCleanup(os.close, master)
        self.addCleanup(os.close, slave)
        process = self.query(context, os.ttyname(slave))
        output = bytearray()

        def prompted():
            if select.select([master], [], [], 0.1)[0]:
                output.extend(os.read(master, 4096))
            return b"Fixture_prompt" in output and b"PIN" in output

        self.wait_for(prompted)
        os.write(master, b"terminal-fixture\n")
        stdout, stderr = process.communicate(timeout=15)
        self.assertIn("D terminal-fixture", stdout, stderr)
        self.assertNotIn("ERR ", stdout)
        self.assert_closed()

    def test_plain_ssh_uses_supplied_tty_despite_agent_desktop_environment(self):
        self.terminal_query("canix-pinentry-v1:tty")

    def test_unmarked_request_uses_terminal_instead_of_agent_desktop(self):
        self.terminal_query(None)

    def test_headless_unmarked_request_fails_without_opening_a_gui(self):
        process = self.query(None)
        stdout, _ = process.communicate(timeout=15)
        self.assertIn("ERR ", stdout)
        self.assertNotIn("D ", stdout)
        self.assert_closed()

    def test_noninteractive_gpg_client_forwards_zellij_and_signs(self):
        challenge = self.root / "client-challenge.txt"
        signature = self.root / "client-challenge.sig"
        challenge.write_text("Disposable GPG client routing fixture\n")
        env = dict(self.env, ZELLIJ_PANE_ID=str(self.origin["id"]), ZELLIJ_SESSION_NAME=self.session,
                   PINENTRY_USER_DATA="canix-pinentry-v1:desktop", SSH_CONNECTION="fixture-ssh-connection")
        env.pop("DISPLAY", None)
        env.pop("WAYLAND_DISPLAY", None)
        process = subprocess.Popen([BASH, "--noprofile", "--norc", "-c", 'exec "$@"', "fixture", str(self.client),
                                    "--batch", "--status-fd", "1", "--local-user", self.fingerprint,
                                    "--output", str(signature), "--detach-sign", str(challenge)],
                                   env=env, text=True, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        self.queries.append(process)
        self.enter(self.prompt_pane("Passphrase"), "client-fixture-pin")
        stdout, stderr = process.communicate(timeout=15)
        self.assertEqual(process.returncode, 0, stderr)
        self.assertIn("[GNUPG:] SIG_CREATED ", stdout)
        verification = self.command([GPG, "--batch", "--status-fd", "1", "--verify", str(signature), str(challenge)]).stdout
        self.assertIn(f"[GNUPG:] VALIDSIG {self.fingerprint} ", verification)
        self.assert_closed()

    def test_missing_session_falls_back_with_assuan_protocol_intact(self):
        self.terminal_query("canix-pinentry-v1:zellij:0:missing-fixture-session")

    def age_query(self):
        env = dict(self.age_env, ZELLIJ_PANE_ID=str(self.origin["id"]), ZELLIJ_SESSION_NAME=self.session,
                   PINENTRY_USER_DATA="canix-pinentry-v1:desktop", SSH_CONNECTION="fixture-ssh-connection")
        process = subprocess.Popen([RAGE, "--decrypt", "--identity", str(self.identity), str(self.encrypted)],
                                   env=env, text=True, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        self.queries.append(process)
        return process

    def test_rage_plugin_confirm_and_pin_use_direct_context_with_piped_stdio(self):
        process = self.age_query()
        pane = self.prompt_pane("Fixture_confirmation")
        self.zellij("action", "write-chars", "--pane-id", pane, "y\n")
        # Confirmation and secret callbacks start independent pinentry processes.
        self.enter(self.prompt_pane("Fixture_age_PIN"), "age-fixture-pin")
        stdout, stderr = process.communicate(timeout=15)
        self.assertEqual(process.returncode, 0, stderr)
        self.assertEqual(stdout, "decrypted fixture\n")
        self.assert_closed()

    def test_rage_plugin_cancel_does_not_open_a_second_prompt(self):
        process = self.age_query()
        pane = self.prompt_pane("Fixture_confirmation")
        self.zellij("action", "close-pane", "--pane-id", pane)
        stdout, _ = process.communicate(timeout=15)
        self.assertNotEqual(process.returncode, 0)
        self.assertEqual(stdout, "")
        self.assert_closed()

    def test_concurrent_rage_requests_keep_separate_popups_and_cleanup(self):
        processes = [self.age_query(), self.age_query()]

        def prompt_panes(prompt):
            return [f"terminal_{p['id']}" for p in self.panes()
                    if p["title"] == "Hardware key PIN"
                    and prompt in self.zellij("action", "dump-screen", "--pane-id", f"terminal_{p['id']}")]

        confirms = self.wait_for(lambda: (panes if len(panes := prompt_panes("Fixture_confirmation")) == 2 else None))
        for pane in confirms:
            self.zellij("action", "write-chars", "--pane-id", pane, "y\n")
        pins = self.wait_for(lambda: (panes if len(panes := prompt_panes("Fixture_age_PIN")) == 2 else None))
        self.assertEqual(len(list((self.root / "r").glob("canix-pinentry-*"))), 2)
        for pane in pins:
            self.enter(pane, "age-fixture-pin")
        for process in processes:
            stdout, stderr = process.communicate(timeout=15)
            self.assertEqual(process.returncode, 0, stderr)
            self.assertEqual(stdout, "decrypted fixture\n")
        self.assert_closed()

    def test_active_prompt_timeout_ends_request_without_terminal_fallback(self):
        env = dict(self.env, ZELLIJ_PANE_ID=str(self.origin["id"]), ZELLIJ_SESSION_NAME=self.session)
        process = subprocess.Popen([DIRECT], env=env, text=True, stdin=subprocess.PIPE,
                                   stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        self.queries.append(process)
        process.stdin.write("SETDESC Timeout_fixture\nSETTIMEOUT 1\nGETPIN\nBYE\n")
        process.stdin.flush()
        self.prompt_pane("Timeout_fixture")
        stdout, stderr = process.communicate(timeout=15)
        self.assertIn("ERR ", stdout)
        self.assertNotIn("D ", stdout)
        self.assertNotIn("using the requesting terminal", stderr)
        self.assert_closed()

    def test_shell_context_refresh_discards_a_stale_desktop_marker(self):
        env = dict(self.env, PINENTRY_USER_DATA="canix-pinentry-v1:desktop")
        env.pop("DISPLAY", None)
        env.pop("WAYLAND_DISPLAY", None)
        result = subprocess.run([DIRECT, "--context"], env=env, text=True, capture_output=True, check=True, timeout=5)
        self.assertEqual(result.stdout, "canix-pinentry-v1:tty\n")

    def test_direct_desktop_request_uses_real_qt_on_the_callers_display(self):
        self.desktop_query(agent=False)

    def test_agent_desktop_request_uses_the_clients_display_instead_of_startup(self):
        self.desktop_query(agent=True)

    def desktop_query(self, agent):
        read_fd, write_fd = os.pipe()
        server = subprocess.Popen([XVFB, "-displayfd", str(write_fd), "-screen", "0", "800x600x24", "-nolisten", "tcp"],
                                  pass_fds=[write_fd], stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
        os.close(write_fd)
        self.addCleanup(server.stderr.close)
        self.addCleanup(lambda: server.wait(timeout=5))
        self.addCleanup(server.terminate)
        self.assertTrue(select.select([read_fd], [], [], 5)[0], "Xvfb did not start")
        with os.fdopen(read_fd) as display:
            env = dict(self.env, DISPLAY=":" + display.readline().strip())
        env.pop("WAYLAND_DISPLAY", None)
        if agent:
            process = self.query("canix-pinentry-v1:desktop", env=env)
        else:
            process = subprocess.Popen([DIRECT], env=env, text=True, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            self.queries.append(process)
            process.stdin.write("SETTITLE Toolbelt fixture\nSETDESC Desktop_fixture\nSETPROMPT PIN\nGETPIN\nBYE\n")
            process.stdin.flush()

        def window():
            result = subprocess.run([XDOTOOL, "search", "--onlyvisible", "--class", "pinentry"],
                                    env=env, text=True, capture_output=True, timeout=5, check=False)
            return result.stdout.strip().splitlines()[0] if result.returncode == 0 else None

        target = self.wait_for(window)
        subprocess.run([XDOTOOL, "type", "--window", target, "qt-fixture-pin"], env=env, check=True, timeout=5)
        # Qt destroys the dialog on keydown; a subsequent targeted keyup would
        # race the disappearing X window and spuriously report BadWindow.
        subprocess.run([XDOTOOL, "keydown", "--window", target, "Return"], env=env, check=True, timeout=5)
        stdout, stderr = process.communicate(timeout=15)
        self.assertIn("D qt-fixture-pin", stdout, stderr)
        self.assertEqual(process.returncode, 0, stderr)
        self.assert_closed()

    @classmethod
    def cleanup(cls):
        for process in cls.queries:
            if process.poll() is None:
                process.kill()
                process.communicate()
        cls.command([GPGCONF, "--kill", "gpg-agent"])
        cls.zellij("kill-session", cls.session, target=False)
        for process, master in cls.clients:
            if process.poll() is None:
                process.terminate()
            process.wait(timeout=15)
            os.close(master)
        cls.temp.cleanup()


if __name__ == "__main__":
    unittest.main(verbosity=2)
