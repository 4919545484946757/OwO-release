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
| `steps[].command` | ❌ | `check` 步骤可由宿主执行的命令（0 = 通过）；空 = 人工判定。执行经 Tool Host，工具本身不执行 |
| `steps[].depends_on` | ❌ | 前置步骤 id；必须已通过才允许开始 |
| `steps[].detail` | ❌ | 给执行者的上下文 |

校验规则（`Playbook::validate`）：id/name 非空、至少一步、≤64 步、步骤 id 唯一、
依赖存在、禁止自依赖、无环；未知依赖**不会**被误报成环（两者分开报告）。

示例：`playbooks/code-review.json`、`playbooks/release-check.json`、
`playbooks/bugfix.json`（先复现再修的缺陷流程）、`playbooks/weekly-report.json`
（周报整理）。需要新剧本时用 `playbook init <id>` 生成骨架。

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

# 生成骨架（默认 playbooks/<id>.json；--force 覆盖）
owo-agent playbook init my-flow
owo-agent playbook init my-flow --out playbooks/my-flow.json --force
# 从已有剧本派生（保留步骤/判据/命令，改 id 与 name）
owo-agent playbook init review2 --from playbooks/code-review.json

# 归档与历史（同一剧本多次运行可对比）
owo-agent playbook archive playbooks/code-review.json --marker 2026-11-18-a   # 中途快照；完成时会自动留档 complete-<时间戳>
owo-agent playbook history playbooks/code-review.json
owo-agent playbook history --json playbooks/code-review.json
owo-agent playbook diff playbooks/code-review.json --from 2026-11-18-a --to 2026-11-18-b   # 也显示失败类型变化（timeout → exit_nonzero）
# 标记也可用 latest / previous / oldest；--to current 直接和当前状态比
owo-agent playbook diff playbooks/code-review.json --from previous --to latest

# 机器可读（status / next / record / reset / list / init / archive / history / diff 都支持）
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
| `playbook_check { file, step }` | **execute** | 执行 `check` 步骤声明的 `command`：exit 0 记通过，否则记失败；内部复用 `run_command` 的沙箱/审计 |
| `playbook_run { file, max_steps? }` | **execute** | 连续执行所有可直接运行的 `check` 步骤（一次调用一次审批）；遇到模型/人工步骤、无命令的 check、失败或达到上限即停，返回已执行清单与下一步 |

模型循环：`playbook_next` → 多个连续 `check` 用 `playbook_run` 一次跑完（单个用
`playbook_check`），需要模型的步骤用既有工具做、再用 `playbook_record` 记录 → 再 `next`。
失败步骤保留在可执行集里（`attempts` 累计），是否继续重试由模型或推进策略决定。
`playbook_next` 返回的步骤带 `warnings`（高风险命令提示）——模型在执行前就能看到，
但提示不等于授权，执行仍走审批。

**派发日志（durable 语义的最小实现）**：`playbook_check` / `playbook_run` 在执行前写
`<剧本>.journal.jsonl` 的 `start`、完成后写 `end`。若进程在两者之间退出，下次
`playbook_next` 返回 `pending_unknown`，`check` / `run` 会**拒绝重跑**并提示先用
`playbook_record` 人工确认（人工记录即终态）；这样避免"命令已产生副作用却因崩溃被重跑"。
日志超过 256 KB 时自动压缩为最近 200 条，长跑剧本不会无限增长。

安全边界：路径经会话工作区约束（越界拒绝）；读取上限 256 KB、备注上限 2000 字符；
`playbook_record` 的副作用登记为 Write，照常走审批。

**失败转规则（样板）**：`post_tool_use` 是通知型钩子，可把 `playbook_check` /
`playbook_run` 的失败落成待转清单，人工复核后变成 `check` 断言或调整
`max_attempts`——这是"事故 → 检查项"的最小闭环：

```json
{
  "hooks": [
    {
      "event": "post_tool_use",
      "matcher": "playbook_*",
      "command": "python docs/examples/hooks/record-playbook-failure.py learned-failures.md"
    }
  ]
}
```

脚本见 `docs/examples/hooks/record-playbook-failure.py`：只记录失败（ok=false）的
`playbook_check`/`playbook_run`，追加到清单文件；不阻断、不改结果（退出码恒 0）。

## 6. 与 `/advance` 协同（剧本驱动的自主推进）

```text
/advance --playbook playbooks/code-review.json
```

