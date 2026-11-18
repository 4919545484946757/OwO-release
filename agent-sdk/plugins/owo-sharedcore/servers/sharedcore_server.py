#!/usr/bin/env python3
"""SharedCore：多 Agent 共享核心上下文的协调服务器（stdio MCP，JSON-RPC 2.0）。

只依赖 Python 标准库。所有状态都写在 `<workspace>/.sharedcore/`：

    project.json     项目目标、架构、规则、版本（core_version / context_revision）
    tasks.json       任务 DAG（依赖、读写范围、验收、状态）
    contracts.json   接口/数据契约（版本 + 提议/接受）
    events.jsonl     只追加的共享事件日志

设计原则（对齐 docs/goal/多agent共享上下文设计方案.md）：
- 共享结构化状态，不共享聊天记录；
- 每次写入原子替换 + revision 递增；契约更新用 expected_version 做 CAS；
- 本服务器不执行命令、不访问网络；git/测试由宿主 Agent 工具完成。
"""

import datetime
import json
import os
import re
import sys
import tempfile

STATE_DIR = ".sharedcore"
SCHEMA_VERSION = 1
MAX_BRIEF_CHARS = 12000


# ---------------------------------------------------------------------------
# 存储
# ---------------------------------------------------------------------------


def now_iso():
    return datetime.datetime.now(datetime.timezone.utc).isoformat(timespec="seconds")


def state_dir(workspace):
    return os.path.join(workspace, STATE_DIR)


def atomic_write(path, payload):
    directory = os.path.dirname(path)
    os.makedirs(directory, exist_ok=True)
    handle = tempfile.NamedTemporaryFile(
        "w", encoding="utf-8", dir=directory, delete=False, suffix=".tmp"
    )
    try:
        json.dump(payload, handle, ensure_ascii=False, indent=2)
        handle.write("\n")
        handle.flush()
        os.fsync(handle.fileno())
    finally:
        handle.close()
    os.replace(handle.name, path)


def read_json(path, default=None):
    try:
        with open(path, "r", encoding="utf-8-sig") as handle:
            return json.load(handle)
    except (OSError, ValueError):
        return default


def project_path(workspace):
    return os.path.join(state_dir(workspace), "project.json")


def tasks_path(workspace):
    return os.path.join(state_dir(workspace), "tasks.json")


def contracts_path(workspace):
    return os.path.join(state_dir(workspace), "contracts.json")


def events_path(workspace):
    return os.path.join(state_dir(workspace), "events.jsonl")


def require_project(workspace):
    project = read_json(project_path(workspace))
    if not project:
        raise ToolError("sharedcore 尚未初始化：先调用 sharedcore_init")
    return project


def load_tasks(workspace):
    payload = read_json(tasks_path(workspace), {}) or {}
    return payload


def save_tasks(workspace, payload):
    atomic_write(tasks_path(workspace), payload)


def load_contracts(workspace):
    payload = read_json(contracts_path(workspace), {}) or {}
    payload.setdefault("contracts", {})
    return payload


def save_contracts(workspace, payload):
    atomic_write(contracts_path(workspace), payload)


def bump_revision(workspace, project):
    project["context_revision"] = int(project.get("context_revision") or 0) + 1
    project["updated_at"] = now_iso()
    atomic_write(project_path(workspace), project)
    return project["context_revision"]


def append_event(workspace, event):
    project = read_json(project_path(workspace), {}) or {}
    revision = int(project.get("context_revision") or 0)
    path = events_path(workspace)
    seq = 0
    if os.path.exists(path):
        with open(path, "r", encoding="utf-8") as handle:
            seq = sum(1 for _ in handle)
    event = dict(event)
    event.setdefault("event_id", "evt-%06d" % (seq + 1))
    event.setdefault("ts", now_iso())
    event["context_revision"] = revision
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "a", encoding="utf-8") as handle:
        handle.write(json.dumps(event, ensure_ascii=False) + "\n")
    return event


def recent_events(workspace, limit=10):
    path = events_path(workspace)
    if not os.path.exists(path):
        return []
    try:
        with open(path, "r", encoding="utf-8") as handle:
            lines = handle.readlines()[-limit:]
    except OSError:
        return []
    events = []
    for line in lines:
        try:
            events.append(json.loads(line))
        except ValueError:
            continue
    return events


