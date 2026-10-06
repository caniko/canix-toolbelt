"""Compile-feature-only synthetic qualification within the private worker netns."""

import hashlib
import http.server
import json
import os
import subprocess
import sys
import threading
from pathlib import Path

manifest = json.loads(Path("/control/synthetic.json").read_text())
opencode = manifest["executable"]
assert hashlib.sha256(Path(opencode).read_bytes()).hexdigest() == manifest["sha256"]
requests = []


class Provider(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_args):
        pass

    def do_POST(self):
        assert self.path == "/v1/chat/completions", self.path
        assert self.headers.get("Authorization") == "Bearer synthetic-only"
        length = int(self.headers["Content-Length"])
        assert 0 < length <= 512000
        body = json.loads(self.rfile.read(length))
        requests.append(body)
        assert body["model"] == "offline-review"
        assert len(requests) <= 2
        text = json.dumps(body)
        assert "HOSTILE_POLICY_SENTINEL" not in text
        chunks = [
            {"choices": [{"delta": {"role": "assistant"}, "finish_reason": None}], "usage": None},
            {"choices": [{"delta": {"content": "OFFLINE_SYNTHETIC_ACCEPTED"}, "finish_reason": None}], "usage": None},
            {"choices": [{"delta": {}, "finish_reason": "stop"}], "usage": None},
            {"choices": [], "usage": {"prompt_tokens": 10, "completion_tokens": 1, "total_tokens": 11}},
        ]
        payload = ("".join("data: " + json.dumps(chunk) + "\n\n" for chunk in chunks) + "data: [DONE]\n\n").encode()
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)


provider = http.server.HTTPServer(("127.0.0.1", 0), Provider)
threading.Thread(target=provider.serve_forever, daemon=True).start()
config = {
    "update": "disable",
    "model": "synthetic/offline-review",
    "providers": {
        "synthetic": {
            "name": "Offline synthetic provider",
            "package": "aisdk:@ai-sdk/openai-compatible",
            "settings": {"apiKey": "synthetic-only", "baseURL": "http://127.0.0.1:" + str(provider.server_port) + "/v1"},
            "models": {
                "offline-review": {
                    "name": "Offline review fixture",
                    "capabilities": {"tools": True, "input": ["text"], "output": ["text"]},
                    "cost": {"input": 0, "output": 0},
                    "limit": {"context": 100000, "output": 10000},
                }
            },
        }
    },
}
config_path = Path("/work/config/synthetic.json")
config_path.write_text(json.dumps(config))
Path("/work/config/models.json").write_text("{}")
env = os.environ.copy()
env.update(OPENCODE_CONFIG=str(config_path), OPENCODE_MODELS_PATH="/work/config/models.json",
           OPENCODE_DISABLE_MODELS_FETCH="1", OPENCODE_DISABLE_FFF="1", OPENCODE_FILEWATCHER_DISABLE="1")
completed = subprocess.run([opencode, "run", "--standalone", "--format", "json", "--model", "synthetic/offline-review",
                            "--title", "offline-qualification"],
                           input=sys.stdin.buffer.read(), capture_output=True, env=env, timeout=60, check=False)
provider.shutdown()
sys.stderr.buffer.write(completed.stderr)
assert completed.returncode == 0, (completed.returncode, completed.stdout.decode(errors="replace"))
assert b"OFFLINE_SYNTHETIC_ACCEPTED" in completed.stdout, completed.stdout
assert len(requests) == 1, {"request_count": len(requests)}
events = [json.loads(line) for line in completed.stdout.splitlines() if line.startswith(b"{")]
sessions = {event["sessionID"] for event in events if "sessionID" in event}
assert len(sessions) == 1, events
assert not Path("/repo/hostile-ran").exists()
print(json.dumps({"pinned_sha256": manifest["sha256"], "requests": requests,
                  "sessions": sorted(sessions), "events": events, "standalone": True}))
