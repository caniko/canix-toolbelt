"""Exercise generated artifacts inside the disposable roborev NixOS test VM."""

import hashlib
import itertools
import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

import tomllib


def main():
    assert Path("/etc/roborev-fixture").read_text() == "isolated-nixos-test\n"
    projections = json.loads(Path(sys.argv[1]).read_text())
    projection = projections[0]
    scratch = Path(sys.argv[2]).resolve()
    scratch.mkdir(parents=True, exist_ok=True)
    runtime = Path(os.environ["XDG_RUNTIME_DIR"])
    unit_dir = runtime / "systemd/user"
    unit_dir.mkdir(parents=True, exist_ok=True)
    home = Path(os.environ["HOME"])
    root = home / "fixture"
    root.mkdir(mode=0o700)
    receipt = {"tier": "hm-generated-unit-protocol-recorder-vm", "exit_code": 1}
    cleanup_errors = []
    try:
        agent = root / 'agent $literal %pct "quote" \\back'
        agent.mkdir(mode=0o700)
        environment_file = root / 'environment $literal %pct "quote" \\back'
        environment_file.write_text("RR_ROTATION=synthetic-before\n")
        environment_file.chmod(0o600)
        required_file = root / "required"
        required_file.write_text("SYNTHETIC_RUNTIME_ONLY\n")
        data = Path(projection["dataDir"])
        data.mkdir(mode=0o700)
        (data / "config.toml").symlink_to(projection["configFile"])
        settings = tomllib.loads((data / "config.toml").read_text())
        assert settings["opencode_cmd"] == str(agent) and settings["default_agent"] == "opencode"
        for generation in projections:
            for artifact in (generation["unitFile"], generation["configFile"],
                             generation["wrappedPackage"] + "/bin/roborev"):
                assert b"SYNTHETIC_RUNTIME_ONLY" not in Path(artifact).read_bytes(), artifact
        name = root.name + ".service"
        unit = unit_dir / name
        shutil.copyfile(projection["unitFile"], unit)
        (scratch / "generated-unit.service").write_text(unit.read_text())

        def systemctl(*arguments, expected=0):
            result = subprocess.run(["systemctl", "--user", *arguments],
                                    capture_output=True, text=True, timeout=150, check=False)
            with (scratch / "unit-systemctl.log").open("a") as log:
                log.write(json.dumps({"arguments": arguments, "exit_code": result.returncode}) + "\n")
                log.write(result.stdout + result.stderr)
            if expected == 0:
                assert result.returncode == 0, result.stdout + result.stderr
            else:
                assert result.returncode != 0, f"unexpected success: {arguments}"
            return result

        control_groups = set()
        installed_services = {name}

        def remember_group(service):
            group = systemctl("show", service, "--property=ControlGroup", "--value").stdout.strip()
            assert group.startswith("/user.slice/"), group
            control_groups.add(group)

        try:
            systemctl("daemon-reload")
            systemctl("start", name, expected=1)
            assert not (data / "invocations.jsonl").exists(), "directory reached the recorder"
            systemctl("stop", name)
            agent.rmdir()
            agent.write_text("#!/bin/sh\nexit 0\n")
            agent.chmod(0o600)
            # Stopped failed units may already have been garbage-collected.
            # This disposable user manager owns only our fixture services.
            systemctl("reset-failed")
            systemctl("start", name, expected=1)
            assert not (data / "invocations.jsonl").exists(), "nonexecutable file reached the recorder"
            systemctl("stop", name)
            agent.unlink()
            agent.symlink_to(projection["agentTarget"])
            systemctl("reset-failed")
            systemctl("start", name)
            remember_group(name)
            records = [json.loads(line) for line in (data / "invocations.jsonl").read_text().splitlines()]
            expected = {"args": ["daemon", "run", "--config", str(data / "config.toml")],
                        "home": str(home), "data": str(data), "path": projection["runtimePath"],
                        "rotation": "synthetic-before", "telemetry": "0", "cwd": str(home)}
            assert all(records[-1][key] == value for key, value in expected.items()), records
            assert all(records[-1]["tools"].values()), records
            direct = subprocess.run(
                [projection["wrappedPackage"] + "/bin/roborev", "config", "validate", "--global"],
                env={"HOME": str(home), "PATH": "/nonexistent"},
                capture_output=True, text=True, timeout=20, check=False,
            )
            assert direct.returncode == 0, direct.stdout + direct.stderr
            caller = json.loads((data / "invocations.jsonl").read_text().splitlines()[-1])
            assert caller["data"] == str(data) and caller["path"] == projection["runtimePath"]
            assert all(caller["tools"].values()), caller
            environment_file.write_text("RR_ROTATION=synthetic-after\n")
            systemctl("restart", name)
            assert json.loads((data / "invocations.jsonl").read_text().splitlines()[-1])["rotation"] == "synthetic-after"
            required_file.unlink()
            systemctl("restart", name, expected=1)
            systemctl("stop", name)
            required_file.write_text("SYNTHETIC_RUNTIME_ONLY\n")
            required_file.chmod(0o000)
            systemctl("reset-failed")
            systemctl("start", name, expected=1)
            systemctl("stop", name)
            required_file.chmod(0o600)
            environment_file.unlink()
            systemctl("reset-failed")
            systemctl("start", name, expected=1)
            systemctl("stop", name)
            unit.unlink()
            systemctl("reset-failed")
            systemctl("daemon-reload")
            installed_services.remove(name)

            # Activate complete HM generations against the VM's own user
            # manager. sd-switch must keep an unchanged service and restart
            # for independently changed TOML, wrapper and package identities.
            environment_file.write_text("RR_ROTATION=synthetic-switch\n")
            (data / "config.toml").unlink()
            (data / "reviews.db").write_text("PRESERVED_MUTABLE_STATE\n")
            switch_pids = []
            for index, generation in enumerate([*projections[:1], *projections]):
                installed_services.add("roborev.service")
                activation = subprocess.run(
                    [generation["generation"] + "/activate"],
                    env={key: value for key, value in os.environ.items()
                         if key in ("HOME", "USER", "LOGNAME", "PATH", "XDG_RUNTIME_DIR", "DBUS_SESSION_BUS_ADDRESS")},
                    capture_output=True, text=True, timeout=180, check=False,
                )
                (scratch / f"service-switch-{index}.log").write_text(activation.stdout + activation.stderr)
                assert activation.returncode == 0, activation.stdout + activation.stderr
                systemctl("is-active", "roborev.service")
                remember_group("roborev.service")
                pid = int(systemctl("show", "roborev.service", "--property=MainPID", "--value").stdout)
                assert pid > 0
                switch_pids.append(pid)
                latest = json.loads((data / "invocations.jsonl").read_text().splitlines()[-1])
                assert latest["pid"] == pid and latest["args"][:2] == ["daemon", "run"], latest
                assert latest["workers"] == (2 if index < 2 else 3), latest
                assert latest["path"] == generation["runtimePath"], latest
                assert latest["recorder_revision"] == generation["recorderRevision"], latest
                assert latest["hello"] == (generation["helloCommand"] if generation["expectHello"] else None), latest
                assert (data / "reviews.db").read_text() == "PRESERVED_MUTABLE_STATE\n"
            assert switch_pids[0] == switch_pids[1], switch_pids
            assert all(before != after for before, after in itertools.pairwise(switch_pids[1:])), switch_pids
            (scratch / "unit-invocations.jsonl").write_text((data / "invocations.jsonl").read_text())
            receipt.update({"directoryAgent": "rejected",
                       "nonexecutableAgent": "rejected", "unreadableRequiredFile": "rejected",
                       "symlinkAgent": "accepted", "specialCharacters": "preserved",
                       "callerPath": "replaced", "runtimeTools": "available",
                       "syntheticSecretArtifactScan": "absent",
                       "samePathRotation": "read-after-restart", "missingRequiredFile": "rejected",
                       "missingEnvironmentFile": "rejected", "artifacts": projections,
                       "serviceSwitch": "unchanged-kept-config-wrapper-and-package-restarted",
                       "serviceSwitchPids": switch_pids,
                       "artifact_sha256": [{key: hashlib.sha256(Path(generation[key]).read_bytes()).hexdigest()
                                            for key in ("unitFile", "configFile")}
                                           | {"wrapper": hashlib.sha256(Path(generation["wrappedPackage"] + "/bin/roborev").read_bytes()).hexdigest()}
                                           for generation in projections]})
        finally:
            test_error = sys.exc_info()[1]
            if test_error is not None:
                receipt["test_error"] = f"{type(test_error).__name__}: {test_error}"
            for service in installed_services:
                try:
                    systemctl("stop", service)
                    state = systemctl("show", service, "--property=MainPID", "--property=ControlPID").stdout
                    assert "MainPID=0" in state and "ControlPID=0" in state, state
                except (AssertionError, OSError, subprocess.SubprocessError) as error:
                    cleanup_errors.append(f"{service}: {error}")
            for group in control_groups:
                for processes in (Path("/sys/fs/cgroup") / group.lstrip("/")).rglob("cgroup.procs"):
                    if processes.read_text().strip():
                        cleanup_errors.append(f"descendants remain: {processes}")
            try:
                unit.unlink(missing_ok=True)
                (home / ".config/systemd/user/roborev.service").unlink(missing_ok=True)
                systemctl("daemon-reload")
            except (AssertionError, OSError, subprocess.SubprocessError) as error:
                cleanup_errors.append(str(error))
            receipt["cleanup"] = "verified" if not cleanup_errors else "failed"
            receipt["cleanup_errors"] = cleanup_errors
            receipt["exit_code"] = int(test_error is not None or bool(cleanup_errors))
        assert not cleanup_errors, cleanup_errors
    finally:
        original_error = sys.exc_info()[1]
        cleanup_error = None
        try:
            shutil.rmtree(root)
        except OSError as error:
            cleanup_error = error
            cleanup_errors.append(f"fixture files: {error}")
        receipt["cleanup"] = "verified" if not cleanup_errors else "failed"
        receipt["cleanup_errors"] = cleanup_errors
        if original_error is not None:
            receipt.setdefault("test_error", f"{type(original_error).__name__}: {original_error}")
        receipt["exit_code"] = int(original_error is not None or bool(cleanup_errors))
        (scratch / "unit-results.json").write_text(json.dumps(receipt, indent=2) + "\n")
        if cleanup_error is not None and original_error is None:
            raise cleanup_error
    print(json.dumps(receipt))


if __name__ == "__main__":
    main()
