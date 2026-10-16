"""Reject source, event and attempt substitutions in retained producer custody."""

import contextlib
import copy
import hashlib
import importlib.util
import io
import json
import os
import subprocess
import sys
import tempfile
import unittest
import warnings
import zipfile
from pathlib import Path
from unittest.mock import patch

spec = importlib.util.spec_from_file_location(
    "producer_evidence", Path(__file__).resolve().parents[1] / "scripts/retain-producer-evidence.py")
producer_evidence = importlib.util.module_from_spec(spec)
spec.loader.exec_module(producer_evidence)


class HostedSourceIdentity(unittest.TestCase):
    def checkout(self, event, ref_type, attempt, payload, *, checkout_matches=True):
        """Run the real helper with private process-boundary fixtures."""
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            tools = root / "bin"
            tools.mkdir()
            head = root / "head"
            head.write_text("b" * 40)
            event_file = root / "event.json"
            event_file.write_text(json.dumps(payload))
            effects = root / "effects.jsonl"
            stub = "#!" + sys.executable + "\n" + '''
import json, os, pathlib, sys
kind = pathlib.Path(sys.argv[0]).name
head = pathlib.Path(os.environ["FIXTURE_HEAD"])
if kind == "jq":
    value = json.loads(pathlib.Path(sys.argv[-1]).read_text())
    try:
        for key in sys.argv[-2].lstrip(".").split("."):
            value = value[key]
    except (KeyError, TypeError):
        sys.exit(1)
    if not isinstance(value, str):
        sys.exit(1)
    print(value)
else:
    with pathlib.Path(os.environ["FIXTURE_EFFECTS"]).open("a") as trace:
        trace.write(json.dumps(sys.argv[1:]) + "\\n")
    if sys.argv[1:3] == ["checkout", "--detach"]:
        if os.environ["FIXTURE_CHECKOUT_MATCHES"] == "1":
            head.write_text(sys.argv[3])
    elif sys.argv[1:3] == ["rev-parse", "HEAD"]:
        print(head.read_text())
'''
            for name in ["git", "jq"]:
                path = tools / name
                path.write_text(stub)
                path.chmod(0o700)
            env = dict(os.environ)
            env.update({"PATH": str(tools) + os.pathsep + env["PATH"],
                        "RUNNER_ENVIRONMENT": "github-hosted", "GITHUB_EVENT_NAME": event,
                        "GITHUB_REF_TYPE": ref_type, "GITHUB_RUN_ATTEMPT": str(attempt),
                        "GITHUB_EVENT_PATH": str(event_file), "GITHUB_SHA": "b" * 40,
                        "FIXTURE_HEAD": str(head), "FIXTURE_EFFECTS": str(effects),
                        "FIXTURE_CHECKOUT_MATCHES": "1" if checkout_matches else "0"})
            script = Path(__file__).resolve().parents[1] / "scripts/prepare-hosted-source.sh"
            process = subprocess.run(["bash", str(script)], env=env, cwd=root,
                                     capture_output=True, timeout=10, check=False)
            return process.returncode, head.read_text(), effects.exists()

    def test_trusted_verification_preserves_signed_source_instead_of_default_branch(self):
        for attempt in [1, 2]:
            with self.subTest(attempt=attempt):
                status, head, effects = self.checkout(
                    "workflow_run", "branch", attempt, {"workflow_run": {"head_sha": "a" * 40}})
                self.assertEqual(status, 0)
                self.assertEqual(head, "a" * 40)
                self.assertTrue(effects)

    def test_trusted_verification_has_no_default_branch_fallback(self):
        for payload in [{}, {"workflow_run": {}}, {"workflow_run": {"head_sha": None}},
                        {"workflow_run": {"head_sha": "not-a-revision"}}]:
            with self.subTest(payload=payload):
                status, head, effects = self.checkout("workflow_run", "branch", 1, payload)
                self.assertNotEqual(status, 0)
                self.assertEqual(head, "b" * 40)
                self.assertFalse(effects)

    def test_signed_tag_recovery_can_retry_its_original_source(self):
        status, head, _ = self.checkout("push", "tag", 2, {})
        self.assertEqual(status, 0)
        self.assertEqual(head, "b" * 40)

    def test_branch_pr_and_manual_qualification_retries_have_no_checkout_effect(self):
        for event in ["push", "pull_request", "workflow_dispatch"]:
            with self.subTest(event=event):
                status, head, effects = self.checkout(
                    event, "branch", 2, {"pull_request": {"head": {"sha": "a" * 40}}})
                self.assertNotEqual(status, 0)
                self.assertEqual(head, "b" * 40)
                self.assertFalse(effects)

    def test_original_pr_uses_its_head_instead_of_synthetic_merge(self):
        status, head, _ = self.checkout(
            "pull_request", "branch", 1, {"pull_request": {"head": {"sha": "a" * 40}}})
        self.assertEqual(status, 0)
        self.assertEqual(head, "a" * 40)

    def test_unknown_event_has_no_checkout_effect(self):
        status, head, effects = self.checkout("repository_dispatch", "branch", 1, {})
        self.assertNotEqual(status, 0)
        self.assertEqual(head, "b" * 40)
        self.assertFalse(effects)

    def test_wrong_observed_checkout_cannot_pass_signed_source_verification(self):
        status, head, _ = self.checkout(
            "workflow_run", "branch", 1, {"workflow_run": {"head_sha": "a" * 40}},
            checkout_matches=False)
        self.assertNotEqual(status, 0)
        self.assertEqual(head, "b" * 40)


