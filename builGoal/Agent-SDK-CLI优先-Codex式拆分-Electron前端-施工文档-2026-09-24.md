# Agent SDK：CLI 优先 + Codex 式模块拆分 + Electron 前端 —— 施工文档

- 日期：2026-09-24
- 分支：`agent-frame`
- 适用仓库：`agent-sdk/`（活跃开发目标；根目录 OwO C++ 输入法不动）
- 依据（事实优先级从高到低）：
  1. 当前源码 + 真实构建/运行证据；
  2. 本文件；
  3. `agent-sdk/docs/plans/archive/2026-09/ROADMAP-PARITY.md`、`agent-sdk/docs/architecture/ARCH-MICROKERNEL.md`；
  4. `builGoal/Agent-SDK-后续任务实施指南-2026-09-18.md`（下称“指南”）、
     `builGoal/技术文档-AI智能体输入法.md`（v0.6）。

> 本文件是**开工前文档**。按用户要求：先写本施工文档，再动代码。所有“已完成/未完成”
> 都以真实命令输出与落盘证据为准；没有证据的一律标“待验证”。

---

## 0. 一句话结论

用户原始诉求里**三条已是现状、两条需要校准、一条必须否决**：

| 诉求 | 裁决 | 说明 |
|---|---|---|
| “要一个能正常运行的完整 CLI” | **采纳，P0 第一优先** | CLI 已有 40+ 子命令，缺的是**本机从零可复现的构建/运行验证**（当前 `target/` 为空） |
| “把 codex 代码下载下来直接用” | **否决（不采用）** | 见 §1.2：许可证、运行时、原生依赖、离线环境、契约面五条硬冲突 |
| “和 codex 一样的功能” | **采纳，转为可验收清单** | 用 `ROADMAP-PARITY.md` 的 G1–G15 差距矩阵作为“一样”的定义与验收项 |
| “把 core 等大文件都拆了，像 codex 一个 crate 一个文件” | **采纳但校准** | 拆**大文件为同 crate 内的一模块一文件**（用户真实意图）；**不是**按体积新建 crate（违反 ARCH §0 事务边界判据） |
| “前端 Rust TUI 暂时放弃，改用 Electron 打包” | **采纳** | TUI 冻结（不再新增功能），Electron 壳（`desktop/electron/`）升为正式前端 |

一句话施工线：**先让 CLI 从零构建并跑通运行态冒烟（P0）→ 再按域拆大文件（P1–P3）
→ 再把 Electron 壳接通并打包（P4）→ 最后按 G1–G15 补 codex 同款能力（P5+）。
每一步都必须有可复现命令与退出码证据。**

---

## 1. 需求逐条校准

### 1.1 “完整的 CLI 运行正常”

现状（按源码核对，非印象）：

- 二进制：`crates/owo-agent-cli`，`[[bin]] name = "owo-agent"`，入口 `src/main.rs`（189 行）。
- 子命令（`main.rs` 的 `enum Commands`，共 17 个）：`turn / serve / daemon / repl / tui /
  init / eval / product-eval / bench / cloud / plugin / audit / backup / doctor / worker /
  capabilities`，`None` 缺省进 REPL。
- 命令实现：`src/commands/`（16 个 `.rs` + `repl/` 目录）+ `src/tui.rs`（52 KB）+
  `src/product_eval_cmd.rs` + `src/support.rs` + `src/ui_output.rs` + `src/worker_child.rs`。
- 依赖：`core / server / eval-facade / protocol / client / build-info`；外部含 `ratatui`、
  `crossterm`、`rustyline`、`colored`、`clap`。

**因此“CLI 端运行正常”在本仓库的验收口径不是“再造一个 CLI”，而是：**

1. 从零 `cargo build -p owo-agent-cli` 退出码 0；
2. `owo-agent --version` 携带完整身份字段（api/commit/dirty/built_at/source）；
3. `owo-agent --help` 列出全部子命令；
4. `owo-agent doctor` 能给出可读诊断；
5. 运行态冒烟（真实进程 + 真实 HTTP + 真实落盘 + 无孤儿进程）通过
   （复用 `scripts/mk-smoke.ps1` / `scripts/ci-gate.ps1`）。

> 证据栏见 §8 与 §10 执行记录。

### 1.2 为什么不“下载 codex 代码直接使用”（硬冲突，逐条）

| 维度 | codex（OpenAI） | 本仓库（OwO Agent SDK） | 冲突后果 |
|---|---|---|---|
| 许可证 | Apache-2.0 | GPL-3.0-only | 直接并入需处理许可兼容与署名，且污染既有许可边界 |
| 语言/运行时 | Rust 为主 + 大量自研协议 | Rust + 既有 `owo-agent-*` 微内核契约 | API/类型面完全不兼容，无法“直接替换” |
| 原生依赖 | 自带沙箱/平台绑定 | ONNX Runtime / Sherpa / UIA / SendInput（已有自研沙箱 M3） | 双份原生栈，触发原生重链与体积膨胀（指南 §10 红线） |
| 网络环境 | 需外网拉取 | 本机为受限环境，且模型凭据只走环境变量 | 无法保证可复现，且违反“不浏览/不外部下载”的作业约束 |
| 验收契约 | 无 | M1 明确要求会话/审计/diff/revert/工具权限保持工作，改动需带契约测试 | 引入外部实现会打断 M1 验收链 |

