"""Real unchanged daemon: private single-job queue barrier and response-loss custody."""

import hashlib
import http.client
import json
import os
import shutil
import socket
import subprocess
import sys
import time
from contextlib import closing
from pathlib import Path

from namespace import communicate, fixture, processes, reap, require_isolation, wait_for


class UnixHTTP(http.client.HTTPConnection):
    def __init__(self, endpoint):
        super().__init__("localhost", timeout=10)
        self.endpoint = endpoint

    def connect(self):
        self.sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.sock.settimeout(self.timeout)
        self.sock.connect(str(self.endpoint))


def main():
    require_isolation()
    binary, controller = (str(Path(arg).resolve()) for arg in sys.argv[1:3])
    scratch = Path(sys.argv[3]).resolve()
    scratch.mkdir(parents=True, exist_ok=True)
    receipt = {
        "tier": "offline-real-daemon-admission-barrier",
        "artifact": binary,
        "artifact_sha256": hashlib.sha256(Path(binary).read_bytes()).hexdigest(),
        "controller": controller,
        "controller_sha256": hashlib.sha256(Path(controller).read_bytes()).hexdigest(),
        "enqueue_effects": 0,
        "backend": "synthetic-adapter-only",
    }
    with fixture(scratch / "admission-results.json", receipt, "rr-admit-") as root:
        home, data, repo, state = (root / name for name in ("home", "data", "repo", "admission"))
        for directory in (home, data, repo, state):
            directory.mkdir(mode=0o700)
        env = {
            "PATH": os.pathsep.join(str(Path(shutil.which(tool)).resolve().parent) for tool in ("git", "sh", "cat")),
            "HOME": str(home), "TMPDIR": str(root), "ROBOREV_DATA_DIR": str(data),
            "ROBOREV_TELEMETRY_ENABLED": "0", "GIT_CONFIG_GLOBAL": "/dev/null",
            "GIT_CONFIG_SYSTEM": "/dev/null", "GIT_TERMINAL_PROMPT": "0",
            "XDG_CONFIG_HOME": str(home / "config"), "XDG_DATA_HOME": str(home / "share"),
            "XDG_CACHE_HOME": str(home / "cache"),
            # Disposable fixture identity; no repository is published.
            "GIT_AUTHOR_NAME": "Roborev Fixture", "GIT_AUTHOR_EMAIL": "fixture@example.invalid",
            "GIT_COMMITTER_NAME": "Roborev Fixture", "GIT_COMMITTER_EMAIL": "fixture@example.invalid",
        }
        daemon = None
        peer = None

        def run(args, expected=0):
            child = subprocess.Popen(args, cwd=repo, env=env, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            exclude = tuple(process.pid for process in (daemon, peer) if process is not None)
            stdout, stderr = communicate(child, 30, exclude=exclude)
            assert child.returncode == expected, (args, stdout, stderr)
            return stdout

        run(["git", "init", "--initial-branch=main"])
        (repo / "fixture.txt").write_text("base\n")
        run(["git", "add", "fixture.txt"])
        run(["git", "commit", "-m", "base"])
        base = run(["git", "rev-parse", "HEAD"]).strip()
        (repo / "fixture.txt").write_text("head\n")
        run(["git", "commit", "-am", "head"])
        head = run(["git", "rev-parse", "HEAD"]).strip()
        delivery = scratch / "authenticated-delivery.json"
        peer_endpoint = root / "peer.sock"
        env["CANIX_ADMISSION_FIXTURE_PEER"] = str(peer_endpoint)
        document = {"schema_version": 2, "summary": "OFFLINE_ADMISSION_REVIEW", "verdict": "pass", "findings": []}
        config = data / "config.toml"
        config.write_text('server_addr = "unix://"\nmax_workers = 1\nisolate_reviews = true\n'
                          'default_agent = "opencode"\n'
                          + f'opencode_cmd = {json.dumps(controller)}\n'
                          + '[ci]\nenabled = false\n[web]\nenabled = false\n'
                          '[mcp]\nenabled = false\n[sync]\nenabled = false\n')
        endpoints = []
        generations = []
        log = (scratch / "daemon.log").open("w")

        def start():
            nonlocal daemon
            with socket.socket(socket.AF_UNIX, socket.SOCK_DGRAM) as notify:
                notify_path = root / "notify"
                notify_path.unlink(missing_ok=True)
                notify.bind(str(notify_path))
                notify.settimeout(30)
                daemon = subprocess.Popen([binary, "daemon", "run", "--config", str(config)], cwd=repo,
                                          env=env | {"NOTIFY_SOCKET": str(notify_path)}, stdout=log, stderr=log)
                assert b"READY=1" in notify.recv(4096)
            found = [path for path in root.rglob("*") if path.is_socket() and path.name != "notify"]
            assert len(found) == 1, found
            assert found[0].stat().st_mode & 0o777 == 0o600
            endpoints[:] = found
            stat = Path(f"/proc/{daemon.pid}/stat").read_text().rsplit(") ", 1)[1].split()
            generations.append({"pid": daemon.pid, "start_ticks": stat[19]})

        def stop():
            nonlocal daemon
            daemon.terminate()
            assert daemon.wait(timeout=30) == 0
            daemon = None
            wait_for(lambda: (reap() or not processes()), "private daemon descendants removed")

        def api(method, path, body=None, discard=False):
            with closing(UnixHTTP(endpoints[0])) as connection:
                connection.request(method, path, body=json.dumps(body) if body is not None else None,
                                   headers={"Content-Type": "application/json"})
                response = connection.getresponse()
                assert 200 <= response.status < 300, (path, response.status, response.read())
                if discard:
                    # The effect succeeded, but its generated job receipt is unavailable.
                    return None
                data_bytes = response.read(1024 * 1024 + 1)
                assert len(data_bytes) <= 1024 * 1024, "oversized daemon response"
                return json.loads(data_bytes)

        def inventory():
            observed = api("GET", "/api/jobs?limit=2&include_panel_members=true")
            assert observed["has_more"] is False, "truncated inventory cannot establish uniqueness"
            return observed["jobs"]

        def no_delivery():
            assert api("GET", "/api/status")["queue_paused"] is True
            observed = inventory()
            assert len(observed) == 1 and observed[0]["status"] == "queued", observed
            assert not delivery.exists(), "adapter launched before durable controller binding"
            return observed[0]

        try:
            start()
            assert not inventory(), "fresh admission needs an originally empty database"
            assert api("POST", "/api/queue/pause", {})["queue_paused"] is True
            metadata = (data / "reviews.db").stat()
            manifest = {"data": str(data), "db_device": metadata.st_dev, "db_inode": metadata.st_ino,
                        "config_sha256": hashlib.sha256(config.read_bytes()).hexdigest()}
            binding_path = root / "binding.json"
            binding_path.write_text(json.dumps({
                "request": {"request_url": "https://github.com/example/fixture/pull/1", "request_id": "admission-1",
                            "authorized_request_sha256": "a" * 64, "execution_policy_sha256": "b" * 64,
                            "base": base, "head": head},
                "controller_id": "c" * 64,
                "daemon_identity_sha256": hashlib.sha256(json.dumps(manifest, sort_keys=True).encode()).hexdigest(),
            }))
            handle = root / "original-admission.json"
            handle.write_text(run([controller, "register", str(binding_path), str(state)]))
            handle.chmod(0o600)
            assert json.loads(run([controller, "enqueue", str(handle)])) == "enqueue_unknown"
            request = {"repo_path": str(repo), "git_ref": f"{base}..{head}", "job_type": "range",
                       "agent": "opencode", "model": "fixture/admission", "panel": "none", "min_severity": "low"}
            api("POST", "/api/enqueue", request, discard=True)
            receipt["enqueue_effects"] += 1
            original = no_delivery()
            for _ in range(2):
                time.sleep(0.3)
                no_delivery()
                stop()
                start()
                observed = no_delivery()
                assert (observed["id"], observed["uuid"]) == (original["id"], original["uuid"])
                current = (data / "reviews.db").stat()
                assert (current.st_dev, current.st_ino) == (metadata.st_dev, metadata.st_ino)
                run([controller, "enqueue", str(handle)], expected=1)
            observed = no_delivery()
            assert observed["git_ref"] == request["git_ref"] and observed["job_type"] == "range"
            assert observed["repo_path"] == str(repo) and observed["agent"] == "opencode"
            job_path = root / "job.json"
            job_path.write_text(json.dumps({"id": observed["id"], "uuid": observed["uuid"]}))
            assert json.loads(run([controller, "bind", str(handle), str(job_path)])) == "job_bound"
            assert json.loads(run([controller, "dispatch", str(handle)])) == "dispatch_unknown"
            no_delivery()
            peer_log = (scratch / "peer.log").open("w")
            peer = subprocess.Popen([controller, "peer", str(daemon.pid), controller,
                                     str(peer_endpoint), str(handle), str(delivery)], cwd=repo, env=env,
                                    stdout=peer_log, stderr=peer_log)
            wait_for(peer_endpoint.is_socket, "request-exclusive authenticated adapter endpoint")
            api("POST", "/api/queue/unpause", {})
            wait_for(lambda: inventory()[0]["status"] == "done", "the original single job completes")
            assert peer.wait(timeout=10) == 0
            peer = None
            peer_log.close()
            authenticated = json.loads(delivery.read_text())
            assert authenticated["job"] == {"id": original["id"], "uuid": original["uuid"]}
            assert authenticated["daemon"]["pid"] == daemon.pid
            assert authenticated["daemon"]["start_ticks"] == int(generations[-1]["start_ticks"])
            assert authenticated["adapter"]["pid"] != daemon.pid
            assert authenticated["invocation"]["prompt"].find("fixture.txt") != -1
            saved = json.loads(run([binary, "--server", "unix://" + str(endpoints[0]), "show", "--json", "--job", str(original["id"])]))
            assert saved["job"]["uuid"] == original["uuid"] and saved["structured_output"] == document
            assert saved["prompt"] == authenticated["invocation"]["prompt"]
            receipt.update({"base": base, "head": head, "job_id": original["id"], "job_uuid": original["uuid"],
                            "daemon_generations": generations, "queue_restart_persistence": "pass",
                            "lost_response_reconciliation": "original-exclusive-inventory", "adapter_calls": 1,
                            "database_custody": manifest, "saved_review": saved,
                            "authenticated_delivery": authenticated})
        finally:
            if peer is not None:
                peer.terminate()
                peer.wait(timeout=10)
                peer = None
            if daemon is not None:
                stop()
            log.close()
    print(json.dumps(receipt))


if __name__ == "__main__":
    main()
