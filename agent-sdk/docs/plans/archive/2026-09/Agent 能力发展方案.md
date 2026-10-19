# OwO Agent 能力发展方案（v1.0）

> 日期：2026-09-24
> 范围：`crates/owo-agent-core` + `crates/owo-agent-server` + `desktop/web`
> 依据：① 对本仓库的逐模块审查 ② 对 Codex / Trae / OpenCode / Claude Code / Cursor 的公开架构调研
> 前置：`效能核心改造方案.md`（v1.0，已落地，工具层与提示词已达标）——本方案不再重复其内容

---

## 一、审查结论

### 1.1 现状盘点（已验证的强项）

**安全外壳**是当前最扎实的部分，已达同类产品第一梯队：

| 能力域 | 代表模块 | 状态 |
|---|---|---|
| 沙箱隔离 | `sandbox.rs`（IsolationLevel / NetworkPolicy / FileScope） | ✅ 分级隔离 + 平台探测 |
| 权限审批 | `permissions.rs`（Read/Write/Execute/Inject 四级 + deny 优先） | ✅ 矩阵完整，含运行时 deny 热生效 |
| 审计链 | `audit_chain.rs`（HMAC 链式记录 + 导出校验） | ✅ 防篡改，可离线验证 |
| 自动复核 | `autoreview.rs`（Heuristic → Model 两级链） | ✅ 已实现但未接入主循环 |
| 快照回滚 | `session.rs` snapshots + `/revert` `/rewind` `/redo` | ✅ 文件级可回退 |
| 凭据管理 | `credentials.rs`（Windows 凭据管理器 + 环境变量引用） | ✅ 密钥不落盘 |
| 注入防护 | `injection.rs`（工具结果消毒 + 严重度分级） | ✅ |
| 契约治理 | `lib.rs` 路由契约测试 + `/openapi.json` + `/schemas` | ✅ 防回归 |
| 多 agent | `fleet.rs` / `worker_pool.rs` / `goal.rs` / `subagent.rs` | ✅ 控制面完整 |
| 感知执行 | `perception.rs` / `computer_use.rs` / `learn.rs` / OCR / 视觉 | ✅ 独有能力 |
| 工具层 | `tools.rs`（read_file v2 / grep / edit_file / run_command v2） | ✅ 已完成效能改造 |
| 桌面工作台 | `desktop/web`（菜单栏 + 设置页 + 10 扩展面板 + 对话流） | ✅ Codex 风格已成型 |

**测试基线**：946 项通过 / 0 失败（`效能核心改造方案.md` 实施记录）。

### 1.2 差距清单（对标结论，按优先级排序）