**结论**：不下载、不并入 codex 源码。“和 codex 一样”落地为**行为对齐**（G1–G15），
实现仍由本仓库自有 crate 承载。若确需参考，仅允许“读其公开文档提炼能力清单”，
不允许拷贝代码进入本仓库。

### 1.3 “像 codex 一个 crate 一个文件”的正确理解

用户的真实诉求是**大文件太难维护，要拆小**。但本仓库已有一份经实测确立的拆分判据
（`docs/architecture/ARCH-MICROKERNEL.md` §0）：**按事务边界拆，不按文件大小拆**。并且 M0–M15 已完成
16 步微内核拆分，`core` 已从 64,701 行降到约 36,787 行。

所以本次拆分的正确形态是两条并行：

1. **crate 间**（继续微内核路线，只在满足“零出边+零入边”或“同迁”时执行）：
   已基本收口，剩余 `agent/tools/computer_use/session/goal/fleet/workswarm/worker_pool/trace`
   属“总线型”，暂不硬拆。
2. **crate 内**（本次重点，用户要的“拆大文件”）：把单个超长 `.rs` 拆成
   `modname/mod.rs` + `modname/<子域>.rs`，`mod.rs` 只做模块声明与 `pub use` 再导出，
   **公共面 1:1 等价**（编译器证明）。这既满足“一模块一文件”，又不动 crate 边界与契约。

这属于**纯文件级重构**：不新增依赖、不改公共 API、不改路由契约，风险最低、收益最直接。

### 1.4 前端：TUI 冻结、Electron 转正

现状：`desktop/` 下已有三套：`electron/`（Electron 33 + Vue）、`tauri/`、`web/`。

裁决：

- `owo-agent tui`（ratatui 全屏 TUI）**冻结**：保留命令可用，不再新增功能，不作为交付前端。
- `desktop/tauri/` **冻结**：不再作为打包主产物。
- `desktop/electron/` **转正**：作为对用户交付的前端与打包入口（P4）。
- 前端**无状态**：一切经 Daemon HTTP API，禁止在渲染进程内起第二套 runtime
  （指南 §0.1 / §2.4 第 6 条）。

---

## 2. 现状核实（本开工时点的事实）

### 2.1 工作区与成员

`agent-sdk/Cargo.toml` 的 workspace 共 21 个成员（crates/ 下 20 个 + `devtools/product-eval`
为独立 workspace，用 `exclude` 隔离）。依赖方向固定为：

```
protocol / kernel / contracts ...（依赖根）
        ▲
     core ──► server ──► cli
        ▲
     client（只依赖 protocol）
```

### 2.2 大文件实测清单（行数，`Get-Content | Count`）

`crates/owo-agent-core/src`（前 12）：

| 行数 | 文件 |
|---:|---|
| 4546 | `workswarm.rs` |
| 2355 | `computer_use.rs` |
| 2042 | `gateway.rs` |
| 1818 | `tools.rs` |
| 1773 | `agent.rs` |
| 1639 | `goal.rs` |
| 1431 | `worker_pool.rs` |
| 1355 | `fleet.rs` |
| 1101 | `sqlite_store.rs` |
| 1042 | `session.rs` |
| 893 | `builtin_team_templates.rs` |
| 887 | `fleet_transport.rs` |

`crates/owo-agent-server/src`（前 12）：

| 行数 | 文件 |
|---:|---|
| 2313 | `workswarm_api.rs` |
| 2138 | `lib.rs` |
| 1616 | `desktop_world_api.rs` |
| 1276 | `goal_api.rs` |
| 1063 | `fleet_api.rs` |
| 1002 | `turn_api.rs` |
| 991 | `notes_api.rs` |
| 964 | `workswarm_metrics.rs` |
| 955 | `human_inbox_api.rs` |
| 946 | `observability_api.rs` |
| 913 | `workspace_change_tracker.rs` |
| 884 | `settings_api.rs` |

`crates/owo-agent-cli/src`：`tui.rs` 52,771 字节（≈1400 行）为最大单文件。

### 2.3 构建环境实测

- `agent-sdk/target/` **不存在**：本机从未在该工作树构建过，P0 为“冷构建”。
- 磁盘：T 盘剩余约 **82 GB**（满足全量档 ≥20 GB 红线）。
- 内存：总 31.6 GB / 空闲 17.6 GB（已用 44.3%，低于 80% 红线）。
- 逻辑处理器：32。→ 手写命令必须显式限并发（红线下节）。

