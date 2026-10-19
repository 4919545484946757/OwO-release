# 对标现代 Agent 宿主：能力矩阵与迭代路线（阶段二）

> 阶段一（微内核拆分）见 `docs/architecture/ARCH-MICROKERNEL.md`；本文是**拆分收口后**的模块级优化迭代依据。
> 事实优先级：当前源码与真实运行证据 > 本文 > 计划。
>
> 状态：**起草（2026-09-20）**。第 1 节的能力清单已按源码核对；第 2 节"目标"一列
> 中标注 **待定** 的行需要先确认对标基准（见第 5 节）。

---

## 1. 现状清单（按源码核对，不是印象）

| 维度 | 现状 | 证据 |
|---|---|---|
| 工具面（内置） | ≈30 个：文件 `read_file`/`write_file`/`list_dir`/`search_files`；命令 `run_command`；委派 `subagent`/`explore`；技能 `use_skill`；浏览器 `browser_screenshot`/`browser_download_image`/`browser_close`；桌面 `desktop_*`（12 个，含窗口列表/OCR/等待/快捷键）；视觉 `screen_ocr`/`screen_vision`/`ocr_region`/`vision_ground`/`vision_verify` | `crates/owo-agent-core/src/tools.rs`（`register_file_read_tools`/`register_delegation_tools`/`register_browser_tools`/`register_mcp_tools`）与各模块 `ToolSpec` |
| 工具面（外部） | MCP（stdio + HTTP）已打通，schema 预算压缩、健康熔断、限流 | `owo-agent-mcp`（M10）、`mcp_health`（M13 归位 policy）、`schema_budget` |
| 权限与审批 | 四维规格（filesystem/command/network/persistence）、profile、grant 凭证、审批中心、deny 优先、只收紧不放宽 | `owo-agent-policy`（M12）、server `permissions_center_api`（7 测试） |
| 会话与上下文 | 压缩（保留 system + 近期尾部、工具调用与结果不拆）、undo/redo、rewind、fork、模型覆盖、SQLite 持久化 | core `session`/`agent`、server `session_api`/`turn_api` |
| 编排 | goal（计划→步骤→重规划）、workflow（DSL + 审批 + 检查点回滚）、workswarm（团队/角色 DAG）、fleet（远程节点）、subagent（深度限制） | core `goal`/`workflow`/`workswarm`/`fleet`/`subagent`、server 对应 api |
| 记忆与技能 | 语义记忆（JSONL + 剪枝 + 检索）、观察流、操作学习与主动建议、`.owskill` 技能包分享、`skills/` 内置 5 个（browser/documents/pdf/spreadsheets/user） | `owo-agent-memory`（M11）、`skills/` |
| 插件 | 清单 + 签名校验 + 风险扫描 + 生命周期（安装/启用/更新/回滚/撤销）、市场（本地 + 远端 registry） | `owo-agent-plugins`（M7）、server `plugin_api`/`plugin_market_api`（16 测试） |
| 客户端形态 | Rust CLI（40+ 子命令）+ TUI、**Electron 桌面壳**（ADR-003：由 Tauri 收敛而来，经 Tauri 兼容桥复用同一份 Web UI）、Web UI（panels/permissions/views）、TypeScript SDK（含 openapi.json + 单测） | `crates/owo-agent-cli`、`desktop/electron`、`desktop/web`、`clients/ts` |
| 可观测性 | `/health`、Prometheus 指标、SLO 五条基线 + 错误预算、请求台账（隐私安全）、traces、审计链（HMAC + 锚点，可导出可验签） | server `observability_api`（26 测试）、`request_ledger_api`、`traces_api`、`owo-agent-tool-safety::audit_chain`（26 测试） |
| 评测 | ProductEval devtool（独立 workspace，20/20 参考运行）、配对对照报告、dataset builder | `devtools/product-eval`（M1）、server `product_eval_api`（10 测试） |
| 安全边界 | 沙箱（Job Object + AppContainer + 网络策略）、拒绝即不执行、拒绝必留收据、写前快照/diff/revert、无半写 | `owo-agent-tool-safety`（M3）、core `execution_boundary_contract_tests`（4 条契约） |

