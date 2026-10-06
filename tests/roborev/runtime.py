"""Offline daemon smoke test. All agents, repositories and state are disposable."""

import hashlib
import json
import os
import shutil
import socket
import subprocess
import sys
from pathlib import Path

from namespace import communicate, fixture, processes, reap, require_isolation, wait_for


def main():
    require_isolation()
    binary = str(Path(sys.argv[1]).resolve())
    scratch = Path(sys.argv[2]).resolve()
    scratch.mkdir(parents=True, exist_ok=True)
    receipt = {"tier": "offline-real-daemon-mock-agent", "exit_code": 1,
               "artifact": binary, "artifact_sha256": hashlib.sha256(Path(binary).read_bytes()).hexdigest()}
    # Unix socket addresses have a small fixed limit. Evidence paths may be
    # arbitrarily nested; keep only the disposable runtime under a short root.
    with fixture(scratch / "last-runtime-results.json", receipt, "rr-") as root:
        daemon = None
        home, data, repo = [root / name for name in ("home", "data", "repo")]
        for directory in (home, data, repo):
            directory.mkdir(mode=0o700)
        # A synthetic protocol fixture, not an installed coding harness.
        documents = {agent: json.dumps({"schema_version": 2,
                                       "summary": f"OFFLINE_MOCK_REVIEW_{agent}",
                                       "verdict": "pass", "findings": []})
                     for agent in ("opencode", "codex", "claude-code")}
        events = {
            "opencode": {"type": "text", "part": {"type": "text", "text": documents["opencode"]}},
            "codex": {"type": "item.completed", "item": {"type": "agent_message", "text": documents["codex"]}},
            "claude-code": {"type": "result", "subtype": "success", "result": documents["claude-code"]},
        }
        mocks = {}
        for agent, event in events.items():
            mock = root / f"mock {agent}"
            invocation = root / f"{agent}.invocation"
            mock.write_text(
                "#!/bin/sh\n"
                'case " $* " in *" --help "*) echo "--sandbox --thread-source --ignore-user-config --tools --effort"; exit 0;; esac\n'
                f'printf "%s\\n" "$@" >>"{invocation}"\n'
                "cat >/dev/null\n"
                f"printf '%s\\n' '{json.dumps(event)}'\n"
            )
            mock.chmod(0o700)
            mocks[agent] = mock
        env = {key: value for key, value in os.environ.items() if key in ("PATH", "LANG")}
        env["PATH"] = os.pathsep.join(str(Path(shutil.which(tool)).resolve().parent)
                                       for tool in ("git", "cat", "sh", "sleep"))
        env.update({
            "HOME": str(home), "ROBOREV_DATA_DIR": str(data),
            "ROBOREV_TELEMETRY_ENABLED": "0", "GIT_CONFIG_GLOBAL": "/dev/null",
            "GIT_CONFIG_SYSTEM": "/dev/null", "GIT_TERMINAL_PROMPT": "0",
            "XDG_CONFIG_HOME": str(home / "config"),
            "XDG_DATA_HOME": str(home / "share"), "XDG_CACHE_HOME": str(home / "cache"),
            "TMPDIR": str(root),
            # Explicitly disposable fixture metadata; nothing is published.
            "GIT_AUTHOR_NAME": "Roborev Fixture", "GIT_AUTHOR_EMAIL": "fixture@example.invalid",
            "GIT_COMMITTER_NAME": "Roborev Fixture", "GIT_COMMITTER_EMAIL": "fixture@example.invalid",
        })

        def run(args, expected=0, limit=90, **kwargs):
            command = subprocess.Popen(args, env=env, cwd=repo, text=True,
                                       stdout=subprocess.PIPE, stderr=subprocess.PIPE, **kwargs)
            stdout, stderr = communicate(command, limit, exclude=(daemon.pid if daemon is not None else None,))
            result = subprocess.CompletedProcess(args, command.returncode, stdout, stderr)
            if result.returncode != expected:
                raise AssertionError(f"{args}: exit {result.returncode}\n{result.stdout}\n{result.stderr}")
            return result

        run(["git", "init", "--initial-branch=main"])
        (repo / "fixture.txt").write_text("offline fixture\n")
        run(["git", "add", "fixture.txt"])
        run(["git", "commit", "-m", "fixture"])
        head = run(["git", "rev-parse", "HEAD"]).stdout.strip()
        config = data / "config.toml"
        config.write_text(
            'server_addr = "unix://"\nmax_workers = 2\n'
            'job_timeout_minutes = 1\n'
            'default_agent = "opencode"\nisolate_reviews = true\n'
            + "".join(f'{key}_cmd = {json.dumps(str(mocks[agent]))}\n'
                      for agent, key in [("opencode", "opencode"), ("codex", "codex"), ("claude-code", "claude_code")]) +
            '[ci]\nenabled = false\n[web]\nenabled = false\n'
            '[mcp]\nenabled = false\n[sync]\nenabled = false\n'
            '[agent.codex]\nignore_review_user_config = false\n'
        )
        run([binary, "config", "validate", "--global"])
        notify = socket.socket(socket.AF_UNIX, socket.SOCK_DGRAM)
        notify.bind(str(root / "notify"))
        notify.settimeout(30)
        env["NOTIFY_SOCKET"] = str(root / "notify")
        reviews = {}
        with (root / "daemon.log").open("w+") as log:
            daemon = subprocess.Popen([binary, "daemon", "run", "--config", str(config)],
                                      env=env, cwd=repo, stdout=log, stderr=log)
            try:
                assert b"READY=1" in notify.recv(4096), "foreground daemon did not notify readiness"
                notify.close()
                for agent in mocks:
                    run([binary, "review", "--agent", agent, "--panel", "none", "--wait", head])
                    record = (root / f"{agent}.invocation").read_text()
                    assert record.strip(), f"missing invocation of {agent}"
                    if agent == "opencode":
                        assert record.startswith("run\n--format\njson\n"), record
                    elif agent == "codex":
                        assert record.startswith("exec\n") and "--json\n" in record, record
                        assert "--ignore-user-config" not in record, record
                    else:
                        assert "stream-json\n" in record and "Read,Glob,Grep" in record, record
                    review = json.loads(run([binary, "show", "--json", head]).stdout)
                    assert review["agent"] == agent and review["job"]["agent"] == agent, review
                    assert review["job"]["git_ref"] == head and review["job"]["status"] == "done", review
                    assert review["job_id"] == review["job"]["id"], review
                    assert f"OFFLINE_MOCK_REVIEW_{agent}" in review["output"], review
                    assert review["job_id"] not in {value["job_id"] for value in reviews.values()}, review
                    # Re-read by job identity so a later same-head review cannot
                    # stand in for this adapter's own persisted result.
                    saved = json.loads(run([binary, "show", "--json", "--job", str(review["job_id"])]).stdout)
                    assert saved["id"] == review["id"] and saved["output"] == review["output"], saved
                    reviews[agent] = {"job_id": review["job_id"], "review_id": review["id"],
                                      "head": head, "sentinel": f"OFFLINE_MOCK_REVIEW_{agent}"}
                    (scratch / f"review-{agent}.json").write_text(json.dumps(saved, indent=2) + "\n")
                assert (data / "reviews.db").is_file(), "missing isolated database"
                # unix:// uses a UID-scoped path under TMPDIR, not dataDir.
                sockets = [path for path in root.rglob("*") if path.is_socket() and path.name != "notify"]
                assert sockets and all(path.stat().st_mode & 0o777 == 0o600 for path in sockets), [(str(path), oct(path.stat().st_mode & 0o777)) for path in sockets]
                duplicate = run([binary, "daemon", "run", "--config", str(config)], expected=1)
                assert "already" in duplicate.stderr.lower() or "lock" in duplicate.stderr.lower(), duplicate.stderr
                original = mocks["opencode"].read_text()
                for name, body in [("nonzero", "exit 37"), ("malformed", "echo not-json"), ("empty", "exit 0")]:
                    mocks["opencode"].write_text("#!/bin/sh\ncat >/dev/null\n" + body + "\n")
                    failure = run([binary, "review", "--agent", "opencode", "--panel", "none", "--wait", head], expected=1)
                    assert "failed" in failure.stderr.lower(), (name, failure.stderr)
                mocks["opencode"].unlink()
                missing = run([binary, "review", "--agent", "opencode", "--panel", "none", "--wait", head], expected=1)
                assert "opencode" in missing.stderr.lower(), missing.stderr
                mocks["opencode"].write_text("#!/bin/sh\ncat >/dev/null\nexec sleep 90\n")
                mocks["opencode"].chmod(0o700)
                # Upstream retries timed-out attempts; this is not a one-attempt timer.
                timeout = run([binary, "review", "--agent", "opencode", "--panel", "none", "--wait", head], expected=1, limit=360)
                assert any(term in timeout.stderr.lower() for term in ("timed out", "timeout", "deadline")), timeout.stderr
                mocks["opencode"].write_text(original)
            finally:
                daemon.terminate()
                try:
                    daemon.wait(timeout=125)
                except subprocess.TimeoutExpired:
                    daemon.kill()
                    daemon.wait()
                    raise AssertionError("fixture daemon failed graceful stop")
                assert daemon.returncode == 0, f"fixture daemon exit {daemon.returncode}"
                log.flush()
                log.seek(0)
                (scratch / "last-daemon.log").write_text(log.read())
                notify.close()
        daemon = None
        wait_for(lambda: (reap() or not processes()), "foreground daemon descendants removed")
        # CLI status implicitly starts an unmanaged foreground-service alternative.
        # It remains confined to this fixture and must be stopped before cleanup.
        env.pop("NOTIFY_SOCKET", None)
        try:
            status = run([binary, "status", "--json"])
            assert '"daemon"' in status.stdout, status.stdout
            for agent, identity in reviews.items():
                persisted = json.loads(run([binary, "show", "--json", "--job", str(identity["job_id"])]).stdout)
                assert persisted["agent"] == agent and persisted["job"]["git_ref"] == head, persisted
                assert identity["sentinel"] in persisted["output"], persisted
        finally:
            run([binary, "daemon", "stop"])
        wait_for(lambda: (reap() or not processes()), "autostarted daemon descendants removed")
        receipt.update({"readiness": "pass", "mockReviews": reviews, "socketMode": "0600",
                    "negativeCases": ["nonzero", "malformed", "empty", "missing", "timeout"],
                    "concurrentDaemon": "rejected", "historyAcrossRestart": "preserved",
                    "stoppedStatus": "autostarts", "head": head,
                    "version": run([binary, "version"]).stdout.strip()})
    print(json.dumps(receipt))


if __name__ == "__main__":
    main()
