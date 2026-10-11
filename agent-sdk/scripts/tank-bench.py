#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""30 分钟真实模型基准：从零构建 2D 坦克大战 + 自训练 AI 对手。

由 OwO Agent（本地 daemon + 真实模型）完成全部编码/训练，本脚本只负责：
* 建会话、下发需求、自动审批权限请求、自动应答提问；
* 30 分钟硬预算：到点停止读取并标记 TIMEOUT；
* 所有 SSE 事件带时间戳写入 JSONL，供事后阶段耗时分析。

用法：
  python tank-bench.py --base http://127.0.0.1:24119 --workspace <ws> \
      --events <events.jsonl> --budget-seconds 1800
退出码：0=final；4=预算耗尽；2=断言失败；3=异常。
"""
import argparse
import json
import os
import queue
import sys
import threading
import time
import urllib.error
import urllib.request

if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    sys.stderr.reconfigure(encoding="utf-8", errors="replace")


class ApiError(RuntimeError):
    def __init__(self, status, body):
        super().__init__("HTTP %s %s" % (status, body[:500]))
        self.status = status
        self.body = body


def request_json(base, path, body=None, method=None, timeout=60, token=None):
    data = None if body is None else json.dumps(body, ensure_ascii=False).encode("utf-8")
    request = urllib.request.Request(
        base + path, data=data, method=method or ("POST" if data is not None else "GET")
    )
    if data is not None:
        request.add_header("Content-Type", "application/json")
    if token:
        request.add_header("Authorization", "Bearer " + token)
    with urllib.request.urlopen(request, timeout=timeout) as response:
        return json.loads(response.read().decode("utf-8"))


def acquire_token(api):
    payload = request_json(api, "/auth/token", timeout=30)
    token = payload.get("token")
    if not token:
        raise RuntimeError("GET /auth/token 没有返回 token：%s" % payload)
    return token


OBJECTIVE = """从零构建一个可玩的 2D 坦克大战游戏，并自训练一个模型来操作 AI 机器人对手。

【交付路径（必须按此结构，缺一不可）】
1. `tank-game/index.html`：浏览器直接打开即玩（零依赖 HTML5 Canvas + JS，可用多个 js 文件）。
   - 坦克移动/旋转、开炮、障碍物、子弹碰撞、血条、命中/击毁、胜负判定与重开；
   - **本地双人**：同一键盘同时操作（P1: WASD+空格；P2: 方向键+回车），互不冲突；
   - 可切换「双人对战 / 玩家 vs AI」模式；AI 由自训练模型驱动。
2. `ai/train.py`：Python 3 + numpy，通过自博弈或启发式对局生成数据训练小型神经网络（如 MLP），
   固定随机种子，导出 `ai/model.json` 权重；训练必须真实运行（用 run_command）。
3. 游戏端加载 `ai/model.json` 做前向推理驱动 AI（推理实现与训练一致，写在 `tank-game/ai.js`）。
4. `tests/game.test.mjs`（Node，`node --test` 可跑）：覆盖移动/碰撞/胜负/输入映射/模型前向一致性。
5. `README.md`：操作说明、AI 训练与加载方式、测试运行命令、已知限制。

【过程要求】
- 不要向我提问；缺少的信息按最合理假设处理，并在总结中说明。
- **不要长篇思考或先写完整设计文档**：每一步立即调用工具（write_file/run_command 等），
  单条消息除文件内容外尽量简短，把计划放进代码与 README，而不是聊天里。
- 所有文件写入/命令执行必须用工具真实落盘、真实运行，不要只给代码片段。
- 总预算 30 分钟（含训练）。按「游戏可玩闭环 → 训练并导出权重 → AI 接入 → 测试 → README」
  的顺序推进，每完成一块立即自检。
- 只有全部交付完成且 `node --test` 通过后，才允许结束；结束时最后一行单独输出 `GOAL_DONE`，
  并在此之前输出中文总结（交付清单、验证命令与结果、AI 训练方式与耗时、未完成项）。
"""

CONTINUE = """继续上一轮未完成的部分，不要提问。剩余预算约 {minutes} 分钟。

【当前仍缺失的交付】
{missing}