# ---------------------------------------------------------------------------
# 工具错误
# ---------------------------------------------------------------------------


class ToolError(Exception):
    pass


# ---------------------------------------------------------------------------
# 调度核心
# ---------------------------------------------------------------------------


def task_list(tasks_payload):
    tasks = tasks_payload.get("tasks") or {}
    return list(tasks.values())


def deps_done(task, tasks):
    for dependency in task.get("depends_on") or []:
        other = tasks.get(dependency)
        if other is None or other.get("status") != "done":
            return False
    return True


def missing_deps(task, tasks):
    return [
        dependency
        for dependency in task.get("depends_on") or []
        if dependency not in tasks or tasks[dependency].get("status") != "done"
    ]


def scopes_overlap(left, right):
    """写范围冲突：任一路径前缀相同即视为可能冲突（保守）。"""
    for first in left or []:
        for second in right or []:
            a = first.strip().replace("\\", "/").rstrip("/")
            b = second.strip().replace("\\", "/").rstrip("/")
            if not a or not b:
                continue
            if a == b or a.startswith(b + "/") or b.startswith(a + "/"):
                # 通配范围按前缀处理
                if "*" in a or "*" in b:
                    return True
                if a == b:
                    return True
                if a.startswith(b + "/") or b.startswith(a + "/"):
                    return True
    return False


def plan_payload(workspace, max_parallel=4):
    tasks_payload = load_tasks(workspace)
    tasks = tasks_payload.get("tasks") or {}
    done = {key for key, value in tasks.items() if value.get("status") == "done"}
    ready = []
    blocked = {}
    for key, task in sorted(tasks.items()):
        status = task.get("status")
        if status in ("done", "skipped", "failed", "cancelled"):
            continue
        if status == "running":
            continue
        missing = missing_deps(task, tasks)
        if missing:
            blocked[key] = missing
        else:
            ready.append(task)
    # 就绪任务按优先级（高在前）+ id 排序
    ready.sort(key=lambda item: (-int(item.get("priority") or 0), item.get("id")))
    batches = []
    conflicts = []
    remaining = list(ready)
    while remaining:
        batch = []
        deferred = []
        for task in remaining:
            if len(batch) >= max_parallel:
                deferred.append(task)
                continue
            overlapping = []
            for chosen in batch:
                if scopes_overlap(task.get("write_scope"), chosen.get("write_scope")):
                    overlapping.append(chosen.get("id"))
            if overlapping:
                conflicts.append(
                    {"task": task.get("id"), "overlaps": overlapping}
                )
                deferred.append(task)
            else:
                batch.append(task)
        if not batch:
            # 全部互相冲突：逐批串行，避免死循环
            batch, deferred = [remaining[0]], remaining[1:]
        batches.append([task.get("id") for task in batch])
        remaining = deferred
    return {
        "ready": [task.get("id") for task in ready],
        "batches": batches,
        "blocked": blocked,
        "conflicts": conflicts,
        "done": sorted(done),
        "max_parallel": max_parallel,
    }


# ---------------------------------------------------------------------------
# 工具实现
# ---------------------------------------------------------------------------


def require_workspace(args):
    workspace = str(args.get("workspace") or "").strip()
    if not workspace:
        raise ToolError("缺少 workspace（绝对路径）")
    workspace = os.path.abspath(workspace)
    if not os.path.isdir(workspace):
        raise ToolError("workspace 不存在或不是目录：%s" % workspace)
    return workspace


def tool_init(args):
    workspace = require_workspace(args)
    path = project_path(workspace)
    reset = bool(args.get("reset"))
    if os.path.exists(path) and not reset:
        raise ToolError("sharedcore 已初始化；需要重建请传 reset=true")
    name = str(args.get("name") or os.path.basename(workspace.rstrip("\\/")) or "project")
    rules = [str(item) for item in args.get("rules") or [] if str(item).strip()]
    architecture = args.get("architecture") or {}
    project = {
        "schema_version": SCHEMA_VERSION,
        "project_id": "sharedcore-" + re.sub(r"[^a-z0-9-]+", "-", name.lower()).strip("-")[:40],
        "name": name,
        "goal": str(args.get("goal") or ""),
        "baseline_commit": str(args.get("baseline_commit") or ""),
        "architecture": architecture,
        "rules": rules,
        "core_version": 1,
        "context_revision": 0,
        "created_at": now_iso(),
        "updated_at": now_iso(),
    }
    atomic_write(path, project)
    save_tasks(workspace, {"schema_version": SCHEMA_VERSION, "next_id": 1, "tasks": {}})
    save_contracts(workspace, {"schema_version": SCHEMA_VERSION, "contracts": {}})
    open(events_path(workspace), "w", encoding="utf-8").close()
    append_event(
        workspace,
        {"type": "project.initialized", "source": "sharedcore", "resource": "project"},
    )
    return {"ok": True, "project": project}


