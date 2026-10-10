"""Reject source, event and attempt substitutions in retained producer custody."""

import copy
import importlib.util
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "producer_evidence", Path(__file__).resolve().parents[1] / "scripts/retain-producer-evidence.py")
producer_evidence = importlib.util.module_from_spec(spec)
spec.loader.exec_module(producer_evidence)


class ProducerIdentity(unittest.TestCase):
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
