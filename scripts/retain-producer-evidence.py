"""Preserve existing hosted CI results; observation never reruns a producer."""

import hashlib
import io
import json
import os
import re
import subprocess
import sys
import time
import zipfile
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime, timezone
from pathlib import Path
from urllib.parse import urlencode


def validate_run(run, head, workflow, event="pull_request"):
    assert event in ("pull_request", "push"), "Unsupported custody event"
    assert run["head_sha"] == head, "Producer source changed"
    assert run["run_attempt"] == 1, "Producer attempt is not one"
    assert run["event"] == event, "Wrong producer event"
    assert run["path"] == workflow, "Wrong producer workflow"
    assert type(run["id"]) is int and run["id"] > 0


def validate_event_source(payload, event, head, workflow_sha):
    assert event in ("pull_request", "push"), "Unsupported custody event"
    assert re.fullmatch(r"[0-9a-f]{40}", head), "Invalid source head"
    if event == "push":
        assert payload["after"] == head == workflow_sha, "Push source changed"
        assert payload["ref"].startswith("refs/heads/"), "Only branch pushes qualify"
        assert payload["deleted"] is False, "Deleted branches cannot qualify"
    else:
        assert payload["pull_request"]["head"]["sha"] == head, "PR source changed"


def provider_log_members(raw, conclusion):
    with zipfile.ZipFile(io.BytesIO(raw)) as archive:
        names = archive.namelist()
        assert len(names) == len(set(names)), "Duplicate provider log members"
        members = {name: hashlib.sha256(archive.read(name)).hexdigest()
                   for name in names if not name.endswith("/")}
    assert members or conclusion != "success", "Successful producer has no provider log files"
    return members


def main():
    assert os.environ["RUNNER_ENVIRONMENT"] == "github-hosted"
    assert os.environ["GITHUB_RUN_ATTEMPT"] == "1"
    event = os.environ["GITHUB_EVENT_NAME"]
    head = os.environ["SOURCE_HEAD"]
    payload_bytes = Path(os.environ["GITHUB_EVENT_PATH"]).read_bytes()
    validate_event_source(json.loads(payload_bytes), event, head, os.environ["GITHUB_SHA"])
    repo = os.environ["GITHUB_REPOSITORY"]
    assert repo == "caniko/canix-toolbelt"
    assert subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip() == head
    destination = Path(sys.argv[1])
    destination.mkdir(parents=True, exist_ok=False)
    workflows = [".github/workflows/ci.yaml", ".github/workflows/nix-builds.yaml"]
    receipt = {"schema": "toolbelt.producer-custody.v1", "qualified": False,
               "head": head, "repository": repo, "run_id": os.environ["GITHUB_RUN_ID"],
               "run_attempt": 1, "workflow_ref": os.environ["GITHUB_WORKFLOW_REF"],
               "workflow_sha": os.environ["GITHUB_WORKFLOW_SHA"],
               "event": event, "event_sha256": hashlib.sha256(payload_bytes).hexdigest(),
               "custody_tool_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
               "observation_is_execution": False, "capture_complete": False, "producers": []}

    def write(name, data):
        (destination / name).write_text(json.dumps(data, indent=2) + "\n")

    def api(endpoint):
        return subprocess.check_output(["gh", "api", f"repos/{repo}/{endpoint}"], timeout=90)

    def observe(run):
        workflow = run["path"]
        validate_run(run, head, workflow, event)
        folder = destination / str(run["id"])
        folder.mkdir()
        (folder / "before.json").write_text(json.dumps(run, indent=2) + "\n")
        with (folder / "watch.log").open("w") as log:
            watched = subprocess.run(["gh", "run", "watch", str(run["id"]), "--repo", repo,
                                      "--exit-status", "--interval", "60"],
                                     stdout=log, stderr=subprocess.STDOUT, check=False)
        terminal = json.loads(api(f"actions/runs/{run['id']}"))
        validate_run(terminal, head, workflow, event)
        assert terminal["status"] == "completed", "Producer did not reach terminal state"
        (folder / "terminal.json").write_text(json.dumps(terminal, indent=2) + "\n")
        jobs = json.loads(subprocess.check_output([
            "gh", "api", "--paginate", "--slurp",
            f"repos/{repo}/actions/runs/{run['id']}/attempts/1/jobs?per_page=100"], timeout=90))
        (folder / "jobs.json").write_text(json.dumps(jobs, indent=2) + "\n")
        raw = api(f"actions/runs/{run['id']}/attempts/1/logs")
        (folder / "provider-logs.zip").write_bytes(raw)
        members = provider_log_members(raw, terminal["conclusion"])
        (folder / "provider-log-members.json").write_text(json.dumps(members, indent=2) + "\n")
        return {"run_id": run["id"], "head": head, "workflow": workflow, "event": event,
                "run_attempt": 1, "status": terminal["status"], "conclusion": terminal["conclusion"],
                "watch_exit_status": watched.returncode, "url": terminal["html_url"],
                "provider_logs_complete": bool(members), "provider_log_member_count": len(members),
                "raw_logs_sha256": hashlib.sha256(raw).hexdigest()}

    try:
        (destination / "event.json").write_bytes(payload_bytes)
        with (destination / "identity-tests.log").open("w") as log:
            subprocess.run([sys.executable, "-m", "unittest", "discover", "-v", "-s", "tests",
                            "-p", "test_producer_evidence.py"], stdout=log, stderr=subprocess.STDOUT, check=True)
        (destination / "source.txt").write_bytes(subprocess.check_output([
            "git", "show", "--no-patch", "--format=%H%n%T%n%P", "HEAD"]))
        write("source-workflow-hashes.json", {path: hashlib.sha256(Path(path).read_bytes()).hexdigest()
                                              for path in workflows})
        query = urlencode({"head_sha": head, "event": event, "per_page": 100})
        # Actions creates sibling workflow records asynchronously. This is a
        # bounded discovery wait, never a producer retry or a rerun request.
        for attempt in range(12):
            runs = json.loads(api("actions/runs?" + query))
            write(f"discovery-{attempt}.json", runs)
            selected = [run for run in runs["workflow_runs"] if run["path"] in workflows]
            if len(selected) == len(workflows):
                break
            assert len(selected) < len(workflows), "Duplicate producer observations"
            time.sleep(10)
        assert sorted(run["path"] for run in selected) == sorted(workflows), "Missing producer workflow"
        for run in selected:
            validate_run(run, head, run["path"], event)
        with ThreadPoolExecutor(max_workers=2) as pool:
            for result in pool.map(observe, selected):
                receipt["producers"].append(result)
        receipt["capture_complete"] = True
    finally:
        receipt["observed_at"] = datetime.now(timezone.utc).isoformat()
        receipt["members"] = {str(path.relative_to(destination)): hashlib.sha256(path.read_bytes()).hexdigest()
                              for path in sorted(destination.rglob("*")) if path.is_file()}
        write("receipt.json", receipt)


if __name__ == "__main__":
    main()