---

## 3. 施工红线（强制，违反即停）

1. **并发**：定向档 `-j 2` / `--test-threads=2`；完整 workspace / core / release /
   原生重链档一律 `-j 1` / `--test-threads=1`。禁止依赖 Cargo 默认 32 路并发，禁止一次跑两组 cargo。
2. **原生依赖前置**：任何 `cargo` 前先 `. scripts\resolve-ort.ps1; Resolve-OwoOrtEnv -Quiet`，
   否则 `ort-sys`/`sherpa-onnx-sys` 退化为联网下载并静默挂死。
3. **资源门**：可用内存 <6 GB 或已用 ≥80% 不得启动构建；构建卷剩余 <20 GB（全量档）/
   <6 GB（定向档）不得启动。统一走 `scripts/ci-shared.ps1` 的 `Invoke-CiCargo`。
4. **编码**：全部源文件 UTF-8；`.ps1` 必须带 UTF-8 BOM（前三字节 `239,187,191`）；
   含中文的 `.rs/.md` 禁止经 GBK 控制台中转。
5. **契约**：core 改动带契约测试；server 新增/改路由同步 `tests/route_contract_tests.rs`；
   保持 `cargo fmt` 与 `clippy` 干净。
6. **权限**：默认 deny；审批与主 Agent 分离；不写模型凭据进任何仓库文件。
7. **并行协作**：按 `AGENTS-COORD.md` 认领文件，同一文件同一时间只允许一个 Agent 修改。
8. **文件拆分等价性**：拆分后公共面必须 1:1 等价（优先 glob `pub use`，让编译器证明），
   调用方零改动；拆分与行为改动**不得混在同一提交**。

---

## 4. 目标结构（Codex 式：一个模块一个文件）

### 4.1 crate 内拆分模板

拆分前：

```
crates/<pkg>/src/<big>.rs        # 单文件 1500–4500 行
```

拆分后：

```
crates/<pkg>/src/<big>/
├── mod.rs            # 仅：mod 声明 + pub use 再导出 + 顶层文档注释
├── <subdomain_a>.rs  # 一个子域一个文件
├── <subdomain_b>.rs
└── tests.rs          # 原文件内 #[cfg(test)] 原样搬入（或就近分片）
```

约束：

- `mod.rs` 里只写 `mod x; pub use x::*;`，**不写字面符号表**（历史踩坑：手写符号表 E0432）。
- `pub(crate)` 条目：同 crate 内拆分可见性不变，无需提权。
- `crate::` 相对路径不变（仍在同 crate，只是模块路径变一层）。
- 拆分后必须 `cargo fmt` + `cargo check -p <pkg>` 通过。

### 4.2 拆分进度（按“体积 × 调用方广度 × 风险”排序）

> 状态以 `cargo check -p <pkg>` + 定向/全量 lib 测试通过为准（证据见 §8）。

**core（全部通过 `cargo check` + lib 测试）**
1. ✅ `workswarm.rs` 4546 → `workswarm/{mod,error,roles,registry,types,util,tests}.rs`
2. ✅ `computer_use.rs` 2355 → `computer_use/{mod,sim,actions,task,tools,drivers,tests}.rs`
3. ✅ `gateway.rs` 2042 → `gateway/{mod,message,config,tests}.rs`
4. ✅ `goal.rs` 1639 → `goal/{mod,types,tests}.rs`
5. ✅ `worker_pool.rs` 1431 → `worker_pool/{mod,protocol,pool,tests}.rs`
6. ✅ `fleet.rs` 1355 → `fleet/{mod,bus,supervision,wait,fanout,tests}.rs`
7. ✅ `session.rs` 1042 → `session/{mod,model,store,tests}.rs`
8. ⏳ `agent.rs` 1773（run_turn 巨型 impl + 配置/助手；下一步）
9. ⏸ `tools.rs` 1813：**有意保持集中**——它是“工具注册表 + 内建工具”的统一提供面
   （用户口径：只提供工具/辅助且功能统一的应集中），不再机械拆分。

**server（全部通过 `cargo check` + lib/集成测试）**
10. ◑ `lib.rs` 2138 → **1393**；抽出 `openapi.rs`（openapi_spec + path_param + permission_spec_schema）
11. ✅ `notes_api.rs` 991 → `notes_api/{mod, store, support, handlers}`
12. ✅ `fleet_api.rs` 1063 → `fleet_api/{mod, hub, handlers}`
13. ✅ `goal_api.rs` 1276 → `goal_api/{mod, state, handlers}`
14. ✅ `turn_api.rs` 1002 → `turn_api/{mod, queue, approval, wire, handlers, tests}`
15. ✅ `desktop_world_api.rs` 1616 → `desktop_world_api/{mod, hub, handlers}`
16. ✅ `workswarm_api.rs` 2313 → `workswarm_api/{mod, state, workers, runtime, dto, handlers, tests}`
17. ✅ `observability_api.rs` 946 → `observability_api/{mod, metrics}`
18. ✅ `human_inbox_api.rs` 955 → `human_inbox_api/{mod, drafts}`
19. ✅ `settings_api.rs` 884 → `settings_api/{mod, provider_test}`
20. ✅ `workspace_change_tracker.rs` 913 → `workspace_change_tracker/{mod, git, tracker, tests}`
21. ✅ `workswarm_metrics.rs` 964 → `workswarm_metrics/{mod, metrics, sanitize, util, tests}`

