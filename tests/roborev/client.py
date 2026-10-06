"""Drive the real repo-review CLI and Harbor daemon in an offline namespace."""

import hashlib
import json
import os
import shutil
import socket
import sqlite3
import subprocess
import sys
import threading
from pathlib import Path

from namespace import communicate, fixture, processes, reap, require_isolation, wait_for


class LostReply:
    """Forward real Unix HTTP, discarding exactly one committed enqueue reply."""

    def __init__(self, path, endpoint):
        self.endpoint = endpoint
        self.dropped = 0
        self.errors = []
        self.failures = []
        self.stopping = threading.Event()
        self.listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.listener.bind(str(path))
        self.listener.listen()
        self.listener.settimeout(0.1)
        self.worker = threading.Thread(target=self.serve)
        self.worker.start()

    def serve(self):
        while not self.stopping.is_set():
            try:
                caller, _ = self.listener.accept()
            except TimeoutError:
                continue
            try:
                with caller, socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as upstream:
                    caller.settimeout(5)
                    request = b""
                    while b"\r\n\r\n" not in request:
                        chunk = caller.recv(4096)
                        assert chunk, "incomplete proxy request"
                        request += chunk
                        assert len(request) < 16 * 1024 * 1024
                    headers, body = request.split(b"\r\n\r\n", 1)
                    length = int(next(line.split(b":", 1)[1] for line in headers.split(b"\r\n")
                                      if line.lower().startswith(b"content-length:")))
                    while len(body) < length:
                        chunk = caller.recv(4096)
                        assert chunk, "truncated proxy body"
                        body += chunk
                        assert len(body) < 16 * 1024 * 1024
                    upstream.settimeout(5)
                    upstream.connect(str(self.endpoint))
                    upstream.sendall(headers + b"\r\n\r\n" + body)
                    response = b""
                    while chunk := upstream.recv(65536):
                        response += chunk
                        assert len(response) < 16 * 1024 * 1024
                    if headers.startswith(b"POST /api/enqueue ") and self.dropped == 0:
                        assert response.startswith((b"HTTP/1.1 201", b"HTTP/1.1 200")), response
                        self.dropped += 1
                    else:
                        caller.sendall(response)
            except OSError as error:
                # Preserve connection errors as HTTP failures; never synthesize a review.
                try:
                    caller.close()
                finally:
                    self.errors.append(str(error))
            except AssertionError as error:
                self.failures.append(str(error))
                self.stopping.set()

    def close(self):
        self.stopping.set()
        self.worker.join(timeout=10)
        self.listener.close()
        assert not self.worker.is_alive(), "proxy thread did not stop"
        assert not self.failures, self.failures


