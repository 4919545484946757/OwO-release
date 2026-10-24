"""媒体工具离线冒烟：本地 mock 端点 + 真 daemon 回合。

用途：在没有真实模型/云端密钥的环境里验证 `generate_image` / `text_to_speech`
的完整链路——审批 → HTTP 请求 → 产物落盘 → 事件记录（含内联 data_url）。

用法（仓库根或 agent-sdk 下均可）：
    python agent-sdk/scripts/media-tool-smoke.py            # 三个场景全跑
    python agent-sdk/scripts/media-tool-smoke.py --tool generate_image
    python agent-sdk/scripts/media-tool-smoke.py --tool generate_image --url

退出码：0 全部通过；1 有场景失败（会打印失败原因）。

注意：CLI 会拉起 daemon 子进程并继承句柄，因此这里把输出写日志文件、
用 wait(timeout) 判超时；用 PIPE + communicate 会永远等不到 EOF。
"""

import argparse
import json
import os
import shutil
import socket
import sqlite3
import subprocess
import sys
import tempfile
import time
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
EXE = REPO / "target" / "debug" / "owo-agent.exe"
MOCK = REPO / "scripts" / "mock-openai-tool-provider.py"

PROMPTS = {
    "generate_image": "生成一张图片并保存到工作区",
    "text_to_speech": "把这句话合成语音",
}
EXTENSIONS = {"generate_image": ".png", "text_to_speech": ".mp3"}
MAGIC = {"generate_image": "89504e47", "text_to_speech": "494433"}


def free_port() -> int:
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def _stop_daemon(data_dir: Path) -> None:
    """冒烟用的 daemon 是用独立 --data-dir 拉起的，跑完必须收掉。

    `owo-agent daemon stop` 带交互确认，脚本里直接按 daemon.json 的 pid 结束进程；
    失败不影响结论（仅打印），但会留下后台进程，所以这里尽量做成幂等。
    """
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
    except (ProcessLookupError, PermissionError, OSError):
        pass
    for _ in range(40):
        try:
            os.kill(pid, 0)
        except OSError:
            return
        time.sleep(0.05)


def run_case(tool: str, url_mode: bool, workdir: Path) -> list[str]:
    failures: list[str] = []
    port = free_port()
    ws = workdir / f"{tool}{'-url' if url_mode else ''}-ws"
    data = workdir / f"{tool}{'-url' if url_mode else ''}-data"
    for path in (ws, data):
        shutil.rmtree(path, ignore_errors=True)
        path.mkdir(parents=True, exist_ok=True)

    mock_args = [sys.executable, str(MOCK), "--port", str(port), "--tool", tool]
    if url_mode:
        mock_args.append("--image-url")
    mock_log = open(workdir / f"mock-{tool}.log", "w", encoding="utf-8")
    mock = subprocess.Popen(mock_args, stdout=mock_log, stderr=subprocess.STDOUT)
    try:
        env = dict(os.environ)
        env.update({
            "OPENAI_API_KEY": "mock-key",
            "OPENAI_BASE_URL": f"http://127.0.0.1:{port}/v1",
            "OPENAI_MODEL": "mock-model",
            "OWO_IMAGE_BASE_URL": f"http://127.0.0.1:{port}/v1",
            "OWO_IMAGE_API_KEY": "mock-key",
            "OWO_AGENT_DATA": str(data),
            "OWO_CLOUD_ENABLED": "false",
            "OWO_MODEL_STREAM_IDLE_TIMEOUT_SECS": "30",
        })
        cmd = [str(EXE), "turn", "--workspace", str(ws), "--prompt", PROMPTS[tool],
               "--no-approval", "--data-dir", str(data)]
        log_path = workdir / f"turn-{tool}.log"
        with open(log_path, "w", encoding="utf-8") as log:
            proc = subprocess.Popen(cmd, env=env, stdout=log, stderr=subprocess.STDOUT)
            started = time.time()
            try:
                code = proc.wait(timeout=240)
            except subprocess.TimeoutExpired:
                proc.kill()
                code = "timeout"
            elapsed = round(time.time() - started, 1)
        text = log_path.read_text(encoding="utf-8", errors="replace")

        if code != 0:
            failures.append(f"{tool}: 回合退出码 {code}（{elapsed}s）")
        if f"审批 {tool} 请求 execute 权限" not in text:
            failures.append(f"{tool}: 没有触发 Execute 审批")
        if "风险：" not in text or "outputs/" not in text:
            failures.append(f"{tool}: 审批卡缺少风险说明")

        outputs = sorted((ws / "outputs").glob("*" + EXTENSIONS[tool]))
        if not outputs:
            failures.append(f"{tool}: outputs/ 里没有 {EXTENSIONS[tool]} 产物")
        else:
            head = outputs[0].read_bytes()[:4].hex()
            if not head.startswith(MAGIC[tool]):
                failures.append(f"{tool}: 产物魔数不符（{head}）")

        db = next(data.rglob("index.db"), None)
        if not db:
            failures.append(f"{tool}: 没有 index.db，无法核对事件")
        else:
            with sqlite3.connect(str(db)) as conn:
                rows = conn.execute(
                    "select payload_json from turn_events where payload_json like ?",
                    (f"%{tool}%",),
                ).fetchall()
            blob = "\n".join(row[0] for row in rows)
            if '"ok":true' not in blob:
                failures.append(f"{tool}: 事件里没有成功的 tool_result")
            expected = "data:image/png" if tool == "generate_image" else "data:audio/mpeg"
            if expected not in blob:
                failures.append(f"{tool}: 事件缺少内联 {expected}")
        print(f"[{'PASS' if not failures else 'FAIL'}] {tool}{' url' if url_mode else ''} ({elapsed}s)")
    finally:
        mock.terminate()
        mock_log.close()
        _stop_daemon(data)
    return failures


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--tool", choices=sorted(PROMPTS), action="append")
    parser.add_argument("--url", action="store_true", help="图片用下载 URL 模式验证")
    args = parser.parse_args()

    if not EXE.exists():
        print(f"缺少可执行文件：{EXE}\n请先 cargo build -p owo-agent-cli")
        return 1

    cases = []
    for tool in (args.tool or sorted(PROMPTS)):
        cases.append((tool, False))
        if tool == "generate_image" and (args.url or args.tool is None):
            cases.append((tool, True))

    workdir = Path(tempfile.mkdtemp(prefix="owo-media-smoke-"))
    failures: list[str] = []
    for tool, url_mode in cases:
        failures.extend(run_case(tool, url_mode, workdir))

    print(f"\n工作目录：{workdir}")
    if failures:
        print("失败项：")
        for item in failures:
            print(" - " + item)
        return 1
    print(f"全部通过（{len(cases)} 个场景）")
    return 0


if __name__ == "__main__":
    sys.exit(main())