**待办：巨型 impl 再拆**
- `workswarm/mod.rs`（3682）与 `gateway/mod.rs`（1085）的巨型 `impl`：需对被移出的方法/字段做
  `pub(super)` 提权，按文件单独验收。

**手法与约束（每步都执行）**
- 只做**同 crate 内一模块一文件**；`mod.rs` 只写 `mod 声明 + glob pub use`，公共面 1:1 等价，调用方零改动。
- 私有辅助若跨新模块使用才提权为 `pub(super)`；绝不降低既有 `pub` 可见性。
- 切片边界必须带上条目的 `#[derive]/doc`，否则出现“expected item after doc comment”。
- `include_str!` 等相对路径随目录加深要多一级 `..`。
- 每步 `cargo fmt`（必要时连跑两次）+ `cargo check` + 定向测试；`cargo fix` 清理未用 import。

---

## 5. 分阶段任务与验收

### P0 — CLI 冷构建与运行态（第一优先，本次立即执行）

- 动作：注入 ORT → 限并发构建 `owo-agent-cli` → 运行 `--version` / `--help` / `doctor`。
- 验收：
  - `cargo build -p owo-agent-cli -j 2 --locked` exit 0；
  - `target/debug/owo-agent.exe --version` 含 `api= commit= dirty= built_at= source=`；
  - `--help` 列出 17 个子命令；
  - 运行态冒烟入口可用（`scripts/mk-smoke.ps1`，视时间纳入 P0 或紧随其后）。
- 证据：`docs/qa/logs/` 落盘构建日志；本文件 §10 追加。

### P1 — core 三大文件拆分（workswarm / computer_use / gateway）

- 动作：按 §4.1 模板拆分，公共面 glob 再导出。
- 验收：`cargo fmt`、`cargo check -p owo-agent-core`、定向测试
  （`workswarm_tests`、`computer_use` 相关、`gateway_tests`）全绿；调用方零改动。
- 证据：`docs/qa/logs/` + §10。

### P2 — server `lib.rs` 与两大 api 拆分

- 验收：`cargo check -p owo-agent-server` + `route_contract_tests` 全绿（路由面不变）。
- 注意：路由新增/修改必须同步契约测试。

### P3 — core/cli 长尾文件拆分

- 验收：定向测试 + 运行态冒烟。

### P4 — Electron 前端收口与打包

- 动作：`desktop/electron/` 接通 Daemon HTTP API；新增 `npm run package`（electron-builder）产物；
  前端只做渲染与审批，无第二 runtime。
- 验收：Electron 起窗 → 连 Daemon → 发一轮对话 → 审批 → diff 可见；强杀 Daemon 后自动重连；
  打包产物可双击启动。
- 证据：`docs/qa/evidence/`。

### P5 — Codex 同款能力（按 G1–G15）

- 顺序：W1 工具面（G1 精细编辑 → G2 glob → G3 后台任务）→ W2 上下文（G5/G6）→
  W3 长任务（G4/G9）→ W4 客户端收敛（G7/G8）→ W5 差异化 → W6 编辑器集成（G12/G15/G13）。
- 每项按 `ROADMAP-PARITY.md` §4 统一模板验收。

---

## 6. 明确不做（本轮）

- 不下载/不并入 OpenAI codex 源码（见 §1.2）。
- 不为“拆文件”而新建 crate（违反 ARCH §0 事务边界判据）。
- 不删除 TUI 命令（冻结 ≠ 删除；删除需单独评审）。
- 不改 `owo-agent-server/src/lib.rs` 之外的 M1 契约面行为；拆分与行为改动分离提交。

---

## 7. 验收命令（标准写法，含限并发）

```powershell
cd agent-sdk
. scripts\resolve-ort.ps1; Resolve-OwoOrtEnv -Quiet | Out-Null
$env:CARGO_BUILD_JOBS = "2"; $env:RUST_TEST_THREADS = "2"

# 定向构建
cargo build -p owo-agent-cli --locked -j 2

# 定向检查（拆分后）
cargo check -p owo-agent-core --locked -j 2
cargo check -p owo-agent-server --all-targets --locked -j 2

# 定向测试
cargo test -p owo-agent-core --test workswarm_tests --locked -j 2 -- --test-threads=2
cargo test -p owo-agent-server --test route_contract_tests --locked -j 2 -- --test-threads=2

# 运行态冒烟（完整 workspace 档自动降为 -j 1）
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\mk-smoke.ps1 -Tag cli-p0
```

