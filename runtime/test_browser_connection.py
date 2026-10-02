import json
import os
import subprocess
import sys
import tempfile
import threading
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from unittest.mock import Mock, patch

from browser_connection import default_browser, firefox_launcher
from opencode_browser import OPERATIONS, Host


class DefaultBrowserTests(unittest.TestCase):
    def test_actual_xdg_default_and_arguments_are_used(self):
        with tempfile.TemporaryDirectory() as root:
            root = Path(root)
            (root / "applications").mkdir()
            (root / "applications/user-floorp.desktop").write_text(
                '[Desktop Entry]\nExec="/nix/store/example/bin/floorp" --name Floorp %U\n'
            )
            xdg = root / "xdg"
            xdg.write_text("#!/bin/sh\nprintf '%s\\n' user-floorp.desktop\n")
            xdg.chmod(0o700)
            with patch.dict(os.environ, {"XDG_DATA_HOME": str(root), "XDG_DATA_DIRS": str(root)}):
                self.assertEqual(default_browser({"xdgSettings": str(xdg)}), {
                    "family": "firefox", "executable": "/nix/store/example/bin/floorp",
                    "arguments": ["--name", "Floorp"],
                })

    def test_unsupported_default_does_not_substitute_chromium(self):
        with tempfile.TemporaryDirectory() as root:
            root = Path(root)
            (root / "applications").mkdir()
            (root / "applications/browser.desktop").write_text('[Desktop Entry]\nExec=unsupported-browser %U\n')
            xdg = root / "xdg"
            xdg.write_text("#!/bin/sh\nprintf '%s\\n' browser.desktop\n")
            xdg.chmod(0o700)
            with (
                patch.dict(os.environ, {"XDG_DATA_HOME": str(root), "XDG_DATA_DIRS": str(root)}),
                self.assertRaisesRegex(ValueError, "no supported WebDriver"),
            ):
                default_browser({"xdgSettings": str(xdg)})

    def test_explicit_browser_needs_no_desktop_connection(self):
        browser = {"family": "firefox", "executable": "/browser", "arguments": []}
        self.assertEqual(default_browser({"browser": browser}), browser)


class FirefoxLauncherTests(unittest.TestCase):
    def test_wrapped_browser_keeps_original_argv_environment_and_gecko_metadata(self):
        with tempfile.TemporaryDirectory() as root:
            root = Path(root)
            prefix = root / "package with spaces"
            (prefix / "bin").mkdir(parents=True)
            metadata = prefix / "lib/floorp-bin-12.17.2"
            metadata.mkdir(parents=True)
            (metadata / "platform.ini").write_text("[Build]\nMilestone=155.0\n")
            (metadata / "floorp").write_text("not the selected wrapper")
            executable = prefix / "bin/floorp"
            executable.write_text(
                f"#!{sys.executable}\nimport os, sys, json\n"
                "print(json.dumps([sys.argv, os.environ['BROWSER_WRAPPER_TEST']]))\n"
            )
            executable.chmod(0o700)
            # Desktop executables often reach a store package through a profile symlink.
            selected = root / "profile-floorp"
            selected.symlink_to(executable)
            launcher = firefox_launcher(str(selected), root / "automation")
            self.assertEqual((Path(launcher).parent / "platform.ini").read_text(),
                             (metadata / "platform.ini").read_text())
            with patch.dict(os.environ, {"BROWSER_WRAPPER_TEST": "preserved"}):
                result = subprocess.run([launcher, "-profile", "path with spaces", "--name=Floorp"],
                                        check=True, capture_output=True, text=True, timeout=10)
            self.assertEqual(json.loads(result.stdout),
                             [[str(selected), "-profile", "path with spaces", "--name=Floorp"], "preserved"])

    def test_native_or_unknown_browser_is_not_replaced(self):
        with tempfile.TemporaryDirectory() as root:
            executable = Path(root) / "bin/firefox"
            executable.parent.mkdir()
            executable.touch()
            self.assertEqual(firefox_launcher(str(executable), Path(root) / "automation"),
                             str(executable))
            (executable.parent / "platform.ini").write_text("[Build]\nMilestone=155.0\n")
            self.assertEqual(firefox_launcher(str(executable), Path(root) / "automation"),
                             str(executable))

    def test_ambiguous_package_metadata_is_rejected(self):
        with tempfile.TemporaryDirectory() as root:
            root = Path(root)
            executable = root / "bin/floorp"
            executable.parent.mkdir()
            executable.touch()
            for name in ("one", "two"):
                metadata = root / "lib" / name
                metadata.mkdir(parents=True)
                (metadata / "floorp").touch()
                (metadata / "platform.ini").write_text("[Build]\nMilestone=155.0\n")
            with self.assertRaisesRegex(ValueError, "Ambiguous"):
                firefox_launcher(str(executable), root / "automation")


