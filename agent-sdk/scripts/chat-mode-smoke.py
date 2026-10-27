"""纯对话模式 + 自定义提示词的离线端到端冒烟。

真实 daemon + 本地 mock 模型端点，验证：
1. `POST /session {mode:"chat"}` 建出的会话模式落库，`GET /session/{id}` 读回。
2. mock 模型在纯对话模式下强行请求 `write_file` 时，执行层拒绝并给出
   「纯对话模式不开放工具」的明确文案（不是静默忽略、不是 200 假装成功）。
3. `/prompts` 提示词库 CRUD 与 `/session/{id}/system-prompt` 应用/清除。

用法（仓库根或 agent-sdk 下均可）：
    python agent-sdk/scripts/chat-mode-smoke.py

退出码：0 全部通过；1 有断言失败（打印失败原因）。
需要先 `cargo build -p owo-agent-cli`（debug 产物）。
"""

import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
EXE = REPO / "target" / "debug" / "owo-agent.exe"
MOCK = REPO / "scripts" / "mock-openai-tool-provider.py"


def free_port() -> int:
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def http(method: str, url: str, body=None, token: str = "", timeout: float = 30):
    data = None
    headers = {"Content-Type": "application/json"}
    if body is not None:
        data = json.dumps(body, ensure_ascii=False).encode("utf-8")
    if token:
        headers["Authorization"] = f"Bearer {token}"
    request = urllib.request.Request(url, data=data, headers=headers, method=method)
    with urllib.request.urlopen(request, timeout=timeout) as response:
        raw = response.read()
        return response.status, json.loads(raw.decode("utf-8")) if raw else None


def http_text(method: str, url: str, body=None, token: str = "", timeout: float = 30) -> str:
    data = json.dumps(body, ensure_ascii=False).encode("utf-8") if body is not None else None
    headers = {"Content-Type": "application/json"}
    if token:
        headers["Authorization"] = f"Bearer {token}"
    request = urllib.request.Request(url, data=data, headers=headers, method=method)
    with urllib.request.urlopen(request, timeout=timeout) as response:
        return response.read().decode("utf-8", errors="replace")


def wait_health(base: str, deadline: float) -> bool:
    while time.time() < deadline:
        try:
            status, _ = http("GET", f"{base}/health", timeout=2)
            if status == 200:
                return True
        except (urllib.error.URLError, OSError):
            pass
        time.sleep(0.2)
    return False


def kill_daemon(data_dir: Path) -> None:
    descriptor = data_dir / "runtime" / "daemon.json"
    if not descriptor.exists():
        return
    try:
        pid = int(json.loads(descriptor.read_text(encoding="utf-8")).get("pid", 0))
    except (ValueError, OSError, json.JSONDecodeError):
        return
    if pid <= 0:
        return
    try:
        os.kill(pid, 9)
    except OSError:
        pass


