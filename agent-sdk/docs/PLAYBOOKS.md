# 任务剧本（Playbook）：按域定义工作流

> 状态：定义层与进度状态机已落地（2026-11-18）；执行驱动是下一步。
> 代码：`crates/owo-agent-contracts/src/playbook.rs`、`crates/owo-agent-cli/src/commands/playbook.rs`

## 1. 它解决什么

固定的任务套路（代码评审、发布前检查、周报、数据核对）不该每次让模型重新想一遍。
剧本把「这类任务怎么做」写成文件：**步骤 + 依赖 + 完成判据 + 步骤类型**，执行时按
进度推进，而不是从零规划。

与现有编排资产的分工（互不替代）：

| 资产 | 管什么 | 不做什么 |
|---|---|---|
| `TeamTemplate` / `workflow` | 多 Agent 团队：谁做、写哪里、角色 DAG | 单 Agent 的固定步骤套路 |
| `goal` / `GoalRunner` | 一次性目标的计划与验收 | 可复用的领域套路 |
| **Playbook（本模块）** | 单 Agent 的领域步骤顺序与判据 | 不执行命令、不调度 Agent、不碰工作区 |

## 2. 文件格式

```json
{
  "id": "release-check",
  "name": "发布前检查",
  "description": "……",
  "steps": [
    { "id": "clean", "title": "工作树干净", "kind": "check", "acceptance": "git status --short 为空" },
    { "id": "build", "title": "release 构建", "kind": "check", "acceptance": "构建命令 exit 0", "depends_on": ["clean"] },
    { "id": "notes", "title": "整理发布说明", "kind": "model", "acceptance": "覆盖所有用户可见改动", "depends_on": ["build"] },
    { "id": "signoff", "title": "人工签核", "kind": "human", "acceptance": "负责人确认", "depends_on": ["notes"] }
  ]
}
```

| 字段 | 必填 | 说明 |
|---|---|---|
| `id` / `name` | ✅ | 剧本标识与人读名称 |
| `description` | ❌ | 用途说明 |
| `steps[].id` / `title` | ✅ | 步骤标识（剧本内唯一）与标题 |
| `steps[].kind` | ❌ | `model`（默认，需要模型回合）/ `check`（确定性校验）/ `human`（人工确认） |
| `steps[].acceptance` | `check` 必填 | 完成判据；`check` 无判据无从判定，直接判非法 |
| `steps[].depends_on` | ❌ | 前置步骤 id；必须已通过才允许开始 |
| `steps[].detail` | ❌ | 给执行者的上下文 |

校验规则（`Playbook::validate`）：id/name 非空、至少一步、≤64 步、步骤 id 唯一、
依赖存在、禁止自依赖、无环；未知依赖**不会**被误报成环（两者分开报告）。

示例：`playbooks/code-review.json`、`playbooks/release-check.json`。

## 3. 进度状态机

`PlaybookRun` 记录每步状态（可序列化持久化）：

```text
Pending ──pass──► Passed { evidence }
   │  └──fail──► Failed { reason, attempts }   （可重试，attempts 累计）
   └──skip─────► Skipped { reason }
```

派生规则：

- `next_ready()`：按**定义顺序**返回第一个 Pending 且前置全部 Passed 的步骤；
- `is_blocked()`：依赖链上出现 Failed/Skipped → 本步永久阻塞（需要人处理前置）；
- `progress()`：`total / passed / failed / skipped / pending / blocked` + 完成百分比；
- `is_complete()`：只有 passed + skipped 覆盖全部步骤才算完成（**失败与阻塞不算完成**）。

这些规则是纯函数，不依赖时间与 IO；`marker` 字段留给调用方写时间戳/批次号。

## 4. CLI

```powershell
# 校验（结构/依赖/环/大小）；不合法 → 退出码 1
owo-agent playbook validate playbooks/code-review.json

# 打印拓扑序 + 判据 + 下一步（不执行）
owo-agent playbook plan playbooks/release-check.json

# 进度闭环（不执行任何命令，只记录结果）
owo-agent playbook next   playbooks/code-review.json        # 首次会创建状态文件
owo-agent playbook record playbooks/code-review.json scope --pass "看了 12 个文件"
owo-agent playbook record playbooks/code-review.json build --fail "cargo check: E0308"
owo-agent playbook status playbooks/code-review.json
owo-agent playbook reset  playbooks/code-review.json        # 清空重来

# 目录总览：每个剧本的步骤数与进度（默认 playbooks/）
owo-agent playbook list
owo-agent playbook list --json

# 机器可读（status / next / record / reset / list 都支持）
owo-agent playbook status --json playbooks/code-review.json
```

运行状态默认写在与剧本同目录的 `<剧本名>.state.json`（示例目录已用 `.gitignore`
排除 `*.state.json`）；用 `--run <路径>` 可以改写到别处（例如数据目录）。

状态文件就是 `PlaybookRun` 的 JSON，任何客户端都能读写同一份进度：

```json
{
  "playbook_id": "code-review",
  "states": {
    "scope": { "state": "passed", "evidence": "看了 12 个文件" },
    "build": { "state": "failed", "reason": "cargo check: E0308", "attempts": 1 }
  }
}
```

## 5. 模型侧接入（可选工具组，默认关闭）

剧本工具属于**额外能力**：新增两个工具会改变模型可见工具面（上下文成本 + 行为引导），
因此默认不装配。在 `<workspace>/settings.json` 显式开启：

```json
{ "tool_capabilities": { "playbooks": true } }
```

| 工具 | 副作用 | 用途 |
|---|---|---|
| `playbook_next { file }` | **read** | 返回下一步 + 完成判据 + 整体进度；只读，不写状态文件 |
| `playbook_record { file, step, outcome, note }` | **write** | 记录 `pass/fail/skip`，写同目录 `<剧本>.state.json` |

模型循环：`playbook_next` → 按判据执行这一步（用既有工具，权限/审批/审计不变）→
`playbook_record` 记录结果 → 再 `next`。失败步骤保留在可执行集里（`attempts` 累计），
是否继续重试由模型或推进策略决定。

安全边界：路径经会话工作区约束（越界拒绝）；读取上限 256 KB、备注上限 2000 字符；
`playbook_record` 的副作用登记为 Write，照常走审批。

## 6. 安全边界

- 本模块**不执行任何命令**：`check` 步骤的 `acceptance` 只是文本判据，真正执行必须由
  宿主经 Tool Host（权限/审批/审计/沙箱一个都不能绕）。
- 输入按上限截断（步骤数、单步文本长度由 schema 控制），不做递归解释。
- 文件读取容忍 UTF-8 BOM（Windows 编辑器常见），不改变内容语义。

## 7. 下一步

1. **执行驱动**：`playbook run` 按 `next_ready()` 推进；`check` 步骤经 Tool Host 执行并
   记录证据，`model` 步骤走一次 Agent 回合，`human` 步骤挂审批；
2. **运行记录**：`PlaybookRun` 落盘到 `<data>/playbooks/runs/`，支持中断续跑；
3. **与 `/advance` 协同**：`/advance` 负责"按进度不空转"，剧本负责"按什么顺序做"；
4. **桌面面板**：剧本列表 + 运行进度（见 `docs/UI-SURFACING.md` 的 surfacing 清单）。
