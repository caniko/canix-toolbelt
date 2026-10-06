"""Real-daemon lifecycle checks inside a disposable PID/network namespace."""

import hashlib
import json
import os
import shutil
import signal
import socket
import sqlite3
import subprocess
import sys
import tempfile
import time
from pathlib import Path

from namespace import cleanup, communicate, processes, reap, require_isolation, wait_for


def main():
    require_isolation()
    binary = str(Path(sys.argv[1]).resolve())
    scratch = Path(sys.argv[2]).resolve()
    scratch.mkdir(parents=True, exist_ok=True)
    receipt = {"tier": "offline-real-daemon-lifecycle", "exit_code": 1,
               "artifact": binary, "artifact_sha256": hashlib.sha256(Path(binary).read_bytes()).hexdigest()}
    cleanup_errors = []
    temporary = tempfile.TemporaryDirectory(prefix="rr-life-", dir=os.environ.get("ROBOREV_TEST_TMPDIR"))
    root = Path(temporary.name)
    home, data, repo = [root / name for name in ("home", "data", "repo")]
    daemon = None
    log = None
    try:
        for directory in (home, data, repo):
            directory.mkdir(mode=0o700)
        env = {"PATH": os.pathsep.join(str(Path(shutil.which(tool)).resolve().parent)
                                       for tool in ("git", "sh", "cat")),
               "HOME": str(home), "TMPDIR": str(root), "ROBOREV_DATA_DIR": str(data),
               "ROBOREV_TELEMETRY_ENABLED": "0", "GIT_CONFIG_GLOBAL": "/dev/null",
               "GIT_CONFIG_SYSTEM": "/dev/null", "GIT_TERMINAL_PROMPT": "0",
               "XDG_CONFIG_HOME": str(home / "config"), "XDG_DATA_HOME": str(home / "share"),
               "XDG_CACHE_HOME": str(home / "cache"),
               # Explicitly disposable Git metadata; no fixture is published.
               "GIT_AUTHOR_NAME": "Roborev Fixture", "GIT_AUTHOR_EMAIL": "fixture@example.invalid",
               "GIT_COMMITTER_NAME": "Roborev Fixture", "GIT_COMMITTER_EMAIL": "fixture@example.invalid"}

        def run(arguments):
            command = subprocess.Popen(arguments, env=env, cwd=repo, stdout=subprocess.PIPE,
                                       stderr=subprocess.PIPE, text=True)
            stdout, stderr = communicate(command, 90, exclude=(daemon.pid if daemon is not None else None,))
            with (scratch / "commands.log").open("a") as command_log:
                command_log.write(json.dumps({"argv": arguments, "exit_code": command.returncode}) + "\n")
                command_log.write(stdout + stderr)
            assert command.returncode == 0, stdout + stderr
            return stdout

        mock = root / "mock-opencode"
        mock.write_text(f"#!{sys.executable}\n" + '''
import json, os, subprocess, sys, time
from pathlib import Path
root = Path(__file__).parent
if "--help" in sys.argv:
    print("--format --tools")
    raise SystemExit(0)
sys.stdin.read()
blocked = (root / "block").exists()
child = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(300)"])
with (root / "agent-processes.jsonl").open("a") as log:
    log.write(json.dumps({"pid": os.getpid(), "child": child.pid, "blocked": blocked}) + "\\n")
try:
    body = json.dumps({"schema_version": 2, "summary": "LIFECYCLE_COMPLETED", "verdict": "pass", "findings": []})
    split = len(body) // 2 if blocked else 0
    if blocked:
        print(json.dumps({"type": "text", "part": {"type": "text", "text": body[:split]}}), flush=True)
        while not (root / "release").exists():
            time.sleep(0.1)
    print(json.dumps({"type": "text", "part": {"type": "text", "text": body[split:]}}), flush=True)
finally:
    child.terminate()
    child.wait(timeout=5)
''')
        mock.chmod(0o700)
        config = data / "config.toml"
        config.write_text('server_addr = "unix://"\nmax_workers = 1\njob_timeout_minutes = 1\n'
                          'default_agent = "opencode"\nisolate_reviews = true\n'
                          f'opencode_cmd = {json.dumps(str(mock))}\n'
                          '[ci]\nenabled = false\n[web]\nenabled = false\n'
                          '[mcp]\nenabled = false\n[sync]\nenabled = false\n')
        run(["git", "init", "--initial-branch=main"])
        (repo / "fixture.txt").write_text("lifecycle fixture\n")
        run(["git", "add", "fixture.txt"])
        run(["git", "commit", "-m", "fixture"])
        head = run(["git", "rev-parse", "HEAD"]).strip()

        def rows(query, parameters=()):
            with sqlite3.connect(f"file:{data / 'reviews.db'}?mode=ro", uri=True) as db:
                return db.execute(query, parameters).fetchall()

        def saved_review(job, expected_head):
            saved = rows("SELECT r.id, r.agent, r.structured_output, j.git_ref, j.status "
                         "FROM reviews r JOIN review_jobs j ON j.id=r.job_id WHERE r.job_id=?", (job,))
            assert len(saved) == 1, saved
            identity, agent, document, reviewed_head, status = saved[0]
            assert agent == "opencode" and reviewed_head == expected_head and status == "done", saved
            document = json.loads(document)
            assert document == {"schema_version": 2, "summary": "LIFECYCLE_COMPLETED",
                                "verdict": "pass", "findings": []}, document
            (scratch / f"review-{job}.json").write_text(json.dumps({"review_id": identity, "job_id": job,
                "head": reviewed_head, "agent": agent, "structured_output": document}, indent=2) + "\n")

        def start(label, paused=False):
            nonlocal daemon, log
            notify_path = root / "notify"
            notify_path.unlink(missing_ok=True)
            with socket.socket(socket.AF_UNIX, socket.SOCK_DGRAM) as notify:
                notify.bind(str(notify_path))
                notify.settimeout(30)
                log = (scratch / f"daemon-{label}.log").open("w")
                daemon = subprocess.Popen([binary, "daemon", "run", "--config", str(config),
                                           f"--queue-paused={'true' if paused else 'false'}"],
                                          env=env | {"NOTIFY_SOCKET": str(notify_path)},
                                          cwd=repo, stdout=log, stderr=log)
                assert b"READY=1" in notify.recv(4096), label
            assert daemon.poll() is None, label

        def stop():
            nonlocal daemon, log
            daemon.terminate()
            assert daemon.wait(timeout=40) == 0, "daemon failed graceful shutdown"
            daemon = None
            log.close()
            log = None
            reap()

        def enqueue_blocked():
            (root / "release").unlink(missing_ok=True)
            (root / "block").touch()
            before = rows("SELECT COALESCE(MAX(id), 0) FROM review_jobs")[0][0]
            run([binary, "review", "--agent", "opencode", "--panel", "none", head])
            job = wait_for(lambda: rows("SELECT id FROM review_jobs WHERE id > ? AND status='running'", (before,)),
                           "job running")[0][0]
            wait_for(lambda: (root / "agent-processes.jsonl").exists()
                     and len((root / "agent-processes.jsonl").read_text().splitlines()) >= job,
                     "mock invocation")
            return job

        start("graceful")
        first = enqueue_blocked()
        daemon.terminate()
        time.sleep(0.3)
        assert daemon.poll() is None, "active review was not drained"
        (root / "release").touch()
        assert daemon.wait(timeout=40) == 0, "draining daemon did not finish"
        daemon = None
        log.close()
        log = None
        reap()
        status = rows("SELECT status, error FROM review_jobs WHERE id=?", (first,))
        assert status == [("done", None)], status
        saved_review(first, head)
        assert not processes(), f"graceful shutdown leaked descendants: {processes()}"
        receipt["gracefulActiveReview"] = "drained-and-persisted"

        start("forced")
        forced = enqueue_blocked()
        daemon.kill()
        assert daemon.wait(timeout=10) == -signal.SIGKILL
        daemon = None
        log.close()
        log = None
        reap()
        assert rows("SELECT status FROM review_jobs WHERE id=?", (forced,)) == [("running",)]
        assert not rows("SELECT id FROM reviews WHERE job_id=?", (forced,)), "partial output became a saved review"
        remaining = processes()
        receipt["daemonOnlySigkillDescendants"] = remaining
        # A daemon-only SIGKILL is intentionally distinct from systemd's whole
        # cgroup stop. Clean up only this private namespace's observed children.
        for pid in remaining:
            os.kill(pid, signal.SIGKILL)
        wait_for(lambda: (reap() or not processes()), "forced fixture descendants removed")
        stale_sockets = [str(path) for path in root.rglob("*") if path.is_socket() and path.name != "notify"]
        assert stale_sockets, "forced stop did not leave a stale socket to exercise"
        receipt["staleSockets"] = stale_sockets

        start("stale-recovery", paused=True)
        assert rows("SELECT status, worker_id FROM review_jobs WHERE id=?", (forced,)) == [("queued", None)]
        assert not rows("SELECT id FROM reviews WHERE job_id=?", (forced,))
        assert rows("PRAGMA integrity_check") == [("ok",)]
        receipt["staleRecovery"] = "socket-and-lock-reacquired-interrupted-job-requeued-no-review"
        stop()
        (root / "release").touch()
        (root / "block").unlink()
        start("resumed")
        wait_for(lambda: rows("SELECT id FROM reviews WHERE job_id=?", (forced,)), "requeued job completed")
        assert rows("SELECT status FROM review_jobs WHERE id=?", (forced,)) == [("done",)]
        saved_review(forced, head)
        stop()
        receipt["interruptedJob"] = {"job_id": forced, "head": head, "replayed": "done"}

        # Install and execute only this temporary repository's actual selected
        # post-commit hook while the daemon is stopped.
        before = rows("SELECT COALESCE(MAX(id), 0) FROM review_jobs")[0][0]
        run([binary, "install-hook", "--binary", binary])
        hook = repo / ".git/hooks/post-commit"
        assert binary in hook.read_text(), "hook did not bake the selected binary"
        (scratch / "post-commit-hook").write_text(hook.read_text())
        (repo / "fixture.txt").write_text("hook fixture\n")
        run(["git", "add", "fixture.txt"])
        run(["git", "commit", "-m", "hook fixture"])
        hook_head = run(["git", "rev-parse", "HEAD"]).strip()
        hook_job = wait_for(lambda: rows("SELECT id FROM review_jobs WHERE id>? AND git_ref=?", (before, hook_head)),
                            "stopped hook autostart and enqueue")[0][0]
        wait_for(lambda: rows("SELECT id FROM reviews WHERE job_id=?", (hook_job,)), "hook review completed")
        saved_review(hook_job, hook_head)
        run([binary, "daemon", "stop"])
        wait_for(lambda: (reap() or not processes()), "autostarted daemon cleanup")
        receipt["stoppedPostCommitHook"] = {"behavior": "autostarts-and-enqueues", "job_id": hook_job, "head": hook_head}
        receipt["databaseIntegrity"] = rows("PRAGMA integrity_check")[0][0]
        receipt["version"] = run([binary, "version"]).strip()
    finally:
        original_error = sys.exc_info()[1]
        if daemon is not None and daemon.poll() is None:
            daemon.kill()
            daemon.wait(timeout=10)
        if log is not None:
            log.close()
        cleanup_errors.extend(cleanup())
        try:
            temporary.cleanup()
        except OSError as error:
            cleanup_errors.append(str(error))
        receipt["cleanup"] = "verified" if not cleanup_errors else "failed"
        receipt["cleanup_errors"] = cleanup_errors
        if original_error is not None:
            receipt["error"] = f"{type(original_error).__name__}: {original_error}"
        receipt["exit_code"] = int(original_error is not None or bool(cleanup_errors))
        (scratch / "lifecycle-results.json").write_text(json.dumps(receipt, indent=2) + "\n")
        if original_error is None:
            assert not cleanup_errors, cleanup_errors
    print(json.dumps(receipt))


if __name__ == "__main__":
    main()
