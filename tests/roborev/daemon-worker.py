"""Native actual-worker integration with a separate offline unchanged daemon.

All inputs, daemon state, sockets and Git identity belong to this fixture. The
controller stays in native namespaces to observe the systemd worker. The daemon
and actual backend each have private loopback-only networks. This is explicitly
same-UID offline evidence, not root custody or production brokerage.
"""
import hashlib
import http.client
import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import time
from contextlib import closing
from pathlib import Path


class UnixHTTP(http.client.HTTPConnection):
    def __init__(self, endpoint):
        super().__init__("localhost", timeout=10)
        self.endpoint = endpoint

    def connect(self):
        self.sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.sock.settimeout(self.timeout)
        self.sock.connect(str(self.endpoint))


def wait_for(predicate, description, timeout=45):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        observed = predicate()
        if observed:
            return observed
        time.sleep(0.1)
    raise AssertionError(f"timed out: {description}")


def main():
    daemon_binary, admission_binary, worker_binary, config_path, evidence = map(Path, sys.argv[1:])
    assert os.geteuid() != 0
    evidence.mkdir(parents=True, exist_ok=False)
    root = Path(tempfile.mkdtemp(prefix="rr-integrated-", dir=os.environ["TMPDIR"]))
    root.chmod(0o700)
    home, data, repo = (root / name for name in ("home", "data", "repo"))
    for path in (home, data, repo):
        path.mkdir(mode=0o700)
    endpoint, peer_endpoint = root / "api.sock", root / "peer.sock"
    env = {
        "PATH": os.pathsep.join(str(Path(shutil.which(tool)).resolve().parent) for tool in ("git", "sh", "cat")),
        "HOME": str(home), "TMPDIR": str(root), "ROBOREV_DATA_DIR": str(data),
        "ROBOREV_TELEMETRY_ENABLED": "0", "GIT_CONFIG_GLOBAL": "/dev/null", "GIT_CONFIG_SYSTEM": "/dev/null",
        "GIT_TERMINAL_PROMPT": "0", "GIT_AUTHOR_NAME": "Disposable Integrated Fixture",
        "GIT_AUTHOR_EMAIL": "fixture@example.invalid", "GIT_COMMITTER_NAME": "Disposable Integrated Fixture",
        "GIT_COMMITTER_EMAIL": "fixture@example.invalid", "XDG_CONFIG_HOME": str(home / "config"),
        "XDG_DATA_HOME": str(home / "share"), "XDG_CACHE_HOME": str(home / "cache"),
        "CANIX_DAEMON_WORKER_FIXTURE_PEER": str(peer_endpoint),
    }
    receipt = {"tier": "offline-real-daemon-actual-worker-synthetic-backend", "fixture_root": str(root),
               "production_accepted": False, "provider_calls": False, "enqueue_effects": 0, "daemon_generations": []}
    handles, logs = [], []
    daemon = None

    def run(args, expected=0):
        result = subprocess.run([str(arg) for arg in args], cwd=repo, env=env,
                                capture_output=True, text=True, timeout=35, check=False)
        assert result.returncode == expected, (args, result.stdout, result.stderr)
        return result.stdout

    def api(method, path, body=None, discard=False):
        with closing(UnixHTTP(endpoint)) as connection:
            connection.request(method, path, body=json.dumps(body) if body is not None else None,
                               headers={"Content-Type": "application/json"})
            response = connection.getresponse()
            assert 200 <= response.status < 300, (path, response.status, response.read())
            payload = response.read(1024 * 1024 + 1)
            assert len(payload) <= 1024 * 1024
            return None if discard else json.loads(payload)

    def inventory():
        response = api("GET", "/api/jobs?limit=2&include_panel_members=true")
        assert response["has_more"] is False
        return response["jobs"]

    def start():
        nonlocal daemon
        notify_path = root / "notify.sock"
        notify_path.unlink(missing_ok=True)
        with socket.socket(socket.AF_UNIX, socket.SOCK_DGRAM) as notify:
            notify.bind(str(notify_path))
            notify.settimeout(35)
            log = (evidence / f"daemon-{len(receipt['daemon_generations'])}.log").open("w")
            logs.append(log)
            daemon = subprocess.Popen([shutil.which("unshare"), "--user", "--map-current-user", "--keep-caps", "--net",
                                       sys.executable, str(Path(__file__).with_name("offline-current-user.py")),
                                       str(daemon_binary), "daemon", "run", "--config", str(data / "config.toml")],
                                      cwd=repo, env=env | {"NOTIFY_SOCKET": str(notify_path)}, stdout=log, stderr=log)
            handles.append(daemon)
            assert b"READY=1" in notify.recv(4096)
        stat = Path(f"/proc/{daemon.pid}/stat").read_text().rsplit(") ", 1)[1].split()
        receipt["daemon_generations"].append({"pid": daemon.pid, "start_ticks": int(stat[19])})
        assert endpoint.stat().st_mode & 0o777 == 0o600
        assert Path(f"/proc/{daemon.pid}/ns/net").stat().st_ino != Path("/proc/self/ns/net").stat().st_ino
        assert {line.split(":", 1)[0].strip() for line in Path(f"/proc/{daemon.pid}/net/dev").read_text().splitlines()[2:]} == {"lo"}

    def stop():
        nonlocal daemon
        daemon.terminate()
        assert daemon.wait(timeout=35) == 0
        daemon = None

    def peer(mode, name):
        peer_endpoint.unlink(missing_ok=True)
        log = (evidence / (name + ".log")).open("w")
        logs.append(log)
        child = subprocess.Popen([str(worker_binary), mode, str(root / "plan.json"), str(daemon.pid),
                                  str(endpoint), str(peer_endpoint), str(evidence / (name + ".json"))],
                                 cwd=repo, env=env, stdout=log, stderr=log)
        handles.append(child)
        wait_for(peer_endpoint.is_socket, "authenticated peer listener")
        return child

    def rerun():
        result = api("POST", rerun_path, {"job_id": original["id"]})
        assert result["job_id"] == original["id"], result
        observed = inventory()
        assert len(observed) == 1 and observed[0]["uuid"] == original["uuid"]

    try:
        run(["git", "-c", "core.hooksPath=/dev/null", "init", "--initial-branch=main"])
        (repo / "fixture.txt").write_text("base\n")
        run(["git", "add", "fixture.txt"])
        run(["git", "-c", "commit.gpgsign=false", "-c", "core.hooksPath=/dev/null", "commit", "-m", "base"])
        base = run(["git", "rev-parse", "HEAD"]).strip()
        (repo / "fixture.txt").write_text("head\n")
        run(["git", "-c", "commit.gpgsign=false", "-c", "core.hooksPath=/dev/null", "commit", "-am", "head"])
        head = run(["git", "rev-parse", "HEAD"]).strip()
        (data / "config.toml").write_text(f'server_addr = "unix://{endpoint}"\nmax_workers = 1\nisolate_reviews = true\n'
                                          f'default_agent = "opencode"\nopencode_cmd = {json.dumps(str(worker_binary))}\n'
                                          '[ci]\nenabled = false\n[web]\nenabled = false\n[mcp]\nenabled = false\n[sync]\nenabled = false\n')
        start()
        assert not inventory()
        api("POST", "/api/queue/pause", {})
        openapi = api("GET", "/openapi.json")
        operations = {spec["operationId"]: path for path, item in openapi["paths"].items()
                      for method, spec in item.items() if method == "post" and "operationId" in spec}
        rerun_path = next(path for operation, path in operations.items() if "rerun" in operation.lower())
        cancel_path = next(path for operation, path in operations.items() if "cancel" in operation.lower() and "job" in operation.lower())
        receipt["selected_native_operations"] = {"rerun": rerun_path, "cancel": cancel_path}
        db = (data / "reviews.db").stat()
        custody = {"device": db.st_dev, "inode": db.st_ino,
                   "config_sha256": hashlib.sha256((data / "config.toml").read_bytes()).hexdigest()}
        request = {"repo_path": str(repo), "git_ref": f"{base}..{head}", "job_type": "range", "agent": "opencode",
                   "model": "fixture/integrated", "panel": "none", "min_severity": "low"}
        binding = {"request": {"request_url": "https://forge.invalid/owner/fixture/pull/1", "request_id": "integrated-1",
                               "authorized_request_sha256": hashlib.sha256(json.dumps(request, sort_keys=True).encode()).hexdigest(),
                               "execution_policy_sha256": "b" * 64, "base": base, "head": head}, "controller_id": "c" * 64,
                   "daemon_identity_sha256": hashlib.sha256(json.dumps(custody, sort_keys=True).encode()).hexdigest()}
        (root / "binding.json").write_text(json.dumps(binding))
        plan = json.loads(run([worker_binary, "initialize", config_path, root / "binding.json", root / "controller", repo / ".git/objects"]))
        (root / "plan.json").write_text(json.dumps(plan))
        (root / "plan.json").chmod(0o600)
        handle = root / "admission.json"
        handle.write_text(json.dumps(plan["admission"]))
        handle.chmod(0o600)
        run([admission_binary, "enqueue", handle])
        api("POST", "/api/enqueue", request, discard=True)
        receipt["enqueue_effects"] += 1
        original = inventory()[0]
        assert original["status"] == "queued" and not (root / "controller/worker-result").exists()
        stop()
        start()
        assert api("GET", "/api/status")["queue_paused"] is True
        assert (data / "reviews.db").stat().st_ino == db.st_ino
        assert inventory()[0]["uuid"] == original["uuid"]
        run([admission_binary, "enqueue", handle], expected=1)
        (root / "job.json").write_text(json.dumps({"id": original["id"], "uuid": original["uuid"]}))
        run([admission_binary, "bind", handle, root / "job.json"])
        run([admission_binary, "dispatch", handle])
        lost = peer("drop-peer", "lost-delivery")
        api("POST", "/api/queue/unpause", {})
        assert lost.wait(timeout=60) == 1
        wait_for(lambda: inventory()[0]["status"] == "failed", "lost output leaves a terminal daemon job")
        first = json.loads((evidence / "lost-delivery.json").read_text())
        assert first["recovered"] is False and first["reply_dropped"] is True
        run([admission_binary, "reserve-only", handle], expected=1)
        api("POST", "/api/queue/pause", {})
        stop()
        start()
        assert (data / "reviews.db").stat().st_ino == db.st_ino
        assert api("GET", "/api/status")["queue_paused"] is True
        altered = peer("capture-hold-peer", "altered-context-delivery")
        rerun()
        api("POST", "/api/queue/unpause", {})
        wait_for((root / "controller/capture-ready.json").exists, "authenticated context capture held")
        invocation = json.loads((root / "controller/capture-ready.json").read_text())
        import re
        sources = re.findall(r'<prior-range-reviews file="([^"]+)">', invocation["prompt"])
        assert len(sources) == 1
        source = Path(sources[0])
        assert source.is_relative_to(data / "ci-worktrees") and source.is_file() and not source.is_symlink()
        context_before = source.read_bytes()
        source.write_bytes(context_before + b"\n<!-- changed disposable context -->\n")
        (root / "controller/release-capture").write_text("release\n")
        assert altered.wait(timeout=30) == 1
        assert "original consumed execution fence changed" in (evidence / "altered-context-delivery.log").read_text()
        assert not (evidence / "altered-context-delivery.json").exists()
        changed = json.loads((evidence / "altered-context-delivery.attempt.json").read_text())
        assert changed["binding"]["input_manifest_sha256"] != first["binding"]["input_manifest_sha256"]
        assert changed["context_observation"]["context_sha256"] != first["context_observation"]["context_sha256"]
        assert changed["context_observation"]["worker_prompt_sha256"] == first["context_observation"]["worker_prompt_sha256"]
        run([admission_binary, "reserve-only", handle], expected=1)
        wait_for(lambda: inventory()[0]["status"] == "failed", "changed input never completes the original job")
        api("POST", "/api/queue/pause", {})
        recovered = peer("peer", "recovered-delivery")
        rerun()
        api("POST", "/api/queue/unpause", {})
        assert recovered.wait(timeout=60) == 0
        wait_for(lambda: inventory()[0]["status"] == "done", "retained output completes the original retried job")
        second = json.loads((evidence / "recovered-delivery.json").read_text())
        assert second["recovered"] is True and second["output_sha256"] == first["output_sha256"]
        assert second["binding"] == first["binding"]
        assert second["context_observation"]["source_path"] != first["context_observation"]["source_path"]
        assert second["context_observation"]["context_sha256"] == first["context_observation"]["context_sha256"]
        assert second["context_observation"]["worker_prompt_sha256"] == first["context_observation"]["worker_prompt_sha256"]
        saved = json.loads(run([daemon_binary, "--server", f"unix://{endpoint}", "show", "--json", "--job", str(original["id"])]))
        assert saved["structured_output"]["summary"] == "OFFLINE_INTEGRATED_WORKER"
        assert saved["prompt"] == second["invocation"]["prompt"]
        api("POST", "/api/queue/pause", {})
        held = peer("hold-peer", "cancelled-delivery")
        rerun()
        api("POST", "/api/queue/unpause", {})
        wait_for((root / "controller/hold-ready.json").exists, "authenticated retained-output delivery held")
        api("POST", cancel_path, {"job_id": original["id"]})
        wait_for(lambda: inventory()[0]["status"] == "canceled", "native cancellation")
        (root / "controller/release-hold").write_text("release\n")
        assert held.wait(timeout=30) == 1
        assert not (evidence / "cancelled-delivery.json").exists()
        run([admission_binary, "reserve-only", handle], expected=1)
        receipt.update({"base": base, "head": head, "job_id": original["id"], "job_uuid": original["uuid"],
                        "authenticated_deliveries": [first, second], "saved_review": saved,
                        "same_job_retry_after_reply_loss": "pass", "daemon_restart_output_recovery": "pass",
                        "changed_actual_context_rejects_retained_output_and_reexecution": "pass",
                        "changed_input_observation": changed,
                        "cancelled_delivery_cannot_reexecute_or_redeliver": "pass", "database_custody": custody})
        result = root / "controller/worker-result"
        runtime = json.loads((result / "receipt.json").read_text())
        assert runtime["cgroup_empty"] is True and runtime["backend_success"] is True
        assert runtime["binding"] == first["binding"]
        assert runtime["stdout_sha256"] == first["output_sha256"]
        receipt["worker_receipt"] = runtime
        receipt["backend_executions"] = len(list((root / "controller").glob("worker-result/receipt.json")))
        assert receipt["backend_executions"] == 1
    except BaseException as error:
        receipt["error"] = f"{type(error).__name__}: {error}"
        raise
    finally:
        for child in reversed(handles):
            if child.poll() is None:
                child.terminate()
                child.wait(timeout=40)
        for log in logs:
            log.close()
        receipt["cleanup"] = "fixture child handles reaped; systemd worker has separate verified cleanup"
        receipt["files"] = {str(path.relative_to(root)): hashlib.sha256(path.read_bytes()).hexdigest()
                            for path in root.rglob("*") if path.is_file() and not path.is_symlink()}
        receipt["artifacts"] = {name: {"path": str(path), "sha256": hashlib.sha256(path.read_bytes()).hexdigest()}
                                for name, path in [("daemon", daemon_binary), ("admission", admission_binary), ("adapter", worker_binary)]}
        (evidence / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
    print(json.dumps({key: value for key, value in receipt.items() if key not in ["files", "saved_review", "authenticated_deliveries"]}))


if __name__ == "__main__":
    main()
