"""Run the managed OwOBot gateway and optional Chromium workbench on Windows."""

import argparse
from contextlib import contextmanager, nullcontext
from http.cookies import SimpleCookie
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import tempfile
from threading import Thread
from urllib.request import Request
from uuid import uuid4

from windows_smoke import HTTP, UPSTREAM, expect, isolated_environment, process, until


def browser_session(origin, auth, model=None):
    request = Request(
        origin + "/owobot/open", headers={**auth, "Content-Type": "application/json"},
        data=json.dumps(model or {}).encode("utf-8"), method="POST",
    )
    with HTTP.open(request, timeout=45) as response:
        attributes = SimpleCookie(response.headers["Set-Cookie"])["owo_collaboration"]
        assert attributes["httponly"] and attributes["samesite"] == "Strict"
        assert attributes["path"] == "/owobot/api/"
        body = json.load(response)
        assert set(body) == {"groupId", "url"} and body["url"] == "/owobot/"
        return body["groupId"], {"Cookie": f"owo_collaboration={attributes.value}"}


def event_hello(origin, auth):
    request = Request(origin + "/owobot/api/events?screens=off", headers=auth)
    with HTTP.open(request, timeout=10) as stream:
        assert stream.headers.get_content_type() == "text/event-stream"
        for line in stream:
            if line.startswith(b"data: "):
                frame = json.loads(line[len(b"data: "):])
                if frame.get("kind") == "hello":
                    return frame
    raise AssertionError("EventSource did not receive a hello frame")


def daemon_port(daemon, data):
    descriptor = data / "runtime" / "daemon.json"
    return until(daemon, "OwO Daemon did not start", lambda: json.loads(
        descriptor.read_text(encoding="utf-8"),
    )["port"])


def running_sidecar_ports(daemon, data):
    """Read the child listening address from the owning process on Windows."""
    script = (
        f"$children = (Get-CimInstance -ClassName Win32_Process -Filter "
        f"'ParentProcessId={daemon.pid}' | Where-Object Name -EQ 'node.exe').ProcessId; "
        "if (!$children) { exit 2 }; "
        "(Get-NetTCPConnection -State Listen -LocalAddress 127.0.0.1 | "
        "Where-Object { $children -contains $_.OwningProcess }).LocalPort"
    )
    result = subprocess.run(
        ["powershell.exe", "-NoProfile", "-NonInteractive", "-Command", script],
        text=True, capture_output=True, timeout=30, creationflags=subprocess.CREATE_NO_WINDOW,
    )
    assert result.returncode == 0, result.stderr
    ports = [int(line) for line in result.stdout.splitlines() if line.isdigit()]
    assert len(ports) == 1, f"expected one loopback listener, found {ports}"
    return ports[0]


def stage_fixture(node, destination):
    destination.mkdir()
    shutil.copyfile(node, destination / "node.exe")
    shutil.copytree(UPSTREAM / "dist-owobot", destination / "dist")
    shutil.copytree(UPSTREAM / "dist-server", destination / "dist-server")
    (destination / "package.json").write_text(
        json.dumps({"type": "module", "version": "0.1.158"}), encoding="utf-8",
    )


class SelectedModel:
    name = "owobot-desktop-selection"
    text = "The selected desktop model answered."
    credential = "owobot-loopback-fixture-key"

    def __init__(self, port):
        self.calls = []
        endpoint = f"http://127.0.0.1:{port}/v1"
        reference = "1" * 32
        name = "OWO_CUSTOM_MODEL_SECRET_" + reference
        self.selection = {"model": self.name, "connection": {
            "model": self.name, "base_url": endpoint, "api_format": "openai", "api_key_env": name,
        }}
        self.env = {name: self.credential, "OWO_CUSTOM_MODEL_BINDING_" + reference: json.dumps({
            "model": self.name, "base_url": endpoint, "api_format": "openai",
        })}

    def check(self, daemon, data, thread, port, cookie, call_offset, role):
        expected_readers = {
            "lead": {"read_file", "list_dir", "search_files"},
            "worker": {"read_file", "list_dir", "search_files", "grep", "find_symbol"},
            "reviewer": {"read_file", "list_dir", "search_files", "grep", "git_status", "git_diff", "git_log", "find_symbol"},
        }[role]
        def terminal():
            for path in (data / "owobot").glob(f"*/events/{thread}.ndjson"):
                for line in path.read_text(encoding="utf-8").splitlines():
                    event = json.loads(line)
                    if event.get("type") == "turn.completed":
                        return event
            return None

        completed = until(daemon, "OwOBot's selected-model turn did not complete", terminal)
        assert completed.get("ok") is True, (completed, self.calls)
        calls = self.calls[call_offset:]
        assert calls and all(call["model"] == self.name and call["credential_bound"] for call in calls), calls
        assert all(set(call["tools"]) == expected_readers for call in calls), calls
        answer = until(daemon, "Selected-model text did not reach the conversation", lambda:
                       next((group for group in expect(port, "/owobot/api/bots?messages=20", 200, headers=cookie)["groups"]
                             if group["threadId"] == thread and any(self.text in (msg.get("text") or "") for msg in group["messages"])), None))
        assert self.text in json.dumps(answer)
        for path in (data / "owobot").glob("*/config.json"):
            saved = path.read_text(encoding="utf-8")
            assert self.credential not in saved and "OWO_CUSTOM_MODEL_SECRET" not in saved
        print(f"PASS the {role} role used the selected model and only its project readers", flush=True)