def tool_status(args):
    workspace = require_workspace(args)
    project = require_project(workspace)
    tasks = (load_tasks(workspace).get("tasks") or {})
    counts = {}
    for task in tasks.values():
        counts[task.get("status") or "unknown"] = counts.get(task.get("status") or "unknown", 0) + 1
    plan = plan_payload(workspace, int(args.get("max_parallel") or 4))
    return {
        "project": {
            "name": project.get("name"),
            "goal": project.get("goal"),
            "core_version": project.get("core_version"),
            "context_revision": project.get("context_revision"),
            "baseline_commit": project.get("baseline_commit"),
        },
        "tasks": {"total": len(tasks), "by_status": counts},
        "plan": {
            "batches": plan["batches"],
            "blocked": plan["blocked"],
            "conflicts": plan["conflicts"],
        },
        "contracts": load_contracts(workspace)["contracts"],
        "recent_events": recent_events(workspace, 10),
    }


def tool_context(args):
    workspace = require_workspace(args)
    project = require_project(workspace)
    payload = {
        "name": project.get("name"),
        "goal": project.get("goal"),
        "architecture": project.get("architecture") or {},
        "rules": project.get("rules") or [],
        "core_version": project.get("core_version"),
        "context_revision": project.get("context_revision"),
        "contracts": load_contracts(workspace)["contracts"],
    }
    task_id = args.get("task_id")
    if task_id:
        task = (load_tasks(workspace).get("tasks") or {}).get(str(task_id))
        if not task:
            raise ToolError("任务不存在：%s" % task_id)
        payload["task"] = task
    text = json.dumps(payload, ensure_ascii=False)
    return {"context": payload, "chars": len(text)}


def normalize_scope(items):
    return [str(item).strip() for item in (items or []) if str(item).strip()]


def tool_task_add(args):
    workspace = require_workspace(args)
    require_project(workspace)
    tasks_payload = load_tasks(workspace)
    title = str(args.get("title") or "").strip()
    if not title:
        raise ToolError("缺少 title")
    next_id = int(tasks_payload.get("next_id") or 1)
    task_id = "T-%03d" % next_id
    tasks_payload["next_id"] = next_id + 1
    task = {
        "id": task_id,
        "title": title,
        "owner": str(args.get("owner") or ""),
        "status": "ready",
        "priority": int(args.get("priority") or 0),
        "depends_on": [str(item) for item in args.get("depends_on") or []],
        "read_scope": normalize_scope(args.get("read_scope")),
        "write_scope": normalize_scope(args.get("write_scope")),
        "acceptance": [str(item) for item in args.get("acceptance") or []],
        "notes": str(args.get("notes") or ""),
        "branch": "",
        "worktree": "",
        "evidence": "",
        "created_at": now_iso(),
        "updated_at": now_iso(),
    }
    tasks_payload.setdefault("tasks", {})[task_id] = task
    save_tasks(workspace, tasks_payload)
    bump_revision(workspace, require_project(workspace))
    append_event(
        workspace,
        {
            "type": "task.created",
            "source": "sharedcore",
            "resource": "tasks/" + task_id,
            "affected_tasks": [task_id],
        },
    )
    return {"ok": True, "task": task}


def tool_plan(args):
    workspace = require_workspace(args)
    require_project(workspace)
    return plan_payload(workspace, int(args.get("max_parallel") or 4))