> 完整 workspace / release / 原生重链：一律 `-j 1 -- --test-threads=1`，并经 `Invoke-CiCargo`。

---

## 8. 执行记录（随做随更）

### 2026-09-24 · P0 完成（CLI 冷构建 + 运行态已验证）

- 环境核实：`target/` 不存在（冷构建）；磁盘余 ~82 GB；内存空闲 17.6/31.6 GB；32 逻辑处理器。
- 构建：`cargo build -p owo-agent-cli -j 2 --locked` → **exit 0，6m25s**（日志
  `%TEMP%\opencode\cli-build.log`）。
- 运行（均为真实进程，退出码 0）：
  - `owo-agent --version` → `owo-agent 0.1.0 api=0.7 commit=787e970 dirty=false built_at=… source=compiled`
  - `owo-agent --help` → 17 个子命令齐全
  - `owo-agent doctor` → 7 项全 `[ok]`（数据目录/SQLite/凭据/网关韧性/服务/备份）
  - `owo-agent capabilities` → 7 项能力目录
- 结论：**"完整 CLI 运行正常"的 P0 验收成立**。

### 2026-09-24 · P1 拆分完成（workswarm + gateway）

| 目标 | 结果 | 验收 |
|---|---|---|
| `core/src/workswarm.rs` | 4546 → `workswarm/{mod 3682, error 29, roles 167, registry 156, types 165, util 232, tests 110}` | `cargo check -p owo-agent-core` exit 0；`--lib workswarm` 5/5；`--test workswarm_tests` 15/15 |
| `core/src/gateway.rs` | 2042 → `gateway/{mod 1085, message 218, config 148, tests 607}` | `cargo check` exit 0；`--lib gateway` 20 passed / 1 ignored |
| `server/src/lib.rs` | 2138 → 1393；抽出 `server/src/openapi.rs`（openapi_spec + path_param + permission_spec_schema） | `cargo check -p owo-agent-server` exit 0；`--test route_contract_tests` 32/32 |

拆分手法：机械切片（保留原行）→ `mod.rs` 只做 `mod` 声明 + glob `pub use`，公共面 1:1
等价；仅有 3 类可见性调整（私有辅助函数提权 `pub(super)`、`RunMeta::{file_path,save,load}`
提权、遗漏 import 补齐）。`cargo fmt` 干净，调用方零改动。

### 2026-09-24 · 工具调用死循环整改（对标 Codex/OpenCode）

问题：`AgentConfig.max_turns = 60` 且 `tool_concurrency = 4`，若弱模型陷入「同一工具调用」
死循环（写文件失败后原样重发），一次任务会被放大成**最多数百次**工具执行；原实现没有任何
重复调用或工具总量约束。

整改（`crates/owo-agent-core/src/agent.rs`）：

- 新增 `AgentConfig::max_tool_calls_per_turn`（默认 64，`OWO_AGENT_MAX_TOOL_CALLS`）：
  单回合工具调用总量上限，超限立即以可读原因结束回合。
- 新增 `AgentConfig::max_repeated_tool_calls`（默认 3，`OWO_AGENT_MAX_REPEATED_TOOL_CALLS`）：
  同一 `name + 规范化参数`（键序无关，`canonical_json`）的调用重复超过上限即**拦截不执行**，
  回灌"请改变策略"提示；仍计入总量上限，连续被拦截最终触发总上限停止。
- `PreparedCall` 增加 `guard_error`；并发组资格判定天然排除被拦截调用。

验收（`cargo test -p owo-agent-core --lib`）：
`repeated_identical_tool_call_is_loop_guarded` ok（重复第 3 次起拦截，实际只执行 2 次）、
`tool_call_signature_ignores_key_order` ok。

### 2026-09-24 · 大文件批量拆分（core 7 个 + server lib）

| 文件 | 拆分结果 | 验收 |
|---|---|---|
| `core/workswarm.rs` 4546 | `workswarm/{mod 3682, error, roles, registry, types, util, tests}` | lib 5/5；集成 `workswarm_tests` 15/15 |
| `core/computer_use.rs` 2355 | `computer_use/{mod, sim, actions, task, tools, drivers, tests}` | lib 6/6 |
| `core/gateway.rs` 2042 | `gateway/{mod 1085, message, config, tests}` | lib 20 passed/1 ignored |
| `core/goal.rs` 1639 | `goal/{mod 1330, types, tests}` | lib 2/2 |
| `core/worker_pool.rs` 1431 | `worker_pool/{mod, protocol, pool, tests}` | lib 7/7 |
| `core/fleet.rs` 1355 | `fleet/{mod, bus, supervision, wait, fanout, tests}` | lib 8/8 |
| `core/session.rs` 1042 | `session/{mod, model, store, tests}` | lib 13/13 |
| `server/lib.rs` 2138 | lib **1393** + `openapi.rs` | `route_contract_tests` 32/32 |