要求：
- 立即用工具继续完成上述缺失项，不要只输出总结或计划；
- 每完成一个文件/步骤就继续下一步，全部完成后必须真实运行 `node --test tests/game.test.mjs`
  与 AI 训练脚本，并在 README 写明命令；
- 只有全部交付完成且测试通过，才允许结束；结束时最后一行单独输出 `GOAL_DONE`，并给出中文总结。
"""

CONTINUE = """继续上一轮未完成的部分，不要提问。剩余预算约 {minutes} 分钟。
优先保证：可玩闭环 + AI 权重真实存在 + 测试通过 + README。完成后输出中文总结。"""


class Bench(object):
    def __init__(self, api, token, events_path):
        self.api = api
        self.token = token
        self.events_path = events_path
        self.events_file = open(events_path, "a", encoding="utf-8")
        self.log_file = open(events_path + ".log", "a", encoding="utf-8")
        self.started = time.time()
        self.tool_uses = []
        self.tool_results = []
        self.permissions = []
        self.questions = 0
        self.final = None
        self.failed = []
        self.stats = None
        self.last_event = ""
        self.reasoning_deltas = 0
        self.token_deltas = 0

    def log(self, message):
        line = "  [%4ds] %s" % (int(time.time() - self.started), message)
        print(line, flush=True)
        self.log_file.write(line + "\n")
        self.log_file.flush()

    def record(self, event):
        event = dict(event)
        event["_t"] = round(time.time() - self.started, 3)
        self.events_file.write(json.dumps(event, ensure_ascii=False) + "\n")
        self.events_file.flush()

    def run_turn(self, session_id, prompt, deadline):
        body = json.dumps({"prompt": prompt}, ensure_ascii=False).encode("utf-8")
        request = urllib.request.Request(
            self.api + "/session/" + session_id + "/turn", data=body, method="POST"
        )
        request.add_header("Content-Type", "application/json")
        if self.token:
            request.add_header("Authorization", "Bearer " + self.token)
        response = urllib.request.urlopen(request, timeout=3600)
        lines = queue.Queue()

        def reader():
            try:
                for raw in response:
                    lines.put(raw.decode("utf-8", errors="replace"))
            except Exception as error:  # noqa: BLE001 - 流结束/超时都到此
                lines.put({"__reader_error__": repr(error)})
            finally:
                lines.put(None)

        thread = threading.Thread(target=reader, daemon=True)
        thread.start()
        while True:
            remaining = deadline - time.time()
            if remaining <= 0:
                self.log("BUDGET_EXHAUSTED 关闭流")
                try:
                    response.close()
                except Exception:  # noqa: BLE001
                    pass
                return "timeout"
            try:
                item = lines.get(timeout=min(5.0, remaining))
            except queue.Empty:
                continue
            if item is None:
                return "closed"
            if isinstance(item, dict):
                self.log("SSE_READER_ERROR %s" % item)
                return "error"
            line = item.strip()
            if not line.startswith("data:"):
                continue
            try:
                event = json.loads(line[5:].strip())
            except json.JSONDecodeError:
                continue
            self.record(event)
            etype = event.get("type")
            self.last_event = etype or ""
            if etype == "permission_request":
                request_id = event.get("request_id")
                self.permissions.append(event)
                self.log("approval %s tool=%s" % (request_id, event.get("tool")))
                try:
                    request_json(
                        self.api,
                        "/session/%s/permission/%s" % (session_id, request_id),
                        {"allow": True},
                        token=self.token,
                    )
                except ApiError as error:
                    self.log("approval FAILED %s" % error)
            elif etype == "user_question":
                self.questions += 1
                question_id = event.get("question_id")
                self.log("question: %s" % event.get("question"))
                try:
                    request_json(
                        self.api,
                        "/session/%s/answer/%s" % (session_id, question_id),
                        {
                            "question_id": question_id,
                            "answer": "按原任务最合理假设继续，不要提问，并把假设写进总结。",
                        },
                        token=self.token,
                    )
                except ApiError as error:
                    self.log("answer FAILED %s" % error)
            elif etype == "tool_use":
                self.tool_uses.append((time.time() - self.started, event.get("tool")))
                self.log("tool use %s" % event.get("tool"))
            elif etype == "tool_result":
                self.tool_results.append((time.time() - self.started, event.get("tool"), bool(event.get("ok"))))
                status = "ok" if event.get("ok") else "error"
                self.log("tool result %s -> %s" % (event.get("tool"), status))
            elif etype == "final":
                self.final = event.get("text")
                self.log("final")
                return "final"
            elif etype == "turn_failed":
                self.failed.append(event)
                self.log("turn_failed %s" % event.get("message"))
                return "failed"
            elif etype in ("turn_stats", "usage"):
                self.stats = event
                self.log("stats %s" % json.dumps(event, ensure_ascii=False)[:200])
            elif etype in ("token_delta", "reasoning_delta"):
                if etype == "reasoning_delta":
                    self.reasoning_deltas += 1
                    if self.reasoning_deltas % 2000 == 0:
                        self.log("reasoning deltas=%d" % self.reasoning_deltas)
                else:
                    self.token_deltas += 1
                    if self.token_deltas % 500 == 0:
                        self.log("token deltas=%d" % self.token_deltas)
                continue
            else:
                self.log("event %s" % json.dumps(event, ensure_ascii=False)[:160])


STEPS = [
    """第 1 步（只做这一步，立即用工具，不要提问、不要长篇计划）：