def main():
    require_isolation()
    binary, client, scratch = [Path(value).resolve() for value in sys.argv[1:4]]
    scratch.mkdir(parents=True, exist_ok=False)
    receipt = {"tier": "offline-client-real-daemon-synthetic-agent", "artifacts": {
        name: {"path": str(path), "sha256": hashlib.sha256(path.read_bytes()).hexdigest()}
        for name, path in (("roborev", binary), ("repo-review", client))}, "cases": {}}
    with fixture(scratch / "client-results.json", receipt, "rr-client-") as root:
        home, data, repo, state = [root / name for name in ("home", "data", "repo", "state")]
        for directory in (home, data, repo, state):
            directory.mkdir(mode=0o700)
        env = {"PATH": os.pathsep.join(str(Path(shutil.which(tool)).resolve().parent)
                                      for tool in ("git", "sh", "cat", "sleep")),
               "HOME": str(home), "TMPDIR": str(root), "ROBOREV_DATA_DIR": str(data),
               "ROBOREV_TELEMETRY_ENABLED": "0", "GIT_CONFIG_GLOBAL": "/dev/null",
               "GIT_CONFIG_SYSTEM": "/dev/null", "GIT_TERMINAL_PROMPT": "0",
               "XDG_CONFIG_HOME": str(home / "config"), "XDG_DATA_HOME": str(home / "share"),
               "XDG_STATE_HOME": str(home / "state"), "XDG_CACHE_HOME": str(home / "cache"),
               # Explicitly disposable fixture metadata; nothing is published.
               "GIT_AUTHOR_NAME": "Client Fixture", "GIT_AUTHOR_EMAIL": "fixture@example.invalid",
               "GIT_COMMITTER_NAME": "Client Fixture", "GIT_COMMITTER_EMAIL": "fixture@example.invalid"}
        daemon = None
        calls = 0

        def run(args, expected=0):
            nonlocal calls
            command = subprocess.Popen([str(arg) for arg in args], cwd=repo, env=env,
                                       text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            stdout, stderr = communicate(command, 90, exclude=(daemon.pid if daemon else None,))
            calls += 1
            (scratch / f"command-{calls:03}.json").write_text(json.dumps({
                "argv": [str(arg) for arg in args], "exit": command.returncode,
                "stdout": stdout, "stderr": stderr}, indent=2) + "\n")
            assert command.returncode == expected, (args, command.returncode, stdout, stderr)
            return stdout.strip()

        def rows(sql, parameters=()):
            with sqlite3.connect(f"file:{data / 'reviews.db'}?mode=ro", uri=True) as db:
                return db.execute(sql, parameters).fetchall()

        def commit(text):
            (repo / "fixture.txt").write_text(text)
            run(["git", "add", "fixture.txt"])
            run(["git", "commit", "-m", "disposable fixture"])
            return run(["git", "rev-parse", "HEAD"])

        def document(findings):
            value = {"schema_version": 2, "summary": "OFFLINE_CLIENT_REVIEW",
                     "verdict": "fail" if findings else "pass", "findings": findings}
            (root / "document.json").write_text(json.dumps({"type": "text", "part": {
                "type": "text", "text": json.dumps(value)}}) + "\n")
            return value

        agent = root / "mock-opencode"
        agent.write_text("#!/bin/sh\ncat >/dev/null\n"
                         + f'while [ ! -f "{root}/release" ]; do sleep 0.1; done\n'
                         + f'cat "{root}/document.json"\n')
        agent.chmod(0o700)
        config = data / "config.toml"
        config.write_text('server_addr = "unix://"\nmax_workers = 1\nisolate_reviews = true\n'
                          'default_agent = "opencode"\nreview_min_severity = "high"\n'
                          + f'opencode_cmd = {json.dumps(str(agent))}\n'
                          + '[ci]\nenabled = false\n[web]\nenabled = false\n'
                          '[mcp]\nenabled = false\n[sync]\nenabled = false\n')
        run(["git", "init", "--initial-branch=trunk"])
        run(["git", "config", "commit.gpgsign", "false"])
        run(["git", "config", "core.hooksPath", "/dev/null"])
        base = commit("base\n")
        run(["git", "checkout", "-b", "feature"])
        head = commit("head\n")
        original_refs = run(["git", "show-ref"])
        original_status = run(["git", "status", "--porcelain=v1"])
        clean = document([])
        policy_path = root / "policy.json"

        def review(expected, extra=()):
            value = json.loads(run([client, "--config", policy_path, "--state-dir", state,
                                    "local", "--base", "trunk", "--timeout-seconds", "0", *extra], expected))
            assert value["route"]["provider"] == "roborev", value
            assert value["advisory"] and value["merge_blockers"], value
            return value

        def start():
            nonlocal daemon
            notify_path = root / "notify"
            notify_path.unlink(missing_ok=True)
            with socket.socket(socket.AF_UNIX, socket.SOCK_DGRAM) as notify:
                notify.bind(str(notify_path))
                notify.settimeout(30)
                log = (scratch / f"daemon-{calls:03}.log").open("w")
                try:
                    daemon = subprocess.Popen([binary, "daemon", "run", "--config", config], cwd=repo,
                                              env=env | {"NOTIFY_SOCKET": str(notify_path)}, stdout=log, stderr=log)
                    assert b"READY=1" in notify.recv(4096)
                finally:
                    log.close()
            endpoints = [p for p in root.rglob("*") if p.is_socket() and p not in (notify_path, root / "client.sock")]
            assert len(endpoints) == 1, endpoints
            return endpoints[0]

        def stop():
            nonlocal daemon
            daemon.terminate()
            assert daemon.wait(timeout=30) == 0
            daemon = None
            wait_for(lambda: (reap() or not processes()), "daemon descendant cleanup")

        endpoint = start()
        proxy = LostReply(root / "client.sock", endpoint)
        policy_path.write_text(json.dumps({"roborev": {"socket": str(root / "client.sock"), "qualified": True,
                                                     "qualification_receipt": "offline synthetic fixture only"}}))
        try:
            first = review(22)
            request = first["route"]["id"]
            assert first["route"]["candidate"]["base"] == base and first["route"]["candidate"]["head"] == head
            assert review(21)["route"]["id"] == request
            assert proxy.dropped == 1
            (root / "release").touch()
            wait_for(lambda: rows("SELECT id FROM reviews"), "canonical result")
            completed = review(0)
            snapshot = next(p for p in (state / "snapshots").iterdir() if p.is_dir())
            assert rows("SELECT COUNT(*) FROM review_jobs")[0][0] == 1
            persisted = json.loads(run([binary, "--server", "unix://" + str(endpoint), "show", "--json", "--job", "1"]))
            assert persisted["structured_output"] == clean, persisted
            assert persisted["job"]["repo_path"] == str(snapshot), persisted
            assert persisted["job"]["git_ref"] == f"{base}..{head}", persisted
            assert persisted["job"]["min_severity"] == "low", persisted
            assert completed["outcome"]["review"]["id"] == str(persisted["id"])
            receipt["cases"]["full_range"] = {"request_id": request, "job_id": persisted["job_id"],
                                                "review_id": persisted["id"], "base": base, "head": head}
            receipt["cases"]["lost_enqueue_reply_adopted_once"] = "pass"
            with sqlite3.connect(data / "reviews.db") as db:
                db.execute("UPDATE review_jobs SET min_severity='high' WHERE id=1")
            filtered = review(22)
            assert filtered["outcome"]["review"] is None, filtered
            assert rows("SELECT COUNT(*) FROM review_jobs")[0][0] == 1
            with sqlite3.connect(data / "reviews.db") as db:
                db.execute("UPDATE review_jobs SET min_severity='low' WHERE id=1")
            receipt["cases"]["filtered_job_rejected_without_replay"] = "pass"
            stop()
            review(22)
            assert not processes(), "client autostarted a daemon"
            endpoint = start()
            assert proxy.endpoint == endpoint
            assert review(0)["outcome"]["review"]["id"] == str(persisted["id"])
            assert rows("SELECT COUNT(*) FROM review_jobs")[0][0] == 1
            assert run(["git", "show-ref"]) == original_refs and run(["git", "status", "--porcelain=v1"]) == original_status
            receipt["cases"]["restart_resume_no_duplicate"] = "pass"
            receipt["cases"]["unavailable_daemon_no_autostart"] = "pass"
            rejected = json.loads(run([client, "--config", policy_path, "--state-dir", state, "local",
                                      "--base", "trunk", "--expected-head", "0" * 40, "--timeout-seconds", "0"], 22))
            assert "expected head" in rejected["diagnostic"], rejected
            assert rows("SELECT COUNT(*) FROM review_jobs")[0][0] == 1
            receipt["cases"]["stale_expected_head_no_dispatch"] = "pass"
            low = {"severity": "low", "problem": "OFFLINE_UNRESOLVED", "fix": "address fixture", "location": "fixture.txt:1"}
            document([low])
            next_head = commit("next head\n")
            review(21)
            wait_for(lambda: rows("SELECT id FROM reviews WHERE job_id=2"), "findings result")
            findings = review(20)
            assert len(findings["outcome"]["review"]["findings"]) == 1, findings
            assert findings["route"]["candidate"]["head"] == next_head
            assert review(20)["route"]["id"] == findings["route"]["id"]
            run([client, "--config", policy_path, "--state-dir", state, "disposition", findings["route"]["id"],
                 "--finding", findings["outcome"]["review"]["findings"][0]["id"],
                 "--reason", "synthetic fixture disposition", "--evidence", "offline source-backed fixture"])
            assert review(0)["outcome"]["review"]["id"] == findings["outcome"]["review"]["id"]
            receipt["cases"]["unresolved_low_finding_blocks"] = "pass"
            receipt["cases"]["exact_finding_disposition_fresh_observation"] = "pass"
            document([])
            (repo / "fixture.txt").write_text("dirty tracked change\n")
            (repo / "untracked.txt").write_text("EXCLUDED_UNTRACKED_FIXTURE\n")
            original_dirty = run(["git", "diff", "--binary", "HEAD"])
            plan = json.loads(run([client, "--config", policy_path, "--state-dir", state,
                                   "plan", "--working-tree"]))
            assert "untracked.txt" in plan["excluded_untracked"], plan
            review(21, ("--working-tree",))
            wait_for(lambda: rows("SELECT id FROM reviews WHERE job_id=3"), "working tree result")
            # v0.71 measures coverage only for committed reviews. A completed
            # dirty review is collectible, but unknown coverage must stay blocked.
            dirty = review(22, ("--working-tree",))
            assert dirty["route"]["candidate"]["base"] == dirty["route"]["candidate"]["head"] == next_head
            assert dirty["outcome"]["review"]["id"] == "3"
            assert dirty["outcome"]["review"]["completeFindings"] is False, dirty
            assert run(["git", "diff", "--binary", "HEAD"]) == original_dirty
            assert (repo / "untracked.txt").read_text() == "EXCLUDED_UNTRACKED_FIXTURE\n"
            assert rows("SELECT git_ref FROM review_jobs WHERE id=3") == [("dirty",)]
            receipt["cases"]["tracked_working_tree_untracked_excluded_unknown_coverage_blocks"] = "pass"
            with sqlite3.connect(data / "reviews.db") as db:
                frozen_diff = db.execute("SELECT diff_content FROM review_jobs WHERE id=3").fetchone()[0]
                db.execute("UPDATE review_jobs SET diff_content='DIFFERENT_SYNTHETIC_DIFF' WHERE id=3")
            changed = review(22, ("--working-tree",))
            assert "working-tree job diff or identity mismatch" in changed["outcome"]["blockers"], changed
            with sqlite3.connect(data / "reviews.db") as db:
                db.execute("UPDATE review_jobs SET diff_content=? WHERE id=3", (frozen_diff,))
            assert rows("SELECT COUNT(*) FROM review_jobs")[0][0] == 3
            receipt["cases"]["changed_persisted_diff_rejected_without_replay"] = "pass"
            unable = {"schema_version": 2, "summary": "OFFLINE_UNABLE_TO_REVIEW",
                      "verdict": "unable_to_review", "findings": []}
            (root / "document.json").write_text(json.dumps({"type": "text", "part": {
                "type": "text", "text": json.dumps(unable)}}) + "\n")
            commit("unsupported verdict\n")
            review(21)
            wait_for(lambda: rows("SELECT id FROM review_jobs WHERE id=4 AND status='failed'"), "unable review failure")
            rejected = review(22)
            assert "daemon job failed, interrupted or returned an unknown execution state" in rejected["outcome"]["blockers"], rejected
            review(22)
            assert rows("SELECT COUNT(*) FROM review_jobs")[0][0] == 4
            receipt["cases"]["unable_verdict_failed_job_cannot_qualify_or_replay"] = "pass"
            stop()
            receipt["version"] = run([binary, "version"])
            # Both identities are synthetic remote strings; local planning never
            # contacts either forge. Unknown Greptile evidence cannot dispatch it.
            policy = json.loads(policy_path.read_text())
            policy["greptile"] = {"enabled": True}
            policy_path.write_text(json.dumps(policy))
            run(["git", "remote", "add", "origin", "https://github.com/caniko/offline-fixture.git"])
            for host in ("github.com", "codefloe.com"):
                run(["git", "remote", "set-url", "origin", f"https://{host}/caniko/offline-fixture.git"])
                planned = json.loads(run([client, "--config", policy_path, "--state-dir", state,
                                          "plan", "--base", "trunk"]))
                assert planned["repository"]["host"] == host and planned["provider"] == "roborev", planned
                assert planned["dispatches_review"] is False
                receipt["cases"][f"{host}_offline_routing"] = planned["routing_reason"]
        finally:
            proxy.close()
            database = data / "reviews.db"
            if database.exists():
                retained = scratch / "reviews.db"
                with sqlite3.connect(f"file:{database}?mode=ro", uri=True) as source, sqlite3.connect(retained) as destination:
                    source.backup(destination)
                receipt["database"] = {"path": str(retained), "sha256": hashlib.sha256(retained.read_bytes()).hexdigest()}
            manifest = {}
            for original in sorted(state.rglob("*.json")):
                relative = original.relative_to(state)
                retained = scratch / "client-state" / relative
                retained.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
                retained.write_bytes(original.read_bytes())
                retained.chmod(0o600)
                manifest[str(relative)] = hashlib.sha256(retained.read_bytes()).hexdigest()
            (scratch / "client-state-manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
            receipt["client_state_manifest_sha256"] = hashlib.sha256((scratch / "client-state-manifest.json").read_bytes()).hexdigest()
        receipt["proxy"] = {"dropped_enqueue_replies": proxy.dropped, "connection_errors": proxy.errors}
    print(json.dumps(receipt))


if __name__ == "__main__":
    main()