全量 core lib 回归：**192 passed / 0 failed / 1 ignored**。`cargo fmt --check` 干净，
`cargo check -p owo-agent-core` 无 warning（`cargo fix` 清理完毕）。

拆分中实测的坑（已固化上文手法）：
1. 切片起点切掉 `#[derive(...)]`/doc → `expected item after doc comment` 与
   “X 未实现 Serialize/Debug/Clone”连环报错（computer_use、goal、session、fleet 各命中一次）。
   修法：切片起点回退一行带上属性，或把注释手动移回条目。
2. 目录加深后 `include_str!(".../scripts/browser-driver.js")` 必须多一级 `..`。
3. `pub use mod::*;` 只再导出 `pub` 项；测试若用私有辅助，需在测试模块显式
   `use super::<sub>::*;`，或把目标提权 `pub(super)`。
4. `cargo fmt` 偶发首轮未落盘，`--check` 仍报同一条——连跑两次即稳定。

### 待续

- `core/tools.rs`：按用户口径**有意集中**，不拆。
- 下一步：CLI Agent 核心交互功能优化（见新增 §9）。

### 2026-09-24 · 巨型 impl 拆分（gateway/mod + workswarm/mod）

| 文件 | 结果 | 验收 |
|---|---|---|
| `core/gateway/mod.rs` 1081 | `gateway/{provider, stream, resilience}` + mod.rs 瘦身 | lib gateway 20 passed/1 ignored |
| `core/workswarm/mod.rs` 3680 | `workswarm/coord_{accessors,load,run,human,lifecycle,artifacts,handoff,steer}.rs` + `role_worker.rs`；mod.rs 仅 143 行 | lib workswarm 5/5；`workswarm_tests` 15/15 |

手法：把 `impl TeamCoordinator` 按方法域切成 8 个独立 `impl TeamCoordinator` 块（Rust 允许跨模块写同一类型 impl），
方法统一提权 `pub(crate)`、字段 `pub(crate)`；`RoleWorker` 独立成文件。

新增坑（已固化）：

1. **方法域切片会留下“下一条目的 doc 注释”**：切片边界若取在方法签名行，则上一条目尾部会带出下一条目的
   `///` 文档 → `E0584 found a documentation comment that doesn't document anything`。
   修法：把文件末尾（`}` 之前）的连续 `///` 块搬到下一个文件的 `impl ... {` 之后。
2. **私有类型出现在 `pub(crate)` 字段/方法签名里**：`private_interfaces` 警告；把 `PhaseClaim`/`StepOutput`
   一并提权 `pub(crate)`。
3. **`use super::*;` 足以支撑切出的 `impl` 块**：父模块的私有 `use`（如 `use util::*;`）对子模块 glob 可见，
   无需在每个 coord 文件重复完整 import 块。
4. **gateway 的 `#[path]` 无关**：`gateway.rs` 原文件仍在 git HEAD，可 `git show HEAD:<file>` 重建被覆盖的
   `mod.rs` 源，避免硬编码行号漂移。

### 2026-09-24 · server 收尾批次（workswarm_api + 5 个大文件 + agent）

| 文件 | 结果 | 验收 |
|---|---|---|
| `core/agent.rs` 1950 | `agent/{mod, config, tests}` | core lib **192/192** |
| `server/workswarm_api.rs` 2313 | `workswarm_api/{mod, state, workers, runtime, dto, handlers, tests}` | `workswarm_api_tests` 16/16 |
| `server/observability_api.rs` 946 | `observability_api/{mod, metrics}` | `observability_tests` 27/27 |
| `server/human_inbox_api.rs` 955 | `human_inbox_api/{mod, drafts}` | `human_inbox_api_tests` 11/11 |
| `server/settings_api.rs` 884 | `settings_api/{mod, provider_test}` | server lib 71/71 |
| `server/workspace_change_tracker.rs` 913 | `workspace_change_tracker/{mod, git, tracker, tests}` | `workswarm_api_tests` 16/16 |
| `server/workswarm_metrics.rs` 964 | `workswarm_metrics/{mod, metrics, sanitize, util, tests}` | server lib 71/71 |

收尾批次新踩的坑（已固化）：

1. **`#[path]` 子模块链**：`artifact_review_api.rs` 里 `#[path="human_inbox_api.rs"]`、
   `workswarm_api` 里 4 个 `#[path="<x>.rs"]`，被指向的文件变目录后全部要改成
   `#[path="<x>/mod.rs"]`；`include_str!` 同理。