| # | 差距 | 现状证据 | 对标做法 | 影响 |
|---|---|---|---|---|
| G1 | **审批策略单轴**：只有 `settings.read_only` 布尔 + 前端 localStorage 的 ask/auto/full | `permissions.rs` `is_read_only()`；`app.js` `ACCESS_MODES` 仅前端生效 | Codex 拆成 `approval_policy`（何时问）× `sandbox_mode`（能碰什么）两个正交旋钮，作用域随会话 | 前端切「完全访问」不影响服务端策略，多入口行为不一致 |
| G2 | **无 per-tool 规则 DSL**：无法表达「`git push *` 一律 ask」这类细粒度策略 | `Policy::level_for` 是硬编码 match 表 | OpenCode 的 `(action, resource, effect)` 三元组 + glob resource | 用户只能全放行或全询问，无法按命令/路径定制 |
| G3 | **自动复核未接入**：`autoreview.rs` 写好了但主循环没用 | `agent.rs` 未引用 `AutoReviewChain` | Codex `--approve-for-me`：低风险自动放行、critical 才升级给人 | 审批疲劳，用户被迫全放行（G1 的「完全访问」正是这个症状） |
| G4 | **项目规则无层级**：只读工作区根目录的 `AGENTS.md` / `CLAUDE.md` | `context.rs` `load_project_rules` 只 join workspace | Codex：global → git root → cwd 逐级拼接，closer wins，默认 32 KiB 上限 | monorepo 子包规则失效；规则过长无保护 |
| G5 | **压缩后无重注入清单** | `agent.rs` `maybe_compact` 只做摘要 + `compact_truncate` | Claude Code：根规则/plan/最近 5 个文件/已调用 skill 正文逐项定义恢复策略 | 压缩后模型「忘记」关键约束 |
| G6 | **无 hooks 机制**：确定性护栏缺失 | 无对应模块 | Claude Code `PreToolUse` 以 exit code 2 阻断 / `PreCompact` 落盘 / `SessionStart` 注入 | 模型不可信时缺少确定性兜底 |
| G7 | **缺少 `todo` 类工具**：长任务无显式进度载体 | `tools.rs` 无 todo 工具 | Claude Code `TodoWrite` / OpenCode `todowrite` | 长任务中途跑偏，用户看不到进度 |
| G8 | **前端无 Plan 模式**：计划无法呈现与批准 | `desktop/web` 无 plan 视图 | Trae Spec 模式 / Cursor Plan 模式：先出计划、批准后才写 | 大改动不敢交给 agent |
| G9 | **前端 diff 审阅粒度粗**：仅右侧检查器整体展示 | `app.js` `refreshDiffs` 按文件列 diff 文本 | Cursor：diff 逐块接受/拒绝 + `Restore Checkpoint` | 无法只接受部分改动 |
| G10 | **面板结果仍有裸文本**：eval 报告 / trace 回放 / 子代理结果为 `textContent` 拼接 | `app.js:2687/2761/2800` | 全项目「去裸数据化」既定方向 | 与既有 10 面板改造标准不一致 → **已修（见四·附）** |
| G11 | **无 skill 按需加载**：技能正文直接注入 | `skill.rs` | Trae / Claude：启动只注 name+description，命中才载正文（单 skill ≤5k token、合计 ≤25k） | 技能多了吃光上下文 |
| G12 | **无会话内斜杠命令**：`/new` 已硬编码，其余能力缺入口 | `app.js` prompt placeholder 仅提 `/new` | OpenCode `/compact` `/undo` `/redo` `/details` `/export` | 上下文治理能力不可达 → **已修（见四·附）** |

---

## 二、对标要点速览（调研摘要）

| 产品 | 最值得借鉴的一点 |
|---|---|
| **Codex** | 沙箱与审批解耦成正交旋钮；`AGENTS.md` 就近覆盖 + 32 KiB 硬上限；auto-review 子 agent 按风险分级升级 |
| **Trae** | 规则四种生效方式（Always / globs / description 智能判断 / 手动 `#` 引用）；SOLO 子 agent 独立上下文并行 + 自纠错 |
| **OpenCode** | headless 服务 + 瘦客户端（TUI/server 分离，`/tui/*` 反向驱动）；`(action, resource, effect)` 权限三元组；compaction 是独立维护型 agent |
| **Claude Code** | 压缩后「可枚举的重注入清单」；hooks 做确定性护栏（`PreToolUse` exit 2 阻断）；子 agent 只回传最终消息 |
| **Cursor** | 双轨检索（Instant Grep + 可选语义索引）；diff 逐块接受/拒绝 + `Restore Checkpoint`；plan 文件可人工编辑 |

**共性结论**：领先产品的差距不在「工具数量」，而在 ① 权限空间的正交表达 ② 上下文的确定性治理 ③ 人工可控的介入点（plan / diff / checkpoint）。

---

## 三、发展方案

### 3.1 设计原则

1. **契约冻结不破**：不改已有 `/openapi.json` 端点的请求/响应形状，新增能力走新增端点或新增可选字段（`#[serde(default)]`）。
2. **后端单一事实源**：策略、规则、计划、上下文状态一律后端持久化，前端只做投影——修正 G1 的根因。
3. **默认安全**：新增能力默认 `Ask`，显式配置才放行。
4. **可观测**：每个新决策点写审计（`audit_chain.rs`）。

### 3.2 P0 批次（权限空间正交化 + 自动复核接入）—— 修正 G1/G2/G3

**P0-1 策略模型升级（`permissions.rs`）**

```rust
pub struct Policy {
    workspace: PathBuf,
    read_only: Arc<AtomicBool>,          // 保留：Plan 模式硬闸
    deny_command_fragments: Vec<String>, // 保留：命令片段黑名单
    runtime_deny: Arc<Mutex<Vec<String>>>,
    rules: Arc<RwLock<Vec<PermissionRule>>>, // 🆕 动态规则层
}

pub struct PermissionRule {
    pub action: RuleAction,   // edit | shell | read | network
    pub resource: String,     // glob，如 "git push *" / "**/*.lock"
    pub effect: RuleEffect,   // allow | ask | deny
}
```