前提：工具组已启用（`tool_capabilities.playbooks: true`，见 §5）。

行为：

- 宿主每轮读取剧本状态，把**当前步骤（id/标题/判据/命令/重试预算/上次失败类型）与进度**拼进推进提示；
- 模型只推进这一步，并用 `playbook_check` / `playbook_run` / `playbook_record` 记录；
- **完成由宿主按剧本状态判定**（不是模型自报）：全部步骤 passed/skipped 才结束；
- 某个步骤失败导致下游阻塞且没有可执行步骤时，循环停下并说明；
- 失败/停滞仍走 `/advance` 的推进策略（有界重试 + 停滞检测）；
- `/goal status` 会额外显示剧本整体进度与下一步（仅 `--playbook` 会话内）。

这样"按什么顺序做"由剧本决定，"不空转"由推进策略决定，两者互不越界；
`/goal` 不受影响。

## 7. 安全边界

- 本模块**不执行任何命令**：`check` 步骤的 `acceptance` 只是文本判据，真正执行必须由
  宿主经 Tool Host（权限/审批/审计/沙箱一个都不能绕）。
- `validate` 对 `check` 命令做**风险提示**（`rm -rf`、`curl | sh`、`mkfs` 等
  `check_command_risky`）：**附替代建议**，默认不阻断，`--strict` 才视为失败；
  列表/面板也会显示"高风险命令 N 条"。提示不是授权，执行前仍需审批。
- 输入按上限截断（步骤数、单步文本长度由 schema 控制），不做递归解释。
- 文件读取容忍 UTF-8 BOM（Windows 编辑器常见），不改变内容语义。

## 8. HTTP 面（桌面面板后端）

只读写**工作区内**剧本与其同目录状态文件，不执行命令：

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/playbooks` | 工作区 `playbooks/` 目录总览（含进度与 `next`；非法条目带 `issues` 不中断） |
| GET | `/playbooks/status?file=` | 单个剧本逐步状态（`file` 为工作区内相对路径） |
| GET | `/playbooks/history?file=` | 归档列表（含每步状态，前端可本地对比两次运行） |
| POST | `/playbooks/archive` | `{file, marker?}`：把当前状态快照到 `<剧本>.runs/`（面板「保存快照」；默认 `ui-<时间戳>`） |
| POST | `/playbooks/init` | `{id}`：在工作区 `playbooks/` 新建三步骨架（面板「新建剧本」）；重名/非法 id 400 |
| POST | `/playbooks/reset` | `{file}`：先保存 `reset-<时间戳>` 快照，再清空状态与派发日志（面板「重置」，二次确认） |
| POST | `/playbooks/record` | `{file, step, outcome: pass\|fail\|skip, note}`，写 `<剧本>.state.json` |

契约已同步：`crates/owo-agent-server/src/openapi.json`、`clients/ts/openapi.json`、
`clients/ts/src/schema.d.ts`，并由 `route_contract_tests` 校验可达与一致性。
依赖注入测试见 `crates/owo-agent-server/tests/playbook_api_tests.rs`。

**路径安全**：只接受工作区内路径（相对路径或工作区内绝对路径）；`..` 逃逸、
工作区外路径、`*.state.json` 与非 `*.json` 一律 400。列表下发的 `path` 是
**工作区相对路径**（正斜杠），客户端可原样回传给 status/record。

桌面「任务剧本」面板（`desktop/web/panels/playbook.panel.js`）已接入：工作台
「自动化 → 任务剧本」。列表显示进度与下一步；展开后逐步展示状态/命令，并提供
通过/失败/跳过记录、新建剧本、保存快照、重置进度（二次确认 + 自动快照）、点击历史行做「该归档 → 当前」的逐步对比，或用下拉选择两次归档直接对比（均含失败类型变化）
（命令执行仍由 Agent 的 `playbook_check` 完成，面板不提供执行入口）。

## 9. 下一步

1. **执行驱动**：`playbook run` 按 `next_ready()` 推进；`check` 步骤经 Tool Host 执行并
   记录证据，`model` 步骤走一次 Agent 回合，`human` 步骤挂审批；
2. **运行记录**：`PlaybookRun` 落盘到 `<data>/playbooks/runs/`，支持中断续跑；
3. **与 `/advance` 协同**：`/advance` 负责"按进度不空转"，剧本负责"按什么顺序做"；
4. **桌面面板**：剧本列表 + 运行进度（见 `docs/UI-SURFACING.md` 的 surfacing 清单）。