2. **`#[path]` 子模块的 `super` 不是 crate 根**：`human_inbox_api` 是
   `artifact_review_api` 的 `#[path]` 子模块，其 `super` = `artifact_review_api`；
   拆出 `drafts.rs` 后 `super::human_inbox_store` 必须写成 `super::super::human_inbox_store`。
3. **切片尾部的 `#[derive]`**：条目属性（`#[derive]`/`#[cfg]`）常落在上一条目末尾，
   切片多切一行即报 `expected item after attributes`（agent/config、wct/git 各命中一次）。
4. **`pub(crate) use` 再导出 vs `pub(super)`**：被 `lib.rs` 或 `#[path]` 子模块引用的入口，
   子模块内必须 `pub(crate)`（`pub(super)` 在子模块里只到父模块）；`cargo fix` 会删
   “库内没直接用但测试需要”的 re-export，清 warning 后要复查测试。

### 最终回归（2026-09-24）

- `cargo check -p owo-agent-core -p owo-agent-server -p owo-agent-cli` → exit 0，无 warning。
- `cargo fmt --check`（core + server）→ 干净。
- core lib **192 passed**；server lib **71 passed**；`route_contract_tests` **32**、
  `workswarm_api_tests` **16**、`change_set_api_tests` **7**、`project_workspace_api_tests` **3**、
  `observability_tests` **27**、`workswarm_progress/recovery` **2/4** 全绿。
- 此前批次：`workswarm_tests` 15、`notes_api_tests` 13、`fleet_api_tests` 12、
  `goal_api_tests` 32、`desktop_world_api_tests` 14、`human_inbox_api_tests` 11 全绿。

### 2026-09-24 · server API 拆分（6 个）

| 文件 | 结果 | 验收 |
|---|---|---|
| `notes_api.rs` 991 | `notes_api/{mod, store, support, handlers}` | `notes_api_tests` 13/13 |
| `fleet_api.rs` 1063 | `fleet_api/{mod, hub, handlers}` | `fleet_api_tests` 12/12 |
| `goal_api.rs` 1276 | `goal_api/{mod, state, handlers}` | `goal_api_tests` 32/32 |
| `turn_api.rs` 1002 | `turn_api/{mod, queue, approval, wire, handlers, tests}` | server lib 71/71；route 32/32 |
| `desktop_world_api.rs` 1616 | `desktop_world_api/{mod, hub, handlers}` | `desktop_world_api_tests` 14/14 |

server API 拆分实测的额外坑（已固化）：

1. **`#[path = "../src/<x>.rs"] mod <x>;` 集成测试**：源文件变目录后必须改成
   `../src/<x>/mod.rs`（notes/fleet/goal 各命中一次）。
2. **`pub(super)` 在子模块里语义变了**：顶层模块里 `pub(super)` = crate 根可见，移入子模块后
   只剩父模块可见；被 `lib.rs` 直接调用的入口（如 `turn_api::{turn,turn_events,...}`）
   必须改 `pub(crate)` 并在 `mod.rs` `pub(crate) use <sub>::*;`。
3. **私有结构体字段被 handler 直接访问**：按报错逐个把字段/方法提权 `pub(super)`/`pub(crate)`；
   **不要用正则批量给 4 空格缩进行加可见性**——会误伤多行函数签名的参数（turn_api 实测）。
4. **`cargo fix` 会删掉“库内未被直接使用、但测试需要”的 `pub(crate) use`**：
   修完 warning 后若跑测试报缺类型，需把 re-export 加回（turn_api 的 `queue`/`wire` 各一次）。


---

## 9. CLI Agent 核心交互优化（进行中）

目标：把 REPL/`turn` 的**对话主循环**体验对齐 Codex/OpenCode——审批看得清、取消不泄漏、
失败可恢复、命令可发现。已落地（均带测试/编译验证）：

### 9.1 审批卡：从「只看工具名」到「看清批准的是什么」

`crates/owo-agent-cli/src/support.rs` 的 `ConsoleApprover::decide` 现在展示：
- 工具 + 权限等级；
- 审批原因 `reason`；
- 风险说明 `risk_note`（有则红字提示）；
- **脱敏参数摘要**（优先 `redacted_args`，紧凑 JSON，超 240 字符按字符截断）。

新增纯函数 `summarize_permission_args` + 单测（空值/空对象跳过、超长截断且带省略号）。
`y/yes/1/once` → 允许，其余 → 拒绝（`Decision` 仅 Allow/Deny；scope 属 Daemon 侧）。

### 9.2 取消（Ctrl+C）：修掉任务泄漏 + 回合失败可恢复

`crates/owo-agent-cli/src/commands/repl.rs::run_turn` 旧实现每回合 `tokio::spawn` 一个
`ctrl_c` 监听任务且永不结束——回合多了会累积任务，且**回合结束后按 Ctrl+C 也会误报“正在中止”**。
改为 `tokio::select!`：只在当前回合内监听 Ctrl+C，中止后等待回合协作收尾，再走统一的
保存/审计/摘要路径。

