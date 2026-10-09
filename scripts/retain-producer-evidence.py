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


def validate_run(run, head, workflow):
    assert run["head_sha"] == head, "Producer source changed"
    assert run["run_attempt"] == 1, "Producer attempt is not one"
    assert run["event"] == "pull_request", "Wrong producer event"
    assert run["path"] == workflow, "Wrong producer workflow"
    assert type(run["id"]) is int and run["id"] > 0


def main():
    assert os.environ["RUNNER_ENVIRONMENT"] == "github-hosted"
    assert os.environ["GITHUB_RUN_ATTEMPT"] == "1"
    assert os.environ["GITHUB_EVENT_NAME"] == "pull_request"
    head = os.environ["SOURCE_HEAD"]
    assert re.fullmatch(r"[0-9a-f]{40}", head)
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
               "observation_is_execution": False, "capture_complete": False, "producers": []}

    def write(name, data):
        (destination / name).write_text(json.dumps(data, indent=2) + "\n")

    def api(endpoint):
        return subprocess.check_output(["gh", "api", f"repos/{repo}/{endpoint}"], timeout=90)

    def observe(run):
        workflow = run["path"]
        validate_run(run, head, workflow)
        folder = destination / str(run["id"])
        folder.mkdir()
        (folder / "before.json").write_text(json.dumps(run, indent=2) + "\n")
        with (folder / "watch.log").open("w") as log:
            watched = subprocess.run(["gh", "run", "watch", str(run["id"]), "--repo", repo,
                                      "--exit-status", "--interval", "60"],
                                     stdout=log, stderr=subprocess.STDOUT, check=False)
        terminal = json.loads(api(f"actions/runs/{run['id']}"))
        validate_run(terminal, head, workflow)
        assert terminal["status"] == "completed", "Producer did not reach terminal state"
        (folder / "terminal.json").write_text(json.dumps(terminal, indent=2) + "\n")
        jobs = json.loads(subprocess.check_output([
            "gh", "api", "--paginate", "--slurp",
            f"repos/{repo}/actions/runs/{run['id']}/attempts/1/jobs?per_page=100"], timeout=90))
        (folder / "jobs.json").write_text(json.dumps(jobs, indent=2) + "\n")
        raw = api(f"actions/runs/{run['id']}/attempts/1/logs")
        (folder / "provider-logs.zip").write_bytes(raw)
        with zipfile.ZipFile(io.BytesIO(raw)) as archive:
            names = archive.namelist()
            assert names and len(names) == len(set(names)), "Missing or duplicate provider log members"
            members = {name: hashlib.sha256(archive.read(name)).hexdigest()
                       for name in names if not name.endswith("/")}
        (folder / "provider-log-members.json").write_text(json.dumps(members, indent=2) + "\n")
        return {"run_id": run["id"], "head": head, "workflow": workflow,
                "run_attempt": 1, "status": terminal["status"], "conclusion": terminal["conclusion"],
                "watch_exit_status": watched.returncode, "url": terminal["html_url"],
                "raw_logs_sha256": hashlib.sha256(raw).hexdigest()}

    try:
        with (destination / "identity-tests.log").open("w") as log:
            subprocess.run([sys.executable, "-m", "unittest", "discover", "-v", "-s", "tests",
                            "-p", "test_producer_evidence.py"], stdout=log, stderr=subprocess.STDOUT, check=True)
        (destination / "source.txt").write_bytes(subprocess.check_output([
            "git", "show", "--no-patch", "--format=%H%n%T%n%P", "HEAD"]))
        write("source-workflow-hashes.json", {path: hashlib.sha256(Path(path).read_bytes()).hexdigest()
                                              for path in workflows})
        query = urlencode({"head_sha": head, "event": "pull_request", "per_page": 100})
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
            validate_run(run, head, run["path"])
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
