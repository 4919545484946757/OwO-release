# SharedCore 插件实现计划

> 参考：`docs/goal/多agent共享上下文设计方案.md`（v1.0，569 行）。
> 形态：**OwO Agent Plugin 1.0.0 规范包**（`plugin.json` + `skills/` + `mcp.json`），
> 启用即可用，不需要改宿主默认行为。

## 1. 目标拆解

| 设计文档模块 | 本仓库实现方式 | 阶段 |
|---|---|---|
| Context Hub（五层上下文 + 版本） | `.sharedcore/` 下的结构化 JSON：`project.json` / `tasks.json` / `contracts.json` / `events.jsonl` | P0（本轮） |
| Task Scheduler（任务 DAG） | `sharedcore_plan`：依赖检查 + 写范围冲突检测 + 并行批次 | P0（本轮） |
| Agent Runtime | 复用宿主已有的 `subagent` / `fanout_subagents` 工具；共享上下文以 **brief** 注入子 Agent | P0（本轮） |
| Workspace Manager | 复用宿主 `worktree` 能力（Agent 用命令创建）；任务记录 `branch`/`worktree` 字段 | P1 |
| Event Bus | `events.jsonl` 追加式事件；`sharedcore_event` + 状态迁移自动写事件 | P0（本轮） |
| Integration Gate | P1：合并前校验（范围/契约版本/测试命令），宿主命令执行 | P1 |
| 持久化/恢复 | JSON 原子写（临时文件 + rename）+ `context_revision` 乐观并发 | P0（本轮） |

## 2. 插件形态

```
plugins/owo-sharedcore/
├── plugin.json                 # Agent Plugins 1.0.0 清单
├── mcp.json                    # stdio MCP 服务器（python 标准库）
├── servers/sharedcore_server.py
└── skills/sharedcore/SKILL.md  # 何时用、怎么用、边界
```

- **随时启动**：`owo-agent plugin install ./plugins/owo-sharedcore`（或市场安装）后
  `/mcp reload` 即可；不需要时禁用插件即可移除工具面。
- **像普通对话**：用户只需说"用 SharedCore 并行开发……"，技能指导主 Agent 完成
  init → 拆任务 → plan → 生成 brief → `fanout_subagents` → 回收结果 → 下一批。
- **不共享对话**：子 Agent 拿到的是任务契约 + 相关规则/契约切片（brief），不是整段聊天。

## 3. 工具集（第一版）

| 工具 | 作用 |
|---|---|
| `sharedcore_init` | 建立项目共享上下文（目标/架构/规则/基线 commit） |
| `sharedcore_status` | 项目摘要、任务计数、可运行任务、阻塞项、最近事件 |
| `sharedcore_context` | 读取核心规则 + 契约 + 指定任务（有界输出） |
| `sharedcore_task_add` | 新增任务（依赖/读写范围/验收） |
| `sharedcore_plan` | 计算就绪任务与并行批次，检测写范围冲突 |
| `sharedcore_claim` | 领取任务（owner/branch/worktree） |
| `sharedcore_update` | 状态迁移（done/failed/blocked/running），自动记事件 |
| `sharedcore_contract` | 契约提案/接受（expected_version CAS） |
| `sharedcore_event` | 追加共享事件（契约变更/阻塞/冲突） |
| `sharedcore_brief` | 生成给子 Agent 的结构化任务简报 |

## 4. 一致性规则

- 所有写入：写临时文件后 `os.replace` 原子替换；`context_revision` 递增。
- 契约更新携带 `expected_version`，不匹配即拒绝（避免覆盖他人决策）。
- 事件只追加、不可修改；`event_id` 单调（`evt-<revision>-<seq>`）。
- 写范围冲突：两个就绪任务的 `write_scope` 有路径前缀重叠时，不放进同一并行批次。

## 5. 验收

1. `owo-agent plugin check plugins/owo-sharedcore` 通过（清单/技能/MCP 合法）。
2. MCP 直连测试：`initialize` → `tools/list` → `init` → `task_add` ×3 →
   `plan`（依赖分批正确）→ `brief`（含规则与写范围）→ `update` → `status`。