def tool_claim(args):
    workspace = require_workspace(args)
    require_project(workspace)
    task_id = str(args.get("task_id") or "")
    tasks_payload = load_tasks(workspace)
    tasks = tasks_payload.setdefault("tasks", {})
    task = tasks.get(task_id)
    if not task:
        raise ToolError("任务不存在：%s" % task_id)
    if task.get("status") == "done":
        raise ToolError("任务已完成：%s" % task_id)
    if task.get("status") == "running":
        raise ToolError("任务已被领取：%s（owner=%s）" % (task_id, task.get("owner")))
    missing = missing_deps(task, tasks)
    if missing:
        raise ToolError("前置未完成：%s" % "、".join(missing))
    task["status"] = "running"
    task["owner"] = str(args.get("owner") or task.get("owner") or "")
    task["branch"] = str(args.get("branch") or task.get("branch") or "")
    task["worktree"] = str(args.get("worktree") or task.get("worktree") or "")
    task["updated_at"] = now_iso()
    save_tasks(workspace, tasks_payload)
    bump_revision(workspace, require_project(workspace))
    append_event(
        workspace,
        {
            "type": "task.claimed",
            "source": task["owner"] or "sharedcore",
            "resource": "tasks/" + task_id,
            "affected_tasks": [task_id],
        },
    )
    return {"ok": True, "task": task}


ALLOWED_STATUS = {"running", "done", "failed", "blocked", "skipped"}


def tool_update(args):
    workspace = require_workspace(args)
    require_project(workspace)
    task_id = str(args.get("task_id") or "")
    status = str(args.get("status") or "")
    if status not in ALLOWED_STATUS:
        raise ToolError("status 只允许：%s" % "、".join(sorted(ALLOWED_STATUS)))
    tasks_payload = load_tasks(workspace)
    tasks = tasks_payload.setdefault("tasks", {})
    task = tasks.get(task_id)
    if not task:
        raise ToolError("任务不存在：%s" % task_id)
    previous = task.get("status")
    task["status"] = status
    if args.get("evidence"):
        task["evidence"] = str(args.get("evidence"))[:4000]
    if args.get("reason"):
        task["notes"] = (task.get("notes") or "") + "\n" + str(args.get("reason"))[:2000]
    task["updated_at"] = now_iso()
    save_tasks(workspace, tasks_payload)
    bump_revision(workspace, require_project(workspace))
    event_type = {
        "done": "task.completed",
        "failed": "task.failed",
        "blocked": "agent.blocked",
        "skipped": "task.skipped",
        "running": "task.resumed",
    }[status]
    append_event(
        workspace,
        {
            "type": event_type,
            "source": task.get("owner") or "sharedcore",
            "resource": "tasks/" + task_id,
            "affected_tasks": [task_id],
            "payload": {"from": previous, "to": status},
        },
    )
    return {"ok": True, "task": task, "plan": plan_payload(workspace)}


def tool_contract(args):
    workspace = require_workspace(args)
    require_project(workspace)
    contracts_payload = load_contracts(workspace)
    contracts = contracts_payload["contracts"]
    contract_id = str(args.get("contract_id") or "").strip()
    if not contract_id:
        raise ToolError("缺少 contract_id")
    action = str(args.get("action") or "propose")
    expected = args.get("expected_version")
    current = contracts.get(contract_id)
    current_version = int((current or {}).get("version") or 0)
    if expected is not None and int(expected) != current_version:
        raise ToolError(
            "契约版本冲突：期望 %s，实际 %s（重新读取后再试）"
            % (expected, current_version)
        )
    if action == "propose":
        contract = {
            "id": contract_id,
            "version": current_version + 1,
            "owner": str(args.get("owner") or ""),
            "status": "proposed",
            "notes": str(args.get("notes") or "")[:2000],
            "updated_at": now_iso(),
        }
        contracts[contract_id] = contract
        save_contracts(workspace, contracts_payload)
        bump_revision(workspace, require_project(workspace))
        append_event(
            workspace,
            {
                "type": "contract.proposed",
                "source": contract["owner"] or "sharedcore",
                "resource": "contracts/" + contract_id,
                "old_version": current_version,
                "new_version": contract["version"],
                "breaking_change": bool(args.get("breaking_change")),
            },
        )
        return {"ok": True, "contract": contract}
    if action == "accept":
        if not current or current.get("status") != "proposed":
            raise ToolError("没有待接受的契约提案：%s" % contract_id)
        if expected is not None and int(expected) != current_version:
            raise ToolError("契约版本冲突：期望 %s，实际 %s" % (expected, current_version))
        current["status"] = "accepted"
        current["updated_at"] = now_iso()
        contracts[contract_id] = current
        save_contracts(workspace, contracts_payload)
        project = require_project(workspace)
        project["core_version"] = int(project.get("core_version") or 1) + 1
        bump_revision(workspace, project)
        append_event(
            workspace,
            {
                "type": "contract.updated",
                "source": str(args.get("owner") or current.get("owner") or "sharedcore"),
                "resource": "contracts/" + contract_id,
                "old_version": current_version - 1 if current_version else 0,
                "new_version": current_version,
                "breaking_change": bool(args.get("breaking_change")),
            },
        )
        return {"ok": True, "contract": current, "core_version": project["core_version"]}
    raise ToolError("action 只允许 propose / accept")


