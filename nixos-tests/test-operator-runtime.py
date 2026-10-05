"""Exercise the installed controller and its real Systemd adapter in isolation."""
import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path

controller, policy_path = sys.argv[1:]
manager_source = r'''
import json, os, sys
from pathlib import Path
root = Path(os.environ["FIXTURE_ROOT"])
args = sys.argv[1:]
with (root / "calls").open("a") as log:
    log.write(json.dumps(args) + "\n")
fault = os.environ.get("FIXTURE_FAULT", "")
if args[0] == "reset-failed":
    # A cold inactive unit may be collected immediately after a show query.
    if fault == "reset" or not fault:
        sys.exit(1)
if args[0] == "show":
    prop = args[2]
    if fault == prop:
        sys.exit(1)
    if prop == "LoadState":
        if fault == "not-found":
            print("not-found")
        else:
            print("loaded")
    else:
        print({"ActiveState": "failed" if fault else "inactive",
               "Result": "success", "ExecMainStatus": "0"}[prop])
'''

with tempfile.TemporaryDirectory() as directory:
    root = Path(directory)
    manager = root / "systemctl"
    manager.write_text(f"#!{sys.executable}\n" + manager_source)
    manager.chmod(0o700)
    policy = json.loads(Path(policy_path).read_text())
    policy.update(state_dir=str(root), request_path=str(root / "requested"),
                  sentinel_path=str(root / "running"), max_attempts=2,
                  retry_delays=["0s"], quiesce_units=[])
    path = root / "policy.json"
    path.write_text(json.dumps(policy))
    command = [controller, "operator", "run", "--config", str(path),
               "--systemctl", str(manager)]
    for fault in ("LoadState", "not-found", "reset", "Result", "ExecMainStatus"):
        environment = dict(os.environ, FIXTURE_ROOT=str(root), FIXTURE_FAULT=fault)
        result = subprocess.run(command, env=environment, timeout=30, check=False)
        assert result.returncode == 20, (fault, result.returncode)
        state = json.loads((root / "state.json").read_text())
        assert state["failures"] == {stage["name"]: 2 for stage in policy["stages"]}, state
        assert state["completed"] == []
        assert not (root / "requested").exists()
        assert not (root / "running").exists()
        calls = [json.loads(line) for line in (root / "calls").read_text().splitlines()]
        reset_count = 0 if fault in ("LoadState", "not-found") else 2 * len(policy["stages"])
        assert len([call for call in calls if call[0] == "reset-failed"]) == reset_count
        (root / "calls").unlink()
        print(f"Packaged Systemd adapter: {fault} failures exhaust durable retries")
    # A genuinely fresh marker after a terminal failure starts a new run.
    previous = state["run_id"]
    (root / "requested").touch()
    subprocess.run(command, env=dict(os.environ, FIXTURE_ROOT=str(root)), check=True, timeout=30)
    state = json.loads((root / "state.json").read_text())
    assert state["run_id"] != previous
    assert state["completed"] == [stage["name"] for stage in policy["stages"]]
    calls = [json.loads(line) for line in (root / "calls").read_text().splitlines()]
    assert not any(call[0] == "reset-failed" for call in calls)
    print("Packaged controller: fresh terminal request executes a distinct run")