创建 `tank-game/index.html` 与 `tank-game/game.js`，实现完整可玩的 2D 坦克大战：
Canvas 渲染、障碍物、坦克移动/旋转、开炮、子弹碰撞、血条、命中/击毁、胜负判定与重开；
同一键盘本地双人（P1: WASD+空格；P2: 方向键+回车）；顶部按钮可切换「双人对战 / 玩家 vs AI」；
AI 模式调用 `tank-game/ai.js` 暴露的 `window.TankAI.decide(state)`（若未加载则回退随机策略）。
写完文件后立即结束，不要运行训练或测试。""",
    """第 2 步（只做这一步，立即用工具）：
创建 `tank-game/ai.js`：加载 `ai/model.json` 的 MLP 权重做前向推理，暴露
`window.TankAI = { ready, decide(state) }`，state 至少含自身/敌方位置朝向、血量、最近子弹；
输入归一化与训练脚本保持一致；权重缺失时回退到确定性启发式（追击+避弹），不得抛异常。
写完立即结束，不要运行训练或测试。""",
    """第 3 步（只做这一步，立即用工具）：
创建 `ai/train.py`（Python3 + numpy，不用 torch）：实现小型 MLP（如 8→32→16→4，tanh/relu），
用启发式自博弈生成对局数据（固定随机种子），训练并导出 `ai/model.json`（含层结构+权重+归一化参数），
支持 `--games` / `--epochs` 参数；文件末尾 `if __name__ == "__main__":` 可直接运行。
写完立即结束，不要运行。""",
    """第 4 步（只做这一步，立即用工具）：
用 run_command 运行 `python ai/train.py --games 200 --epochs 30`，确认 `ai/model.json` 真实生成；
打印权重文件大小与训练摘要（样本数/最终 loss）。若脚本报错，修复 `ai/train.py` 后重跑，直到成功。
完成后立即结束。""",
    """第 5 步（只做这一步，立即用工具）：
创建 `tests/game.test.mjs`（Node 内置 `node:test` + `assert`），覆盖：
输入映射（WASD/方向键→方向）、子弹与坦克碰撞、胜负判定、以及从 `ai/model.json` 读取权重做一次
前向推理的数值一致性；然后 run_command `node --test tests/game.test.mjs`。
若失败，修复测试或游戏/AI 代码后重跑，直到通过。完成后立即结束。""",
    """第 6 步（只做这一步，立即用工具）：