- 决策顺序：`read_only` 硬闸 → `deny` 规则 → `allow` 规则 → 级别默认（Read=Allow，其余=Ask）
- 新增 `POST /policy/rules`（读/写规则列表）、`GET /policy/rules`
- **验收**：单测覆盖 glob 匹配、deny 优先级、read_only 压倒 allow

**P0-2 会话级审批策略（`settings.rs` + `TurnRequest`）**

- `Settings` 新增 `approval_policy: ApprovalPolicy`（`ReadOnly` / `AutoEdit` / `FullAuto`），**服务端生效**
- 新增 `POST /session/{id}/policy`：会话级覆盖（不写全局设置）
- 前端 `ACCESS_MODES` 改为写服务端，取消 localStorage 单机状态
- **验收**：三档策略下同一 `write_file` 调用分别得到 Deny / Allow / Allow；`AutoEdit` 下 `run_command` 仍 Ask

**P0-3 自动复核接入主循环（`agent.rs`）**

- `AgentConfig` 新增 `reviewer: Option<Arc<dyn Reviewer>>`；`ChannelApprover` 在 `Ask` 决策前先过 `AutoReviewChain`
- 分级：`Safe` → 自动放行（写审计 `auto_approved`）；`Suspicious` → 转人工；`Critical`（网络外泄 / 凭据访问 / 命令黑名单命中）→ **永不自动放行**，直接拒或强制人工
- **验收**：注入一条 `curl` 外泄命令，`FullAuto` 下仍被拦；普通 `edit_file` 在 `AutoEdit` 下免弹窗

### 3.3 P1 批次（上下文治理 + 确定性护栏）—— 修正 G4/G5/G6/G11

**P1-1 层级规则加载（`context.rs`）**

```
解析顺序（closer wins，root → leaf 拼接）：
  ~/.owo/AGENTS.md              全局
  <git root>/AGENTS.md          仓库根
  <workspace>/AGENTS.md         工作区
  <workspace>/**/AGENTS.md      子目录（随 read_file 命中时按需追加）
总量上限 32 KiB（超出截断并标注）
```

- 兼容 `CLAUDE.md` / `.trae/rules/*.md`（只读，不写）
- 新增 `GET /project/rules` 返回「解析链 + 每份大小 + 生效顺序」（前端可视化）
- **验收**：三级嵌套 AGENTS.md 的拼接顺序与覆盖语义符合预期；超限被截断并标注

**P1-2 压缩重注入清单（`agent.rs`）**

`maybe_compact` 摘要完成后，按固定清单重注入：
1. 系统提示词（configured + 工程纪律）
2. 项目规则（重新从磁盘读，非用快照）
3. 当前 Plan（若有）
4. 最近修改的 ≤5 个文件路径 + 首 40 行
5. 已调用 skill 的正文（单 ≤5k token、合计 ≤25k、超限丢最旧）

- **验收**：压缩后断言清单项均在消息序列中

**P1-3 Hooks（新增 `hooks.rs`）**

```rust
pub enum HookEvent { PreToolUse, PostToolUse, SessionStart, PreCompact, Stop }
// handler: Command { cmd } | Http { url } | Prompt { text }
// PreToolUse 退出码 2 = 阻断（确定性，不经过模型）
```

- 配置：`.owo/hooks.json`（项目）+ `~/.owo/hooks.json`（全局）
- 端到端超时 5s，失败不阻断（除 exit 2）
- 新增 `GET /hooks`、`POST /hooks/test`
- **验收**：一个 `PreToolUse` hook 阻断 `write_file *.lock`；hook 超时不阻塞主循环

**P1-4 Skill 按需加载（`skill.rs` + `context.rs`）**

- 系统提示只注入 `name + description`（等价 Claude 的启动扫描）
- `use_skill` 命中后正文才入上下文，并记入 P1-2 的清单
- **验收**：10 个技能全启用时提示词体积不随技能数线性增长

### 3.4 P2 批次（人工介入点 + 前端品质）—— 修正 G7/G8/G9/G10/G12

**P2-1 `todo` 工具（`tools.rs`）**

- 新增 `todo_write`（Level::Read，仅维护会话内清单）+ `GET /session/{id}/todos`
- 前端对话区顶部渲染进度条（N/M 完成）
- **验收**：长任务中清单持久化到会话，刷新后可恢复

**P2-2 Plan 模式（后端 + 前端）**

