"""Exercise the real Windows OwOBot services in isolated directories.

The OwO Daemon runs with its model disabled. This covers the group-to-ACP
boundary and the unavailable-model terminal state. Live model acceptance uses
a separately configured test workspace.
"""

import argparse
from contextlib import contextmanager
import json
import os
from pathlib import Path
import secrets
import socket
import subprocess
import tempfile
import time
from urllib.error import HTTPError
from urllib.request import ProxyHandler, Request, build_opener
from uuid import uuid4

UPSTREAM = Path(__file__).resolve().parents[1] / "upstream"
HTTP = build_opener(ProxyHandler({}))


@contextmanager
def process(command, *, cwd, env, log):
    with log.open("wb") as stream:
        child = subprocess.Popen(
            command, cwd=cwd, env=env, stdin=subprocess.PIPE,
            stdout=stream, stderr=stream, creationflags=subprocess.CREATE_NO_WINDOW,
        )
        try:
            yield child
        finally:
            if child.poll() is None:
                child.terminate()
            child.wait(timeout=10)


def request(port, path, *, method=None, body=None, headers=None):
    headers = dict(headers or {})
    payload = None
    if body is not None:
        headers["Content-Type"] = "application/json"
        payload = json.dumps(body).encode("utf-8")
    req = Request(
        f"http://127.0.0.1:{port}{path}", data=payload, headers=headers, method=method,
    )
    try:
        with HTTP.open(req, timeout=5) as response:
            return response.status, json.load(response)
    except HTTPError as error:
        return error.code, {}


def expect(port, path, status, **kwargs):
    actual, result = request(port, path, **kwargs)
    statuses = status if isinstance(status, tuple) else (status,)
    assert actual in statuses, f"{path} returned {actual}, expected {statuses}"
    return result


def until(child, description, probe):
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline and child.poll() is None:
        try:
            value = probe()
            if value:
                return value
        except (OSError, ValueError):
            pass
        time.sleep(0.25)
    raise RuntimeError(description)


def fixture_ports():
    with socket.socket() as http, socket.socket() as webhook:
        http.bind(("127.0.0.1", 0))
        webhook.bind(("127.0.0.1", 0))
        return http.getsockname()[1], webhook.getsockname()[1]


def isolated_environment(home):
    allowed = {
        "systemroot", "windir", "path", "pathext", "comspec", "temp", "tmp",
        "programfiles", "programfiles(x86)", "programdata", "number_of_processors", "os",
    }
    environment = {name: value for name, value in os.environ.items() if name.lower() in allowed}
    environment.update(HOME=str(home), USERPROFILE=str(home), LOCALAPPDATA=str(home), APPDATA=str(home))
    return environment


def verify_group(port, auth, workspace, owo_port, owo_auth, data, sidecar, daemon):
    selection = {"instanceId": "owo", "model": "agent-default"}
    members = []
    for name in ("Coordinator", "Reviewer"):
        created = expect(port, "/api/bots", 201, headers=auth, body={
            "name": name, "soul": "Read-only local fixture.", "cwd": str(workspace),
            "computer": "none", "approvalMode": "ask", "useDefaults": False,
            "modelSelection": selection,
        })
        members.append(created["bot"]["id"])
    group = expect(port, "/api/groups", 201, headers=auth, body={
        "name": "OwOBot fixture", "memberIds": members,
        "setup": {"bulletin": "", "defaultResponder": {"kind": "member", "botId": members[0]}},
    })["group"]
    route = f"/api/groups/{group['id']}"
    expect(port, route, 200, method="PATCH", headers=auth, body={"cwd": str(workspace)})
    expect(port, f"{route}/messages", (200, 202), headers=auth, body={
        "text": "Read the project directory.",
        "threadId": group["threadId"],
        "sendId": str(uuid4()),
    })
    sessions = until(
        daemon, "MausBot did not create an OwO ACP session",
        lambda: expect(owo_port, "/sessions", 200, headers=owo_auth),
    )
    assert len(sessions) == 1
    assert Path(sessions[0]["workspace"]).samefile(workspace)

    # Use the persisted canonical event instead of a timeout or a UI status.
    events = data / "events" / f"{group['threadId']}.ndjson"

    def completed_event():
        for line in events.read_text(encoding="utf-8").splitlines():
            event = json.loads(line)
            if event["type"] == "turn.completed":
                return event
        return None

    event = until(sidecar, "OwOBot room turn did not reach a terminal state", completed_event)
    assert event["provider"] == "customAcp" and event["ok"] is False
    print("PASS group message opened an OwO session in the pinned workspace and settled", flush=True)


