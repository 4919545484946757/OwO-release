"""确定性本地 OpenAI 兼容端点：给客户端/媒体工具做离线冒烟。

除了 `/chat/completions`（可指定首轮触发的工具），还实现了：
- `POST /images/generations`：返回 1x1 PNG 的 b64（也支持 `--image-url` 模式返回 url）；
- `POST /audio/speech`：返回 ID3 头的假 mp3 字节。
"""

from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import argparse
import base64
import json
import threading


# 1x1 透明 PNG
TINY_PNG = base64.b64decode(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg=="
)
TINY_MP3 = b"ID3\x04\x00\x00\x00\x00\x00\x00" + b"\xff\xfb\x90\x00" * 16


class Handler(BaseHTTPRequestHandler):
    calls = 0
    lock = threading.Lock()
    tool = "write_file"
    image_url_mode = False

    def log_message(self, _format, *_args):
        return

    def _json(self, payload, status=200):
        body = json.dumps(payload, ensure_ascii=False).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)
        self.wfile.flush()

    def _bin(self, body, content_type):
        self.send_response(200)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)
        self.wfile.flush()

    def do_GET(self):  # noqa: N802 - BaseHTTPRequestHandler API
        # 图片 url 模式：工具会二次下载，这里必须能直接取到字节。
        if self.path.endswith("/files/tiny.png"):
            self._bin(TINY_PNG, "image/png")
            return
        self.send_error(404)

    def do_POST(self):  # noqa: N802 - BaseHTTPRequestHandler API
        if self.path.endswith("/reset"):
            length = int(self.headers.get("Content-Length", "0"))
            self.rfile.read(length)
            with self.lock:
                type(self).calls = 0
            self._json({"ok": True})
            return
        if self.path.endswith("/images/generations"):
            length = int(self.headers.get("Content-Length", "0"))
            self.rfile.read(length)
            if type(self).image_url_mode:
                item = {"url": f"http://127.0.0.1:{self.server.server_port}/files/tiny.png"}
            else:
                item = {"b64_json": base64.b64encode(TINY_PNG).decode("ascii")}
            self._json({"created": 1, "data": [item]})
            return
        if self.path.endswith("/audio/speech"):
            length = int(self.headers.get("Content-Length", "0"))
            self.rfile.read(length)
            self._bin(TINY_MP3, "audio/mpeg")
            return
        if self.path.endswith("/files/tiny.png"):
            self._bin(TINY_PNG, "image/png")
            return
        if not self.path.endswith("/chat/completions"):
            self.send_error(404)
            return
        length = int(self.headers.get("Content-Length", "0"))
        self.rfile.read(length)
        with self.lock:
            type(self).calls += 1
            call = type(self).calls
        if call == 1:
            chunks = [
                {
                    "choices": [
                        {
                            "delta": {
                                "tool_calls": [
                                    {
                                        "index": 0,
                                        "id": "mock-write-call",
                                        "function": {
                                            "name": type(self).tool,
                                            "arguments": json.dumps(
                                                TOOL_ARGUMENTS[type(self).tool],
                                                ensure_ascii=False,
                                            ),
                                        },
                                    }
                                ]
                            }
                        }
                    ]
                }
            ]
        else:
            chunks = [
                {"choices": [{"delta": {"content": "golden path complete"}}]},
                {
                    "choices": [],
                    "usage": {
                        "prompt_tokens": 1,
                        "completion_tokens": 1,
                        "total_tokens": 2,
                    },
                },
            ]
        payload = "".join(f"data: {json.dumps(chunk, ensure_ascii=False)}\n\n" for chunk in chunks)
        payload += "data: [DONE]\n\n"
        body = payload.encode("utf-8")
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.send_header("Cache-Control", "no-cache")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)
        self.wfile.flush()


TOOL_ARGUMENTS = {
    "write_file": {"path": "cli-golden-output.txt", "content": "cli golden content\n"},
    "generate_image": {"prompt": "a tiny red dot", "output": "smoke-image"},
    "text_to_speech": {"text": "你好，这是一次离线冒烟", "output": "smoke-voice"},
}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--port", type=int, required=True)
    parser.add_argument("--tool", choices=sorted(TOOL_ARGUMENTS), default="write_file")
    parser.add_argument("--image-url", action="store_true", help="图片用 url 返回而不是 b64")
    args = parser.parse_args()
    Handler.tool = args.tool
    Handler.image_url_mode = args.image_url
    server = ThreadingHTTPServer(("127.0.0.1", args.port), Handler)
    print(f"mock provider on 127.0.0.1:{args.port} tool={args.tool} url_mode={args.image_url}", flush=True)
    server.serve_forever()


if __name__ == "__main__":
    main()