class ProducerIdentity(unittest.TestCase):
    def test_main_retains_cancelled_original_bytes_and_never_qualifies_empty_success(self):
        head = "a" * 40
        workflows = [".github/workflows/ci.yaml", ".github/workflows/nix-builds.yaml"]
        empty = io.BytesIO()
        with zipfile.ZipFile(empty, "w"):
            pass
        populated = io.BytesIO()
        with zipfile.ZipFile(populated, "w") as archive:
            archive.writestr("build/log.txt", b"original provider log")
        for conclusion in ["cancelled", "success"]:
            with self.subTest(conclusion=conclusion), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                for workflow in workflows:
                    path = root / workflow
                    path.parent.mkdir(parents=True, exist_ok=True)
                    path.write_text("original workflow bytes\n")
                payload = {"after": head, "ref": "refs/heads/feature", "deleted": False}
                event_path = root / "event-source.json"
                event_bytes = json.dumps(payload).encode()
                event_path.write_bytes(event_bytes)
                runs = [{"head_sha": head, "run_attempt": 1, "event": "push", "path": workflow,
                         "id": index + 1, "status": "completed",
                         "conclusion": "success" if index == 0 else conclusion,
                         "html_url": f"https://github.com/caniko/canix-toolbelt/actions/runs/{index + 1}"}
                        for index, workflow in enumerate(workflows)]
                commands = []

                def output(command, *, commands=commands, runs=runs, **kwargs):
                    commands.append(command)
                    if command == ["git", "rev-parse", "HEAD"]:
                        return head + "\n"
                    if command[0] == "git":
                        return f"{head}\n{'b' * 40}\n{'c' * 40}\n".encode()
                    self.assertEqual(command[:2], ["gh", "api"])
                    endpoint = command[-1]
                    if "actions/runs?" in endpoint:
                        return json.dumps({"workflow_runs": runs}).encode()
                    run_id = int(endpoint.split("actions/runs/")[1].split("/")[0])
                    if endpoint.endswith("/logs"):
                        return populated.getvalue() if run_id == 1 else empty.getvalue()
                    if "/jobs?" in endpoint:
                        return json.dumps([{"jobs": []}]).encode()
                    return json.dumps(runs[run_id - 1]).encode()

                def run(command, *, commands=commands, **kwargs):
                    commands.append(command)
                    status = 1 if command[:4] == ["gh", "run", "watch", "2"] else 0
                    return subprocess.CompletedProcess(command, status)

                env = {"RUNNER_ENVIRONMENT": "github-hosted", "GITHUB_RUN_ATTEMPT": "1",
                       "GITHUB_EVENT_NAME": "push", "SOURCE_HEAD": head,
                       "GITHUB_EVENT_PATH": str(event_path), "GITHUB_SHA": head,
                       "GITHUB_REPOSITORY": "caniko/canix-toolbelt", "GITHUB_RUN_ID": "99",
                       "GITHUB_WORKFLOW_REF": "caniko/canix-toolbelt/.github/workflows/retain-producer-evidence.yaml@refs/heads/feature",
                       "GITHUB_WORKFLOW_SHA": head}
                destination = root / "packet"
                with contextlib.chdir(root), patch.dict(os.environ, env), \
                        patch.object(producer_evidence.sys, "argv", ["retain", str(destination)]), \
                        patch.object(producer_evidence.subprocess, "check_output", side_effect=output), \
                        patch.object(producer_evidence.subprocess, "run", side_effect=run):
                    if conclusion == "cancelled":
                        producer_evidence.main()
                    else:
                        with self.assertRaisesRegex(AssertionError, "Successful producer"):
                            producer_evidence.main()
                receipt = json.loads((destination / "receipt.json").read_bytes())
                self.assertFalse(receipt["qualified"])
                self.assertEqual(receipt["capture_complete"], conclusion == "cancelled")
                self.assertEqual((destination / "2/provider-logs.zip").read_bytes(), empty.getvalue())
                self.assertEqual((destination / "event.json").read_bytes(), event_bytes)
                for name, digest in receipt["members"].items():
                    self.assertEqual(hashlib.sha256((destination / name).read_bytes()).hexdigest(), digest)
                if conclusion == "cancelled":
                    cancelled = next(row for row in receipt["producers"] if row["run_id"] == 2)
                    self.assertEqual(cancelled["conclusion"], "cancelled")
                    self.assertEqual(cancelled["watch_exit_status"], 1)
                    self.assertFalse(cancelled["provider_logs_complete"])
                    self.assertEqual(cancelled["provider_log_member_count"], 0)
                    self.assertEqual(cancelled["raw_logs_sha256"], hashlib.sha256(empty.getvalue()).hexdigest())
                self.assertFalse(any("rerun" in command for command in commands))

    def test_cancelled_before_execution_preserves_an_empty_provider_archive(self):
        raw = io.BytesIO()
        with zipfile.ZipFile(raw, "w"):
            pass
        self.assertEqual(
            producer_evidence.provider_log_members(raw.getvalue(), "cancelled"), {})
        with self.assertRaises(AssertionError):
            producer_evidence.provider_log_members(raw.getvalue(), "success")

    def test_successful_producer_requires_actual_log_files(self):
        raw = io.BytesIO()
        with zipfile.ZipFile(raw, "w") as archive:
            archive.writestr("build/", b"")
        with self.assertRaises(AssertionError):
            producer_evidence.provider_log_members(raw.getvalue(), "success")

    def test_original_provider_members_are_hashed_and_duplicate_names_are_rejected(self):
        raw = io.BytesIO()
        with zipfile.ZipFile(raw, "w") as archive:
            archive.writestr("build/result.txt", b"original producer bytes")
        self.assertEqual(
            producer_evidence.provider_log_members(raw.getvalue(), "success"),
            {"build/result.txt": hashlib.sha256(b"original producer bytes").hexdigest()})
        with warnings.catch_warnings():
            warnings.simplefilter("ignore", UserWarning)
            with zipfile.ZipFile(raw, "a") as archive:
                archive.writestr("build/result.txt", b"different bytes")
        for conclusion in ["success", "cancelled", "failure"]:
            with self.subTest(conclusion=conclusion), self.assertRaises(AssertionError):
                producer_evidence.provider_log_members(raw.getvalue(), conclusion)

    def test_push_capture_retains_its_event_and_rejects_pr_or_attempt_substitution(self):
        head = "a" * 40
        workflow = ".github/workflows/ci.yaml"
        run = {"head_sha": head, "run_attempt": 1, "event": "push", "path": workflow, "id": 123}
        producer_evidence.validate_run(run, head, workflow, "push")
        for key, value in [("event", "pull_request"), ("event", "workflow_dispatch"),
                           ("head_sha", "b" * 40), ("run_attempt", 2), ("id", True)]:
            with self.subTest(key=key, value=value), self.assertRaises(AssertionError):
                producer_evidence.validate_run(dict(run, **{key: value}), head, workflow, "push")

    def test_event_source_binds_branch_push_or_pr_before_observation(self):
        head = "a" * 40
        push = {"after": head, "ref": "refs/heads/feature", "deleted": False}
        pr = {"pull_request": {"head": {"sha": head}}}
        producer_evidence.validate_event_source(push, "push", head, head)
        producer_evidence.validate_event_source(pr, "pull_request", head, "b" * 40)
        for event, name, sha in [(dict(push, after="b" * 40), "push", head),
                                 (dict(push, ref="refs/tags/v1"), "push", head),
                                 (dict(push, deleted=True), "push", head),
                                 (push, "push", "b" * 40), (pr, "workflow_dispatch", head),
                                 ({"pull_request": {"head": {"sha": "b" * 40}}}, "pull_request", head)]:
            original = copy.deepcopy(event)
            with self.subTest(event=event, name=name), self.assertRaises(AssertionError):
                producer_evidence.validate_event_source(event, name, head, sha)
            self.assertEqual(original, event)

    def test_rejects_substituted_head_attempt_event_workflow_or_run(self):
        head = "a" * 40
        workflow = ".github/workflows/ci.yaml"
        run = {"head_sha": head, "run_attempt": 1, "event": "pull_request",
               "path": workflow, "id": 123}
        producer_evidence.validate_run(run, head, workflow)
        for key, value in [("head_sha", "b" * 40), ("run_attempt", 2), ("event", "push"),
                           ("path", ".github/workflows/other.yaml"), ("id", 0), ("id", "123"), ("id", True)]:
            with self.subTest(key=key, value=value), self.assertRaises(AssertionError):
                producer_evidence.validate_run(dict(run, **{key: value}), head, workflow)

    def test_rejects_missing_identity_fields(self):
        head = "a" * 40
        workflow = ".github/workflows/nix-builds.yaml"
        run = {"head_sha": head, "run_attempt": 1, "event": "pull_request",
               "path": workflow, "id": 456}
        for key in run:
            changed = {name: value for name, value in run.items() if name != key}
            with self.subTest(key=key), self.assertRaises((AssertionError, KeyError)):
                producer_evidence.validate_run(changed, head, workflow)
