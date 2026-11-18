# 变更记录

> 记录用户可见的能力变化。内部进度索引见 `docs/PROGRESS-2026-11-18.md`，
> 前沿调研见 `docs/TRENDS-2026-11.md`。功能状态以当前源码与测试为准。

## 2026-11-18（SharedCore 多 Agent 共享上下文）

### 新增

- **SharedCore 插件**（`plugins/owo-sharedcore`，Agent Plugins 1.0.0）：项目级共享上下文
  （规则/架构/契约/任务 DAG/事件）落在 `<workspace>/.sharedcore/`。
- 10 个 MCP 工具：`init` / `status` / `context` / `task_add` / `plan` / `claim` /
  `update` / `contract` / `event` / `brief`。
- 并行调度：依赖检查 + 写范围前缀冲突检测 + 并行批次（`max_parallel`）；契约提案/接受使用
  `expected_version` CAS；事件只追加。
- 使用方式：启用插件后像普通对话一样说"用 SharedCore 并行开发……"，技能指导主 Agent
  拆任务、生成 brief、用 `fanout_subagents` 并行执行。
- 自检：`scripts/sharedcore-selfcheck.py` 端到端覆盖批次/冲突/CAS/简报；宿主实测 MCP
  已连接（tools=10）。
- 能力目录新增 `agents.sharedcore`（Beta）。

## 2026-11-18（工作流与插件治理）

### 新增

**插件生态**

- Agent Plugins 1.0.0 兼容层：`plugin.json` + `skills/` + `mcp.json`，闭模式清单校验、
  窄失败边界（拒绝插件 / 废组件 / 跳过技能 / 跳过服务器）、路径包含检查。
- 插件可声明多个 MCP 服务器（stdio / streamable-http），支持 `env` / `cwd` / `headers`。
- 插件贡献 hooks：`com.owo.agent` 命名空间（清单内联或 `hooks.json`）。
- 插件页显示「已签名」状态（只报有无，验签在安装/更新/check 时做）。
- 规范包可选签名：`plugin.sig.json`（ed25519 覆盖包摘要）；作者侧 `plugin sign`，
  `plugin check` 显示验签状态；市场安装/更新要求签名有效。
- 市场可安装/更新规范包；吊销列表对两种格式生效；`plugin rollback` 从备份恢复旧版。

**任务剧本（工作流）**

- 剧本格式：`plan` / `check` / `human` 步骤，依赖、判据、命令、`max_attempts`。
- CLI：`validate`（文件/目录，`--strict`）、`plan`、`next`、`record`、`status`、`reset`、
  `list`、`init`（骨架或 `--from` 派生）、`archive`/`history`/`diff`（支持 latest/previous/oldest 与 `--to current`）、`export`/`import`（单文件包，含状态与归档）。
- 模型工具组（可选）：`playbook_next` / `playbook_record` / `playbook_check` / `playbook_run`。
- `/advance --playbook`：宿主按剧本推进，完成由宿主判定；提示含重试预算与上次失败类型。
- HTTP：`GET /playbooks`、`GET /playbooks/status`、`GET /playbooks/history`、
  `POST /playbooks/record`、`POST /playbooks/archive`。
- 桌面「任务剧本」面板：新建剧本、进度、逐步记录、命令复制、保存快照、重置进度（二次确认 + 自动快照）、历史对比（含两次归档对比）、风险与失败类型。
- 模板：code-review / release-check / bugfix / weekly-report；目录批量校验。

**耐久语义**

- 派发日志：check 执行前写 `start`、完成后写 `end`；崩溃后 `pending_unknown` 可见，
  check/run 拒绝重跑并提示人工确认。
- 失败分类：`exit_nonzero` / `timeout` / `command_unavailable`，status/diff/面板可见，
  并映射为恢复建议（`failure_recovery_hint`）。
- 推进受阻时列出重试耗尽的步骤，并给出「人工确认或提高 max_attempts」的建议。
- 完成时自动留档 `complete-<时间戳>`；失败重试达上限后不再进入可执行集；派发日志超过 256 KB 自动压缩。

**hooks**

- `post_tool_use` 真正派发（通知型，payload 含 tool/args/session_id/ok/error；exit 2 只记审计）。
- 失败转规则样板：`docs/examples/hooks/record-playbook-failure.py`。

### 改进

- 功能目录登记「Agent Plugins 规范包」与「任务剧本」两项能力（CLI 与面板同源）。
- 风险提示覆盖 6 类危险命令并附替代建议；`playbook list/plan/next/status` 均可见。
- 所有剧本 `--json` 输出可被机器解析（修复目录校验 stdout 混入提示）。
- `/playbooks` 与 `/playbooks/history` 读取上限 500 条，超出返回 `truncated`，避免超大目录读放大。
- 状态/归档/剧本文件读取容忍 UTF-8 BOM（Windows 编辑器）。
- 工具面 token 预算回归阈值：默认 ≤6k、剧本组增量 ≤0.8k、全能力 ≤10k。

### 修复

- 市场安装规范包时报「缺少 manifest.json」。
- 吊销列表在市场侧从未加载、规范包校验也不检查吊销。
- `update` 的备份目录名用了新版本（应记录被替换的旧版本）。
- CLI 无法安装未签名包（现与市场共用 `OWO_PLUGIN_REQUIRE_SIGNATURE=0`）。
- `plugin.json` 带 BOM 时误报 JSON 非法。
- `post_tool_use` 配置后从不触发。
- 桌面全量测试的三处漂移（导航路由数、CRLF 提取）。

### 验证

- Rust：**1 114** 条（contracts 59 / policy 72 / core 711 / plugins 43 / mcp 17 / cli 127 /
  playbook-api 11 / plugin-list-signed 1 / route-contract 54 / plugin-market 19）。
- 桌面：`node --test` **811** 条（71 个文件）。
- 真实页面：插件页、任务剧本列表/记录/归档/历史对比/风险提示均有截图证据
  （`docs/qa/evidence/`）。

### 兼容性

- `/goal` 语义与默认工具面不变；剧本工具需显式开启 `tool_capabilities.playbooks`。
- 旧剧本与旧状态文件无需迁移（新字段均可选、None 不序列化）。
- HTTP 只读写剧本与状态文件，不执行命令；执行仍经 Tool Host 审批。
