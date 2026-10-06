"""Persist and inspect an exact base..head review in an offline namespace."""

import hashlib
import json
import os
import shutil
import socket
import sqlite3
import subprocess
import sys
from pathlib import Path

from namespace import communicate, fixture, processes, reap, require_isolation, wait_for


def main():
    require_isolation()
    binary = str(Path(sys.argv[1]).resolve())
    producer = str(Path(sys.argv[3]).resolve()) if len(sys.argv) > 3 else None
    scratch = Path(sys.argv[2]).resolve()
    scratch.mkdir(parents=True, exist_ok=True)
    receipt = {"tier": "offline-real-daemon-comparison", "artifact": binary,
               "artifact_sha256": hashlib.sha256(Path(binary).read_bytes()).hexdigest()}
    with fixture(scratch / "comparison-results.json", receipt, "rr-range-") as root:
        home, data, repo = [root / name for name in ("home", "data", "repo")]
        for directory in (home, data, repo):
            directory.mkdir(mode=0o700)
        env = {"PATH": os.pathsep.join(str(Path(shutil.which(tool)).resolve().parent) for tool in ("git", "sh", "cat")),
               "HOME": str(home), "TMPDIR": str(root), "ROBOREV_DATA_DIR": str(data), "ROBOREV_TELEMETRY_ENABLED": "0",
               "XDG_CONFIG_HOME": str(home / "config"), "XDG_DATA_HOME": str(home / "share"), "XDG_CACHE_HOME": str(home / "cache"),
               "GIT_CONFIG_GLOBAL": "/dev/null", "GIT_CONFIG_SYSTEM": "/dev/null", "GIT_TERMINAL_PROMPT": "0",
               # Disposable fixture identity; nothing is published.
               "GIT_AUTHOR_NAME": "Roborev Fixture", "GIT_AUTHOR_EMAIL": "fixture@example.invalid",
               "GIT_COMMITTER_NAME": "Roborev Fixture", "GIT_COMMITTER_EMAIL": "fixture@example.invalid"}
        daemon = None

        def run(args):
            command = subprocess.Popen(args, env=env, cwd=repo, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            try:
                stdout, stderr = communicate(command, 90, exclude=(daemon.pid if daemon is not None else None,))
            except subprocess.TimeoutExpired as error:
                # Only private offline-namespace processes and synthetic state
                # are inspected. Keep causal evidence before fixture cleanup.
                observed = {}
                for pid in processes():
                    proc = Path(f"/proc/{pid}")
                    try:
                        observed[str(pid)] = {
                            "command": (proc / "cmdline").read_bytes().replace(b"\0", b" ").decode(errors="replace"),
                            "wait_channel": (proc / "wchan").read_text(),
                        }
                    except (FileNotFoundError, ProcessLookupError):
                        continue
                (scratch / "timeout-diagnostic.json").write_text(json.dumps({
                    "command": args, "stdout": error.stdout, "stderr": error.stderr,
                    "namespace_processes": observed,
                }, indent=2) + "\n")
                raise
            assert command.returncode == 0, stdout + stderr
            return stdout

        agent = root / "mock-opencode"
        document = {"schema_version": 2, "summary": "OFFLINE_RANGE_REVIEW", "verdict": "pass", "findings": []}
        event = {"type": "text", "part": {"type": "text", "text": json.dumps(document)}}
        agent.write_text("#!/bin/sh\ncase \" $* \" in *\" --help \"*) echo '--format --tools'; exit 0;; esac\ncat >/dev/null\n"
                         + "printf '%s\\n' '" + json.dumps(event) + "'\n")
        agent.chmod(0o700)
        config = data / "config.toml"
        config.write_text('server_addr = "unix://"\nmax_workers = 1\nisolate_reviews = true\ndefault_agent = "opencode"\n'
                          + f'opencode_cmd = {json.dumps(str(agent))}\n'
                          + '[ci]\nenabled = false\n[web]\nenabled = false\n[mcp]\nenabled = false\n[sync]\nenabled = false\n')
        run(["git", "init", "--initial-branch=main"])
        (repo / "fixture.txt").write_text("base\n")
        run(["git", "add", "fixture.txt"])
        run(["git", "commit", "-m", "base"])
        base = run(["git", "rev-parse", "HEAD"]).strip()
        (repo / "fixture.txt").write_text("head\n")
        run(["git", "add", "fixture.txt"])
        run(["git", "commit", "-m", "head"])
        head = run(["git", "rev-parse", "HEAD"]).strip()
        assert base != head
        comparison = f"{base}..{head}"
        candidate = {"url": "https://github.com/example/fixture/pull/1", "sourceRepository": "example/fixture",
                     "sourceBranch": "main", "targetBranch": "main", "head": head, "base": base, "draft": False, "open": True}
        with socket.socket(socket.AF_UNIX, socket.SOCK_DGRAM) as notify, (scratch / "daemon.log").open("w") as log:
            notify.bind(str(root / "notify"))
            notify.settimeout(30)
            daemon = subprocess.Popen([binary, "daemon", "run", "--config", str(config)],
                                      env=env | {"NOTIFY_SOCKET": str(root / "notify")}, cwd=repo, stdout=log, stderr=log)
            try:
                assert b"READY=1" in notify.recv(4096)
                endpoints = [path for path in root.rglob("*") if path.is_socket() and path.name != "notify"]
                assert len(endpoints) == 1, endpoints
                # Pin this caller-managed fixture endpoint. This cannot discover
                # or autostart another daemon; implicit-start coverage is separate.
                cli = [binary, "--server", "unix://" + str(endpoints[0])]
                receipt["endpoint_selection"] = "explicit-private-Unix-socket"
                if producer is None:
                    run([*cli, "review", "--quiet", "--agent", "opencode", "--panel", "none", "--min-severity", "low", "--wait", comparison])
                else:
                    producer_input = root / "producer-input.json"
                    producer_input.write_text(json.dumps({"candidate": candidate, "checkout": str(repo),
                                                          "socket": str(endpoints[0]), "state": str(root / "dispatch-state")}))
                    published = json.loads(run([producer, str(producer_input)]))
                    assert published["candidate"] == candidate and published["document"] == document, published
                    assert published["status"] == "done" and published["completeFindings"] is True, published
                    (scratch / "producer-receipt.json").write_text(json.dumps(published, indent=2) + "\n")
                    receipt.update(producer=producer, producer_sha256=hashlib.sha256(Path(producer).read_bytes()).hexdigest())
                # `show <range>` resolves a commit rather than selecting the
                # range job. Select the exact persisted job before reading it.
                with sqlite3.connect(f"file:{data / 'reviews.db'}?mode=ro", uri=True) as db:
                    jobs = db.execute("SELECT id FROM review_jobs WHERE git_ref=?", (comparison,)).fetchall()
                assert len(jobs) == 1, jobs
                saved = json.loads(run([*cli, "show", "--json", "--job", str(jobs[0][0])]))
                assert saved["job"]["git_ref"] == comparison and saved["job"]["status"] == "done", saved
                assert saved["job_id"] == saved["job"]["id"] and saved["agent"] == saved["job"]["agent"] == "opencode", saved
                assert saved["structured_output"] == document, saved
                assert saved["job"]["repo_path"] == str(repo) and saved["job"]["min_severity"] == "low", saved
                (scratch / "saved-comparison.json").write_text(json.dumps({"candidate": candidate, "saved": saved}, indent=2) + "\n")
                receipt.update(head=head, base=base, git_ref=comparison, job_id=saved["job_id"], review_id=saved["id"],
                               canonical_document="verified", selected_agent="opencode", version=run([binary, "version"]).strip())
            finally:
                daemon.terminate()
                assert daemon.wait(timeout=30) == 0
        daemon = None
        wait_for(lambda: (reap() or not processes()), "comparison daemon descendants removed")
    print(json.dumps(receipt))


if __name__ == "__main__":
    main()