**已经明显领先多数同类产品的三项**：Windows 桌面自动化 + UIA/OCR/视觉感知链、
权限四维规格与审批中心、审计链与 SLO 观测。阶段二不该在这三处"对齐别人"，而该
把它们做成可复用的对外能力。

---

## 2. 差距矩阵（对标 DSH / Codex 一类现代 Agent 宿主）

> "目标"一列里，**带 ✅ 的是我已能确定的差距**（因为与具体产品无关、是这类宿主的
> 共同下限）；**待定** 的需要先确定对标基准。

| # | 能力 | 现状 | 目标 | 差距性质 | 验收方式 |
|---|---|---|---|---|---|
| G1 | **精细编辑工具** | 只有整文件 `write_file` | ✅ 段级/补丁级编辑（定位-替换，失败可重读重试），避免"整文件重写" | 工具缺失 | 新增工具 + 契约测试：只改目标片段、其余字节不变；编辑失败返回可读原因不静默 |
| G2 | **路径发现工具** | `list_dir`/`search_files` 有，缺 **glob** | ✅ 按模式找文件（`**/*.rs`），结果按 mtime 排序 | 工具缺失 | 工具单测 + 在大仓库上跑（返回条数与内容正确） |
| G3 | **后台任务** | 无（长命令只能同步等或靠 `run_command` 超时） | ✅ 起/查/停后台作业，输出可增量读取 | 机制缺失 | 端到端：起一个长任务→立刻返回 id→读输出→kill 无残留进程 |
| G4 | **目标/循环的模型侧入口** | **模型可见工具面里没有 goal/workflow/plan 工具**（已全仓扫 `name: "..."` 无匹配）；规则从服务端 API 与 intent 层进（`command_run_create_goal`） | ✅ 模型可自己把长任务交给目标/循环，并收到结构化回收（进度、当前步骤、终态） | 工具缺失（机制已有） | 加 2–3 个工具（起目标/查进度/取消）+ 契约测试；端到端：模型一次调用→目标跑到终态→模型拿到结构化结果 |
| G5 | **指令文件（AGENTS.md 类）** | **已实现**：`load_project_rules` 读工作区根的 `AGENTS.md`/`CLAUDE.md` 并注入 system prompt（`agent.rs:400` ← `owo-agent-contracts::context`），另有 CLI `/init` 生成模板 | ✅ 与 Codex/Claude Code 同级的**子目录层级**规则发现与优先级合并 | 只差层级合并（根目录已具备） | 契约测试：多级 `AGENTS.md` 的合并顺序与覆盖关系；运行时证据：上下文快照可见注入内容 |
| G6 | **技能装载** | `skills/` 5 个（browser/documents/pdf/spreadsheets/user）+ `use_skill` 工具 | ✅ 按需装载、渐进披露（先给目录再给正文），不一次性塞满上下文 | 上下文预算 | 上下文占用对比（装载前后 token 数），技能可被正确调用 |
| G7 | **客户端一致性** | CLI/TUI/桌面/Web/TS SDK 并存 | ✅ 单一权威 Daemon + 薄客户端（指南 §0.1），UI 只做渲染与审批 | 架构收敛 | 源码搜索：客户端不再直接 `Agent`/`SqliteSessionStore`；同时开桌面+CLI 不产生第二套 runtime |
| G8 | **客户端扩展（UI 插件）** | 能力卡允许 `kind ∈ {ui, cli, http}`（`server/src/capabilities.rs:50`），**UI 侧渲染待核** | ✅ 插件可注册 UI tab/命令（指南 §2.3 已列为插件能力） | 半实现（声明有、渲染待核） | 装一个 UI 插件→界面出现入口→卸载即消失 |
| G9 | **会话恢复/检查点** | rewind/fork/undo/redo 有；**CLI 已有 `resume`**（`cli/src/handlers.rs:494`，repl/tui 都有入口） | ✅ 重开进程继续上次会话，含待审批项与未完成任务 | 已具备，需补"待办恢复"的验证 | 端到端：中断→重启→`resume`→上下文与待审批项都恢复 |
| G10 | **审批体验** | 审批中心 + grants 有 | ✅ 审批可"一次性/本会话/本 workspace/永久"分级，且**拒绝也要留收据并回灌模型** | 已基本具备，需产品化 | 现有测试 + UI 走查 |
| G11 | **成本/配额透明** | usage 四维 + 预算 + 硬停（402）、评测成本单价可选 | ✅ 每个 turn 可见 token/成本，超预算前预警 | 已基本具备 | 现有 `usage_tests`（6 条）+ 报告字段 |
| G12 | **编辑器内集成（Hermes 类 agent IDE 的核心）** | **完全没有**：全仓无 VS Code/JetBrains 扩展工程、无 LSP（858 个源文件里 `vscode`/`language.server` 只有 9 处无关命中）。现有形态是「CLI/TUI + Tauri 桌面 + Web 控制台」 | ✅ 编辑器内：选中区/光标上下文作为输入、内联 diff 与逐 hunk 接受/拒绝、诊断信息回灌 | **整块缺失**（最大差距） | 先做 G7（薄客户端）→ 再做扩展；验收：在 VS Code 里选中一段代码→让 agent 改→逐 hunk 审阅→接受后文件落盘可撤销 |
| G13 | **变更审阅（per-hunk）** | 服务端已有 `change_set`（写前快照/diff/revert）+ artifact review 状态机；Web UI 有 diff 渲染（`workswarm/format.js` 19 处、`workswarm/render.js` 55 处） | ✅ 逐 hunk 接受/拒绝，拒绝可回灌模型让其改法 | 服务端已具备，UI 只到"整份审阅" | 契约测试落在 `change_set`；UI 侧端到端 |
| G14 | **代码库索引/符号检索** | 只有文本搜索（`search_files`）+ 语义记忆；无符号级索引 | ✅ 符号/引用级检索，大仓库下可控延迟 | 缺失 | 索引构建时间与查询延迟基准；命中率对照文本搜索 |
| G15 | **光标/选区上下文协议** | 无（没有编辑器就谈不上） | ✅ 客户端把 active file / selection / diagnostics 作为结构化上下文随请求上传 | 依赖 G12 | 协议契约测试（context envelope 的字段与上限） |

