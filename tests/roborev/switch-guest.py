"""Replay already realized program-only generations inside private namespaces."""

import hashlib
import json
import os
import subprocess
import sys
from pathlib import Path

import tomllib
from namespace import fixture, require_isolation


def main():
    require_isolation()
    inputs = json.loads(Path(sys.argv[1]).read_text())
    projection = inputs["projection"]
    scratch = Path(sys.argv[2]).resolve()
    data = Path(projection["dataDir"])
    home, root = data.parent, data.parent.parent
    runtime = root / "runtime"
    receipt = {"tier": "full-hm-program-activation", "artifacts": projection, "exit_code": 1}
    with fixture(scratch / "switch-guest-results.json", receipt, "rr-switch-child-"):
        env = {key: value for key, value in os.environ.items() if key in ("PATH", "LANG")}
        env.update({
            "HOME": str(home), "USER": inputs["username"], "XDG_RUNTIME_DIR": str(runtime),
            "XDG_CONFIG_HOME": str(home / ".config"), "XDG_DATA_HOME": str(home / ".local/share"),
            "XDG_CACHE_HOME": str(home / ".cache"), "XDG_STATE_HOME": str(home / ".local/state"),
            # HM migration/profile probes use private client state, while the
            # existing store is addressed explicitly through the real daemon.
            "NIX_STATE_DIR": str(root / "nix-state"),
            "NIX_REMOTE": "unix:///nix/var/nix/daemon-socket/socket",
            "DBUS_SESSION_BUS_ADDRESS": f"unix:path={runtime}/absent-bus",
            "HOME_MANAGER_BACKUP_EXT": "fixture-backup",
        })
        # A fresh ordinary Nix user has this profile directory before HM runs.
        # Keep that prerequisite wholly inside the disposable client state.
        (home / ".local/state/nix/profiles").mkdir(parents=True, mode=0o700)

        def activate(index, expected=0):
            with (scratch / "switch-activation.log").open("a") as log:
                log.write(json.dumps({"generation": index, "expected": expected}) + "\n")
                log.flush()
                # Only fixture environment values appear in this bounded trace.
                result = subprocess.run(
                    ["bash", "-x", projection["generations"][index]["output"] + "/activate",
                     "--driver-version", "1"], env=env, cwd=home, stdout=log,
                    stderr=subprocess.STDOUT, timeout=600, check=False,
                )
                log.write(json.dumps({"generation": index, "exit_code": result.returncode}) + "\n")
            if expected == 0:
                assert result.returncode == 0, (scratch / "switch-activation.log").read_text()[-6000:]
            else:
                assert result.returncode != 0, "unmanaged configuration was accepted"
                assert "roborev: refusing unmanaged config.toml collision" in (scratch / "switch-activation.log").read_text(), \
                    "activation failed before the unmanaged-collision guard"

        data.mkdir(mode=0o700)
        config = data / "config.toml"
        config.write_text("unmanaged SYNTHETIC_SENTINEL\n")
        activate(0, expected=1)
        assert config.read_text() == "unmanaged SYNTHETIC_SENTINEL\n"
        assert not (data / "config.toml.fixture-backup").exists()
        config.unlink()
        activate(0)
        assert data.stat().st_mode & 0o777 == 0o700 and data.stat().st_uid == os.getuid()
        assert config.is_symlink()
        assert tomllib.loads(config.read_text())["max_workers"] == 2
        mutable = data / "reviews.db"
        mutable.write_bytes(b"SYNTHETIC_MUTABLE_STATE\n")
        state_digest = hashlib.sha256(mutable.read_bytes()).hexdigest()
        first_target = config.resolve()
        activate(0)
        assert config.resolve() == first_target
        activate(1)
        assert config.resolve() != first_target
        assert tomllib.loads(config.read_text())["max_workers"] == 3
        assert hashlib.sha256(mutable.read_bytes()).hexdigest() == state_digest
        # This pinned Nix client creates the XDG-state user profile. A fresh
        # fixture has no legacy ~/.nix-profile compatibility link.
        selected_binary = home / ".local/state/nix/profile/bin/roborev"
        expected_binary = Path(projection["generations"][1]["wrapper"]) / "bin/roborev"
        observed_profile = {"selected": str(selected_binary), "actual": str(selected_binary.resolve()),
                            "exists": selected_binary.is_file(), "expected": str(expected_binary.resolve()),
                            "private_links": {str(p.relative_to(root)): str(p.readlink())
                                              for p in root.rglob("*") if p.is_symlink()}}
        (scratch / "profile-lineage.json").write_text(json.dumps(observed_profile, indent=2) + "\n")
        assert selected_binary.resolve() == expected_binary.resolve(), observed_profile
        versions = subprocess.run([str(selected_binary), "version"], env=env, cwd=home,
                                  capture_output=True, text=True, timeout=30, check=False)
        assert versions.returncode == 0 and "v0.71.0" in versions.stdout, versions.stdout + versions.stderr
        validated = subprocess.run([str(selected_binary), "config", "validate", "--global"], env=env, cwd=home,
                                   capture_output=True, text=True, timeout=30, check=False)
        assert validated.returncode == 0, validated.stdout + validated.stderr
        assert not (root / "unexpected-agent-invocation").exists(), "activation invoked an agent"
        assert not (home / ".config/systemd/user/roborev.service").exists()
        assert not any(path.is_socket() for path in home.rglob("*")), "activation started a daemon"
        receipt.update({"unmanagedCollision": "rejected-without-backup", "repeatActivation": "idempotent",
                        "changedConfiguration": "relinked", "mutableState": "preserved",
                        "agentInvocation": "absent", "service": "absent", "selectedVersion": versions.stdout.strip(),
                        "managedConfigValidation": "pass", "selectedBinary": str(selected_binary.resolve()), "exit_code": 0})
    print(json.dumps(receipt))


if __name__ == "__main__":
    main()