def tool_event(args):
    workspace = require_workspace(args)
    require_project(workspace)
    event_type = str(args.get("type") or "").strip()
    if not event_type:
        raise ToolError("缺少 type")
    event = append_event(
        workspace,
        {
            "type": event_type,
            "source": str(args.get("source") or "sharedcore"),
            "resource": str(args.get("resource") or ""),
            "old_version": args.get("old_version"),
            "new_version": args.get("new_version"),
            "affected_tasks": [str(item) for item in args.get("affected_tasks") or []],
            "breaking_change": bool(args.get("breaking_change")),
            "payload": args.get("payload") or {},
        },
    )
    bump_revision(workspace, require_project(workspace))
    return {"ok": True, "event": event}


def tool_brief(args):
    workspace = require_workspace(args)
    project = require_project(workspace)
    task_id = str(args.get("task_id") or "")
    tasks = load_tasks(workspace).get("tasks") or {}
    task = tasks.get(task_id)
    if not task:
        raise ToolError("任务不存在：%s" % task_id)
    contract_ids = args.get("contracts") or []
    contracts = load_contracts(workspace)["contracts"]
    selected = {
        key: contracts[key]
        for key in contract_ids
        if key in contracts
    }
    lines = [
        "# SharedCore 任务简报：%s %s" % (task["id"], task["title"]),
        "",
        "项目：%s" % project.get("name"),
        "目标：%s" % project.get("goal"),
        "基线 commit：%s" % (project.get("baseline_commit") or "未指定"),
        "core_version：%s / context_revision：%s"
        % (project.get("core_version"), project.get("context_revision")),
        "",
        "## 项目规则（必须遵守）",
    ]
    for rule in (project.get("rules") or [])[:20]:
        lines.append("- " + rule)
    if not (project.get("rules") or []):
        lines.append("- （未声明）")
    architecture = project.get("architecture") or {}
    if architecture:
        lines.append("")
        lines.append("## 架构")
        for key, value in list(architecture.items())[:20]:
            lines.append("- %s：%s" % (key, value))
    if selected:
        lines.append("")
        lines.append("## 相关契约")
        for key, value in selected.items():
            lines.append(
                "- %s v%s（%s）" % (key, value.get("version"), value.get("status"))
            )
            if value.get("notes"):
                lines.append("  - " + str(value["notes"])[:500])
    lines += [
        "",
        "## 任务契约",
        "- 任务：%s" % task["id"],
        "- 依赖：%s" % ("、".join(task.get("depends_on") or []) or "无"),
        "- 只允许修改（write_scope）：",
    ]
    for scope in task.get("write_scope") or []:
        lines.append("  - " + scope)
    if not task.get("write_scope"):
        lines.append("  - （未限定：请先申请明确范围）")
    lines += ["- 可读取（read_scope）："]
    for scope in task.get("read_scope") or []:
        lines.append("  - " + scope)
    if not task.get("read_scope"):
        lines.append("  - 项目内按需读取")
    lines += ["", "## 验收标准"]
    for item in task.get("acceptance") or []:
        lines.append("- " + item)
    if not task.get("acceptance"):
        lines.append("- （未声明）")
    if task.get("notes"):
        lines += ["", "## 备注", str(task["notes"])[:2000]]
    lines += [
        "",
        "## 约束",
        "- 不要修改 write_scope 之外的任何文件；确需跨范围时先报告，由主 Agent 新增任务或扩展范围。",
        "- 需要改接口/数据契约时不要直接改代码，先通过 sharedcore_contract 提案。",
        "- 完成后报告：改动文件、验证命令与结果、已知风险。",
    ]
    text = "\n".join(lines)
    if len(text) > MAX_BRIEF_CHARS:
        text = text[:MAX_BRIEF_CHARS] + "\n…（已截断）"
    return {"brief": text, "task_id": task["id"], "chars": len(text)}