@contextmanager
def selected_provider():
    class Handler(BaseHTTPRequestHandler):
        def do_POST(self):
            body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
            bound = self.headers.get("Authorization") == f"Bearer {provider.credential}"
            provider.calls.append({
                "model": body["model"], "credential_bound": bound,
                "tools": [tool["function"]["name"] for tool in body.get("tools", [])],
            })
            if self.path != "/v1/chat/completions" or not bound or body["model"] != provider.name:
                self.send_error(400)
                return
            chunks = [
                {"index": 0, "delta": {"role": "assistant", "content": provider.text}, "finish_reason": None},
                {"index": 0, "delta": {}, "finish_reason": "stop"},
            ]
            frames = ["data: " + json.dumps({
                "id": "owo-model-fixture", "model": provider.name, "object": "chat.completion.chunk",
                "choices": [chunk],
            }) for chunk in chunks]
            wire = ("\n\n".join(frames + ["data: [DONE]", ""])).encode("utf-8") + b"\n"
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.send_header("Content-Length", str(len(wire)))
            self.end_headers()
            self.wfile.write(wire)

        def log_message(self, *_):
            pass

    with ThreadingHTTPServer(("127.0.0.1", 0), Handler) as server:
        provider = SelectedModel(server.server_port)
        serving = Thread(target=server.serve_forever, daemon=True)
        serving.start()
        try:
            yield provider
        finally:
            server.shutdown()
            serving.join(timeout=3)


def check_wrong_worker(port, auth, session, provider):
    count = len(provider.calls)
    expect(port, f"/session/{session}/turn", 403, headers={
        **auth, "x-owo-owobot-worker": "0" * 64, "x-owo-owobot-turn": "0" * 48,
    }, body={"prompt": "This must not reach the model.", "read_only": True})
    assert len(provider.calls) == count
    print("PASS forged ACP worker capability was rejected before reaching the model", flush=True)