- 会话级 `plan_mode`：只读策略（复用 `Policy::read_only`）+ 首轮产出结构化计划
- `POST /session/{id}/plan/approve` → 解除只读、按计划执行
- 前端：计划卡片（可编辑条目 + 批准/驳回）
- **验收**：Plan 模式下 `edit_file` 被拒；批准后放行

**P2-3 diff 逐块审阅（前端 `app.js`）**

- 右侧检查器的 diff 渲染改为 hunk 列表，每 hunk 带「接受 / 拒绝」复选框（拒绝的 hunk 通过 `edit_file` 反向还原）
- 会话消息上挂「回滚到此处」（调 `/session/{id}/rewind`）
- **验收**：拒绝单个 hunk 后文件与预期一致，其余 hunk 保留

**P2-4 斜杠命令（前端 `app.js`）**

| 命令 | 行为 |
|---|---|
| `/new` | 新建会话（已存在） |
| `/compact` | 触发一次压缩（调新增 `POST /session/{id}/compact`） |
| `/undo` `/redo` | 调 `/rewind` `/redo` |
| `/export` | 调 `/session/{id}/export/markdown` 下载 |
| `/plan` | 切换 Plan 模式 |

- 输入 `/` 弹出命令面板（复用 `openComposerMenu`）
- **验收**：5 个命令均可从 composer 触达

**P2-5 面板结果去裸文本（前端）**

- `eval.panel.js`：报告 → 用例表格（通过/失败徽章 + 耗时列）
- `observability.panel.js`：trace 回放 → 时间线（步骤类型图标 + 耗时条）
- 子代理结果 → 卡片（模式徽章 + 耗时 + Markdown 正文）
- **验收**：三处无 `textContent` 拼接

**P2-6 技能页按需加载可视化**

- 技能卡片展示「描述（常驻） / 正文（按需，约 N token）」两段体积
- **验收**：与 P1-4 的注入策略一致

---

## 四、实施批次与验收

| 批次 | 内容 | 验收方式 |
|---|---|---|
| **B1** | P0-1 + P0-2 | `cargo test -j 4 --workspace --no-fail-fast` 全绿；946 基线不回归；新增权限规则单测 |
| **B2** | P0-3 | 越权命令拦截测试（外泄/凭据/黑名单三案例）；审计含 `auto_approved` 记录 |
| **B3** | P1-1 + P1-4 | 规则层级单测；10 技能提示词体积对比 |
| **B4** | P1-2 + P1-3 | 压缩重注入断言；hook 阻断/超时测试 |
| **B5** | P2-1 + P2-2 | todo 持久化；Plan 模式只读断言 + 批准放行 |
| **B6** | P2-3 ~ P2-6 | playwright 全流程交互验证 + 截图自评 |

**约束**：
- 编译需 `ORT_LIB_PATH`（见 `效能核心改造方案.md` 环境备忘），并行度 `-j 4`
- 每批独立可编译、可回退；不破坏 openapi 契约
- 前端每批跑 playwright 验证（挂载 → 真实交互 → 断言渲染结果）并清理临时文件

---

## 四·附：本轮实施记录（2026-09-24）

### 环境阻塞（影响 B1–B5）——根因已定位，替代构建路径已打通

**精确根因**：本机无任何 MSVC 工具链（`link.exe` / `lld-link.exe` / `cl.exe` / `rc.exe` 全无）、无 MSVC CRT（全盘无 `libcmt.lib` / `vcruntime.lib` / `libcpmt.lib`）、且无外网。而 `rust-toolchain.toml` 固定 `x86_64-pc-windows-msvc`，`ort`（onnxruntime）与 `sherpa-onnx` 的 Windows 预编译包是 **MSVC C++ 静态库**——其符号（`__CxxFrameHandler4`、`__GSHandlerCheck`、`__std_terminate`、MSVC 修饰名的 std 符号 `??1?$basic_string@...`）与 MinGW 的 Itanium C++ ABI 不兼容，故 GNU 目标亦链接不出可执行文件（已实测两次失败）。**这不是「缺链接器」，补单个文件无法解决。**

**已打通的替代路径**：`owo-agent-core` 新增 `native-inference` 特性（默认开启）门控 ort / sherpa-onnx；`owo-agent-server` / `owo-agent-cli` 逐级透传。用 `--no-default-features` + GNU 目标可构建出**完整可用**的可执行文件——OCR 降级 Media.Ocr / Paddle 云，本地 STT 返回「未启用」，其余能力（含 `/fs/pick-directory` 文件夹选择器）全部在线。