# ---------------------------------------------------------------------------
# MCP 协议
# ---------------------------------------------------------------------------

TOOLS = [
    {
        "name": "sharedcore_init",
        "description": "初始化项目共享上下文（目标/架构/规则/基线 commit），状态写入 <workspace>/.sharedcore/。已初始化时需 reset=true 重建。",
        "inputSchema": {
            "type": "object",
            "properties": {
                "workspace": {"type": "string", "description": "项目工作区绝对路径"},
                "name": {"type": "string", "description": "项目名"},
                "goal": {"type": "string", "description": "项目目标（一句话）"},
                "baseline_commit": {"type": "string", "description": "代码基线 commit"},
                "architecture": {"type": "object", "description": "架构键值，如 {backend: Rust}"},
                "rules": {"type": "array", "items": {"type": "string"}, "description": "全局规则"},
                "reset": {"type": "boolean", "description": "重建已有项目状态"},
            },
            "required": ["workspace"],
        },
    },
    {
        "name": "sharedcore_status",
        "description": "查看共享上下文摘要：项目版本、任务计数、并行批次、阻塞与冲突、契约、最近事件。",
        "inputSchema": {
            "type": "object",
            "properties": {
                "workspace": {"type": "string"},
                "max_parallel": {"type": "integer", "description": "计划批次的并发上限，默认 4"},
            },
            "required": ["workspace"],
        },
    },
    {
        "name": "sharedcore_context",
        "description": "读取核心上下文切片（目标/架构/规则/契约），可附带某个任务；用于构造 Agent 提示。",
        "inputSchema": {
            "type": "object",
            "properties": {
                "workspace": {"type": "string"},
                "task_id": {"type": "string"},
            },
            "required": ["workspace"],
        },
    },
    {
        "name": "sharedcore_task_add",
        "description": "新增任务：标题、依赖、读写范围、验收标准；返回任务 id（T-001 起）。",
        "inputSchema": {
            "type": "object",
            "properties": {
                "workspace": {"type": "string"},
                "title": {"type": "string"},
                "owner": {"type": "string"},
                "priority": {"type": "integer"},
                "depends_on": {"type": "array", "items": {"type": "string"}},
                "read_scope": {"type": "array", "items": {"type": "string"}},
                "write_scope": {"type": "array", "items": {"type": "string"}},
                "acceptance": {"type": "array", "items": {"type": "string"}},
                "notes": {"type": "string"},
            },
            "required": ["workspace", "title"],
        },
    },
    {
        "name": "sharedcore_plan",
        "description": "计算可并行批次：依赖满足且写范围不冲突的任务分组，返回 batches/blocked/conflicts。",
        "inputSchema": {
            "type": "object",
            "properties": {
                "workspace": {"type": "string"},
                "max_parallel": {"type": "integer"},
            },
            "required": ["workspace"],
        },
    },
    {
        "name": "sharedcore_claim",
        "description": "领取任务：检查依赖后置为 running，记录 owner/branch/worktree。",
        "inputSchema": {
            "type": "object",
            "properties": {
                "workspace": {"type": "string"},
                "task_id": {"type": "string"},
                "owner": {"type": "string"},
                "branch": {"type": "string"},
                "worktree": {"type": "string"},
            },
            "required": ["workspace", "task_id"],
        },
    },
    {
        "name": "sharedcore_update",
        "description": "更新任务状态：done/failed/blocked/skipped/running，自动记录共享事件与下一步批次。",
        "inputSchema": {
            "type": "object",
            "properties": {
                "workspace": {"type": "string"},
                "task_id": {"type": "string"},
                "status": {"type": "string", "enum": ["running", "done", "failed", "blocked", "skipped"]},
                "evidence": {"type": "string"},
                "reason": {"type": "string"},
            },
            "required": ["workspace", "task_id", "status"],
        },
    },
    {
        "name": "sharedcore_contract",
        "description": "契约提案/接受：propose 递增版本，accept 需 expected_version 匹配（CAS），接受会提升 core_version。",
        "inputSchema": {
            "type": "object",
            "properties": {
                "workspace": {"type": "string"},
                "contract_id": {"type": "string"},
                "action": {"type": "string", "enum": ["propose", "accept"]},
                "owner": {"type": "string"},
                "notes": {"type": "string"},
                "expected_version": {"type": "integer"},
                "breaking_change": {"type": "boolean"},
            },
            "required": ["workspace", "contract_id"],
        },
    },
    {
        "name": "sharedcore_event",
        "description": "追加共享事件（契约变更/冲突/阻塞等），事件只追加不可改。",
        "inputSchema": {
            "type": "object",
            "properties": {
                "workspace": {"type": "string"},
                "type": {"type": "string"},
                "source": {"type": "string"},
                "resource": {"type": "string"},
                "affected_tasks": {"type": "array", "items": {"type": "string"}},
                "breaking_change": {"type": "boolean"},
                "payload": {"type": "object"},
            },
            "required": ["workspace", "type"],
        },
    },
    {
        "name": "sharedcore_brief",
        "description": "为某个任务生成结构化简报（规则/契约/写范围/验收），直接作为子 Agent 的提示。",
        "inputSchema": {
            "type": "object",
            "properties": {
                "workspace": {"type": "string"},
                "task_id": {"type": "string"},
                "contracts": {"type": "array", "items": {"type": "string"}},
            },
            "required": ["workspace", "task_id"],
        },
    },
]