def run(agent, home, root, desktop, electron, provider):
    workspace = root / "Project with spaces"
    workspace.mkdir()
    home_dir = root / "home"
    home_dir.mkdir()
    data = root / "owo"
    env = dict(isolated_environment(home_dir), OWO_AGENT_DATA=str(data),
               OWO_CLOUD_ENABLED="false", OWOBOT_HOME=str(home), OWO_WEB_UI_DIR=str(desktop))
    if provider:
        env.update(provider.env)
    with process([agent, "serve", "--port", "0", "--workspace", workspace],
                 cwd=workspace, env=env, log=root / "daemon.log") as daemon:
        port = daemon_port(daemon, data)
        origin = f"http://127.0.0.1:{port}"
        token = (data / "auth" / "token").read_text(encoding="utf-8").strip()
        auth = {"Authorization": "Bearer " + token}
        expect(port, "/owobot/open", 401, body={})
        expect(port, "/owobot/api/bots", 401, headers=auth)
        selection = provider.selection if provider else None
        group_id, cookie = browser_session(origin, auth, selection)
        assert browser_session(origin, auth, selection)[0] == group_id
        sidecar = running_sidecar_ports(daemon, data)
        expect(sidecar, "/api/bots", 403)
        expect(port, "/owobot/api/auth/session", 403, headers=cookie)
        expect(port, "/owobot/api/groups/other-project", 403, headers=cookie)
        expect(port, f"/owobot/api/groups/{group_id}/messages", 403,
               body={}, headers={**cookie, "Origin": "http://example.invalid"})
        with HTTP.open(origin + "/owobot/", timeout=5) as response:
            assert "/owobot/assets/" in response.read().decode("utf-8")
        event_hello(origin, cookie)
        print("PASS same-origin scoped session, bundled UI, and EventSource through the OwO Daemon", flush=True)

        group = expect(port, "/owobot/api/bots", 200, headers=cookie)["groups"][0]
        assert group["id"] == group_id and len(group["memberIds"]) == 3
        if electron:
            result = subprocess.run([
                electron, Path(__file__).with_name("workbench_browser.cjs"),
                root, origin,
            ], stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, encoding="utf-8",
               errors="replace", timeout=80, creationflags=subprocess.CREATE_NO_WINDOW)
            (root / "browser.log").write_text(result.stdout, encoding="utf-8")
            if result.stdout:
                print(result.stdout.strip(), flush=True)
            result.check_returncode()
        else:
            expect(port, f"/owobot/api/groups/{group_id}/messages", (200, 202),
                   headers={**cookie, "Origin": origin}, body={
                       "text": "Read the project directory.",
                       "threadId": group["threadId"], "sendId": str(uuid4()),
                   })
        sessions = until(daemon, "The manager did not dispatch to OwO ACP", lambda:
                         expect(port, "/sessions", 200, headers=auth))
        assert len(sessions) == 1 and Path(sessions[0]["workspace"]).samefile(workspace)
        print("PASS user message reached OwO ACP in its project directory", flush=True)
        if provider:
            provider.check(daemon, data, group["threadId"], port, cookie, 0, "lead")
            check_wrong_worker(port, auth, sessions[0]["id"], provider)
            outbound = {**cookie, "Origin": origin}
            thread_route = f"/owobot/api/groups/{group_id}"
            for index, (role, mention) in enumerate((("worker", "\u6267\u884c\u5458"), ("reviewer", "\u8bc4\u5ba1\u5458")), start=1):
                task = expect(port, thread_route + "/tasks", (200, 201), headers=outbound, body={})["group"]
                offset = len(provider.calls)
                expect(port, thread_route + "/messages", (200, 202), headers=outbound, body={
                    "text": f"@{mention} Inspect the project material.",
                    "mentions": [group["memberIds"][index]], "threadId": task["threadId"], "sendId": str(uuid4()),
                })
                provider.check(daemon, data, task["threadId"], port, cookie, offset, role)

        expect(port, "/server/shutdown", (200, 202), headers=auth, body={"confirm": True})
        assert daemon.wait(timeout=18) == 0
        with socket.socket() as sock:
            sock.settimeout(2)
            assert sock.connect_ex(("127.0.0.1", sidecar)) != 0
        print("PASS OwO shutdown closed its managed collaboration listener", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--agent", type=Path, required=True)
    runtime = parser.add_mutually_exclusive_group(required=True)
    runtime.add_argument("--node", type=Path, help="Stage built frontend and server with this Node executable")
    runtime.add_argument("--runtime", type=Path, help="Use a staged OwOBot runtime directly")
    scenario = parser.add_mutually_exclusive_group()
    scenario.add_argument("--electron", type=Path, help="Exercise the model-disabled desktop workbench")
    scenario.add_argument("--verify-model", action="store_true", help="Verify the selected model with a loopback fixture")
    parser.add_argument("--desktop-web", type=Path, default=UPSTREAM.parents[1] / "desktop" / "web")
    args = parser.parse_args()
    if os.name != "nt":
        parser.error("this test requires Windows")
    root = Path(tempfile.mkdtemp(prefix="owobot-desktop-")).resolve()
    print(f"Isolated smoke data: {root}", flush=True)
    home = args.runtime.resolve(strict=True) if args.runtime else root / "runtime"
    if args.node:
        stage_fixture(args.node.resolve(strict=True), home)
    with (selected_provider() if args.verify_model else nullcontext()) as provider:
        run(args.agent.resolve(strict=True), home, root, args.desktop_web.resolve(strict=True),
            args.electron.resolve(strict=True) if args.electron else None, provider)


if __name__ == "__main__":
    main()
