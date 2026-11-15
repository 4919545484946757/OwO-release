"""Run the managed OwOBot gateway and optional Chromium workbench on Windows."""

import argparse
from collections import Counter
import csv
import importlib.util
import io
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
from threading import Event, Thread
from urllib.request import Request
from uuid import uuid4

from windows_smoke import HTTP, UPSTREAM, expect, isolated_environment, process, until


demo_spec = importlib.util.spec_from_file_location(
    "verify_signup", UPSTREAM.parent / "demo" / "verify_signup.py",
)
demo = importlib.util.module_from_spec(demo_spec)
demo_spec.loader.exec_module(demo)


def signup_outputs(text):
    incoming = csv.DictReader(io.StringIO(text.lstrip("\ufeff")))
    assert incoming.fieldnames == demo.COLUMNS
    rows = list(incoming)
    name, group, phone = demo.COLUMNS
    csv_out = io.StringIO(newline="")
    writer = csv.DictWriter(csv_out, fieldnames=demo.COLUMNS, lineterminator="\n")
    writer.writeheader()
    for row in rows:
        writer.writerow({name: row[name], group: row[group], phone: "*" * 7 + row[phone][-4:]})
    counts = sorted(Counter(row[group] for row in rows).items())
    total = "\u603b\u8ba1"
    report = "| " + group + " | " + "\u4eba\u6570" + " |\n| --- | --- |\n"
    report += "".join(f"| {label} | {number} |\n" for label, number in counts + [(total, len(rows))])
    return [{"path": demo.DETAILS, "text": csv_out.getvalue()}, {"path": demo.SUMMARY, "text": report}]


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