同时把「回合失败」从直接 `?` 冒泡改为可恢复：打印失败原因 + 提示
`/status`、`/diff`、`/undo`，且会话/审计已落盘（不再吞掉摘要）。

### 9.3 交互增强：slash 命令 Tab 补全

`repl.rs` 新增 `ReplHelper`（rustyline 14 `Helper` + `Completer`）：行首输入 `/` 时按
`SLASH_COMMANDS` 前缀补全命令名（保留行首 `/`，补全后自动补空格）。`/help` 同步补齐
`/agent`、`/skills reload`。

### 9.4 后续候选（待排期）

- 审批 scope（once/task/workspace）在 REPL 本地 Agent 路径的贯通（需 Decision 携带 scope）。
- 多行输入（粘贴/换行）与 `@` 文件路径补全。
- `turn`（Daemon 路径）与 REPL 的审批观感统一。
- 回合中断后的「部分结果」结构化回显。

### 验证

`cargo check -p owo-agent-cli` exit 0；`cargo test -p owo-agent-cli --bins` **28 passed**；
`cargo fmt --check` 干净。

### 9.5 第二批：审批观感统一 + 会话级授权 + 补全下沉（默认 Daemon 路径）

发现：默认 REPL（不带 `--local`）走 **Daemon 客户端**（`repl_daemon.rs`），只有 `--local` 才走本地
`ConsoleApprover`。此前两条路径审批观感不一致、且默认路径不显示参数。本批统一并下沉：

- **统一审批卡** `ui_output::{PermissionCard, print_permission_card, summarize_permission_args}`：
  工具 / 等级 / 原因 / 风险 / 影响（explain）/ 参数（优先 `redacted_args`）。三条路径共用：
  本地 `print_event(TurnEvent::PermissionRequest)`、Daemon REPL `decide_permission`、
  `turn` 命令 `decide_permission`（human 模式走 stdout，plain/jsonl 不污染）。
- **会话级「总是允许」**（仅 `--local`）：`support::SessionApprovals` 粘性记忆；审批时
  `y=本次 / s=本会话总是允许 / N=拒绝`，已批准工具自动放行；新增 `/approvals [clear]` 查看/清除
  （本地 `Approver` 的 `Decision` 不携带 scope，故在 CLI 侧记忆；Daemon 路径 scope 由服务端处理）。
- **补全下沉共用**：`support::{ReplHelper, ReplEditor, new_repl_editor}`，本地与 Daemon REPL 共用；
  除 slash 命令外新增**文件路径补全**（`src/ma` → `src/main.rs`，目录补 `/`，最多 50 项）。
- `/help` 同步 `/agent`、`/skills reload`、`/approvals`。

**验证**：`cargo check -p owo-agent-cli` exit 0 无 warning；`cargo clippy -p owo-agent-cli --bins` exit 0；
`cargo test -p owo-agent-cli --bins` **31 passed**（新增路径补全 / 非路径不补全 / SessionApprovals 3 例）；
`cargo fmt --check` 干净。

### 9.6 修复：REPL 提示符后"多出很多空格"（rustyline 14 Windows 端 ANSI 宽度误算）

**现象**：输入行里提示符 `build ❯` 与用户输入之间出现一大段空格，光标/输入整体右移。

**根因**（读依赖源码确认，非本项目逻辑）：
- `colored` 在真实控制台（`stdout.is_terminal()`）会输出 ANSI 转义（`\x1b[32m…\x1b[0m`）。
- rustyline 14 的 **Unix** 端 `calculate_position` 用 ANSI 感知的 `width(c, &mut esc_seq)`
  （`tty/unix.rs`）；而 **Windows** 端 `calculate_position` 直接用 `c.width()`
  （`tty/windows.rs`），**把转义字节按可见宽度计入**。
- 于是彩色提示串的 `prompt_size.col` = 可见宽度 + 转义长度，光标被放到多算的列上；
  Windows 控制台/中文 IME 会把输入/组合串画在该光标处 → 表现为提示符后多出空格。

**修复**（不改依赖、向后兼容）：
- 传给 `rustyline::readline` 的提示串改为**纯文本**（`support::repl_prompt`，禁止内嵌 ANSI），
  宽度按可见文本正确计算；
- 颜色改由 `ReplHelper::highlight_prompt` 在**渲染期**添加（`build`→绿、`plan`→黄）；
  若控制台未启用 VT（`colors_enabled()==false`），则自动退化为无色纯文本，仍然正确。
- 本地 REPL 与 Daemon REPL 同步改造。

**回归测试**：`repl_prompt_is_plain_text_without_ansi`（提示串不得含 `\x1b`）、
`highlight_prompt_colors_build_plan_without_changing_text`（着色后可见文本不变）。
`cargo test -p owo-agent-cli --bins` → **33 passed**。