---

## 3. 迭代顺序（建议）

按"先修共同下限、再放大力气在差异化，最后做客户端收敛"排：

1. **W1 工具面补齐**：G1（精细编辑）→ G2（glob）→ G3（后台任务）。
   这三项是所有现代宿主的共同下限，且改动面小、可直接写契约测试。
2. **W2 上下文与指令**：G5（AGENTS.md 注入，先核后补）→ G6（技能渐进披露）。
   收益是"同样模型更少 token、更少跑偏"。
3. **W3 长任务与编排**：G4（目标/循环的模型侧入口）→ G9（会话 resume）。
4. **W4 客户端收敛**：G7 → G8（见指南 §9 A2/A5/A7；拆分阶段已经为此铺路：
   core 已经从 64,701 行降到 36,787 行，Daemon 边界比 M0 时清晰得多）。
5. **W5 差异化放大**：把桌面自动化 + 权限规格 + 审计/SLO 做成对外可卖的能力
   （不是"对齐 Codex"，而是"Codex 没有的东西"）。
6. **W6 编辑器内集成**（对标 Hermes 的关键一步，**前置是 W4 完成**）：
   G12 编辑器扩展 → G15 编辑器上下文协议 → G13 逐 hunk 审阅。
   顺序理由：扩展必须是**薄客户端**——如果在扩展里再起一套 runtime，就会踩
   指南 §2.4 第 6 条（壳对子进程的生命周期）与 §0.1（只能有一个权威 Daemon）。
   验收形态：在 VS Code 里选中一段代码 → 要求 agent 改 → 内联看到 diff →
   逐 hunk 接受/拒绝 → 接受的写盘、拒绝的回灌模型换改法；
   同时开编辑器与桌面 UI，**不产生第二套 runtime**（源码搜索 + 运行时 pid 证据）。

---

## 4. 每个模块的验收标准（统一模板）

每个 W 项落地时必须同时给出：