```powershell
cargo +stable-x86_64-pc-windows-gnu build -p owo-agent-cli --bin owo-agent --no-default-features --target x86_64-pc-windows-gnu
```

**MSVC 默认构建**（含本地 ONNX OCR / 本地 STT）仍需等有网络后安装 VS Build Tools 的「使用 C++ 的桌面开发」工作负载；在此之前用上述 GNU 无特性构建推进 B1–B5。

### 审查发现的存量缺陷（已修）

前端有 5 处使用裸 `fetch()`，绕过 `api()` 的 bearer 鉴权，**必然返回 401**：

| 位置 | 功能 |
|---|---|
| `exportPackage` | 导出流程技能包 |
| `importPackage` | 导入 `.owskill` |
| `refreshAutomations` 删除按钮 | 删除定时任务 |
| `exportSession` | 导出会话为 MD/HTML |
| STT 转写 | 语音输入转写 |

**修法**：新增 `apiRaw()`——与 `api()` 同款 bearer 注入 + 401 重试，但不强制 `Content-Type`、不解析 JSON（专供 blob/zip/音频等非 JSON 响应）。5 处全部替换；其余 `fetch` 均为合规（`/health`、`/auth/token`、外部模型连通性探测、SSE 回合流自带 Authorization）。

### 已落地的前端批次

| 项 | 内容 | 对应方案条目 |
|---|---|---|
| P2-5 去裸文本化 | eval 报告 → 用例卡片（通过 N/M 徽章 + 逐用例行 + 错误备注）；trace 回放 → 元信息芯片 + 事件时间线（token_delta 聚合，工具/审批/压缩/汇报分色）；子代理结果 → 模式徽章 + Markdown 卡片 | P2-5 |
| P2-4 斜杠命令 | composer 内 `/` 唤起补全浮层（7 条命令：`new`/`undo`/`redo`/`export`/`stop`/`settings`/`help`），↑↓ 选择、Tab 补全、Enter 执行、Esc 关闭；工具栏新增「/ 命令」入口胶囊 | P2-4 |
| 会话操作可用化 | 新增 `sessionUndo`/`sessionRedo`：`/undo` 从会话详情推导「最后一条 user 消息」位置，无需用户手填条数 | P2-4 |
| 原生弹窗替换 | 新增 `promptModal`（样式化单行输入弹窗），替换 3 处 `window.prompt`（重命名 / 分叉 / 回退）与 1 处 `window.confirm`；`alert` 改 toast | 既有前端标准 |

### 验证结果

playwright + Chrome 全流程验证，**全部通过、零非预期 JS 错误**：

- `apiRaw` 对照：裸 `fetch('/automations')` → 401，`apiRaw('/automations')` → 200
- 斜杠命令：入口唤起 7 项 → 输入 `/e` 过滤为 1 项 → `/help` 回车弹出 7 行命令一览且输入框清空 → Esc 关闭；↑↓ 两次定位到 `/redo`
- 结构化渲染（stub api，不触发真实模型）：eval 4 个芯片 + 3 行用例（2✔/1✘）+ 错误备注；trace 8 条时间线（2 绿 / 1 红 / 2 黄）+ 4 个芯片；子代理 `h2`/`strong` 正常渲染；三处 `pre` 残留均为 0
- 守卫：无会话时 `/export` 与 `/undo` 各弹一次 toast，且未发出任何 API 调用

---

## 五、非目标（明确不做）

- 不引入云端代码库索引（保持本地优先；语义检索仅作为可选增强，默认关闭）
- 不重写现有沙箱 / 审计 / 会话存储 / MCP / 路由契约
- 不做多用户协作与远端 attach（OpenCode 形态暂不追）
- 不引入外部 agent 框架依赖，仅参考其设计语义

---

## 六、与既有方案的关系

| 方案 | 范围 | 状态 |
|---|---|---|
| `效能核心改造方案.md` | 工具层 + 提示词 + 权限矩阵 + 并行执行 | ✅ 已落地（946 测试通过） |
| **本方案** | 权限空间正交化 + 上下文治理 + 人工介入点 + 前端品质 | ⏳ 部分实施：前端 P2-4 / P2-5 已落地（四·附）；P0/P1 已可用 GNU `--no-default-features` 构建推进 |
| LingXi 桌宠桥接 | 另见 LingXi 侧方案 | 不受本方案阻塞（API 面零变化） |