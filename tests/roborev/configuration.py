"""Provider-free coverage, native rendering, runtime ownership and auth tests."""

import http.client
import json
import os
import socket
import subprocess
import sys
from pathlib import Path

import tomllib
import yaml
from schema import check


def main():
    projection, source, vendor, output = map(Path, sys.argv[1:])
    fixture = json.loads(projection.read_text())
    inventory = output / "schema.json"
    inventory.write_text(json.dumps(fixture["inventory"]))
    original = sys.stdout
    try:
        with (output / "configuration-paths.json").open("w") as sys.stdout:
            check(source, vendor, inventory)
    finally:
        sys.stdout = original
    settings = tomllib.loads(Path(fixture["generatedConfig"]).read_text())
    assert settings["hook_timeout_seconds"] == 0 and settings["mouse_enabled"] is False
    assert settings["default_model"] == "" and settings["hidden_columns"] == []
    assert settings["projects"] == {} and "task_column_order" not in settings
    repository = tomllib.loads(Path(fixture["repositoryConfig"]).read_text())
    assert repository == {"review_md_fallback": False, "max_prompt_size": 0, "excluded_branches": [], "ci": {"reviews": {}}}
    for name, root in fixture["integrations"].items():
        root = Path(root)
        hooks = (root / "hooks").read_text()
        parsed = yaml.safe_load(hooks) if name == "hermes" else json.loads(hooks)
        assert parsed["fixture_setting"] == "preserved", name
        assert hooks.count("agent-hook") >= 3 and "--source=roborev-agent-hook" in hooks, name
        mcp = (root / "mcp").read_text()
        parsed = (tomllib.loads(mcp) if name in ("codex", "grok") else
                  yaml.safe_load(mcp) if name == "hermes" else json.loads(mcp))
        key = "mcp_servers" if name in ("codex", "grok", "hermes") else "mcpServers"
        entry = parsed[key]["roborev"]
        assert entry["command"] == fixture["generatedBinary"] and entry["args"] == ["mcp", "serve"], name
        if name in ("gemini", "hermes", "qwen"):
            assert mcp == hooks, name
        skills = list((root / "skills").rglob("SKILL.md"))
        variant = name if name in ("claude", "droid", "grok") else "codex"
        expected = {p.parent.name for p in (source / "internal/skills" / variant).glob("*/SKILL.md")}
        assert {p.parent.name for p in skills} == expected and expected, name
        assert bool(list((root / "skills").rglob("openai.yaml"))) == (name == "codex"), name

    data = Path(fixture["dataDir"])
    data.mkdir(parents=True, mode=0o700)
    (data / "config.toml").symlink_to(fixture["configLink"])
    private = Path(fixture["runtimeConfig"])
    env = dict(os.environ, HOME=str(data.parent), ROBOREV_COLOR_MODE="light")
    # Fresh wrapper invocations read the external file and replace caller env.
    def run(*args, ok=True):
        result = subprocess.run([fixture["binary"], *args], env=env, cwd=data.parent,
                                capture_output=True, text=True, timeout=30, check=False)
        assert (result.returncode == 0) == ok, result.stdout + result.stderr
        return result

    run("config", "validate", "--global", ok=False)
    private.write_text('server_addr = "unix://"\nmax_workers = 2\nauth_key = "' + "a" * 64 + '"\n')
    private.chmod(0o644)
    run("config", "validate", "--global", ok=False)
    private.chmod(0o600)
    run("config", "validate", "--global")
    saved = private.with_suffix(".real")
    private.rename(saved)
    private.symlink_to(saved)
    run("config", "validate", "--global", ok=False)
    private.unlink()
    saved.rename(private)
    # Atomically replace at the same path; wrapper checks and clients re-read it.
    replacement = private.with_suffix(".new")
    replacement.write_text(private.read_text().replace("a" * 64, "b" * 64))
    replacement.chmod(0o600)
    replacement.replace(private)
    run("config", "validate", "--global")
    notify = socket.socket(socket.AF_UNIX, socket.SOCK_DGRAM)
    notify.bind(str(data.parent / "notify"))
    notify.settimeout(30)
    env["NOTIFY_SOCKET"] = str(data.parent / "notify")
    env["TMPDIR"] = str(data.parent)
    with (output / "daemon.log").open("w+") as log:
        daemon = subprocess.Popen([fixture["binary"], "daemon", "run", "--config", str(data / "config.toml")],
                                  env=env, cwd=data.parent, stdout=log, stderr=log)
        try:
            assert b"READY=1" in notify.recv(4096)
            # A real client must authenticate using the same runtime TOML.
            status = json.loads(run("status", "--json").stdout)
            assert "daemon" in status
            sockets = [p for p in data.parent.rglob("*") if p.is_socket() and p.name != "notify"]
            assert len(sockets) == 1, sockets
            connection = http.client.HTTPConnection("localhost", timeout=10)
            connection.sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            connection.sock.connect(str(sockets[0]))
            connection.request("GET", "/api/status")
            assert connection.getresponse().status == 401
            connection.close()
        finally:
            notify.close()
            daemon.terminate()
            daemon.wait(timeout=125)
            assert daemon.returncode == 0
    # Scan only generated configuration/wrapper artifacts, not this test source.
    for path in [Path(fixture["generatedConfig"]), Path(fixture["binary"])]:
        assert ("a" * 64).encode() not in path.read_bytes() and ("b" * 64).encode() not in path.read_bytes()
    (output / "runtime.json").write_text(json.dumps({"runtimeFile": "missing-mode-symlink-rejected-atomic-rotation-accepted",
                                                    "authentication": "real-cli-authenticated-anonymous-401",
                                                    "profiles": sorted(fixture["integrations"])}, indent=2))


if __name__ == "__main__":
    main()