3. 插件被宿主发现：`owo-agent plugin catalog` 列出。
4. 下一步（P1）：Integration Gate、worktree 自动管理、Web UI 任务看板。

## 6. 与既有能力的边界

- 不新造 Agent 运行时：并行执行一律走宿主 `subagent`/`fanout_subagents`（权限/审计不绕过）。
- 不新造会话库：共享上下文是**项目级结构化状态**，不是聊天记录。
- 默认不执行任何命令：`sharedcore_*` 只读写 `.sharedcore/`；git/测试仍由宿主工具执行。

## 7. 实施进度

### 已完成（2026-11-18，第 1 阶段）

- `plugins/owo-sharedcore/`：`plugin.json` + `mcp.json` + `skills/sharedcore/SKILL.md` +
  `servers/sharedcore_server.py`（Python 标准库，stdio JSON-RPC）。
- 10 个工具：`init / status / context / task_add / plan / claim / update / contract / event / brief`。
- 一致性：原子写、`context_revision` 递增、契约 `expected_version` CAS、事件追加。
- 调度：依赖检查 + 写范围前缀冲突检测 + 并行批次（`max_parallel`）。
- 验证：
  - `owo-agent plugin check plugins/owo-sharedcore` 通过（技能 1 / 服务器 1 / 诊断无）；
  - `owo-agent plugin catalog` 列出（`[agent-plugins-1.0.0][clean]`）；
  - `scripts/sharedcore-selfcheck.py` 端到端通过（含批次/冲突/CAS/简报断言）。

### 下一步

1. 用真实 `fanout_subagents` 跑一次并行开发演示（需要模型凭据）；
2. Integration Gate（合并前检查范围/契约/测试命令）与 worktree 管理；
3. 桌面 UI：任务看板（已完成第一版）与集成门状态展示；
4. UI 布局重构（对标 Codex / DSH）：会话页信息层级、设置页、工具页。

### 已完成（第 2 阶段，同日）

- 服务端只读数据面 `GET /sharedcore/state`：项目上下文 + 任务 + 契约 + 最近事件 +
  依赖批次/写范围冲突计算（Rust `sharedcore_api`，2 条契约测试）。
- 桌面「SharedCore」面板：规则/架构、并行批次、阻塞、冲突、任务、契约、事件；
  真实 daemon 验证渲染（截图 `docs/qa/evidence/ui-sharedcore-panel-20261118.png`）。
- 导航：工作台「自动化」分组新增入口（25 个面板），桌面全量 815 条通过。

## 8. 验收记录（2026-11-18 14:35）

| 项 | 结果 |
|---|---|
| `plugin check plugins/owo-sharedcore` | 通过（技能 1 / 服务器 1 / hooks 0 / 诊断无） |
| `plugin catalog` | 列出，`[agent-plugins-1.0.0][clean]` |
| `scripts/sharedcore-selfcheck.py` | 通过：12 工具、依赖分批、写范围冲突、契约 CAS、worktree 命令、gate 阻断与 allow_partial |
| 宿主加载 | daemon 日志：`MCP 服务器 owo-sharedcore/sharedcore 已连接，工具 10 个`（安装验证时的版本；当前工具数 12） |
| Rust 回归 | 定向全量 1114 通过（含 playbook/route/market/server）；`sharedcore_api_tests` 另 2 条通过 |
| 桌面回归 | **815** 通过（含 SharedCore 面板 3 条与导航 25） |
| UI 证据 | `docs/qa/evidence/ui-v2-*.png` 共 9 张（主界面/剧本/SharedCore/设置/插件市场/首页/能力/命令面板） |
| 未验证 | **真实模型下的并行回合**：本机未配置模型提供商，`fanout_subagents` 的实际并行执行未跑；插件自检与宿主加载已覆盖协议层 |

说明：`使用和普通对话一样` 依赖技能把流程写进主 Agent 的提示；实际对话回合需要模型凭据，
本次以自检 + 宿主加载 + 协议单测作为证据，不把"未运行"说成"通过"。