更新 `README.md`：操作说明、AI 训练与加载方式、测试命令、已知限制；
然后用 run_command 列出交付文件清单并再次运行 `node --test tests/game.test.mjs` 确认通过。
最后输出中文总结，并在最后一行单独输出 `GOAL_DONE`。""",
]


def missing_deliverables(workspace):
    def has_file(rel):
        return os.path.isfile(os.path.join(workspace, rel))

    missing = []
    for rel in ("tank-game/index.html", "ai/train.py", "tank-game/ai.js", "tests/game.test.mjs", "README.md"):
        if not has_file(rel):
            missing.append(rel)
    weights = ("model.json", "weights.json", "model.npz", "model.pkl")
    if not any(has_file(os.path.join("ai", name)) for name in weights):
        missing.append("ai/model.json（训练产物权重）")
    return missing


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--base", default="http://127.0.0.1:24119")
    parser.add_argument("--workspace", required=True)
    parser.add_argument("--events", required=True)
    parser.add_argument("--budget-seconds", type=int, default=1800)
    parser.add_argument("--steps", action="store_true", help="按预设 6 步分解推进（弱化单次规划负担）")
    parser.add_argument("--from-step", type=int, default=1, help="从第 N 步开始（配合 --steps 续跑）")
    parser.add_argument("--only-step", type=int, default=0, help="只跑第 N 步（配合 --steps）")
    parser.add_argument("--steps-list", default="", help="只跑这些步（逗号分隔，如 2,5,6）")
    args = parser.parse_args()
    api = args.base.rstrip("/")
    workspace = os.path.abspath(args.workspace)
    deadline = time.time() + args.budget_seconds

    health = request_json(api, "/health")
    if health.get("healthy") is not True:
        print("UNHEALTHY %s" % health, flush=True)
        return 3
    token = acquire_token(api)
    session = request_json(api, "/session", {"workspace": workspace, "model": None}, token=token)
    session_id = session["id"]
    print("SESSION %s workspace=%s budget=%ss" % (session_id, workspace, args.budget_seconds), flush=True)

    bench = Bench(api, token, args.events)
    turns = 0
    if args.steps:
        status = "final"
        selected = set()
        if args.steps_list:
            selected = {int(part) for part in args.steps_list.split(",") if part.strip()}
        for index, step in enumerate(STEPS, 1):
            if args.only_step:
                if index != args.only_step:
                    continue
            elif selected:
                if index not in selected:
                    continue
            elif index < args.from_step:
                continue
            if time.time() >= deadline - 30:
                status = "timeout"
                break
            status = bench.run_turn(session_id, step, deadline)
            turns = index
            print("STEP%d %s elapsed=%ds tools=%d approvals=%d"
                  % (index, status, int(time.time() - bench.started),
                     len(bench.tool_uses), len(bench.permissions)), flush=True)
            if status != "final":
                break
        print("CHECK missing=%s GOAL_DONE=%s"
              % (missing_deliverables(workspace),
                 bool(bench.final and "GOAL_DONE" in bench.final)), flush=True)
    else:
        status = bench.run_turn(session_id, OBJECTIVE, deadline)
        turns = 1
        print("TURN1 %s elapsed=%ds tools=%d approvals=%d questions=%d"
              % (status, int(time.time() - bench.started), len(bench.tool_uses),
                 len(bench.permissions), bench.questions), flush=True)
        while status == "final" and time.time() < deadline - 30 and turns < 12:
            missing = missing_deliverables(workspace)
            done_marker = bool(bench.final and "GOAL_DONE" in bench.final)
            print("CHECK missing=%s GOAL_DONE=%s" % (missing, done_marker), flush=True)
            if not missing and (done_marker or turns >= 3):
                break
            minutes = max(1, int((deadline - time.time()) / 60))
            missing_text = "\n".join("- " + item for item in missing) or \
                "- （交付路径齐全：请真实运行 node --test 与训练脚本，确认全部通过）"
            status = bench.run_turn(
                session_id, CONTINUE.format(minutes=minutes, missing=missing_text), deadline
            )
            turns += 1
            print("TURN%d %s elapsed=%ds tools=%d"
                  % (turns, status, int(time.time() - bench.started), len(bench.tool_uses)), flush=True)

    summary = {
        "session_id": session_id,
        "status": status,
        "turns": turns,
        "elapsed": round(time.time() - bench.started, 1),
        "tool_uses": len(bench.tool_uses),
        "tool_errors": sum(1 for _, _, ok in bench.tool_results if not ok),
        "approvals": len(bench.permissions),
        "questions": bench.questions,
        "turn_failed": bench.failed,
        "stats": bench.stats,
        "final": (bench.final or "")[:4000],
    }
    print("BENCH_DONE " + json.dumps(summary, ensure_ascii=False), flush=True)
    return 0 if status == "final" else (4 if status == "timeout" else 2)


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Exception as error:  # noqa: BLE001 - 基准需要明确失败原因
        import traceback

        traceback.print_exc()
        print("BENCH_ERROR %r" % (error,), flush=True)
        sys.exit(3)
