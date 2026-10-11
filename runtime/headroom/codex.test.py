"""Verify the pinned adapter without running sessions or reading real login data."""

import importlib.util
import json
import os
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("launcher", Path(__file__).with_name("codex.py"))
launcher = importlib.util.module_from_spec(spec)
spec.loader.exec_module(launcher)


class CodexAdapter(unittest.TestCase):
    def exercise(self, config, args):
        with tempfile.TemporaryDirectory() as directory:
            home = Path(directory)
            (home / "config.toml").write_text(config)
            auth = json.dumps({"auth_mode": "chatgpt", "tokens": {"account_id": "synthetic-account"}})
            (home / "auth.json").write_text(auth)
            env = {"CODEX_HOME": directory, "HOME": directory}
            with patch.dict(os.environ, env), patch("os.execve") as execute:
                launcher.launch("/native/codex", 9787, args, os.environ)
                execute.assert_called_once()
                binary, arguments, environment = execute.call_args.args
                self.assertEqual(binary, "/native/codex")
                self.assertEqual(environment["CODEX_HOME"], directory)
                self.assertEqual((home / "config.toml").read_text(), config)
                self.assertEqual((home / "auth.json").read_text(), auth)
                return arguments, environment

    def test_default_provider_and_resume_identity(self):
        arguments, env = self.exercise('model = "gpt-5"\n', ["resume", "synthetic-session"])
        self.assertEqual(arguments[-2:], ["resume", "synthetic-session"])
        self.assertFalse(any(arg.startswith("model_provider=") for arg in arguments))
        self.assertTrue(any(arg.startswith("openai_base_url=") and ":9787/" in arg for arg in arguments))
        self.assertIn(":9787/", env["OPENAI_BASE_URL"])

    def test_custom_profile_preserves_upstream_and_credentials(self):
        config = '''
[profiles.local]
model_provider = "local"
[model_providers.local]
name = "Existing local provider"
base_url = "http://127.0.0.1:8014/v1"
env_key = "EXISTING_TOKEN"
'''
        arguments, env = self.exercise(config, ["--profile", "local", "exec", "synthetic prompt"])
        self.assertTrue(any(arg.startswith("model_providers.local.base_url=") for arg in arguments))
        self.assertFalse(any(arg.startswith("model_provider=") for arg in arguments))
        self.assertIn("http://127.0.0.1:8014/v1", env.values())
        self.assertEqual(arguments[-4:], ["--profile", "local", "exec", "synthetic prompt"])


if __name__ == "__main__":
    unittest.main(verbosity=2)