def event_hello(origin, auth, project=None, cursor=None):
    query = "?screens=off" + (f"&owoProject={project}" if project else "")
    headers = {**auth, "Last-Event-ID": cursor} if cursor else auth
    request = Request(origin + "/owobot/api/events" + query, headers=headers)
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
    assert ports, "no loopback collaboration listener was found"
    return ports


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
    text = "已核对当前项目材料。"
    credential = "owobot-loopback-fixture-key"
    scenario = "\u6574\u7406\u6d3b\u52a8\u62a5\u540d\u8868\uff0c\u751f\u6210\u5206\u7ec4\u7edf\u8ba1\u548c\u8131\u654f\u660e\u7ec6\u3002"
    assignment = "读取 registration.csv，生成并暂存报名统计和脱敏明细。"
    child_result = "已暂存报名统计和脱敏明细，等待评审。"
    review_assignment = "请独立核对暂存版的报名人数、分组和手机号脱敏。"
    review_result = "暂存版评审通过，统计和脱敏明细均已核对。"
    stop_scenario = "Test cancellation of the delegated work."
    stop_assignment = "Wait on the cancellation-test provider."
    stopped_reply = "UNEXPECTED_LATE_WORKER_REPLY"
    summary = "报名交付已完成评审，请查看预览并批准应用。"

    def __init__(self, port):
        self.calls = []
        self.issued_assignment = False
        self.issued_review = False
        self.issued_stop_assignment = False
        self.worker_waiting = Event()
        self.release_worker = Event()
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
            "lead": {"read_file", "list_dir", "search_files", "owobot_delegate", "owobot_read_delivery"},
            "worker": {"read_file", "list_dir", "search_files", "grep", "find_symbol", "owobot_read_delivery", "owobot_stage_delivery"},
            "reviewer": {"read_file", "list_dir", "search_files", "grep", "owobot_read_delivery", "owobot_review_delivery"},
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
        assert self.text in json.dumps(answer, ensure_ascii=False)
        for path in (data / "owobot").glob("*/config.json"):
            saved = path.read_text(encoding="utf-8")
            assert self.credential not in saved and "OWO_CUSTOM_MODEL_SECRET" not in saved
        print(f"PASS the {role} role used the selected model and only its permitted tools", flush=True)


    def response(self, messages, tools):
        text = json.dumps(messages, ensure_ascii=False)
        if "owobot_delegate" in tools and self.stop_scenario in text and not self.issued_stop_assignment:
            self.issued_stop_assignment = True
            return self.tool_response("stop-worker", "owobot_delegate", {"recipient": "worker", "task": self.stop_assignment})
        if "owobot_stage_delivery" in tools and self.stop_assignment in text:
            self.worker_waiting.set()
            self.release_worker.wait(timeout=45)
            return self.text_response(self.stopped_reply)
        if "owobot_delegate" in tools and self.scenario in text and not self.issued_assignment:
            self.issued_assignment = True
            return self.tool_response("delegate", "owobot_delegate", {"recipient": "worker", "task": self.assignment})
        if "owobot_delegate" in tools and self.child_result in text and not self.issued_review:
            self.issued_review = True
            return self.tool_response("review", "owobot_delegate", {"recipient": "reviewer", "task": self.review_assignment})
        if "owobot_review_delivery" in tools and self.review_assignment in text:
            reviewed = self.tool_result(messages, "review-signup")
            if reviewed is not None:
                assert reviewed["saved"]
                return self.text_response(self.review_result)
            preview = self.tool_result(messages, "inspect-signup")
            if preview is None:
                return self.tool_response("inspect-signup", "owobot_read_delivery", {})
            with tempfile.TemporaryDirectory(prefix="owobot-review-") as directory:
                for file in preview["files"]:
                    assert file["path"] in (demo.DETAILS, demo.SUMMARY)
                    (Path(directory) / file["path"]).write_text(file["after"], encoding="utf-8")
                demo.verify(Path(directory))
            return self.tool_response("review-signup", "owobot_review_delivery", {
                "revision": preview["revision"], "approved": True, "note": "共 12 人，A 组 7 人，B 组 5 人。手机号仅保留后四位。",
            })
        if "owobot_delegate" in tools and self.review_result in text:
            return self.text_response(self.summary)
        if "owobot_stage_delivery" in tools and self.assignment in text:
            staged = self.tool_result(messages, "stage-signup")
            if staged is not None:
                assert staged["saved"]
                return self.text_response(self.child_result)
            source = self.tool_result(messages, "read-signup")
            if source is None:
                return self.tool_response("read-signup", "read_file", {"path": "registration.csv"})
            return self.tool_response("stage-signup", "owobot_stage_delivery", {
                "baseRevision": 0, "files": signup_outputs(source["content"]),
            })
        return self.text_response(self.text)

    @staticmethod
    def tool_result(messages, call_id):
        for message in reversed(messages):
            if message["role"] == "tool" and message.get("tool_call_id") == call_id:
                return json.loads(message["content"])
        return None

    @staticmethod
    def tool_response(call_id, name, args):
        return [
            {"index": 0, "delta": {"role": "assistant", "tool_calls": [{
                "index": 0, "id": call_id, "type": "function",
                "function": {"name": name, "arguments": json.dumps(args)},
            }]}, "finish_reason": None},
            {"index": 0, "delta": {}, "finish_reason": "tool_calls"},
        ]

    @staticmethod
    def text_response(text):
        return [
            {"index": 0, "delta": {"role": "assistant", "content": text}, "finish_reason": None},
            {"index": 0, "delta": {}, "finish_reason": "stop"},
        ]


@contextmanager
def selected_provider():
    class Handler(BaseHTTPRequestHandler):
        def do_POST(self):
            body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
            bound = self.headers.get("Authorization") == f"Bearer {provider.credential}"
            tools = [tool["function"]["name"] for tool in body.get("tools", [])]
            provider.calls.append({"model": body["model"], "credential_bound": bound, "tools": tools})
            if self.path != "/v1/chat/completions" or not bound or body["model"] != provider.name:
                self.send_error(400)
                return
            chunks = provider.response(body["messages"], tools)
            frames = ["data: " + json.dumps({
                "id": "owo-model-fixture", "model": provider.name, "object": "chat.completion.chunk",
                "choices": [chunk],
            }) for chunk in chunks]
            wire = ("\n\n".join(frames + ["data: [DONE]", ""])).encode("utf-8") + b"\n"
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.send_header("Content-Length", str(len(wire)))
            self.end_headers()
            try:
                self.wfile.write(wire)
            except ConnectionError:
                if not provider.release_worker.is_set():
                    raise

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