```text
1. 契约测试：新能力的行为边界（含拒绝路径），放在对应 crate 的 tests/ 下；
2. 目标化测试：只跑相关测试目标（scripts/mk-tests-sharded.ps1 -Package <crate> -Tag <tag>），
   受 §2.4 资源红线约束（-j 1 / --test-threads=1，内存与磁盘门禁）；
3. 运行态证据：scripts/mk-smoke.ps1 或专用端到端脚本，落 docs/qa/evidence/；
4. 文档：能力进入 docs/ 的对应章节，并记录"与对标产品的差异点"（我们保留了什么、为什么）。
```

---

## 5. 对标基准（已确认）与顺序

**对标基准**（2026-09-20 确认）：

* **DSH**（DeepSeek Harness）：CLI + TUI + Web GUI 的宿主形态，工具面/子代理/目标/后台作业。
* **Hermes**：**agent IDE 类工具** —— 核心是"编辑器内的 agent"。这决定了 G12–G15
  （编辑器集成、逐 hunk 审阅、索引、编辑器上下文协议）是**必做项而不是可选项**，
  且它们的前置是 G7（薄客户端 + 单一 Daemon）。
* **Codex**（OpenAI）：CLI + IDE 扩展 + 沙箱/审批模式；其补丁式编辑（G1）与
  指令文件（G5，已具备根目录部分）是共同下限。

**顺序**：默认按"先拆分收口（M14 Perception Worker 等）→ 再 W1–W6"。理由是拆分每收一步，
后面的模块级优化就少一层 `crate::` 纠缠（M13 已证明：被依赖的域先搬走，后来者是零成本）。

### 5.1 已核查结论（避免重复讨论）

| 曾经担心的 | 核查结论 | 证据 |
|---|---|---|
| AGENTS.md 是否只在 `/init` 里生成、运行时不用？ | **运行时已注入**：`load_project_rules` 读工作区根的 `AGENTS.md` + `CLAUDE.md`，拼进 system prompt | `crates/owo-agent-contracts/src/context.rs`、`crates/owo-agent-core/src/agent.rs:400` |
| 会话能不能跨进程续跑？ | **能**：CLI 有 `resume`（repl / tui / handlers 三处入口） | `crates/owo-agent-cli/src/handlers.rs:494` |
| 模型能自己起 goal/workflow 吗？ | **不能**（模型可见工具面里没有这些工具名）——这是 G4 的真实差距 | 全仓扫 `name: "<tool>"` 无 goal/workflow/plan 匹配 |
| 插件能不能带 UI 入口？ | **声明层已允许**（能力卡 `kind ∈ {ui,cli,http}`），渲染层待核 | `crates/owo-agent-server/src/capabilities.rs:50` |
| 有没有编辑器扩展 / LSP？ | **没有**。858 个源文件里 `vscode`/`language.server`/`lsp` 仅 9 处无关命中（在白名单与前台应用识别里），也没有 VS Code/JetBrains 扩展目录 | `desktop/`、`clients/`、`crates/` 目录扫描 |
| Web UI 到什么程度？ | 是一个**功能完整的控制台**（15 个面板：workswarm 3,779 行 / eval 1,158 / action-center 1,027 / project-launcher 931 / project-history 679 / observability 537 …），但它是"控制台"不是"编辑器" | `desktop/web/panels/`、`desktop/web/core/` |

---

## 6. 与阶段一的衔接点

| 阶段一的产出 | 对阶段二的作用 |
|---|---|
| `owo-agent-policy`（M12） | G10 审批分级的落点：规格/凭证/效应判定已经独立成 crate，加"永久授权"不必再动 core |
| `owo-agent-memory`（M11） | G6 技能与记忆的落点；`share_skill` 归位后技能包装载只有一个入口 |
| `owo-agent-mcp`（M10） | G3 后台任务可复用它已验证的 stdio/HTTP 传输与超时重连语义 |
| `owo-agent-tool-safety`（M3） | G1/G3 的执行侧必须经它（拒绝即不执行 + 拒绝必留收据），不能绕 |
| `mk-tests-sharded.ps1` / `mk-check.ps1` / `mk-smoke.ps1` | 第 4 节的验收模板直接复用，已是可复现命令 |