class ProtocolTests(unittest.TestCase):
    def test_privileged_start_page_inventory_uses_native_metadata(self):
        driver = Mock()
        driver.script.side_effect = RuntimeError(
            "unsupported operation: ExecuteScript and ExecuteAsyncScript are not supported for privileged browsing contexts: 16"
        )
        values = {"/window/handles": ["window"], "/url": "about:home", "/title": "Floorp"}
        driver.call.side_effect = lambda method, path, *args: values.get(path)
        host = Host(driver)
        tab = host.inventory()["tabs"][0]
        self.assertEqual((tab["url"], tab["title"]), ("about:home", "Floorp"))
        self.assertFalse(tab["canGoBack"])
        self.assertEqual(host.inventory()["tabs"][0]["generation"], tab["generation"])
        # Navigation to an ordinary page resumes document-aware scripting.
        driver.script.side_effect = None
        driver.script.return_value = {"url": "https://example.test", "title": "Page", "loading": False,
                                      "document": 42}
        changed = host.inventory()["tabs"][0]
        self.assertGreater(changed["generation"], tab["generation"])
        self.assertTrue(changed["canGoBack"])

    def test_inventory_does_not_hide_unrelated_driver_failures(self):
        driver = Mock()
        driver.call.return_value = ["window"]
        driver.script.side_effect = RuntimeError("session disconnected")
        with self.assertRaisesRegex(RuntimeError, "session disconnected"):
            Host(driver).inventory()

    def test_capability_inventory_is_shared_with_nix(self):
        source = (Path(__file__).parent.parent / "lib/browserConnection.nix").read_text()
        for method in OPERATIONS:
            self.assertIn('"' + method + '"', source)
        self.assertNotIn("lighthouse", OPERATIONS)
        self.assertNotIn("heap.snapshot", OPERATIONS)

    def test_session_cannot_use_foreign_tabs(self):
        host = Host(None)
        with self.assertRaisesRegex(ValueError, "does not belong"):
            host.switch("tab_foreign")

    def test_stale_refs_are_rejected_before_driver_input(self):
        host = Host(None)
        with self.assertRaisesRegex(ValueError, "expired"):
            host.element({"refs": {}}, "@e1")


class RealBrowserSmoke(unittest.TestCase):
    @unittest.skipUnless(os.environ.get("BROWSER_CONNECTION_SMOKE_CONFIG"), "requires explicit real-browser fixture")
    def test_real_floorp_native_protocol(self):
        class Page(BaseHTTPRequestHandler):
            def do_GET(self):
                content = b'''<!doctype html><title>Adapter smoke</title><h1>Adapter smoke</h1>
                <p>Static text is readable</p><label>Name<input id="name"></label>
                <label>Choice<select id="choice"><option value="one">One</option><option value="two">Two</option></select></label>
                <label>Ready<input id="ready" type="checkbox"></label>
                <button onclick="document.querySelector('h1').textContent='Clicked'">Submit</button>
                <input type="file" aria-label="Upload"><iframe srcdoc="<h1>Inner frame</h1>"></iframe>'''
                self.send_response(200)
                self.send_header("Content-Type", "text/html")
                self.send_header("Content-Length", str(len(content)))
                self.end_headers()
                self.wfile.write(content)

            def log_message(self, *_args):
                pass

        server = ThreadingHTTPServer(("127.0.0.1", 0), Page)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        process = subprocess.Popen(
            [sys.executable, str(Path(__file__).with_name("opencode_browser.py")),
             "--config", os.environ["BROWSER_CONNECTION_SMOKE_CONFIG"]],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True,
        )
        try:
            ready = json.loads(process.stdout.readline())
            self.assertEqual(ready["version"], 1)
            def call(action, files=None):
                process.stdin.write(json.dumps({"id": "test", "command": {"action": action, "files": files or []}}) + "\n")
                process.stdin.flush()
                response = json.loads(process.stdout.readline())["outcome"]
                self.assertEqual(response["type"], "success", response)
                return response["result"]
            tabs = call({"type": "tabs.list"})["value"]["tabs"]
            tab_id = tabs[0]["id"]
            call({"type": "navigate", "tabID": tab_id, "url": f"http://127.0.0.1:{server.server_port}/"})
            snapshot = call({"type": "snapshot", "tabID": tab_id})["value"]
            self.assertIn("Adapter smoke", snapshot["content"])
            self.assertIn("Static text is readable", snapshot["content"])
            def ref(label):
                line = next(line for line in snapshot["content"].splitlines() if f'"{label}"' in line)
                return line.strip().split()[0]

            call({"type": "fill_form", "tabID": tab_id, "fields": [
                {"type": "text", "ref": ref("Name"), "value": "Canix"},
                {"type": "select", "ref": ref("Choice"), "values": ["two"]},
                {"type": "check", "ref": ref("Ready"), "checked": True},
            ]})
            self.assertEqual(call({"type": "evaluate", "tabID": tab_id, "script":
                "[document.querySelector('#name').value,document.querySelector('#choice').value,document.querySelector('#ready').checked]"
            })["value"]["value"], ["Canix", "two", True])
            call({"type": "click", "tabID": tab_id, "ref": ref("Submit")})
            call({"type": "wait", "tabID": tab_id, "condition": "text", "text": "Clicked"})
            frames = call({"type": "frames", "tabID": tab_id})["value"]["frames"]
            self.assertEqual(len(frames), 2)
            self.assertIn("Inner frame", call({"type": "snapshot", "tabID": tab_id, "frameID": "main/0"})["value"]["content"])
            evaluated = call({"type": "evaluate", "tabID": tab_id, "script": "document.title"})["value"]
            self.assertEqual(evaluated["value"], "Adapter smoke")
            screenshot = call({"type": "screenshot", "tabID": tab_id})
            self.assertTrue(screenshot["files"][0]["data"].startswith("iVBOR"))
            captures = call({"type": "files.list", "tabID": tab_id})["value"]["files"]
            self.assertEqual(len(captures), 1)
            self.assertEqual(call({"type": "files.get", "tabID": tab_id, "fileID": captures[0]["id"]})["files"], screenshot["files"])
            other = call({"type": "tabs.open"})["value"]["id"]
            self.assertNotEqual(other, tab_id)
            self.assertEqual(len(call({"type": "tabs.close", "tabID": other})["value"]["tabs"]), 1)
        finally:
            process.stdin.close()
            try:
                process.wait(timeout=10)
            finally:
                if process.poll() is None:
                    process.terminate()
                    process.wait(timeout=10)
                process.stdout.close()
                server.shutdown()
                server.server_close()


if __name__ == "__main__":
    unittest.main()