def run_browser(electron, root, origin, thread=None, launcher_workspace=None):
    args = [electron, Path(__file__).with_name("workbench_browser.cjs"), root, origin]
    browser_env = os.environ.copy()
    browser_env.pop("OWOBOT_SMOKE_DELIVERY_THREAD", None)
    browser_env.pop("OWOBOT_SMOKE_LAUNCHER_WORKSPACE", None)
    if thread:
        browser_env["OWOBOT_SMOKE_DELIVERY_THREAD"] = thread
    if launcher_workspace:
        browser_env["OWOBOT_SMOKE_LAUNCHER_WORKSPACE"] = str(launcher_workspace)
    result = subprocess.run(
        args, env=browser_env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, encoding="utf-8",
        errors="replace", timeout=80, creationflags=subprocess.CREATE_NO_WINDOW,
    )
    name = "launcher-browser.log" if launcher_workspace else "browser.log"
    (root / name).write_text(result.stdout, encoding="utf-8")
    if result.stdout:
        print(result.stdout.strip(), flush=True)
    result.check_returncode()


def verify_reopened_delivery(agent, workspace, env, root, data, selection, group, delivery_route, approved, decision):
    with process([agent, "serve", "--port", "0", "--workspace", workspace],
                 cwd=workspace, env=env, log=root / "reopened-daemon.log") as daemon:
        port = daemon_port(daemon, data)
        origin = f"http://127.0.0.1:{port}"
        token = (data / "auth" / "token").read_text(encoding="utf-8").strip()
        auth = {"Authorization": "Bearer " + token}
        group_id, cookie = browser_session(origin, auth, selection)
        assert group_id == group
        saved = expect(port, delivery_route, 200, headers=cookie)
        assert saved["taskId"] == approved["taskId"]
        assert saved["files"] == approved["files"] and saved["review"] == approved["review"]
        assert saved["status"] == "applied" and saved["decidedAt"]
        replay = expect(port, delivery_route + "/decision", 200, headers={**cookie, "Origin": origin}, body=decision)
        assert replay["replayed"] and replay["taskId"] == saved["taskId"]
        demo.verify(workspace)
        expect(port, "/server/shutdown", (200, 202), headers=auth, body={"confirm": True})
        assert daemon.wait(timeout=18) == 0
    print("PASS reopening OwO recovered the same task, previews, review and applied decision", flush=True)


def verify_worker_cancellation(daemon, port, origin, auth, cookie, route, provider):
    headers = {**cookie, "Origin": origin}
    task = expect(port, route + "/tasks", 201, headers=headers, body={})["task"]
    thread = task["threadId"]
    delivery_route = f"/owobot/api/delivery/{thread}"
    expect(port, route + "/messages", (200, 202), headers=headers, body={
        "text": provider.stop_scenario, "threadId": thread, "sendId": str(uuid4()),
    })
    try:
        assert provider.worker_waiting.wait(timeout=25), "delegated worker did not reach its model"
        record = expect(port, delivery_route, 200, headers=cookie)
        worker = next(turn for turn in record["turns"] if turn["role"] == "worker" and turn["assignmentId"])
        events_route = f"/session/{worker['sessionId']}/turn/events?turn_id={worker['turnId']}"
        assert expect(port, events_route, 200, headers=auth)["state"] == "active"
        expect(port, route + "/interrupt", 200, headers=headers, body={"threadId": thread})

        def worker_stopped():
            event_page = expect(port, events_route, 200, headers=auth)
            group = expect(port, "/owobot/api/bots?messages=20", 200, headers=cookie)["groups"][0]
            if event_page["active"] or group.get("working"):
                return None
            assert event_page["state"] in ("failed", "interrupted"), event_page
            assert all(provider.stopped_reply not in (row.get("text") or "") for row in group["messages"])
            return event_page

        until(daemon, "Interrupt did not stop the active delegated worker", worker_stopped)
    finally:
        provider.release_worker.set()
    print("PASS stopping the room cancelled the live delegated worker before its model replied", flush=True)