def main() -> int:
    if not EXE.exists():
        print(f"缺少可执行文件：{EXE}\n请先 cargo build -p owo-agent-cli")
        return 1

    workdir = Path(tempfile.mkdtemp(prefix="owo-chat-smoke-"))
    ws = workdir / "ws"
    data = workdir / "data"
    for path in (ws, data):
        path.mkdir(parents=True, exist_ok=True)
    # 真实 UserPromptSubmit hook：Agent 回合必须留下标记，纯对话回合必须跳过。
    hook_marker = ws / "hook.marker"
    hook_command = (
        f'echo chat-hook-ran> "{hook_marker}"'
        if os.name == "nt"
        else f"printf chat-hook-ran > '{hook_marker}'"
    )
    (ws / "settings.json").write_text(
        json.dumps(
            {"hooks": [{"event": "UserPromptSubmit", "command": hook_command}]},
            ensure_ascii=False,
            indent=2,
        ),
        encoding="utf-8",
    )

    port = free_port()
    api_port = free_port()
    base = f"http://127.0.0.1:{api_port}"
    failures: list[str] = []
    blob = ""
    mock_log = open(workdir / "mock.log", "w", encoding="utf-8")
    serve_log = open(workdir / "serve.log", "w", encoding="utf-8")
    mock = subprocess.Popen(
        [sys.executable, str(MOCK), "--port", str(port), "--tool", "write_file"],
        stdout=mock_log,
        stderr=subprocess.STDOUT,
    )
    env = dict(os.environ)
    env.update(
        {
            "OPENAI_API_KEY": "mock-key",
            "OPENAI_BASE_URL": f"http://127.0.0.1:{port}/v1",
            "OPENAI_MODEL": "mock-model",
            "OWO_AGENT_DATA": str(data),
            "OWO_CLOUD_ENABLED": "false",
            "OWO_MODEL_STREAM_IDLE_TIMEOUT_SECS": "30",
        }
    )
    serve = subprocess.Popen(
        [str(EXE), "serve", "--port", str(api_port), "--workspace", str(ws)],
        env=env,
        stdout=serve_log,
        stderr=subprocess.STDOUT,
    )
    try:
        if not wait_health(base, time.time() + 60):
            print(f"[FAIL] daemon 未就绪；日志 {serve_log}")
            return 1
        _, token_body = http("GET", f"{base}/auth/token")
        token = token_body["token"]

        # 0) 工作区有 hooks：显式信任后必须装载，Agent 回合会留下 Stop 标记。
        _, trust = http(
            "POST", f"{base}/workspace/trust", {"trusted": True}, token
        )
        if trust.get("trusted") is not True:
            failures.append(f"工作区信任失败：{trust}")
        if not any("settings.hooks" in str(reason) for reason in trust.get("reasons") or []):
            failures.append(f"信任状态没有识别到 hooks：{trust.get('reasons')!r}")

        # 0b) Agent 回合作为对照组：hook 必须在模型调用前执行并留下标记。
        #     只读回合会让 mock 的 write_file 在网关层快速失败，避免等待审批。
        try:
            hook_marker.unlink()
        except FileNotFoundError:
            pass
        _, agent_session = http(
            "POST", f"{base}/session", {"workspace": str(ws), "mode": "agent"}, token
        )
        agent_session_id = agent_session["id"]
        http_text(
            "POST",
            f"{base}/session/{agent_session_id}/turn",
            {"prompt": "hook 探针", "read_only": True},
            token,
        )
        deadline = time.time() + 90
        while time.time() < deadline:
            _, agent_detail = http("GET", f"{base}/session/{agent_session_id}", token=token)
            if not agent_detail.get("active_turn_id"):
                break
            time.sleep(0.5)
        else:
            failures.append("Agent 对照组回合在 90s 内未结束")
        if not hook_marker.exists():
            failures.append("Agent 回合未执行 UserPromptSubmit hook（无法证明 hook 已装载）")
        try:
            hook_marker.unlink()
        except FileNotFoundError:
            pass
        # 对照组已消耗 mock 的首轮工具调用；重置，让纯对话回合重新拿到同一条工具请求。
        http("POST", f"http://127.0.0.1:{port}/reset", {})

        # 1) 纯对话会话创建 + 读回
        _, session = http(
            "POST", f"{base}/session", {"workspace": str(ws), "mode": "chat"}, token
        )
        session_id = session["id"]
        if session.get("mode") != "chat":
            failures.append(f"创建响应缺少 chat 模式：{session.get('mode')!r}")
        _, detail = http("GET", f"{base}/session/{session_id}", token=token)
        if detail.get("mode") != "chat":
            failures.append(f"会话详情模式错误：{detail.get('mode')!r}")

        # 1b) 纯对话可以不选工作区：服务端分配隔离在数据目录下的会话目录。
        _, unattached = http(
            "POST", f"{base}/session", {"workspace": "", "mode": "chat"}, token
        )
        isolated = str(unattached.get("workspace") or "")
        if unattached.get("mode") != "chat":
            failures.append(f"无工作区纯对话创建失败：{unattached}")
        if not isolated or not Path(isolated).is_dir():
            failures.append(f"无工作区纯对话没有返回可用目录：{isolated!r}")
        elif Path(isolated).resolve() != (data / "chat").resolve():
            failures.append(f"无工作区纯对话目录未隔离在数据根下：{isolated!r}")

        # Agent 模式仍然必须有工作区：不能把任务静默落到默认目录。
        try:
            http("POST", f"{base}/session", {"workspace": "", "mode": "agent"}, token)
            failures.append("Agent 模式空工作区本应 400，却创建成功")
        except urllib.error.HTTPError as error:
            if error.code != 400:
                failures.append(f"Agent 模式空工作区返回 {error.code}，期望 400")

        # 2) 提示词库 CRUD + 应用
        _, saved = http(
            "POST",
            f"{base}/prompts",
            {"name": "冒烟提示词", "content": "先给结论，再列证据。"},
            token,
        )
        preset_id = saved["prompt"]["id"]
        _, applied = http(
            "POST",
            f"{base}/session/{session_id}/system-prompt",
            {"prompt": "先给结论，再列证据。"},
            token,
        )
        if applied.get("system_prompt") != "先给结论，再列证据。":
            failures.append("应用提示词后 system_prompt 不符")
        _, detail = http("GET", f"{base}/session/{session_id}", token=token)
        if detail.get("system_prompt") != "先给结论，再列证据。":
            failures.append("会话详情没有读回 system_prompt")

        # 3) 纯对话模式下模型硬要 write_file：执行层必须拒绝并说明原因
        #    该接口返回 text/event-stream，不能按 JSON 解析；结果以会话历史为准。
        http_text(
            "POST", f"{base}/session/{session_id}/turn", {"prompt": "写一个文件"}, token
        )
        deadline = time.time() + 90
        blob = ""
        while time.time() < deadline:
            try:
                _, detail = http("GET", f"{base}/session/{session_id}", token=token)
            except urllib.error.HTTPError as error:
                failures.append(f"读取会话详情失败：{error}")
                break
            blob = json.dumps(detail, ensure_ascii=False)
            try:
                _, events = http(
                    "GET", f"{base}/session/{session_id}/turn/events?after=0", token=token
                )
                blob += json.dumps(events, ensure_ascii=False)
            except urllib.error.HTTPError:
                pass
            if "本轮只提供" in blob:
                break
            time.sleep(0.5)
        if "write_file" not in blob:
            failures.append("回合里没有出现被拒绝的 write_file 记录")
        # 网关在工具 schema 校验阶段就拒绝：错误必须说明"本轮只提供什么"，
        # 否则用户分不清是模式限制还是模型故障。
        if "本轮只提供：web_fetch、web_search、ask_user" not in blob:
            failures.append("拒绝文案没有列出本轮可用工具（纯对话边界不可见）")
        if "未执行任何操作" not in blob:
            failures.append("拒绝文案没有说明工具未执行")

        # 3b) 编辑/重新生成所依赖的 rewind：等回合真正结束后，
        #     保留目标用户消息之前的上下文，返回 removed 且模式不变。
        deadline = time.time() + 90
        detail = {}
        while time.time() < deadline:
            _, detail = http("GET", f"{base}/session/{session_id}", token=token)
            if not detail.get("active_turn_id"):
                break
            time.sleep(0.5)
        else:
            failures.append("回合在 90s 内未结束，无法验证 rewind")
        if hook_marker.exists():
            failures.append("纯对话回合执行了工作区 hook（应跳过本机命令）")
        messages = detail.get("messages") or []
        last_user = max(
            (index for index, message in enumerate(messages) if message.get("role") == "user"),
            default=-1,
        )
        if last_user < 0:
            failures.append("会话历史里找不到用户消息，无法定位 rewind 边界")
        else:
            _, rewound = http(
                "POST",
                f"{base}/session/{session_id}/rewind",
                {"keep": last_user},
                token,
            )
            if rewound.get("ok") is not True:
                failures.append(f"rewind 未返回 ok：{rewound}")
            elif int(rewound.get("removed") or 0) <= 0:
                failures.append(f"rewind 没有移除任何消息：{rewound}")
            _, after_rewind = http("GET", f"{base}/session/{session_id}", token=token)
            kept = after_rewind.get("messages") or []
            if len(kept) != last_user:
                failures.append(
                    f"rewind 后消息数 {len(kept)} != keep {last_user}（编辑边界不可靠）"
                )
            if after_rewind.get("mode") != "chat":
                failures.append(f"rewind 改写了会话模式：{after_rewind.get('mode')!r}")

        # 4) 切回 Agent 模式可读回
        _, flipped = http(
            "POST", f"{base}/session/{session_id}/mode", {"mode": "agent"}, token
        )
        if flipped.get("mode") != "agent":
            failures.append(f"切换 Agent 模式失败：{flipped}")

        # 5) 删除提示词
        status, _ = http("DELETE", f"{base}/prompts/{preset_id}", token=token)
        if status != 200:
            failures.append(f"删除提示词返回 {status}")

        print(f"[{'PASS' if not failures else 'FAIL'}] 纯对话模式 / 提示词（{workdir}）")
    except (urllib.error.URLError, OSError, KeyError, json.JSONDecodeError) as error:
        failures.append(f"HTTP 异常：{error!r}")
        print(f"[FAIL] 纯对话模式 / 提示词：{error!r}")
    finally:
        try:
            serve.terminate()
            serve.wait(timeout=10)
        except (subprocess.TimeoutExpired, OSError):
            serve.kill()
        kill_daemon(data)
        mock.terminate()
        mock_log.close()
        serve_log.close()

    for failure in failures:
        print(f"  - {failure}")
    if failures and blob:
        # 诊断：只截断打印，不带凭据（会话历史里没有 token）。
        print("会话/事件片段：")
        print(blob[:3000])
    if failures:
        print(f"日志目录：{workdir}（serve.log / mock.log）")
        return 1
    shutil.rmtree(workdir, ignore_errors=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
