"""Framework-neutral connection to the user's default browser through WebDriver."""

import configparser
import json
import os
import re
import selectors
import shlex
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
from pathlib import Path


def default_browser(settings):
    if settings.get("browser") is not None:
        return settings["browser"]
    desktop = subprocess.run(
        [settings["xdgSettings"], "get", "default-web-browser"],
        check=True, capture_output=True, text=True, timeout=10,
    ).stdout.strip()
    if not desktop or "/" in desktop or desktop in (".", ".."):
        raise ValueError("XDG did not return a default browser desktop ID")
    roots = [Path(os.environ.get("XDG_DATA_HOME", str(Path.home() / ".local/share")))]
    roots.extend(Path(p) for p in os.environ.get("XDG_DATA_DIRS", "/usr/local/share:/usr/share").split(":"))
    entry = next((p / "applications" / desktop for p in roots if (p / "applications" / desktop).is_file()), None)
    if entry is None:
        raise ValueError(f"Default browser desktop entry not found: {desktop}")
    parser = configparser.ConfigParser(interpolation=None)
    parser.read(entry)
    argv = shlex.split(parser["Desktop Entry"]["Exec"])
    if not argv or argv[0] in ("env", "sh", "bash", "nu"):
        raise ValueError("Default browser uses a shell launcher; configure an explicit browser executable")
    name = Path(argv[0]).name.lower()
    family = (
        "firefox" if any(n in name for n in ("firefox", "floorp", "librewolf", "zen-browser"))
        else "chromium" if any(n in name for n in ("chromium", "chrome", "brave", "vivaldi"))
        else None
    )
    if family is None:
        raise ValueError(f"Default browser {name} has no supported WebDriver connection")
    arguments = [arg for arg in argv[1:] if not arg.startswith("%")]
    if any("%" in arg for arg in arguments):
        raise ValueError("Embedded desktop field codes require an explicit browser override")
    executable = argv[0] if Path(argv[0]).is_absolute() else shutil.which(argv[0])
    if executable is None:
        raise ValueError(f"Default browser executable is not available: {argv[0]}")
    return {"family": family, "executable": executable, "arguments": arguments}


def firefox_launcher(executable, directory):
    """Expose packaged Gecko metadata while executing the selected wrapper."""
    selected = Path(shutil.which(executable) or executable).absolute()
    binary = selected.resolve()
    if binary.parent.name != "bin" or (binary.parent / "platform.ini").is_file():
        return executable
    # Nix's bin wrapper is outside lib/<browser>/, where Gecko keeps its
    # platform.ini. geckodriver otherwise falls back to --version, whose
    # Floorp-branded output is not recognized by mozversion. Do not bypass
    # the wrapper: it supplies the package's runtime libraries and settings.
    candidates = [ini for ini in (binary.parent.parent / "lib").glob("*/platform.ini")
                  if (ini.parent / binary.name).is_file()]
    if not candidates:
        return executable
    if len(candidates) != 1:
        raise ValueError(f"Ambiguous Gecko metadata for selected browser: {binary}")
    directory = Path(directory)
    directory.mkdir(mode=0o700)
    shutil.copyfile(candidates[0], directory / "platform.ini")
    launcher = directory / "firefox"
    launcher.write_text(
        f"#!{sys.executable}\nimport os, sys\n"
        f"os.execv({str(selected)!r}, [{str(selected)!r}, *sys.argv[1:]])\n"
    )
    launcher.chmod(0o700)
    return str(launcher)


class WebDriver:
    def __init__(self, settings):
        self.browser = default_browser(settings)
        self.process = None
        self.session = None
        self.profile = tempfile.TemporaryDirectory(prefix="browser-connection-")
        self.http = urllib.request.build_opener(urllib.request.ProxyHandler({}))
        family = self.browser["family"]
        argv = (
            [settings["drivers"][family], "--host", "127.0.0.1", "--port", "0"]
            if family == "firefox"
            else [settings["drivers"][family], "--port=0", "--allowed-ips=127.0.0.1"]
        )
        try:
            self.process = subprocess.Popen(
                argv, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, start_new_session=True,
            )
            selector = selectors.DefaultSelector()
            selector.register(self.process.stdout, selectors.EVENT_READ)
            deadline = time.monotonic() + 15
            while time.monotonic() < deadline:
                if not selector.select(timeout=0.2):
                    if self.process.poll() is not None:
                        raise RuntimeError("Browser driver exited before becoming ready")
                    continue
                line = self.process.stdout.readline()
                match = re.search(r"Listening on 127\.0\.0\.1:(\d+)|started successfully on port (\d+)", line)
                if match:
                    self.url = "http://127.0.0.1:" + next(g for g in match.groups() if g)
                    break
            else:
                raise TimeoutError("Browser driver did not become ready within 15 seconds")
            selector.close()
            # Driver output must never share the framework's JSON protocol stream.
            import threading
            threading.Thread(target=self._drain, daemon=True).start()
            args = list(self.browser.get("arguments", []))
            if any(arg.lower().split("=", 1)[0] in (
                "-p", "-profile", "--profile", "--user-data-dir", "--profile-directory",
            ) for arg in args):
                raise ValueError("Browser profile arguments conflict with the isolated automation profile")
            if family == "firefox":
                args += ["-no-remote", "-profile", self.profile.name]
                if settings.get("headless"):
                    args += ["-headless"]
                executable = firefox_launcher(self.browser["executable"], Path(self.profile.name) / "launcher")
                options = {"moz:firefoxOptions": {"binary": executable, "args": args}}
                browser_name = "firefox"
            else:
                args += ["--user-data-dir=" + self.profile.name]
                if settings.get("headless"):
                    args += ["--headless=new"]
                options = {"goog:chromeOptions": {"binary": self.browser["executable"], "args": args}}
                browser_name = "chrome"
            result = self.request("POST", "/session", {"capabilities": {"alwaysMatch": {
                "browserName": browser_name, "acceptInsecureCerts": False,
                "unhandledPromptBehavior": "ignore", **options,
            }}})
            self.session = result["sessionId"]
            self.call("POST", "/timeouts", {"script": 30000, "pageLoad": 30000, "implicit": 0})
            self.call("POST", "/url", {"url": "about:blank"})
        except BaseException:
            self.close()
            raise

    def _drain(self):
        for _ in self.process.stdout:
            pass

    def request(self, method, path, body=None, timeout=35):
        request = urllib.request.Request(
            self.url + path, data=None if body is None else json.dumps(body).encode(),
            headers={"Content-Type": "application/json"}, method=method,
        )
        try:
            response = self.http.open(request, timeout=timeout)
        except urllib.error.HTTPError as error:
            response = error
        with response:
            value = json.load(response)["value"]
        if isinstance(value, dict) and "error" in value:
            raise RuntimeError(value["error"] + ": " + value.get("message", "")[:1500])
        return value

    def call(self, method, path, body=None):
        return self.request(method, "/session/" + self.session + path, body)

    def script(self, source, *arguments):
        return self.call("POST", "/execute/sync", {"script": source, "args": list(arguments)})

    def close(self):
        if self.session:
            try:
                self.request("DELETE", "/session/" + self.session, timeout=2)
            except (OSError, RuntimeError, ValueError):
                pass
            self.session = None
        if self.process:
            # The driver owns a separate process group, including its automation
            # browser. Close it even when a hung WebDriver session cannot quit.
            try:
                os.killpg(self.process.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
            try:
                self.process.wait(timeout=1)
            except subprocess.TimeoutExpired:
                os.killpg(self.process.pid, signal.SIGKILL)
                self.process.wait()
            self.process.stdout.close()
        self.profile.cleanup()