def verify_second_project(daemon, port, origin, auth, cookie, selection, group_id, original_route, root, electron):
    project = root / "Second project"
    project.mkdir()
    shutil.copyfile(demo.SOURCE, project / "registration.csv")
    group, _ = browser_session(origin, auth, {**selection, "workspace": str(project)})
    assert group != group_id
    first_cursor = event_hello(origin, cookie, group_id)["cursor"]
    second_cursor = event_hello(origin, cookie, group)["cursor"]
    assert first_cursor.split(":")[0] != second_cursor.split(":")[0]
    assert event_hello(origin, cookie, group, first_cursor)["resumed"] is False
    assert event_hello(origin, cookie, group, second_cursor)["resumed"] is True
    assert expect(port, f"/owobot/api/bots?owoProject={group}", 200, headers=cookie)["groups"][0]["id"] == group
    expect(port, f"/owobot/api/groups/{group_id}?owoProject={group}", 403, headers=cookie)
    expect(port, original_route + f"?owoProject={group}", 404, headers=cookie)
    assert expect(port, original_route + f"?owoProject={group_id}", 200, headers=cookie)["status"] == "applied"
    expect(port, "/owobot/open", 400, headers=auth, body={"workspace": str(project / "missing")})
    run_browser(electron, root, origin, launcher_workspace=project)
    thread = (root / "launcher-thread.txt").read_text(encoding="utf-8")
    assert len(thread) == 36
    selected = f"?owoProject={group}"
    headers = {**cookie, "Origin": origin}
    route = f"/owobot/api/groups/{group}"
    objective = "查看第二个项目的报名资料。"
    task = expect(port, route + "/tasks" + selected, 200, headers=headers,
                  body={"title": objective, "requestId": thread})["task"]
    assert task["threadId"] == thread
    expect(port, route + "/tasks" + selected, 409, headers=headers,
           body={"title": "Different objective", "requestId": thread})
    sent = expect(port, route + "/messages" + selected, (200, 202), headers=headers,
                  body={"text": objective, "threadId": thread, "sendId": thread})
    assert sent["threadId"] == thread
    transcript = expect(port, f"/owobot/api/threads/{thread}/messages" + selected, 200, headers=cookie)
    assert sum(row.get("text") == objective for row in transcript["messages"]) == 1
    expect(port, f"/owobot/api/threads/{thread}/messages?owoProject={group_id}", 404, headers=cookie)
    def has_second_workspace():
        sessions = expect(port, "/sessions", 200, headers=auth)
        return any(Path(row["workspace"]).samefile(project) for row in sessions)
    until(daemon, "The second project did not dispatch to OwO ACP in its directory", has_second_workspace)
    print("PASS two projects used isolated workspaces, live streams, threads, and delivery records", flush=True)