def run(node, agent, root, bundled):
    port, webhook_port = fixture_ports()
    home = root / "home"
    data = root / "collaboration"
    workspace = root / "project"
    owo_data = root / "owo"
    for folder in (home, data, workspace):
        folder.mkdir()
    env = isolated_environment(home)
    cli = subprocess.list2cmdline([
        str(agent), "acp", "--workspace", str(workspace), "--data-dir", str(owo_data),
    ])
    (data / "config.json").write_text(json.dumps({
        "instances": {"owo": {
            "driver": "customAcp", "displayName": "OwO Agent", "config": {"cli": cli},
        }},
        "defaultModelSelection": {"instanceId": "owo", "model": "agent-default"},
    }), encoding="utf-8")
    sidecar_env = dict(env, OWOBOT_MANAGED="1", OMB_DATA_DIR=str(data), OMB_PORT=str(port),
                       OMB_WEBHOOK_PORT=str(webhook_port), OMB_LOOPBACK_TRUST="service",
                       OMB_CLI_OWNER_STDIN="1", OMB_DEV_TRUST_LOCAL_BROWSER="0",
                       OMB_DISABLE_ANNOUNCEMENTS="1")
    daemon_env = dict(env, OWO_AGENT_DATA=str(owo_data), OWO_CLOUD_ENABLED="false")

    with process([agent, "serve", "--port", "0", "--workspace", workspace],
                 cwd=workspace, env=daemon_env, log=root / "daemon.log") as daemon:
        descriptor = owo_data / "runtime" / "daemon.json"
        owo_port = until(daemon, "OwO Daemon did not start", lambda: json.loads(
            descriptor.read_text(encoding="utf-8"),
        )["port"])
        token = (owo_data / "auth" / "token").read_text(encoding="utf-8").strip()
        owo_auth = {"Authorization": "Bearer " + token}
        assert expect(owo_port, "/health", 200)["pid"] == daemon.pid
        server_args = ["dist-server/index.js"] if bundled else ["--experimental-strip-types", "server/index.ts"]
        with process([node, *server_args], cwd=UPSTREAM,
                     env=sidecar_env, log=root / "server.log") as sidecar:
            launch_secret = secrets.token_urlsafe(32)
            sidecar.stdin.write((launch_secret + "\n").encode("ascii"))
            sidecar.stdin.flush()
            until(sidecar, "OwOBot sidecar did not start", lambda: request(port, "/api/health")[0] == 200)
            assert expect(port, "/api/edition", 200)["edition"] == "oss"
            expect(port, "/api/bots", 403)
            expect(port, "/api/threads/fake/messages", 403)
            expect(port, "/api/bots/fake/messages/guarded", 403, body={})
            expect(port, "/api/auth/pairing", 403, body={})
            pairing = expect(port, "/api/auth/pairing", 200,
                             headers={"x-openmausbot-cli-owner": launch_secret},
                             body={"scopes": ["admin", "client"]})
            session = expect(port, "/api/auth/pair", 200,
                             body={"code": pairing["code"], "label": "OwOBot Windows smoke"})
            auth = {"Authorization": "Bearer " + session["token"]}
            assert isinstance(expect(port, "/api/bots", 200, headers=auth)["bots"], list)
            print("PASS OSS sidecar authenticated session and four unauthenticated requests denied", flush=True)
            verify_group(port, auth, workspace, owo_port, owo_auth, data, sidecar, daemon)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--node", type=Path, required=True)
    parser.add_argument("--agent", type=Path, required=True)
    parser.add_argument("--bundled", action="store_true")
    args = parser.parse_args()
    if os.name != "nt":
        parser.error("this smoke test requires Windows")
    root = Path(tempfile.mkdtemp(prefix="owobot-smoke-")).resolve()
    print(f"Isolated smoke data: {root}", flush=True)
    run(args.node.resolve(strict=True), args.agent.resolve(strict=True), root, args.bundled)


if __name__ == "__main__":
    main()