HANDLERS = {
    "sharedcore_init": tool_init,
    "sharedcore_status": tool_status,
    "sharedcore_context": tool_context,
    "sharedcore_task_add": tool_task_add,
    "sharedcore_plan": tool_plan,
    "sharedcore_claim": tool_claim,
    "sharedcore_update": tool_update,
    "sharedcore_contract": tool_contract,
    "sharedcore_event": tool_event,
    "sharedcore_brief": tool_brief,
}


def text_result(payload, is_error=False):
    return {
        "content": [{"type": "text", "text": json.dumps(payload, ensure_ascii=False)}],
        "isError": is_error,
    }


def handle_request(message):
    method = message.get("method")
    identifier = message.get("id")
    if method == "initialize":
        return {
            "jsonrpc": "2.0",
            "id": identifier,
            "result": {
                "protocolVersion": "2025-06-18",
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "owo-sharedcore", "version": "0.1.0"},
            },
        }
    if method == "notifications/initialized":
        return None
    if method == "tools/list":
        return {"jsonrpc": "2.0", "id": identifier, "result": {"tools": TOOLS}}
    if method == "tools/call":
        params = message.get("params") or {}
        name = params.get("name")
        arguments = params.get("arguments") or {}
        handler = HANDLERS.get(name)
        if handler is None:
            return {
                "jsonrpc": "2.0",
                "id": identifier,
                "result": text_result({"error": "未知工具：%s" % name}, True),
            }
        try:
            payload = handler(arguments)
            return {"jsonrpc": "2.0", "id": identifier, "result": text_result(payload)}
        except ToolError as error:
            return {
                "jsonrpc": "2.0",
                "id": identifier,
                "result": text_result({"error": str(error)}, True),
            }
        except Exception as error:  # 防御：不让插件崩溃拖垮宿主
            return {
                "jsonrpc": "2.0",
                "id": identifier,
                "result": text_result({"error": "内部错误：%s" % error}, True),
            }
    return {
        "jsonrpc": "2.0",
        "id": identifier,
        "error": {"code": -32601, "message": "未知方法：%s" % method},
    }


def main():
    # Windows 下 Python 默认 stdio 编码是 GBK；宿主按 UTF-8 收发 JSON-RPC。
    for stream in (sys.stdin, sys.stdout, sys.stderr):
        try:
            stream.reconfigure(encoding="utf-8", errors="replace")
        except (AttributeError, ValueError):
            pass
    stream = sys.stdin
    for line in stream:
        line = line.strip()
        if not line:
            continue
        try:
            message = json.loads(line)
        except ValueError:
            continue
        response = handle_request(message)
        if response is not None:
            sys.stdout.write(json.dumps(response, ensure_ascii=False) + "\n")
            sys.stdout.flush()


if __name__ == "__main__":
    main()
