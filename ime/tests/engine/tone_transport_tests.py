"""Actual WinHTTP loopback exchanges with disposable, synthetic corpus only."""
import http.server
import json
import os
from pathlib import Path
import subprocess
import sys
import threading
import time

probe, root = sys.argv[1], Path(sys.argv[2])
class Handler(http.server.BaseHTTPRequestHandler):
    mode = "ok"
    requests = 0
    def log_message(self, *args):
        pass
    def do_POST(self):
        Handler.requests += 1
        assert self.path == "/v1/chat/completions"
        data = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        assert data["model"] == "local-contract-test"
        assert "tools" not in data and data["stream"] is False
        prompt = data["messages"][1]["content"]
        assert "明天有空吗" in prompt and "好呀到时候见" in prompt
        mode = Handler.mode
        if mode == "redirect":
            self.send_response(302)
            self.send_header("Location", "/forbidden")
            self.send_header("Content-Length", "0")
            self.end_headers()
            return
        if mode in ("cancel", "revoke"):
            time.sleep(2)
        if mode == "error":
            self.send_response(401)
            body = b"do-not-show-provider-body"
        else:
            self.send_response(200)
            content = "3 点见\n好的明天见\n好呀有空"
            if mode == "oversize":
                content = "x" * 70000
            body = json.dumps({"choices": [{"finish_reason": "stop", "message": {"content": content}}]}, ensure_ascii=False).encode()
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        try:
            self.wfile.write(body)
        except (BrokenPipeError, ConnectionResetError, ConnectionAbortedError):
            pass

server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
thread = threading.Thread(target=server.serve_forever, daemon=True)
thread.start()
env = os.environ.copy()
env.update(OWO_TONE_BASE_URL=f"http://127.0.0.1:{server.server_port}/v1", OWO_TONE_MODEL="local-contract-test", OWO_TONE_API_KEY="")
try:
    for mode in ("ok", "paused", "redirect", "error", "oversize", "cancel", "revoke"):
        Handler.mode = mode
        count = Handler.requests
        case_root = root / mode
        case_root.mkdir(parents=True, exist_ok=True)
        started = time.monotonic()
        result = subprocess.run([probe, str(case_root), mode], env=env, capture_output=True, timeout=10)
        assert result.returncode == 0, (mode, result.returncode, result.stdout, result.stderr)
        assert Handler.requests - count == (0 if mode == "paused" else 1), mode
        if mode == "cancel":
            assert time.monotonic() - started < 4, "cancel did not interrupt request"
        print(f"tone_transport:{mode}:passed", flush=True)
finally:
    server.shutdown()
    server.server_close()