def run(agent, home, root, desktop, electron, provider, delivery_electron=None):
    workspace = root / "Project with spaces"
    workspace.mkdir()
    if provider:
        shutil.copyfile(demo.SOURCE, workspace / "registration.csv")
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
        sidecar, = running_sidecar_ports(daemon, data)
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
            run_browser(electron, root, origin)
        else:
            expect(port, f"/owobot/api/groups/{group_id}/messages", (200, 202),
                   headers={**cookie, "Origin": origin}, body={
                       "text": "查看当前项目目录。",
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
                    "text": f"@{mention} 查看当前项目材料。",
                    "mentions": [group["memberIds"][index]], "threadId": task["threadId"], "sendId": str(uuid4()),
                })
                provider.check(daemon, data, task["threadId"], port, cookie, offset, role)
            task = expect(port, thread_route + "/tasks", (200, 201), headers=outbound, body={})["group"]
            send = {"text": provider.scenario, "threadId": task["threadId"], "sendId": str(uuid4())}
            for _ in range(2):
                expect(port, thread_route + "/messages", (200, 202), headers=outbound, body=send)

            def completed_relay():
                snapshot = expect(port, "/owobot/api/bots?messages=40", 200, headers=cookie)
                messages = next((g["messages"] for g in snapshot["groups"] if g["threadId"] == task["threadId"]), [])
                for index, result in enumerate((provider.summary, provider.child_result, provider.review_result)):
                    if not any((message.get("from") or {}).get("botId") == group["memberIds"][index]
                               and result in (message.get("text") or "") for message in messages):
                        return False
                return sum(message.get("text") == provider.scenario for message in messages) == 1

            until(daemon, "OwOBot's worker, reviewer and lead did not finish their relay", completed_relay)
            receipts = expect(port, "/audit?event=tool_receipt&tool=owobot_delegate", 200, headers=auth)
            assert len(receipts) == 2 and all(json.loads(row["detail"])["ok"] is True for row in receipts), receipts
            assert provider.issued_assignment and provider.issued_review
            delivery_route = f"/owobot/api/delivery/{task['threadId']}"
            staged = expect(port, delivery_route, 200, headers=cookie)
            assert staged["status"] == "reviewed" and staged["revision"] == 1
            assert {turn["role"] for turn in staged["turns"]} == {"lead", "worker", "reviewer"}
            assert {turn["role"] for turn in staged["turns"] if turn["assignmentId"]} >= {"worker", "reviewer"}
            assert len(staged["files"]) == 2 and staged["review"]["actor"]["role"] == "reviewer"
            assert sorted(path.name for path in workspace.iterdir() if path.is_file()) == ["registration.csv"]
            expect(port, delivery_route + "/decision", 403, headers={**cookie, "Origin": "http://foreign.invalid"}, body={})
            decision = {"revision": 1, "action": "apply", "idempotencyKey": "desktop-apply-1"}
            if delivery_electron:
                run_browser(delivery_electron, root, origin, task["threadId"])
                approved = expect(port, delivery_route, 200, headers=cookie)
            else:
                approved = expect(port, delivery_route + "/decision", 200, headers=outbound, body=decision)
            assert approved["status"] == "applied" and not approved["replayed"]
            demo.verify(workspace)
            replay = expect(port, delivery_route + "/decision", 200, headers=outbound, body=decision)
            assert replay["replayed"]
            assert (workspace / "registration.csv").read_bytes() == demo.SOURCE.read_bytes()
            decisions = expect(port, "/audit?event=owobot_delivery_decision", 200, headers=auth)
            assert len(decisions) == 1 and decisions[0]["session_id"] == approved["taskId"], decisions
            print("PASS one delegated signup delivery passed review, human apply, 12-person and masking checks, and replay deduplication", flush=True)
            assert browser_session(origin, auth, selection)[0] == group_id
            verify_worker_cancellation(daemon, port, origin, auth, cookie, thread_route, provider)

        sidecars = [sidecar]
        if provider and delivery_electron:
            verify_second_project(daemon, port, origin, auth, cookie, selection, group_id,
                                  delivery_route, root, delivery_electron)
            sidecars = running_sidecar_ports(daemon, data)
            assert len(sidecars) == 2 and sidecar in sidecars
        expect(port, "/server/shutdown", (200, 202), headers=auth, body={"confirm": True})
        assert daemon.wait(timeout=18) == 0
        for listener in sidecars:
            with socket.socket() as sock:
                sock.settimeout(2)
                assert sock.connect_ex(("127.0.0.1", listener)) != 0
        print("PASS OwO shutdown closed its managed collaboration listeners", flush=True)
    if provider:
        verify_reopened_delivery(agent, workspace, env, root, data, selection, group_id, delivery_route, approved, decision)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--agent", type=Path, required=True)
    runtime = parser.add_mutually_exclusive_group(required=True)
    runtime.add_argument("--node", type=Path, help="Stage built frontend and server with this Node executable")
    runtime.add_argument("--runtime", type=Path, help="Use a staged OwOBot runtime directly")
    scenario = parser.add_mutually_exclusive_group()
    scenario.add_argument("--electron", type=Path, help="Exercise the model-disabled desktop workbench")
    scenario.add_argument("--verify-model", action="store_true", help="Verify the selected model with a loopback fixture")
    parser.add_argument("--delivery-electron", type=Path, help="Approve in the desktop UI after the model relay fixture")
    parser.add_argument("--desktop-web", type=Path, default=UPSTREAM.parents[1] / "desktop" / "web")
    args = parser.parse_args()
    if args.delivery_electron and not args.verify_model:
        parser.error("--delivery-electron needs --verify-model")
    if os.name != "nt":
        parser.error("this test requires Windows")
    root = Path(tempfile.mkdtemp(prefix="owobot-desktop-")).resolve()
    print(f"Isolated smoke data: {root}", flush=True)
    home = args.runtime.resolve(strict=True) if args.runtime else root / "runtime"
    if args.node:
        stage_fixture(args.node.resolve(strict=True), home)
    with (selected_provider() if args.verify_model else nullcontext()) as provider:
        run(args.agent.resolve(strict=True), home, root, args.desktop_web.resolve(strict=True),
            args.electron.resolve(strict=True) if args.electron else None, provider,
            args.delivery_electron.resolve(strict=True) if args.delivery_electron else None)


if __name__ == "__main__":
    main()
