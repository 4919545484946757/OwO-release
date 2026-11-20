#!/usr/bin/env python3
"""SharedCore 插件自检：直接以 stdio 驱动 MCP 服务器，覆盖 10 个工具的端到端语义。

用法（仓库根目录）：

    python agent-sdk/scripts/sharedcore-selfcheck.py

只使用临时目录，不触碰真实项目；退出码 0 表示通过。
"""

import json
import os
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SERVER = os.path.join(ROOT, "plugins", "owo-sharedcore", "servers", "sharedcore_server.py")


def main() -> int:
    workspace = tempfile.mkdtemp(prefix="sharedcore-selfcheck-")
    process = subprocess.Popen(
        [sys.executable, SERVER],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        encoding="utf-8",
        bufsize=1,
    )

    def call(request_id, method, params=None):
        payload = {"jsonrpc": "2.0", "id": request_id, "method": method}
        if params is not None:
            payload["params"] = params
        process.stdin.write(json.dumps(payload, ensure_ascii=False) + "\n")
        process.stdin.flush()
        return json.loads(process.stdout.readline())

    def tool(request_id, name, arguments):
        response = call(request_id, "tools/call", {"name": name, "arguments": arguments})
        result = response["result"]
        return json.loads(result["content"][0]["text"]), result.get("isError", False)

    init = call(1, "initialize", {
        "protocolVersion": "2025-06-18",
        "capabilities": {},
        "clientInfo": {"name": "selfcheck", "version": "1"},
    })
    assert init["result"]["serverInfo"]["name"] == "owo-sharedcore"

    names = [item["name"] for item in call(2, "tools/list")["result"]["tools"]]
    assert len(names) == 12, names

    data, error = tool(3, "sharedcore_init", {
        "workspace": workspace,
        "name": "selfcheck",
        "goal": "验证共享上下文与并行批次",
        "baseline_commit": "abc123",
        "architecture": {"backend": "Rust", "frontend": "TS"},
        "rules": ["接口先定契约", "禁止越出写范围", "测试必须通过"],
    })
    assert not error, data

    for request_id, title, depends, scope in [
        (4, "定义 API 契约", [], ["contracts/"]),
        (5, "后端接口", ["T-001"], ["src/backend/"]),
        (6, "前端页面", ["T-001"], ["src/frontend/"]),
        (7, "文档", ["T-002"], ["docs/"]),
    ]:
        _, error = tool(request_id, "sharedcore_task_add", {
            "workspace": workspace,
            "title": title,
            "depends_on": depends,
            "write_scope": scope,
            "acceptance": ["见任务标题"],
        })
        assert not error

    plan, error = tool(8, "sharedcore_plan", {"workspace": workspace})
    assert not error and plan["batches"][0] == ["T-001"], plan
    assert plan["blocked"]["T-002"] == ["T-001"], plan

    _, error = tool(9, "sharedcore_claim", {"workspace": workspace, "task_id": "T-001", "owner": "arch"})
    assert not error
    _, error = tool(10, "sharedcore_update", {
        "workspace": workspace, "task_id": "T-001", "status": "done", "evidence": "contracts/api.yaml",
    })
    assert not error

    plan, error = tool(11, "sharedcore_plan", {"workspace": workspace})
    assert not error and plan["batches"][0] == ["T-002", "T-003"], plan

    brief, error = tool(12, "sharedcore_brief", {"workspace": workspace, "task_id": "T-002"})
    assert not error and "接口先定契约" in brief["brief"] and "src/backend/" in brief["brief"]

    _, error = tool(13, "sharedcore_task_add", {
        "workspace": workspace, "title": "后端补充", "write_scope": ["src/backend/api/"],
    })
    assert not error
    plan, error = tool(14, "sharedcore_plan", {"workspace": workspace})
    assert not error
    assert not any("T-002" in batch and "T-005" in batch for batch in plan["batches"]), plan
    assert any(item["task"] == "T-005" for item in plan["conflicts"]), plan

    contract, error = tool(15, "sharedcore_contract", {
        "workspace": workspace, "contract_id": "auth-api", "action": "propose", "owner": "arch",
    })
    assert not error
    rejected, error = tool(16, "sharedcore_contract", {
        "workspace": workspace, "contract_id": "auth-api", "action": "accept", "expected_version": 0,
    })
    assert error, rejected
    accepted, error = tool(17, "sharedcore_contract", {
        "workspace": workspace,
        "contract_id": "auth-api",
        "action": "accept",
        "expected_version": contract["contract"]["version"],
    })
    assert not error and accepted["contract"]["status"] == "accepted"

    status, error = tool(18, "sharedcore_status", {"workspace": workspace})
    assert not error and status["project"]["core_version"] == 2, status

    worktree, error = tool(19, "sharedcore_worktree", {"workspace": workspace, "task_id": "T-002"})
    assert not error and "worktree add" in worktree["command"], worktree

    gate, error = tool(20, "sharedcore_gate", {"workspace": workspace})
    assert not error and gate["ok"] is False, gate
    assert len(gate["blockers"]) >= 1, gate

    gate, error = tool(21, "sharedcore_gate", {
        "workspace": workspace,
        "changed_files": ["src/backend/main.rs"],
        "allow_partial": True,
    })
    assert not error and gate["ok"] is True, gate
    assert any("src/backend/main.rs" in warning for warning in gate["warnings"]), gate

    process.stdin.close()
    process.wait(timeout=10)
    print("OK batches=%s tasks=%s" % (json.dumps(plan["batches"]), json.dumps(status["tasks"])))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
