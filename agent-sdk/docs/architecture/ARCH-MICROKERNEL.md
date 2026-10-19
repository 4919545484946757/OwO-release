# Agent SDK 微内核化重构：边界、顺序与证据

> 施工依据：`builGoal/Agent-SDK-后续任务实施指南-2026-09-18.md` §2（目标架构）、§3（迁移映射）、
> §4（目标仓库结构）、§9（分阶段计划）、§10（Rust 资源红线）。
> 本文记录**每一步微内核拆分**的边界、验收与可复现证据；事实优先级同指南：
> 当前源码与真实运行证据 > 本文件 > 执行记录 > 历史规划。
> 分支：`agent-frame`。

---

## 0. 拆分的判据（为什么这样拆，而不是"看见大文件就切"）

指南 §0.1 的结论是：不做"每个模块一个微服务"，而是

> **一个权威 Agent Daemon + 一个受信工具宿主 + 按需原生 Worker + 多个薄客户端 + 进程内插件系统。**

而 §2.4 又列出**绝对不能拆散的事务边界**（消息顺序、turn 终态、权限判定与执行与审计、
写前快照/写入/diff/revert、会话库迁移、壳对子进程的生命周期）。所以拆分必须按
**事务边界**而不是按文件大小或命名。

一个候选边界只有同时满足下列条件，才允许成为微内核：

1. **自包含**：候选内的模块 `crate::` 引用**不逃出候选集合**（逃出即需先搬运或做依赖倒置）；
2. **入边可 re-export 满足**：外部模块引用候选内部条目时，只要候选被旧 crate
   重新 `pub use`，调用方**零改动**；
3. **无环**：候选借用的外部模块不得反过来引用候选（双向边 = 真环，必须拆到无环或用
   内核 trait 倒置）；
4. **可独立验收**：拆分后候选 crate 能自己编译、自己跑测试，且整个 workspace 仍能
   编译与运行。

**核心手法（本轮确立，后续每步复用）**：把文件 `git mv` 到新 crate，并在旧 crate 的
`lib.rs` 保留**同名别名模块 + 顶层 `pub use`**：

```rust
// crates/owo-agent-core/src/lib.rs
pub use owo_agent_kernel::{
    audit, capability, cas_store, credentials, deadline, error, injection, lease, platform,
    storage_crypto, whitelist,
};
```

这样两类既有路径同时继续有效：

* crate 内相对路径 `crate::audit::AuditLog`（拆分前 25 个 core 源文件使用）；
* 外部路径 `owo_agent_core::audit::AuditLog`、`owo_agent_core::AgentError`
  （`owo-agent-server`、`owo-agent-cli`、36 个 core 集成测试使用）。

于是"每拆一个微内核后仍能完整运行"不是靠多跑一遍全量测试撞运气，而是**由类型系统保证**：
调用方一行没改，编译器立刻能证明接口面没变。

---

## 1. 拆分顺序（按顺序执行，每步必须独立验收）

顺序依据两个量：**出边数**（候选必须借用外部多少模块，需要先搬运或倒置）与
**入边数**（外部有多少模块依赖候选，可用 re-export 免费满足）。入边多而出边少 =
最安全的先拆。

| 步骤 | 微内核 crate | 内容 | 出边 | 入边 | 状态 |
|---|---|---|---:|---:|---|
| **M0** | `owo-agent-kernel` | error/platform/capability/audit/credentials/cas_store/storage_crypto/whitelist/injection/lease/deadline（11 模块 6,604 行）+ 并入 `tool_args` | **0** | 25 个 core 源文件 | ✅ 已完成 |
| **M1** | `devtools/product-eval`（独立 workspace）+ `owo-agent-eval-facade`（门面） | product_eval / eval / dataset_builder / product_eval_workswarm + 6 个集成测试（约 8.6k 行） | 7（必须依赖 core） | **0** | ✅ 已完成，见 §3 |
| **M2** | `owo-agent-extensions` | notes / automation / change_set / change_set_store / cloud_exec（4,493 行） | **0** | **0** | ✅ 已完成，见 §5 |
| **M3** | `owo-agent-tool-safety` | sandbox + audit_chain（2,694 行） | **0** | `mcp` / `plugin` / `tools` + 4 个集成测试 | ✅ 已完成，见 §6 |
| **M4** | （契约，非新 crate） | 三方事务边界契约测试（`execution_boundary_contract_tests.rs`，4 条） | — | — | ✅ 已完成，见 §8 M4 段 |
| **M5** | `owo-agent-env` | desktop_env（2,442 行）；**首次真正的依赖倒置**：`TaskSurface` 下沉内核 | **0**（倒置后） | `transition` / `world_model` / server | ✅ 已完成，见 §7 |
| **M6** | `owo-agent-env`（扩容） | transition + world_model + experience_store（1,710 行）；与 `desktop_env` 同迁，无需倒置 | **0** | `fleet` / `goal` / `node_agent`（经别名，未改代码） | ✅ 已完成，见 §8 |
| **M7** | `owo-agent-plugins` | plugin（1,158 行）+ 从 `mcp` 下沉的 `McpServerConfig`；**倒置方向 = 配置类型随域走** | **0**（倒置后） | server / cli / 3 个集成测试（经别名，未改代码） | ✅ 已完成，见 §9 |
| **M8** | `owo-agent-contracts` | context / computer_task / plan / skill / skill_health（1,491 行 + lib.rs）；**数据形状与执行者分离** | **0** | `agent` / `executor` / `tools` / server / cli（经别名，未改代码） | ✅ 已完成，见 §10 |
| **M9** | `owo-agent-workswarm` | project_space_store / team_benefit / workswarm_output（2,847 行）；**编排的契约与状态先行、执行侧仍留 core** | **0** | `workswarm` / `team_strategy` / `artifact_pipeline` / `contract_worker` / `worker_profile` + server 5 个 api 模块（经别名，未改代码） | ✅ 已完成，见 §11 |
| **M10** | `owo-agent-mcp` | MCP 宿主第一段：`mcp`（646 行）+ 两台假服务器 + 13 条 MCP 集成测试；**零出边** | **0** | `agent` / `tools` / `tool_effects` + server `mcp_api`（经别名，未改代码） | ✅ 已完成，见 §12（server 与冒烟已在 §13 的同一工作树上补齐） |
| **M11** | `owo-agent-memory` | memory + observe + learn（2,454 行）+ `ProactiveSettings` 随域搬入；两处跨 crate 边**已在前面步骤倒置完毕** | **0** | `action_program` / `computer_use` / `executor` / `share_skill` / `workflow` / `settings` + server（经别名，未改代码） | ✅ 已完成，见 §13 |
| **M12** | `owo-agent-policy` | permissions + permission_spec + grant_store + tool_effects（3,068 行）+ 工具命名契约随域下沉 | **0** | `agent` / `tools` / `subagent` / `autoreview` / `settings` + server 权限中心（经别名，未改代码） | ✅ 已完成，见 §14 |
| **M13** | `owo-agent-policy` + `owo-agent-memory`（归位） | `mcp_health`（532）→ policy、`share_skill`（246）→ memory；出边目标已在 M11/M12 变成 crate | **0** | `agent` / `tools` + server（经别名，未改代码） | ✅ 已完成，见 §15 |
| **M14** | `owo-agent-perception` | UIA / OCR / STT / 视觉 / 场景 / 定位（11 模块 5,870 行）；先写 ADR-002；STT 做成可选 feature | **0**（仅 1 条配置类型随域走） | `executor` / `action_program` / `assert` / `computer_use` + server 5 个 api（经别名，未改代码） | ✅ 已完成，见 §17（ADR-002 §2.1 记录了"切边界≠降成本"的实测更正） |
| **M15** | `owo-agent-executor` + `owo-agent-workflow` | executor（1,356）；workflow + action_program + assert（2,888）；**零倒置闭合集** | **0** | `computer_use` / `goal` / server `workflow_api`（经别名，未改代码） | ✅ 已完成，见 §19 |
| **M16** | 剩余均为"总线型"模块（见 §18 末段） | agent / tools / computer_use / session / goal / fleet / workswarm / worker_pool / trace | — | — | ⏳ 待评估（多数应按 §9 A4 分段或留在 core） |

### 实测耦合数据（用于选序，不是估计）

`scratch-eval-runs/dep-graph-clean.json` 由注释/字符串剥离后的 `crate::` 扫描生成，
下表是四个候选边界的**直接**边（不含传递闭包，传递闭包会把 91 模块连成一片，没有决策价值）：

| 候选边界 | 模块数 | 出边 | 入边 | 说明 |
|---|---:|---|---|---|
| `owo-agent-kernel` | 11 | **无** | 25 文件 | 唯一零出边的成规模集合 → 第一步 |
| Perception（OCR/STT/UIA/ONNX） | 15 | 5：`computer_use`、`learn`、`memory`、`settings`、`transition` | 7 文件 | 出边全为**双向**（真环）；且 `computer_use` 反向引用 perception 达 30 处 |
| Policy/Executor（Tool Host 内核） | 8 | 8：`accessibility`、`audit_chain`、`learn`、`locate`、`mcp`、`ocr`、`scene`、`tools` | 16 文件 | 入边最多（指南 §13 要求"Tool Host 权限不可绕过"）；8 条出边中 6 条双向 |
| WorkSwarm（团队编排） | 18 | 15 | 6 文件 | 出边最多；指南 §9 定档为"暂停新增、可选加载" |
| ProductEval（开发工具） | 3 | 7 | **0 文件** | 唯一零入边候选；`eval`/`dataset_builder` 反向被 `workflow`/`action_program` 引用 |
| Fleet（远程节点/云执行） | 5 | 4 | 6 文件 | 与 WorkSwarm 互相引用（`goal`↔`fleet`） |

结论：**M0 之后不存在"下一个零出边大边界"**。候选取舍见 §6 的取舍记录，原则是
"先把编译器与启动链的痛点搬走，且优先选择能用 re-export 满足入边的一侧"。

---

## 2. M0：`owo-agent-kernel`（已完成）

### 2.1 边界

**放进来**（被多个运行边界共用、且不绑定业务状态机的稳定原语）：

| 模块 | 行数 | 归属理由 |
|---|---:|---|
| `platform.rs` | 720 | 前台应用/窗口枚举/截图/剪贴板，各运行边界的公共底座 |
| `storage_crypto.rs` | 888 | DPAPI/DEK/AEAD 信封，Daemon 与 Tool Host 都要用 |
| `credentials.rs` | 599 | 凭据解析与 Windows 凭据管理器封装 |
| `capability.rs` | 561 | 能力卡与路由判定（能力协商原语，非 Agent 业务） |
| `deadline.rs` | 258 | 阶段预算与转/轮次 waterfall（可观测性与多 Agent 共用） |
| `lease.rs` | 270 | 租约管理器 |
| `whitelist.rs` | 222 | 应用分层白名单 |
| `injection.rs` | 195 | 提示注入净化（跨边界安全工具函数） |
| `cas_store.rs` | 174 | 内容寻址存储（快照/diff/revert 的底座） |
| `audit.rs` | 37 | 审计条目（§2.4 第 3 条事务边界的一半） |
| `error.rs` | 17 | 共享错误类型 |

**明确不放进来**：Agent loop / Provider 网关 / 工具注册表 / 会话状态机（留在 core，
按 §9 A2/A4/A5 继续外迁）；OCR/STT/ONNX/Sherpa（未来 Perception Worker）；任何 HTTP 路由。

依赖方向固定为 `owo-agent-kernel ← owo-agent-core ← owo-agent-server ← owo-agent-cli`。

### 2.2 拆分中真实修好的三个缺陷（不是顺手改，是拆分暴露出来的）

1. **`owo-agent-kernel` 缺 `Win32_UI_Input_KeyboardAndMouse` feature**
   —— `platform::activate_window` 使用 `keybd_event`/`VK_MENU`。旧 core 的
   `windows-sys` 开了该 feature，所以 core 内联编译时看不出来；一旦独立成 crate 就
   报 `E0432`。**这正是"独立 crate 才能暴露的隐式耦合"**：原 manifest 里的 feature
   列表实际是为整个 core 服务的，谁真用它并不清楚。已在 kernel manifest 显式声明，
   并把"不得引入 `Media_Ocr`/`UI_Accessibility`/`Graphics_Imaging`"写成注释约束。

2. **集成测试用 `#[path = "../src/storage_crypto.rs"]` 复制编译源文件**
   —— `crates/owo-agent-core/tests/os_sandbox_integration_tests.rs` 原先把
   `storage_crypto.rs` 再编译一份做 DPAPI 冒烟。文件搬走后该路径失效；即使不搬走，
   它验证的也**不是真正链接进二进制的那份实现**。已改为
   `use owo_agent_kernel::storage_crypto;` + `::*`，从此验证真实产物。

3. **`scripts/ci-shared.ps1` 会掩盖 cargo 的真实退出码**（红线 10 的漏洞）
   —— `Invoke-CiCargo` 靠 `$global:LASTEXITCODE` 收口，但 `$LASTEXITCODE` 只在原生
   命令结束时由 PowerShell 自动写入，经函数调用后丢失；再叠加
   `Assert-CiBuildIdle`/`New-CiFailureState` 的布尔返回值混进输出流，调用方会拿到
   `@($true, 101)` 并退化成 exit 1。实测表现：`cargo check` 因缺 feature 失败（101），
   脚本却报 `exit=1` 且没有任何理由，看起来像"门禁说不出为什么红"。
   已修：新增 `-PassThru` 返回**纯标量**退出码；`$null = Assert-CiBuildIdle`、
   `$null = New-CiFailureState` 丢弃泄漏值。负例已验证（见 §2.4）。

### 2.3 改动文件

| 文件 | 变更 |
|---|---|
| `Cargo.toml`（workspace） | 新增成员 `crates/owo-agent-kernel`、新增 workspace 依赖 |
| `Cargo.lock` | 新增 `owo-agent-kernel` 包与其依赖闭包（无新增外部 crate） |
| `crates/owo-agent-kernel/Cargo.toml`、`src/lib.rs` | 新 crate 清单与边界文档 |
| `crates/owo-agent-core/src/{audit,capability,cas_store,credentials,deadline,error,injection,lease,platform,storage_crypto,whitelist}.rs` | `git mv` 到 kernel（git 识别为 R 重命名，历史保留） |
| `crates/owo-agent-core/Cargo.toml` | 新增 `owo-agent-kernel.workspace = true` |
| `crates/owo-agent-core/src/lib.rs` | 删除 11 个 `pub mod`，加内核兼容层别名 + 注释 |
| `crates/owo-agent-core/tests/os_sandbox_integration_tests.rs` | 改引真实 crate，弃用 `#[path]` |
| `scripts/ci-shared.ps1` | `-PassThru` 真实退出码修复（红线 10） |
| `scripts/mk-check.ps1` | 新增：本重构专用的 `--workspace --all-targets` 门禁入口 |
| `scripts/mk-smoke.ps1` | 新增：运行态验收（真实进程 + 真实 HTTP + 真实落盘 + 无孤儿进程） |
| `docs/architecture/ARCH-MICROKERNEL.md` | 新增：本文（边界、顺序、证据、复用清单） |

**调用方零改动**：`owo-agent-server`（101 文件）、`owo-agent-cli`（21 文件）、
36 个 core 集成测试、Tauri 壳均未修改。

### 2.4 验收证据（可复现）

| 验收项 | 命令 | 结果 | 证据 |
|---|---|---|---|
| **拆分前基线**：workspace 全目标编译 | `scripts/mk-check.ps1 -Tag baseline` | **exit=0**，189 s | `docs/qa/logs/mk-baseline-*.log` |
| 拆分后 workspace 全目标编译 | `scripts/mk-check.ps1 -Tag m0-kernel2` | **exit=0**，52 s | `docs/qa/logs/mk-m0-kernel2-*.log` |
| 内核 crate 自测（独立编译 + 36 单测） | `cargo test -p owo-agent-kernel --locked` | **36 passed / 0 failed** | `docs/qa/logs/mk-kernel-tests-*.log` |
| 内核依赖闭包**不含**重原生依赖与上层 crate | `cargo tree -p owo-agent-kernel --edges normal` | `ort`/`sherpa`/`ndarray`/`rusqlite`/`axum`/`tokio`/`owo-agent-core`/`owo-agent-server` 出现次数**全部为 0**；闭包只剩 crypto/serde/chrono/uuid/windows-sys | `docs/qa/logs/mk-kernel-tree.log` |
| 全量 core 测试（含 36 个集成测试文件） | `cargo test -p owo-agent-core --locked` | **exit=0**，363 s（1 个 `--ignored` 真模型用例按设计跳过） | `docs/qa/logs/mk-core-tests-*.log` |
| **运行态验收**（真实进程 + 真实 HTTP + 真实落盘） | `scripts/mk-smoke.ps1 -Tag m0-kernel` | **13/13 PASS**，ok=True | `docs/qa/evidence/mk-smoke-m0-kernel-*/report.json` |
| `cargo fmt --all -- --check` | 经 `Invoke-CiCargo` | **exit=0** | 会话记录 |
| 退出码不被掩盖（负例） | `Invoke-CiCargo -Arguments @('check','--workspace') -Cwd <不存在的目录> -PassThru` | 返回纯标量 `Int32 1`，非数组 | 会话记录（§2.2 第 3 条） |
| 所有改动文件编码 | 逐文件 `UTF8Encoding(throwOnInvalid:true)` 解码 + mojibake 标记扫描 | 23 个文本文件全部合法 UTF-8，无 mojibake；3 个 `.ps1` 均带 BOM | 会话记录 |

运行态验收的 13 项覆盖（`scripts/mk-smoke.ps1`，退出码 0/1）：

```text
[PASS] daemon.core_ready            core_ready 行已输出（端口 55054，api_version 0.7）
[PASS] http.health_public           /health 200 healthy=True stage=ready
[PASS] http.unauthenticated_401     无 token 的 GET /sessions -> 401（权限面未被拆分削弱）
[PASS] data.auth_token_written      auth/token 已落盘（不打印内容）
[PASS] http.sessions_list           带 token -> 200
[PASS] http.server_status_readable  storage.read_only=False，无迁移告警
[PASS] session.create               POST /session -> 200，真实会话 id
[PASS] session.read_back            GET /session/{id} -> 200
[PASS] session.persisted_in_list    列表中读回 1 条
[PASS] data.index_db                index.db（+ WAL/SHM）已建立
[PASS] data.server_pid              server.pid 已建立
[PASS] audit.event_emitted          server_start 审计事件落盘（kernel::audit 参与运行）
[PASS] process.no_orphan            停止后无残留进程
```

> 资源合规：所有 cargo 调用均经 `scripts/ci-shared.ps1` 的 `Invoke-CiCargo`
> （完整 workspace 档 `-j 1`/`--test-threads=1`，定向档 `-j 2`），并在启动前通过
> `Resolve-OwoOrtEnv` 注入 ORT、执行内存门与磁盘门。本轮实测磁盘事件：
> 构建前 T: 仅剩 **24.7 GB**（低于 §10 红线 5 的 20 GB 全量档要求），按 AGENTS.md
> 允许清单清理 `target/**/incremental` 与 `target/**/*.pdb` 后回收 **25.9 GB**，
> 盘余 **50.6 GB** 才启动构建。

---

## 3. M1：`devtools/product-eval` + `owo-agent-eval-facade`（已完成）

### 3.1 边界与依赖方向

ProductEval 底座（`product_eval` 3,317 行 + `eval` + `dataset_builder` +
`product_eval_workswarm` + 6 个集成测试，约 8.6k 行）整体迁出 core：

```text
server / cli ──► owo-agent-eval-facade ──► devtools/product-eval ──► owo-agent-core
                                                  ▲
                                                  └─ workspace 成员（独立解析，不回流）
core 对 devtools 的依赖 = 0
```

* `devtools/product-eval/` 是**独立 workspace**（自带 `Cargo.lock` 与 `target/`），
  对齐指南 §9 对它的定位：开发时加载、不进生产默认运行时、不拖累用户启动与 Rust 编译。
* `crates/owo-agent-eval-facade` 是 workspace 成员里的薄门面（11 行代码 + 边界文档），
  只做 `pub use owo_agent_product_eval::*;`，让 server/cli 继续用熟悉路径拿评测面。
* **core 不再持有任何评测面**：`pub mod product_eval` / `pub use product_eval::*` /
  `#[path = "product_eval/workswarm_executor.rs"]` 全部删除。

### 3.2 为什么不能用 optional dependency + feature（三条路都实测撞环）

| 尝试 | 结果 |
|---|---|
| product-eval 作为 workspace 成员 + core `optional` 依赖 + 成员写 `default-features = false` | Cargo 警告该开关被忽略（须写在 workspace 定义处），随后 `cyclic package dependency` |
| 把 `default-features = false` 写到 workspace 定义处 | server/cli 需要评测面 → 打开 core 的 `product-eval` feature；**feature 是并集**，devtool 那条 core 边被重新点亮 → 再次成环 |
| devtool 移出 `crates/`、加 workspace `exclude` | path 依赖仍被解析进同一个 package 实例 → 第三次成环 |
| **最终**：devtool 成为独立 workspace + core 零依赖 + 门面 crate 承接消费方 | ✅ 成立，且方向更正确（受信运行时不依赖开发工具） |

### 3.3 拆分暴露并修好的四个缺陷

1. **`workswarm_executor` 从未真正成为模块**：core 用
   `#[path = "product_eval/workswarm_executor.rs"] pub mod product_eval_workswarm;`
   把它挂在 crate 根。搬到新 crate 后 `pub use product_eval::workswarm_executor` 直接
   `E0432`（`no workswarm_executor in product_eval`）。已改为 `product_eval` 的正式子模块。
2. **`required_string` 是 core 内的死代码**：它是 `owo-agent-core::tools` 的
   `pub(crate)`，core 内部**零调用**，唯一真实使用者是开发工具包。已迁到内核
   `owo_agent_kernel::tool_args::required_string`，core 侧改为 `use owo_agent_kernel::required_string;`
   （涉及 `tools.rs` 7 处、`computer_use.rs` 9 处调用点）。**这正是"独立 crate 才能暴露的
   隐式耦合"**：一个 `pub(crate)` 助手把内核原语寄生在 core 里，同时暴露了 core 用不到的
   宽度。
3. **两个 core 集成测试依赖开发工具面**：`transition_tests.rs` / `world_model_tests.rs`
   原先 `use owo_agent_core::dataset_builder::…`。core 已不持有它，改为通过
   **dev-dependency** 引门面（正常依赖仍为零；Cargo 的环检测不覆盖 dev 边）。
4. **测试里写死的仓库相对深度会随目录搬迁失效**：
   `CARGO_MANIFEST_DIR/../../evals/v1/suite.json` 在 `crates/owo-agent-core/tests`
   下是对的，搬到 `devtools/product-eval/tests` 后少了（后来多了）一层。已按最终位置
   校正为 `../../evals/...`，并实测 `Test-Path` 通过。

### 3.4 验收证据（可复现）

| 验收项 | 命令 | 结果 | 证据 |
|---|---|---|---|
| workspace 全目标编译（含门面与 server/cli 重接线） | `scripts/mk-check.ps1 -Tag m1 -WithDevtool` | workspace **exit=0**（163 s）、devtool **exit=0**（32 s） | `docs/qa/logs/mk-m1-*.log` |
| **全量 core 测试**（461 单测 + 36 集成测试文件） | `cargo test -p owo-agent-core --locked` | **exit=0**，330 s | `docs/qa/logs/mk-m1-core-tests-*.log` |
| **全量 server 测试**（63 单测 + 40 个集成测试文件，含 `product_eval_api_tests`、`eval_gate_tests`、`route_contract_tests`） | `cargo test -p owo-agent-server --locked` | **exit=0**，326 s（首轮被 §2.4 红线 7 内存门以 137 中止，清内存后复跑通过） | `docs/qa/logs/mk-m1-server-tests2-*.log` |
| **迁走的 ProductEval 测试在独立 workspace 全绿** | `cargo test --manifest-path devtools/product-eval/Cargo.toml` | **66 passed / 0 failed**（1 个真模型用例按设计 `--ignored`） | `docs/qa/logs/mk-m1-devtool-tests2-*.log` |
| **运行态**：ProductEval 路由真实走通门面 | `scripts/mk-smoke.ps1 -Tag m1-product-eval3` | **15/15 PASS**；`POST /product-eval/runs` → 202 + `run_id`，轮询到 `completed`，进度 **20/20**，`metrics.runs_total=20` | `docs/qa/evidence/mk-smoke-m1-product-eval3-*/report.json` |
| `cargo fmt --all`（两个 workspace） | 经 `Invoke-CiCargo` | 均 exit=0 | 会话记录 |

运行态验收追加的两项（相对 M0 的 13 项）：

```text
[PASS] product_eval.create_run     status=202 run_id=eval-089acbadd3114a759c63f9c39747eac9
[PASS] product_eval.run_completed  status=completed progress=20/20 metrics.runs_total=20
```

> 踩坑记录：ProductEval 运行态门**不能另起第二个服务实例**——`serve.rs` 用 pid 文件做
> 单实例闸门，实测第二次启动直接报“检测到运行中的服务（pid=…）：请先停止该进程再启动”。
> 现在改为单实例 + 把仓库 `evals/` 以目录联接挂进隔离工作区。

### 3.5 改动文件

| 文件 | 变更 |
|---|---|
| `Cargo.toml`（workspace） | 新增成员 `crates/owo-agent-eval-facade`；新增 `exclude = ["devtools/product-eval"]` 及环依赖说明 |
| `crates/owo-agent-eval-facade/` | 新增门面 crate（`pub use owo_agent_product_eval::*`） |
| `devtools/product-eval/` | 新增独立 workspace（`product_eval` / `eval` / `dataset_builder` / `workswarm_executor` + 6 个集成测试 + `.gitignore`） |
| `crates/owo-agent-core/{Cargo.toml,src/lib.rs}` | 移除评测面与 `product-eval` feature；新增 dev-dependency 门面；删除 `required_string` 死代码 |
| `crates/owo-agent-core/src/{tools,computer_use}.rs` | 改用 `owo_agent_kernel::required_string` |
| `crates/owo-agent-core/tests/{transition,world_model}_tests.rs` | 改引门面 |
| `crates/owo-agent-server/{Cargo.toml,src/{lib,eval_api,eval_gate,product_eval_api,desktop_world_api}.rs,tests/product_eval_api_tests.rs}` | 评测面来源改为 `owo_agent_eval_facade`（20 处） |
| `crates/owo-agent-cli/{Cargo.toml,src/{commands/eval.rs,product_eval_cmd.rs}}` | 同上 |
| `crates/owo-agent-kernel/src/{lib.rs,tool_args.rs}` | 新增 `tool_args` 模块（`required_string`） |
| `scripts/mk-check.ps1` | 新增 `-WithDevtool`：把被排除的独立 workspace 纳入同一轮验证 |
| `scripts/mk-smoke.ps1` | 新增 ProductEval 运行态门（路由 → 门面 → 开发工具全链） |
| `docs/architecture/ARCH-MICROKERNEL.md` | 本文 §5 |

---


---

## 4. 复用清单（M1 及以后每一步都照这个模板执行）

```text
1. 选边界：算出边/入边（注释剥离后的 crate:: 扫描，脚本见 scratch-eval-runs/dep-graph-clean.json 生成方式）
2. 查可见性：候选内是否有 pub(crate) 条目被外部使用（有则先升为 pub 或改内聚）
   —— 必须**在 git mv 之前**跑：`Select-String -Path <候选> -Pattern 'pub\(crate\)'` 逐条判断；
   M8 把这条写进清单却没先跑，于是被 E0603 挡在第一次 check 上
3. 查宏：宏定义无法跨 crate 用 `crate::` 访问，候选内有 macro_rules! 就要先规划
4. git mv：保留历史；不要复制
5. 写新 crate：Cargo.toml（依赖最小化 + 边界注释）、src/lib.rs（模块 + 顶层 re-export 1:1 对齐）
6. 旧 crate：删 pub mod，加同名别名模块 + pub use
7. cargo fmt --all
8. 生成锁文件：cargo metadata --format-version 1（离线失败时先跑一次不带 --locked 的构建）
9. 验收：新 crate 单测 + workspace --all-targets 编译 + 运行态冒烟（`scripts/mk-smoke.ps1`）
10. 证据落盘（`docs/qa/logs/`、`docs/qa/evidence/`）并在本文追加一行
```

### 反复出现的坑（已踩）

| 坑 | 症状 | 处置 |
|---|---|---|
| feature 隐式依赖旧 crate 的 manifest | 独立后 `E0432`/`E0433` | 新 crate 显式声明自己真正用到的 feature；在 manifest 写"不得引入"约束 |
| 测试用 `#[path]` 复制源文件 | 文件搬走后编译失败；且验的不是真实实现 | 改为引用新 crate |
| `$LASTEXITCODE` 经函数调用丢失 | 门禁报 1 而 cargo 实际是 101 | `Invoke-CiCargo -PassThru` 返回标量退出码 |
| PowerShell 函数返回值混入输出流 | `-PassThru` 返回 `@($true, 101)` | 调用处 `$null = ...` 丢弃；`-PassThru` 再做标量兜底 |
| `.ps1` 编辑后丢 UTF-8 BOM | `ci-gate -Step utf8` 会红 | 每次改完 `.ps1` 立即复查前三字节 `239,187,191` |
| 无中间目录的相对日志路径 | `StreamWriter` 抛 `DirectoryNotFoundException` | 传绝对路径 |
| **开发工具与运行时互相依赖** | `cyclic package dependency: <crate> depends on itself` | devtool 必须是**独立 workspace**；运行时对它的依赖为零；消费方经薄门面 crate 接入（详见 §3.2） |
| `Exclude` 挡不住 path 依赖 | 已 exclude 仍报环 | exclude 只挡自动成员，不挡 `path =` 引用；真正的隔离要靠独立 workspace + 零反向依赖 |
| workspace 级 `default-features = false` 也挡不住 | 已关默认 feature 仍报环 | Cargo feature 是**并集**，任一成员打开就重新点亮；不要指望用 feature 关掉一条会成环的边 |
| 测试写死仓库相对深度 | 目录搬迁后 `suite.json 不存在` | 用最终位置校正 `CARGO_MANIFEST_DIR` 的相对层数，并在验收前用 `Test-Path` 实测解析结果 |
| 服务端单实例闸门 | 第二次 `serve` 直接退出，无 `core_ready` | 同一轮验收只用**一个**服务实例；需要套件可见时把 `evals/` 联进隔离工作区 |
| 验收门自己写错契约导致的假红 | `POST /notes` 返回 201 但断言 200；`GET /notes` 断言裸数组但实际是 `{count, notes}` | 写运行态断言前先读服务端 handler 与请求/响应结构，不要凭直觉 |
| 用"入边/出边计数"选边界 | 反复撞 `cyclic package dependency` | 改用 **Tarjan SCC + 分量 DAG**：只有"零出边 + 零入边"的分量才能零代价切走（见 §5.1） |
| 把"模块互相引用"直接当成"必须接口倒置" | 多写一堆不必要的 sink/trait | 先问**这条边搬迁后是否跨越 crate 边界**：① 列出出边 ② 判断目标是否与它**同迁** ③ 只有"不同迁且目标反向引用它"的边才是真环。M3 实测 `audit_chain ↔ sandbox` 同迁一 crate → 零倒置（见 §6.1） |
| 契约测试用"被沙箱管着的进程"做正面对照 | 对照恒失败（沙箱正确地拒绝绝对路径写入），断言空转 | 需要"证明某命令确实会写文件"时，必须用**裸进程**（`std::process::Command`）做对照；沙箱内只验证被约束后的行为 |
| 在 `cmd /C` 里裸用 `&&` + 重定向 | exit=1，「语法不正确」 | 复合运算符与重定向混用会解析失败；拆成单条命令 |
| 给不含空格的路径加引号传给 `cmd` | exit=1，「文件名、目录名或卷标语法不正确」 | temp_dir 路径不含空格时**不要加引号**，引号会被当字面量 |
| 用 `use` 行扫描生成新 crate 的依赖清单 | 首次编译报 `E0433: cannot find module or crate chrono/uuid` | 依赖要按 **`use` 行**与**内联全限定路径**（`chrono::Utc::now()`、`uuid::Uuid::new_v4()`）各核一遍。M3 的 `sandbox` 恰好没有这类写法，所以这个坑到 M5 才暴露 |
| 手写 `pub use x::{符号表}` 做迁移再导出 | 首次编译报 E0432（符号名抄错） | 迁移类再导出一律用**glob**（`pub use x::*;`），让公共面等价性由编译器证明；M2 用 glob 所以没暴露，M6 手写就立刻撞上 |
| 搬类型时只确认了顶层 `pub use` | `owo_agent_core::mcp::McpServerConfig` 报 E0603（private）；或遗留 `use crate::mcp::X` 报 E0432 | **顶层 `pub use` 与 `模块::类型` 是两条不同路径**，搬类型后要逐个确认都仍可解析：在源模块写 `pub use <新crate>::X;`，并清掉指向已搬走路径的旧 `use` |
| 搬迁把 `pub(crate)` 的"够用可见性"变成不够用 | `E0603: function is private`（M8 的 `skill::parse_frontmatter`） | 旧 crate 内的 `pub(crate)` 跨 crate 后只剩本 crate 可见；**搬迁前先扫候选里的 `pub(crate)` 条目**，凡外部消费的提权为 `pub` 并补 doc 说明 |
| 只用"有能力的模块"列候选 | 漏掉整整一类零代价边界（M8 的 5 个纯数据模块） | 候选清单要两路出：① 零出边 + 零入边的**能力**模块；② 只有数据形状（struct/enum + 序列化 + 纯函数）的**契约**模块 → 后者应放进依赖最少的 crate |
| 构建卷空间不足 | `LNK1318 非意外的 PDB 错误: LIMIT`（看着像编译错误） | 先清 `target/**/incremental`、`target/**/*.pdb`（M0–M3 共回收约 67 GB） |


## 5. M2：`owo-agent-extensions`（已完成）

### 5.1 用强连通分量（SCC）找"真正能切"的边界

M1 的教训是：靠"入边/出边计数"选边界会反复撞环。M2 改用**结构性判据**——对
`owo-agent-core` 的 76 个模块依赖图（注释/字符串剥离后的真实 `crate::` 引用）跑
Tarjan 强连通分量，把图压缩成分量 DAG。结论：

* 共 **46 个分量**，其中三个是真实环团：
  * **[16] 18 模块 / 16,502 行**：`agent`、`tools`、`permissions`、`session`、`gateway`、
    `executor`、`computer_use`、`learn`、`mcp_health`、`settings`、`sqlite_store`、
    `subagent`、`contract_worker`、`autoreview`、`grant_store`、`permission_spec`、
    `tool_effects`、`schema_budget`；
  * **[26] 8 模块 / 7,015 行**：`fleet`、`goal`、`bus_store`、`execution_target`、
    `worker_pool`、`remote_step`、`fleet_transport`、`fleet_node_protocol`；
  * **[28] 4 模块 / 6,700 行**：`workswarm`、`team_strategy`、`team_prompt`、
    `builtin_team_templates`。
* **完全可分离集合（零出边 + 零入边）只有 5 个模块、4,493 行**：
  `notes`(1,449)、`cloud_exec`(1,316)、`change_set`(838)、`change_set_store`(552)、
  `automation`(338)。

这就是 M2 的对象。判据很硬：**零出边** ⇒ 不依赖 core 任何模块（可独立编译）；
**零入边** ⇒ core 内部无人引用（只需别名 re-export，调用方零改动）；两者同时成立
⇒ **不可能形成 crate 环**。

> 方法可复用：依赖图的生成方式见 §4 第 1 步；SCC 脚本是一次性分析工具，结论已固化
> 在本文，不需要每次重跑。

### 5.2 边界与验收

* 依赖方向：`owo-agent-core ──► owo-agent-extensions ──► owo-agent-kernel`，且 extensions
  **不依赖 core / server / ONNX / Sherpa**（`cargo tree` 实测：这些名字在依赖闭包中
  出现 0 次）。
* core 侧只做一件事：`pub use owo_agent_extensions::{automation, change_set,
  change_set_store, cloud_exec, notes};`（同名别名模块），原有逐条 `pub use` 继续工作。
* **服务端与 CLI 一行未改**（`git status` 实测：M2 只动了 `Cargo.toml`、`Cargo.lock`、
  core 的 `Cargo.toml`/`lib.rs` 与 5 个 `git mv`）——这是"re-export 透明"的直接证据。

关键裁决点（写进 crate 文档，避免后人搬一半）：指南 §2.4 要求"文件写前快照、写入、
diff 和 revert"保持在同一拥有者内。本 crate **只承载快照与恢复的状态机/存储**
（`change_set` / `change_set_store`），真正的写入仍由 core 的 `executor`/`tools` 执行；
迁到 Tool Host（§9 A4）时必须整体复核。

| 验收项 | 命令 | 结果 | 证据 |
|---|---|---|---|
| workspace 全目标编译 | `check --workspace --all-targets`（经 `Invoke-CiCargo -j 1`） | **exit=0**，159 s | `docs/qa/logs/mk-m2-check-*.log` |
| 全量 core 测试 | `cargo test -p owo-agent-core --locked` | **exit=0**，328 s（439 单测 + 36 集成测试文件；比 M1 少 22 个单测，因为随 5 个模块迁走） | `docs/qa/logs/mk-m2-core-tests-*.log` |
| **全量 server 测试**（迁移模块的真实消费者：`notes_api_tests`、`change_set_api_tests`、`human_inbox_api_tests`、`cloud_sse_tests`） | `cargo test -p owo-agent-server --locked -j 1 -- --test-threads=1` | **exit=0**，512 s（63 单测 + 40 集成测试文件全绿；首轮被 §2.4 红线 7 内存门以 137 中止，清内存 + 降并发后复跑通过，未绕过门） | `docs/qa/logs/mk-m2-server-tests2-*.log` |
| 依赖闭包不含 core/server/ONNX | `cargo tree -p owo-agent-extensions` | `owo-agent-core`/`owo-agent-server`/`ort`/`sherpa`/`ndarray` 出现 **0 次** | `docs/qa/logs/mk-m2-tree.log` |
| **运行态**（新增扩展路由门） | `scripts/mk-smoke.ps1 -Tag m2-final` | **18/18 PASS**；`POST /notes` → 201 + id，`GET /notes` → `count=1` 命中，`GET /automations` → 200 | `docs/qa/evidence/mk-smoke-m2-final-*/report.json` |
| `cargo fmt --all` | 经 `Invoke-CiCargo` | exit=0 | 会话记录 |

运行态新增的三项：

```text
[PASS] extensions.notes_create       status=201 id=b61dc9c8-2287-48ee-b742-52603b99f6ab
[PASS] extensions.notes_list         count=1 命中=1（notes 已迁至 owo-agent-extensions）
[PASS] extensions.automations_list   status=200 body=[]
```

> 写这两项时踩了两个契约坑（已修正断言）：`create_note` 的契约状态码是 **201**（CREATED）
> 而不是 200；`list_notes` 返回 `{count, notes:[...]}` **对象信封**而不是裸数组。
> 这类"门自己写错、看起来像功能坏了"的假红，正是 §4 要求先查契约再写断言的原因。

### 5.3 改动文件

| 文件 | 变更 |
|---|---|
| `Cargo.toml`（workspace） | 新增成员 `crates/owo-agent-extensions` + workspace 依赖 |
| `crates/owo-agent-extensions/{Cargo.toml,src/lib.rs}` | 新 crate（边界文档 + 五个模块 + glob 再导出） |
| `crates/owo-agent-core/src/{notes,cloud_exec,change_set,change_set_store,automation}.rs` | `git mv` 到新 crate（其中 2 个文件仅把 `crate::cas_store`/`crate::audit` 改指内核） |
| `crates/owo-agent-core/{Cargo.toml,src/lib.rs}` | 新增依赖 + 别名 re-export |
| `scripts/mk-smoke.ps1` | 新增扩展路由运行态门 |
| `crates/owo-agent-server`、`crates/owo-agent-cli` | **未改动** |



---

## 6. M3：`owo-agent-tool-safety`（已完成）

设计与决定见 **`docs/adr/ADR-001-tool-safety-kernel.md`**；本节是摘要 + 与 ADR 的差异。

### 6.1 结果：比 ADR 原计划简单一半

ADR 原计划「先用注入式 `SandboxAuditSink` 打断环、再整体搬迁」。实测发现**前半步不必要**：

* `audit_chain` 与 `sandbox` 的相互引用**只发生在这一对内部**
  （`audit_chain.rs:14 use crate::sandbox::SandboxAuditLog`；
  `sandbox.rs:637 chain: &mut crate::audit_chain::AuditChain`）；
* 两个模块是**一起**搬进同一个新 crate 的 → 该边不再跨越任何 crate 边界 → 无需倒置。

实际实施 = `git mv` 两个模块 + core 保留别名 re-export，**零接口改动、零调用方改动**。
代价面：`audit_chain.rs` 只改 3 处内核引用（`crate::credentials` / `crate::storage_crypto` ×2
→ `owo_agent_kernel::*`），`sandbox.rs` **一行未改**。

### 6.2 由此修正的判断规则（已写入 §4 复用清单）

原 ADR 把"两个模块互相引用"直接等同于"必须接口倒置"，漏问了一个更基本的问题：
**这条边在搬迁后是否会跨越 crate 边界？** 正确顺序是：

```text
① 列出该模块的全部出边；
② 判断这些目标是否与它「同迁」；
③ 只有「不同迁、且目标反向引用它」的边，才是必须倒置的真环。
```

### 6.3 额外发现：tool-safety 连 Windows 绑定依赖都不需要

`sandbox` 的 Windows 部分是**裸 FFI**（`extern "system"` + `#[link(name =
"kernel32"/"advapi32"/"ntdll")]`，共 3 个 extern 块），**完全不使用 `windows` /
`windows-sys`**（core 里用这两个 crate 的其实是 `executor.rs`(31 处) / `ocr.rs`(4) /
`accessibility.rs`(4)）。因此新 crate 的依赖闭包只有
`owo-agent-kernel` + serde / serde_json / chrono / uuid / sha2 / thiserror —— 对指南 §10
「普通 Agent 改动不触发原生重链」是直接利好。

### 6.4 验收证据

| 验收项 | 命令 | 结果 | 证据 |
|---|---|---|---|
| workspace 全目标编译 | `check --workspace --all-targets`（`-j 1`） | **exit=0**，157 s | `docs/qa/logs/mk-m3-check-*.log` |
| 全量 core 测试（含全部安全契约） | `cargo test -p owo-agent-core --locked` | **exit=0**，320 s；`sandbox_tests`(26)、`os_sandbox_integration_tests`(25)、`production_security_contract_tests`(12) 与 audit_chain 篡改矩阵全绿 | `docs/qa/logs/mk-m3-core-tests-*.log` |
| 全量 server 测试 | `cargo test -p owo-agent-server --locked -j 1 -- --test-threads=1` | **exit=0**，995 s（63 单测 + 40 集成测试文件全绿） | `docs/qa/logs/mk-m3-server-tests-*.log` |
| 依赖闭包 | `cargo tree -p owo-agent-tool-safety` | `owo-agent-core`/`owo-agent-server`/`sherpa`/`ndarray`/`rusqlite`/`windows` 均 **0 次**；`owo-agent-*` 只有 kernel 与自身 | `docs/qa/logs/mk-m3-tree.log` |
| 调用方零改动 | `git status` | `mcp.rs` / `plugin.rs` / `tools.rs` / server / cli **未改动** | 会话记录 |
| 运行态 | `scripts/mk-smoke.ps1 -Tag m3-tool-safety` | **18/18 PASS**（含审计事件落盘、无孤儿进程） | `docs/qa/evidence/mk-smoke-m3-tool-safety-*/report.json` |
| `cargo fmt --all` | 经 `Invoke-CiCargo` | exit=0 | 会话记录 |


## 7. M5：`owo-agent-env`（已完成）——首次真正的依赖倒置

### 7.1 这一步与前几步的本质差别

M0–M3 都是"找到零出边（或同迁）的集合整体搬走"。M5 不是：

* `desktop_env`（2,442 行）的唯一出边是 `crate::computer_use::TaskSurface`；
* `desktop_env` 与 `computer_use` **不同迁**（后者属 18 模块环团 `[16]`，留在 core）；
* 因此按 §4 判据，这条边**必须倒置**，否则外迁后 `desktop_env → computer_use`
  与 `core → desktop_env` 成环。

倒置方式：把 `TaskSurface` 这个**纯 I/O 契约**下沉到内核
（`owo-agent-kernel::task_surface`）。它只用到 `&str` / `i32` / `serde_json::Value`，
不绑定任何业务类型——因此**没有**把"某个执行器的数据类型"带进内核（那正是 ADR-001
里被否决的做法）。实现体 `SimTaskSurface` / `RealTaskSurface` 仍留在
`owo-agent-core::computer_use`，内核只承载契约，并在 core 侧以
`pub use owo_agent_kernel::TaskSurface;` 保持既有路径可用。

倒置后 `desktop_env` 的 `crate::` 引用**实测为空集**，于是整体搬迁成为可能。

### 7.2 验收证据

| 验收项 | 命令 | 结果 | 证据 |
|---|---|---|---|
| workspace 全目标编译 | `check --workspace --all-targets`（`-j 1`） | **exit=0**，166 s | `docs/qa/logs/mk-m5-check2-*.log` |
| 全量 core 测试（含 desktop_env / TaskSurface 适配器契约） | `cargo test -p owo-agent-core --locked` | **exit=0**，372 s；`desktop_env_tests`(19) 与 `surface_adapter_observes_and_acts_but_declares_limits` 全绿 | `docs/qa/logs/mk-m5-core-tests-*.log` |
| 全量 server 测试 | `cargo test -p owo-agent-server --locked -j 1 -- --test-threads=1` | **exit=0**，1,015 s（63 单测 + 40 集成测试文件全绿，含 `desktop_world_api_tests` 14 条） | `docs/qa/logs/mk-m5-server-tests-*.log` | `docs/qa/logs/mk-m5-server-tests-*.log` |
| 依赖闭包 | `cargo tree -p owo-agent-env` | `owo-agent-core`/`owo-agent-server`/`sherpa`/`ndarray`/`rusqlite` 均 **0 次**；`owo-agent-*` 只有 kernel 与自身 | `docs/qa/logs/mk-m5-tree.log` |
| 运行态 | `scripts/mk-smoke.ps1 -Tag m5-env` | **18/18 PASS** | `docs/qa/evidence/mk-smoke-m5-env-*/report.json` |
| 调用方改动 | `git status` | server 的 `desktop_world_api.rs`（3 处）与 devtool 的 `dataset_builder.rs`（1 处）改指新 crate；core 的 `transition`/`world_model` 走别名未改 | 会话记录 |
| `cargo fmt --all` | 经 `Invoke-CiCargo` | exit=0 | 会话记录 |

### 7.3 又一次被 `use` 扫描漏掉的依赖（新坑）

新 crate 首次编译即报 3 个 `E0433`：`chrono::Utc::now()` / `chrono::Utc::now()
.timestamp_millis()` / `uuid::Uuid::new_v4()` —— 这三处是**全限定内联路径**，
基于 `use` 行扫描生成的依赖清单看不到它们。M3 的 `sandbox` 恰好没有这类写法，
所以这个坑到 M5 才暴露。已写入 §4 坑表：**搬模块时依赖要按 `use` 行与内联全限定路径
各核一遍**。

### 7.4 改动文件

| 文件 | 变更 |
|---|---|
| `Cargo.toml`（workspace） | 新增成员 `crates/owo-agent-env` + workspace 依赖 |
| `crates/owo-agent-env/{Cargo.toml,src/lib.rs}` | 新 crate（边界文档 + desktop_env + 顶层再导出） |
| `crates/owo-agent-core/src/desktop_env.rs` | `git mv` 到新 crate（0 处改动，文件原样） |
| `crates/owo-agent-kernel/src/{lib.rs,task_surface.rs}` | 新增 `task_surface` 模块（依赖倒置产物） |
| `crates/owo-agent-core/src/computer_use.rs` | trait 定义替换为 `pub use owo_agent_kernel::TaskSurface;` |
| `crates/owo-agent-core/{Cargo.toml,src/lib.rs}` | 新增依赖 + 别名 re-export |
| `crates/owo-agent-server/{Cargo.toml,src/desktop_world_api.rs,tests/desktop_world_api_tests.rs}` | 改指 `owo-agent_env::desktop_env` |
| `devtools/product-eval/{Cargo.toml,src/dataset_builder.rs}` | 改指 `owo-agent_env::desktop_env` |
| `crates/owo-agent-core/tests/desktop_env_tests.rs` | `TaskSurface` 显式走内核 |


## 8. M6：桌面世界模型栈并入 `owo-agent-env`（已完成）

### 8.1 为什么这一组能整体搬

M5 已把 `desktop_env` 放进 `owo-agent-env`。剩余的桌面世界模型栈三件套正好只依赖它：

| 模块 | 行数 | 出边 |
|---|---:|---|
| `transition` | 506 | `desktop_env`（同 crate）+ `experience_store`（同组） |
| `world_model` | 663 | `desktop_env`（同 crate）+ `transition`（同组） |
| `experience_store` | 541 | **无** |

三者合 1,710 行，搬进 `owo-agent-env` 后 `crate::` 引用全部落在同 crate 内，
**无需任何倒置**。据 §4 判据：`desktop_env` 与它们**同迁**（已是 env 成员），
故不存在跨 crate 边。

### 8.2 这次调用方基本没动

`experience_store` 在 core 内被 `fleet` / `goal` / `node_agent` 引用，`transition` 被
`desktop_env` 引用——但 core 保留了同名别名 re-export，所以**这些都没改**。
server 的 `desktop_world_api.rs` 与 devtool 的 `dataset_builder.rs` 也照旧经
`owo_agent_core::transition` / `world_model` / `experience_store` 使用，同样未动。

**本次只改了 6 个文件**：`Cargo.lock`、core 的 `Cargo.toml` 与 `lib.rs`、
env 的 `lib.rs`，以及 3 个 `git mv`。这是 re-export 策略迄今最省的一次。

### 8.3 又一次被手写符号表坑到（新坑）

我在 env 的 `lib.rs` 里手写了三块 `pub use x::{...}` 符号表，**首次编译就报 4 个
E0432**（`canonical_trace_bytes`、`classify_failure`、`trace_hash`、`TransitionError`
并不存在）。M2 的 `owo-agent-extensions` 用的是 glob，所以没暴露这个问题。

处置：改为 `pub use experience_store::*;` 等三行 glob —— 让迁移后的公共面与拆分前
**完全等价**并由编译器验证，而不是靠人肉抄符号名。已写入 §4 坑表。

### 8.4 验收证据

| 验收项 | 命令 | 结果 | 证据 |
|---|---|---|---|
| workspace 全目标编译 | `check --workspace --all-targets`（`-j 1`） | **exit=0**，155 s | `docs/qa/logs/mk-m6-check2-*.log` |
| 全量 core 测试 | `cargo test -p owo-agent-core --locked` | **exit=0**，302 s；`transition_tests`(10)、`world_model_tests`(8)、`goal_plan_tests`(13，含 `experience_store_records_step_and_goal_outcomes`) 全绿 | `docs/qa/logs/mk-m6-core-tests-*.log` |
| 全量 server 测试 | `cargo test -p owo-agent-server --locked -j 1 -- --test-threads=1` | **exit=0**，948 s（63 单测 + 40 集成测试文件全绿） | `docs/qa/logs/mk-m6-server-tests-*.log` |
| 运行态 | `scripts/mk-smoke.ps1 -Tag m6-env` | **18/18 PASS** | `docs/qa/evidence/mk-smoke-m6-env-*/report.json` |
| 依赖闭包 | `cargo tree -p owo-agent-env` | `owo-agent-core`/`owo-agent-server`/`sherpa`/`ndarray`/`rusqlite` 均 **0 次**；`owo-agent-*` 仍只有 kernel 与自身 | `docs/qa/logs/mk-m6-tree.log` |
| 调用方改动 | `git status` | 仅 6 个文件（见 §8.2） | 会话记录 |

### 8.5 改动文件

| 文件 | 变更 |
|---|---|
| `crates/owo-agent-core/src/{transition,world_model,experience_store}.rs` | `git mv` 到 `owo-agent-env`（三份文件原样未改） |
| `crates/owo-agent-env/src/lib.rs` | 注册三个模块 + glob 再导出 |
| `crates/owo-agent-core/src/lib.rs` | 删除三个 `pub mod`，env 别名行扩为四个模块 |
| `Cargo.lock` / core 的 `Cargo.toml` | 无新增外部依赖 |


## 9. M7：`owo-agent-plugins`（已完成）——配置类型随域走

### 9.1 为什么选 `plugin`

M7 三个候选（`workflow` 1,461 / `memory+observe` 825 / `plugin` 1,158）**都是零入边**
——core 内部没人引用它们，消费者全在 server/cli 侧。差别在出边：

| 候选 | 出边 | 需倒置的边 |
|---|---|---|
| `plugin` | **1**：`mcp`（只需 `McpServerConfig`） | 1 条类型边 |
| `memory+observe` | 1：`learn` | `learn`（1,629 行）也不同迁 → 得连带搬 |
| `workflow` | 4：`action_program`、`assert`、`learn`、`skill_health` | 4 条 |

`plugin` 的代价最小且判据最干净，故选它。

### 9.2 本步的倒置方向：**配置类型随域走**，不是塞进内核

迁移前 `plugin` 的唯一出边是 `crate::mcp::McpServerConfig`。`mcp`（MCP 运行时客户端）
留在 core，所以这条边必须倒置。但方向不是「把类型下沉内核」，理由很直接：

> `McpServerConfig` 是**插件清单里的一个字段**
>（`PluginManifest.mcp: Option<McpServerConfig>`），由插件 manifest 解析而来。
> 它本来就属于插件域，不属于内核。

所以正确做法是让它**随插件一起外迁**，再让 core 的 `mcp.rs` 反向引用它。
实测 `mcp.rs` 对 `plugin` **没有**反向依赖，倒置后无环。

这与 ADR-001 里被否决的「把 `SandboxAuditEvent` 下沉内核」正好形成对照——
判据不是「谁更好拿」，而是**这个类型在概念上属于谁**。已写入 §4 复用清单。

### 9.3 顺带改善：插件内核不再牵连 zip / rusqlite

`plugin.rs` 实测**不直接使用** `zip` 或 `rusqlite`（`zip` 只在 core 的 `share_skill`
里），所以新 crate 的依赖闭包是：

```text
owo-agent-plugins ──► owo-agent-tool-safety ──► owo-agent-kernel
                      + serde/serde_json/sha2/base64/chrono/uuid/ed25519-dalek
```

`cargo tree` 实测：`owo-agent-core` / `owo-agent-server` / `sherpa` / `ndarray` /
`rusqlite` / `zip` 出现次数**全部为 0**。对指南 §10「普通改动不触发原生重链」是又一处改善。

### 9.4 本次修的两个自身错误（都已记档）

1. **`plugin.rs` 里遗留 `use crate::mcp::McpServerConfig;`**：类型搬走后该路径失效，
   首次编译报 `E0432: could not find mcp in the crate root`。改为 `crate::McpServerConfig`
   （定义已在本 crate 根）。
2. **`core::mcp::McpServerConfig` 一度不可达**：`mcp_tests.rs` 用
   `owo_agent_core::mcp::{McpClient, McpServerConfig}`，而我在 `mcp.rs` 里写的是私有
   `use`，报 `E0603: struct is private`。改为 `pub use owo_agent_plugins::McpServerConfig;`，
   让 `core::mcp::` 路径继续可用；同时删掉因类型外迁而不再使用的 `serde` 导入。

教训（已入坑表）：**搬类型时，所有既有路径别名都要逐个确认是否仍可解析**——
顶层 `pub use` 与 `模块::类型` 是两条不同的路径。

### 9.5 验收证据

| 验收项 | 命令 | 结果 | 证据 |
|---|---|---|---|
| workspace 全目标编译 | `check --workspace --all-targets`（`-j 1`） | **exit=0**，56 s | `docs/qa/logs/mk-m7-check3-*.log` |
| 全量 core 测试 | `cargo test -p owo-agent-core --locked` | **exit=0**，306 s；`plugin_lifecycle_tests`(17)、`mcp_tests`(13)、`os_sandbox_integration_tests`(25，含 `plugin_http_mcp_egress_rejected_and_audited` / `plugin_zip_slip_entry_rejected` / `plugin_revocation_blocks_load_and_audits`) 全绿 | `docs/qa/logs/mk-m7-core-tests-*.log` |
| 全量 server 测试 | `cargo test -p owo-agent-server --locked -j 1 -- --test-threads=1` | **exit=0**，942 s（63 单测 + 40 集成测试文件全绿，含 `plugin_market_api_tests` 16 条） | `docs/qa/logs/mk-m7-server-tests-*.log` |
| 依赖闭包 | `cargo tree -p owo-agent-plugins` | core/server/sherpa/ndarray/rusqlite/zip 均 **0 次**；`owo-agent-*` 只有 kernel + tool-safety + 自身 | `docs/qa/logs/mk-m7-tree.log` |
| 运行态 | `scripts/mk-smoke.ps1 -Tag m7-plugins` | **18/18 PASS** | `docs/qa/evidence/mk-smoke-m7-plugins-*/report.json` |

### 9.6 改动文件

| 文件 | 变更 |
|---|---|
| `crates/owo-agent-core/src/plugin.rs` | `git mv` 到新 crate；`crate::sandbox::` → `owo_agent_tool_safety::`（1 处 use 块 + 2 处内联），`crate::mcp::McpServerConfig` → `crate::McpServerConfig` |
| `crates/owo-agent-plugins/{Cargo.toml,src/lib.rs}` | 新 crate（边界文档 + plugin + 从 mcp 下沉的 `McpServerConfig`） |
| `crates/owo-agent-core/src/mcp.rs` | 删除 `McpServerConfig` 定义与 `default_transport`；改为 `pub use owo_agent_plugins::McpServerConfig;`；删无用 serde 导入 |
| `crates/owo-agent-core/src/{lib.rs,agent.rs,settings.rs,tool_effects.rs}` | 别名 re-export + 3 处 `McpServerConfig` 改指 `owo_agent_plugins` |
| `Cargo.toml` / core 的 `Cargo.toml` | 新增成员与依赖；**server/cli 未改**（走别名） |


## 10. M8：`owo-agent-contracts`（已完成）——数据形状与执行者分离

### 10.1 选这五个模块的判据（本轮新确立的第二类边界）

M2/M3 用的是"零出边 + 零入边"的 SCC 判据，切完之后 §5.1 断言"core 里再没有这类集合"。
M8 证明那个断言**下得太早**：漏掉了一整类模块——**只描述数据形状、不持有任何能力**的模块。

| 模块 | 行数 | 内容 | 出边（`crate::`） | 外部依赖 |
|---|---:|---|---|---|
| `computer_task` | 541 | 桌面任务/步骤/证据的结构与校验 | 0 | serde |
| `plan` | 440 | 计划与计划步骤的结构与推进 | 0 | serde / chrono |
| `skill_health` | 247 | 技能健康度快照与评估结果 | 0 | serde |
| `skill` | 231 | SKILL.md frontmatter 解析 | 0 | serde |
| `context` | 32 | 上下文截面 DTO | 0 | serde |
| `lib.rs` | 57 | 边界文档 + glob re-export | — | — |

新判据（已写入 §4 复用清单）：**一个模块如果只有"数据的形状"（struct/enum + 序列化 +
少量纯函数），它就必须活在依赖最少的 crate 里，且搬它不会产生环、不会把任何能力搬走。**
M4 的候选清单里只列了"有能力的模块"，所以漏了这五个。

### 10.2 结果：出现了一个比 `kernel` 更轻的依赖根

`kernel` 虽然在 `owo-agent-*` 内部依赖数为 0，但仍带 `async-trait` 与 `windows-sys`
（`Win32_UI_Input*`，供键鼠注入用）。`owo-agent-contracts` 是**第一个连平台绑定都不带的
crate**：

```text
cargo tree -p owo-agent-contracts  →  顶层只有 chrono / serde / serde_json / uuid
core / server / kernel / tool-safety / sherpa / ndarray / rusqlite / windows-sys：0 次
```

这条对 §10 的资源红线是直接收益：改一个 DTO 字段不再牵动核心的编译单元，
而依赖它的 `core` 仍通过别名 re-export 保持 `owo_agent_core::{plan, skill, ...}` 全部可用。

### 10.3 本步唯一的编译错误，暴露了一类**跨 crate 才出现的隐含边界**

首次 `check --workspace --all-targets` 报 1 个错误（`docs/qa/logs/mk-m8-check-20260919-175819.log`）：

```text
error[E0603]: function `parse_frontmatter` is private
  --> crates\owo-agent-core\src\skill_pack.rs:6:13
121 | pub(crate) fn parse_frontmatter(content: &str) -> ...
note: the function is defined here --> crates\owo-agent-contracts\src\skill.rs:121:1
```

`pub(crate)` 在 core 内部是"全 crate 可见"，跨 crate 之后就只是"新 crate 内部可见"。
**搬迁会静默地把原来够用的可见性变成不够用**——这是 `git mv` + re-export 手法里
唯一一处编译器必须出场才能发现的边界收缩。处置：提权为 `pub` 并补 doc 注释说明
它为什么属于公共面（`skill_pack` 的 frontmatter 解析回退路径依赖它）。
教训已入 §4 坑表：**凡新 crate 需要对外消费的条目，逐个检查 `pub(crate)`**。

### 10.4 验收证据

| 验收项 | 命令 | 结果 | 证据 |
|---|---|---|---|
| workspace 全目标编译 | `scripts/mk-check.ps1 -Tag m8`（`-j 1`） | **exit=0**，147 s | `docs/qa/logs/mk-m8-check2-*.log`（首次失败见 `mk-m8-check-*.log`，即上条 E0603） |
| 全量 core 测试 | `cargo test -p owo-agent-core --locked` | **exit=0**，lib 407 条 + 全部集成测试绿 | `docs/qa/logs/mk-m8-core-tests-*.log` |
| 全量 server 测试 | `cargo test -p owo-agent-server --locked` | **exit=0**，960 s（63 单测 + 全部集成测试文件绿，含 `workswarm_api_tests` 16、`v1_execution_safety_tests` 20、`route_contract_tests` 23） | `docs/qa/logs/mk-m8-server-tests-*.log` |
| 依赖闭包 | `cargo tree -p owo-agent-contracts` | `owo-agent-*` 依赖 0 个；core/server/kernel/sherpa/ndarray/rusqlite/windows-sys 均 0 次 | `docs/qa/logs/mk-m8-tree.log` |
| 运行态 | `scripts/mk-smoke.ps1 -Tag m8-contracts` | **18/18 PASS**（daemon 起来、401/200 鉴权、会话落库、评测 20/20、notes 201、审计与 pid 文件、无残留进程） | `docs/qa/evidence/mk-smoke-m8-contracts-*/report.json` |
| core 体量 | `git ls-tree` + 逐行统计 | **67 文件 / 48,316 行 → 62 文件 / 46,837 行**（−1,479 行） | 本文件 §1 基线口径一致可比 |

### 10.5 改动文件

| 文件 | 变更 |
|---|---|
| `crates/owo-agent-core/src/{computer_task,context,plan,skill,skill_health}.rs` | `git mv` 到新 crate（5 个 rename，无内容改写，除 `skill.rs` 的可见性提权） |
| `crates/owo-agent-contracts/src/skill.rs` | `pub(crate) fn parse_frontmatter` → `pub` + doc 注释 |
| `crates/owo-agent-contracts/{Cargo.toml,src/lib.rs}` | 新 crate（边界文档 + 5 模块 + glob re-export） |
| `crates/owo-agent-core/src/lib.rs` | 删除 5 个 `pub mod`，改为 `pub use owo_agent_contracts::{computer_task, context, plan, skill, skill_health};` |
| `Cargo.toml` / core 的 `Cargo.toml` | 新增 workspace 成员与依赖；**server/cli/集成测试未改**（走别名） |


## 11. M9：`owo-agent-workswarm`（已完成）——编排的契约/状态先行，执行侧留待 A2

候选取舍与两路扫描数据见 §18；本节记录边界、结果与证据。

### 11.1 边界与依赖方向

| 模块 | 行数 | 内容 | `crate::` 出边 |
|---|---:|---|---|
| `project_space_store` | 1,162 | WorkSwarm 项目空间持久化（`ProjectSpaceStore` trait + SQLite 实现） | **0** |
| `team_benefit` | 1,285 | 组队收益判定（配对对照 → 冻结门槛 → `TeamPolicy`/`PolicyGate`，纯函数 + serde） | **0** |
| `workswarm_output` | 400 | Worker 结构化输出契约 V1（`WorkerOutputV1` 解析/校验） | **0** |
| `lib.rs` | 35 | 边界文档 + glob re-export | — |

```text
owo-agent-protocol ← owo-agent-workswarm ← owo-agent-core ← server/cli
```

新 crate 的依赖闭包实测只有 `owo-agent-protocol` 与
`async-trait / chrono / rusqlite / serde / serde_json / thiserror / uuid`，
**对 core 的依赖为零**——否则 core → workswarm 的既有边会变成 crate 环。
`rusqlite` 与 core 同版本同 feature（`bundled`），Cargo 复用同一份编译产物。

### 11.2 为什么这一步值得做：它是"可选加载"的物理前置条件

指南 §3 把 `workswarm.rs`、`goal.rs`、`workflow.rs`、`team_*` 定档为
`extensions/workswarm/`（默认关闭、通过插件 API 注册能力）。但**只要编排代码与 core
同处一个编译单元，"可选加载"就物理上无法实现**——M1 §3.2 已经用三种做法实测过：
optional dependency + feature 会被 workspace 成员并集重新点亮，`exclude` 挡不住
`path =` 引用，最后只能靠独立 workspace + 零反向依赖。

所以正确的迁移顺序是**先切零出边的契约/状态侧，再切执行侧**：本步搬走的三个模块
没有任何出边，搬完立即满足"core 零改动、可独立编译、可独立测试"；执行侧
（`workswarm.rs` 本体 4,328 行）留着与 `goal`/`workflow`/`fleet` 的环一起处理，
按 §18 的建议排到 A2（统一 Daemon）之后。

### 11.3 本步的调用方改动量：**0 行**（除 core 的 lib.rs 接线）

| 引用方 | 原路径 | 是否改动 |
|---|---|---|
| core `workswarm.rs` | `crate::project_space_store::`、`crate::team_benefit::`、`crate::workswarm_output::` | 否 |
| core `team_strategy.rs` | `crate::team_benefit::` | 否 |
| core `artifact_pipeline.rs` / `contract_worker.rs` / `worker_profile.rs` | `crate::workswarm_output::` | 否 |
| server（`workswarm_api` / `artifact_delivery_api` / `artifact_review_api` / `artifact_rework_api` / `human_inbox_api`） | `owo_agent_core::project_space_store::` | 否 |
| `devtools/product-eval`（独立 workspace） | `owo_agent_core::workswarm_output::` | 否 |
| core 的 5 个集成测试 + server 的 2 个集成测试 | `owo_agent_core::{project_space_store,workswarm_output}::` | 否 |

唯一需要说明的"看不见的改动"是 `#[derive(serde::Serialize)]` 这类**内联全限定路径**：
本步三个模块实测没有 `chrono`/`uuid`/`serde_json` 的 `use` 行以外的依赖，但
`project_space_store` 里有 6 处 `chrono::Utc::now()`、1 处 `uuid::Uuid::new_v4()`、
5 处 `#[tokio::test]`，都是 `use` 行扫描看不到的（M5 的坑），所以新 manifest
必须按"`use` 行 + 内联全限定路径"两遍核（见 §4 复用清单）。

### 11.4 验收证据（可复现）

| 验收项 | 命令 | 结果 | 证据 |
|---|---|---|---|
| 新 crate 独立编译 | `cargo check -p owo-agent-workswarm --all-targets` | **exit=0**，15 s（不依赖 core，也不需要 ORT 之外的 native 链） | `docs/qa/logs/mk-m9-check-20260919-182459.log` |
| workspace 全目标编译 | `scripts/mk-check.ps1 -Tag m9`（`-j 1`） | **exit=0**，147 s | `docs/qa/logs/mk-m9-20260919-182518.log` |
| 新 crate 自身测试 | `cargo test -p owo-agent-workswarm --locked --all-targets` | **exit=0**，42 条全绿（project_space_store 6 / team_benefit 25 / workswarm_output 11） | `docs/qa/logs/mk-m9-workswarm-tests-20260919-183259.log` |
| 全量 core 测试 | `cargo test -p owo-agent-core --locked` | **exit=0**，300 s；`workswarm_tests`(15)、`workswarm_recovery_tests`(7)、`workswarm_responsiveness_tests`(4)、`team_strategy_tests`(13)、`artifact_review_tests` 全绿 | `docs/qa/logs/mk-m9-core-tests-20260919-182753.log` |
| 全量 server 测试 | `cargo test -p owo-agent-server --locked` | **exit=0**，471 s（`workswarm_api_tests` 16、`workswarm_recovery_api_tests` 4、`workswarm_progress_api_tests` 2、`artifact_*`/`human_inbox`/`project_workspace` 全绿） | `docs/qa/logs/mk-m9-server-tests-20260919-183332.log` |
| 依赖闭包 | `cargo tree -p owo-agent-workswarm` | 顶层只有 protocol + async-trait/chrono/rusqlite/serde/serde_json/thiserror/uuid；core/server/kernel/tool-safety/sherpa/ndarray/ort/windows/reqwest **0 次** | `docs/qa/logs/mk-m9-tree.log` |
| 运行态 | `scripts/mk-smoke.ps1 -Tag m9-workswarm` | **18/18 PASS** | `docs/qa/evidence/mk-smoke-m9-workswarm-*/report.json` |
| core 体量 | `git ls-tree` + 逐行统计 | **62 文件 / 46,836 行 → 59 文件 / 43,998 行**（−2,838 行）；新 crate 4 文件 / 2,882 行 | 与 §1 / §10 同口径 |

> 注意 core 的 lib 单测条数由 407 降到 365：这不是测试变少，而是
> `project_space_store`/`team_benefit`/`workswarm_output` 的 42 条单测**随代码一起
> 搬到了新 crate**（见上表第 3 行，42 条全绿）。总条数守恒。

### 11.5 改动文件

| 文件 | 变更 |
|---|---|
| `crates/owo-agent-core/src/{project_space_store,team_benefit,workswarm_output}.rs` | `git mv` 到新 crate（3 个 rename，**内容零改写**） |
| `crates/owo-agent-workswarm/{Cargo.toml,src/lib.rs}` | 新 crate（边界文档 + 3 模块 + glob re-export + "不得依赖 core"的 manifest 约束） |
| `crates/owo-agent-core/src/lib.rs` | 删除 3 个 `pub mod` 与它们的文档注释，改为 `pub use owo_agent_workswarm::{project_space_store, team_benefit, workswarm_output};` + 边界注释块 |
| `Cargo.toml` / core 的 `Cargo.toml` | 新增 workspace 成员与依赖；**server / cli / 集成测试 / devtools 未改一行** |

### 11.6 本步遇到的资源红线（不是代码问题，但记档）

新 crate 第一次 `check` 被 §2.4 磁盘门直接拒绝：`卷 T: 仅剩 2.74 GB，normal 档要求 ≥ 6 GB`。
原因是 M8 的 server 全量测试（40+ 个测试二进制链接）把 PDB 写满。按门禁提示清
`target/**/incremental` + `target/**/*.pdb` 回收 **17.66 GB** 后重跑通过。
**没有绕过门禁**（禁止调小并发或加长超时）——这是 §4 坑表里那条"LNK1318 看着像编译
错误、实际是磁盘耗尽"的同一件事在门禁层的正确表现。


## 12. M10：`owo-agent-mcp`（已完成）——MCP 宿主第一段，连测试服务器与集成测试一起搬

候选取舍见 §18；本节记录边界、结果与证据。

### 12.1 边界与依赖方向

| 目标 | 行数 | 内容 | `crate::` 出边 |
|---|---:|---|---|
| `src/mcp.rs` | 646 | MCP 客户端与注册表：stdio / HTTP 传输、工具列表与调用、schema 预算、超时与重连 | **0** |
| `src/bin/mcp_test_server.rs` | 133 | stdio 假服务器（echo/add 等），测试与示例插件共用 | — |
| `src/bin/mcp_http_test_server.rs` | 115 | HTTP 假服务器（SSE / POST） | — |
| `tests/mcp_tests.rs` | 548 | 13 条 MCP 集成测试（登录、列工具、调用、超时重连、热注册、官方插件） | — |
| `src/lib.rs` | 33 | 边界文档 + glob re-export | — |

```text
tool-safety ─┐
             ├─► owo-agent-mcp ─► owo-agent-core ─► server/cli
plugins ─────┘        （dev-dependency 反向：core → mcp --dev--> core）
```

`mcp.rs` 自 M3/M7 起就已经在写 `owo_agent_tool_safety::` 与 `owo_agent_plugins::`
的绝对路径，不再经 crate 根，所以它的 `crate::` 出边实测为 **0**——这是"早先几步的
倒置让后面几步变成零代价"的直接例证。普通依赖里没有 core，唯一的 core 依赖是
`dev-dependencies`（集成测试要构造真实 Agent / 注册表 / 权限策略 / 会话）。
Cargo 的环检测只看普通依赖图，dev-dependency 的环不成环——core 对
`owo-agent-eval-facade` 早就是这个形状。

### 12.2 为什么把两台假服务器和 MCP 集成测试一起搬（这不是"顺手"）

`env!("CARGO_BIN_EXE_<name>")` 只在**声明该 bin 的那个包**的集成测试里可用。MCP 的
13 条测试里有 4 处依赖它（stdio 3 处、http 1 处），所以只搬 `mcp.rs` 会让测试无处可去。
而这台假服务器又只服务于 MCP 这一条边界——三者同迁之后：

* 改一次 MCP 传输只重编 `owo-agent-mcp`，不再牵动核心编译单元；
* 产物路径不变（同一 workspace 共用 `target/`），
  `plugins/example-hello/manifest.json` 里的
  `../../target/debug/owo-mcp-test-server.exe` 继续有效；
* 测试里的 `CARGO_MANIFEST_DIR/../../plugins` 与新位置同深度（`crates/<name>/`），
  路径解析结果不变。

### 12.3 本步踩的两个坑（都是同一类：`use` 行扫描看不到的依赖）

1. `mcp.rs:285` 的 `tracing::info!` —— 内联全限定路径，不在任何 `use` 行里，
   首次 `check` 报 `E0433: cannot find module or crate 'tracing'`。
2. `tests/mcp_tests.rs` 的 5 处 `uuid::Uuid::new_v4()` —— 同样是内联路径，
   `E0433` 再次出现，只是这次在 dev-dependencies 上。

这正是 M5 记档的坑的**第三、第四次**复现。处置已写进 §4 复用清单：依赖清单必须按
「`use` 行 + 内联全限定路径」两遍核；本步开始对新 crate 直接跑
`grep -E '^[a-z_]+::'` 的正则扫描（`tracing::`、`uuid::`、`chrono::`、`sha2::` …），
不再靠肉眼读 `use` 块。

### 12.4 磁盘红线：可回收的不止 incremental / pdb

本步两次被 §2.4 磁盘门拒绝（4.42 GB < 6 GB、16.16 GB < 20 GB）。清
`target/**/incremental` + `target/**/*.pdb` 只回收 11.90 GB，仍不够 strict 档的 20 GB。
实测发现真正的大头是 **`target/debug/deps/*.exe`：849 个历史测试二进制占 29.35 GB**
（每次全量测试都会留下几十个测试可执行文件与其调试信息）。
删除后回收 **29.35 GB**，门禁通过。这条已补进本节，供后续所有步骤复用：

```text
可安全回收（重编即恢复，按收益排序）：
  1. target/debug/deps/*.exe      （实测 29.35 GB）
  2. target/**/*.pdb              （实测 11–18 GB/轮）
  3. target/**/incremental        （实测 0.2–1 GB）
禁止：为了过门禁而调小 -j、加长超时或改阈值（§2.4 明令）。
```

### 12.5 验收证据（可复现）

| 验收项 | 命令 | 结果 | 证据 |
|---|---|---|---|
| 新 crate 独立编译 | `cargo check -p owo-agent-mcp --all-targets` | **exit=0**（首次失败见 12.3 的两个 E0433） | `docs/qa/logs/mk-m10-check-crate3-*.log` |
| workspace 全目标编译 | `scripts/mk-check.ps1 -Tag m10`（`-j 1`） | **exit=0**，156 s | `docs/qa/logs/mk-m10-20260919-184520.log` |
| MCP 边界自身测试 | `cargo test -p owo-agent-mcp --locked --all-targets` | **exit=0**，76 s；**13/13 集成测试全绿**（含 4 处 `CARGO_BIN_EXE_*` 真起假服务器） | `docs/qa/logs/mk-m10-mcp-tests-20260919-184800.log` |
| 全量 core 测试 | `scripts/mk-tests-sharded.ps1 -Package owo-agent-core -Tag m10-core` | **exit=0**；**31/31 分片全绿**（`--lib` 363 passed / 2 ignored + 30 个集成测试目标） | `docs/qa/logs/mk-shard-m10-core-*.log` |
| 全量 server 测试 | `scripts/mk-tests-sharded.ps1 -Package owo-agent-server -Tag m10-server` | **已补齐**（在 M11 步骤的同一工作树上跑完，40/40 分片全绿，见 §13.5） | `docs/qa/logs/mk-shard-m11-server*-*.log` |
| 依赖闭包 | `cargo tree -p owo-agent-mcp -e normal` | 普通依赖只有 `plugins` / `tool-safety` / `reqwest` / `serde_json` / `tokio` / `tracing`；**core / server / workswarm / contracts / env / sherpa / ndarray / ort / rusqlite 均 0 次**。`kernel` 只作为 `plugins → tool-safety → kernel` 的传递依赖出现（含其 `windows-sys`），这是 M3 就定下的受信执行链，不是本步引入的 | `docs/qa/logs/mk-m10-tree.log` |
| 运行态 | `scripts/mk-smoke.ps1 -Tag m10-mcp` | **已补齐**（同 M11 工作树，18/18 PASS，见 §13.5） | `docs/qa/evidence/mk-smoke-m11-memory-20260920-025512/report.json` |
| core 体量 | `git ls-tree` + 逐行统计 | **59 文件 / 43,998 行 → 58 文件 / 43,352 行**（−646 行）；另有 796 行测试/工具（2 个 bin + 1 个测试文件）迁出 | 与 §1 同口径 |

> 注意：下面这段是**当时**的资源受限记录，保留作为证据与教训。其中"server 全量测试
> 与冒烟待补"两条**已在 M11 步骤中补齐**：M11 的验证是在包含 M10 提交的同一工作树上
> 跑的，server **40/40 分片全绿**、冒烟 **18/18 PASS**（见 §13.5）。因此"server 与冒烟
> 是否因 M10 而坏"这个问题已被实证排除；缺的只是一次**纯 M10 树**的独立复跑，而 M10
> 与其后的 M11 之间不存在未验证的窗口。
>
> 当时的情况：core / server 全量测试与运行态冒烟在 M10 执行时多次被 §2.4 **内存门**
> 拦下（可用内存 5.91 GB / 已用 81.3%、4.01 GB / 87.3%——同机有 DeltaForce 等应用占用
> 6.7–11.4 GB）。门禁按设计终止进程树并返回 137，**没有做任何绕过**（未改阈值、
> 未加超时、未降并发档位以外的任何手段）。
>
> * **core 已用分批方式跑完并通过**（31/31）：见上表。
> * **server 当时未跑成**：server 测试必须先编译 `owo-agent-core`（`-p owo-agent-server`
>   的 feature 并集与 core 自身不同，无法复用已有产物），这一步实测需要约 2.5 GB
>   额外内存；本机在该时段只能提供约 0.6 GB 余量（发起时 6.97 GB / 78.0%，
>   60 s 内被顶到 81.2%），因此**连续 7 次都死在"Compiling owo-agent-core"的
>   第 60 秒**，无法收敛。
> * 已武装的守候任务：`scripts/mk-tests-sharded.ps1 -Package owo-agent-server
>   -MinFreeGB 8.5 -MaxUsedPct 74`（只在真有 1 GB 以上余量时才发起编译，不反复
>   抢机器）；内存回落后自动跑完并把日志落在 `docs/qa/logs/mk-shard-m10-server-*.log`。
> * **缺口的影响面是可界定的**：M10 只搬 `mcp.rs` + 2 个假服务器 + MCP 集成测试，
>   未触碰会话/审计/权限/评测/notes 等冒烟覆盖的路径；而 `check --workspace
>   --all-targets` 已覆盖 server/cli 全部目标的编译。缺口仅在"server 全量测试 +
>   端到端冒烟"这两条，补齐前不应视为已验收。
>
> 为了让验证在资源波动中仍能完成而不碰阈值，本步新增了 `scripts/mk-tests-sharded.ps1`：
>
> * 把"一次链接 30–40 个测试二进制"改成**逐目标分批**（`--lib` + 每个集成目标一次，
>   每次只链接 1 个），把 `link.exe` 的峰值内存摊开；
> * 每个分片启动前**主动等**内存窗口（free ≥ 6.6 GB 且 used ≤ 79%），被门禁拒绝
>   （启动前抛异常或运行中 137）时记录原因并等下一轮重试，最多 20 轮；
> * 逐分片记录真实退出码，任一失败即整体非 0。
>
> 它只改变"一次链接几个二进制"，**不改变任何门禁阈值**；core 据此拿到的结果与
> 一次性跑完全等价（同一组测试目标、同一 `--locked`、同一 `-j 1 --test-threads=1`）。

### 12.6 改动文件

| 文件 | 变更 |
|---|---|
| `crates/owo-agent-core/src/mcp.rs` | `git mv` 到新 crate（内容零改写） |
| `crates/owo-agent-core/src/bin/mcp_{test,http_test}_server.rs` | 随 MCP 边界迁到新 crate 的 `src/bin/`（`core/src/bin/` 因此清空并删除） |
| `crates/owo-agent-core/tests/mcp_tests.rs` | 迁到新 crate 的 `tests/`（`CARGO_BIN_EXE_*` 要求 bin 与测试同包） |
| `crates/owo-agent-mcp/{Cargo.toml,src/lib.rs}` | 新 crate（边界文档 + glob re-export + 2 个 bin 目标 + "普通依赖不得含 core"的约束） |
| `crates/owo-agent-core/src/lib.rs` | 删除 `pub mod mcp;`，改为 `pub use owo_agent_mcp::mcp;` + 边界注释块 |
| `crates/owo-agent-core/Cargo.toml` | 删除两个 `[[bin]]`，新增 `owo-agent-mcp` 依赖 |
| `Cargo.toml` | 新增 workspace 成员与依赖；**server / cli 未改一行** |


## 13. M11：`owo-agent-memory`（已完成）——前几步的倒置在这里兑现

候选取舍见 §18；本节记录边界、结果与证据。

### 13.1 边界与依赖方向

| 模块 | 行数 | 内容 | 跨 crate 出边 |
|---|---:|---|---|
| `learn` | 1,631 | 操作学习与主动建议：录制/泛化/动作图/流程技能包/`ProactiveEngine` | 0（改路径后） |
| `observe` | 576 | 桌面观察：`DesktopSnapshot` / `Observation` / `MemoryStore` | 0（改路径后） |
| `memory` | 249 | 语义记忆存储：JSONL 持久化 + 剪枝 + 检索 | 0 |
| `proactive_settings` | 82 | 主动建议的域配置 `ProactiveSettings`（**从 core 的 settings.rs 随域搬入**） | 0 |
| `lib.rs` | 35 | 边界文档 + glob re-export | — |

```text
kernel(M0) ─┐
            ├─► owo-agent-memory ─► owo-agent-core ─► server/cli
contracts(M8)┘        （core 侧 action_program/computer_use/executor/
                       share_skill/workflow 都引用 learn）
```

### 13.2 这一步为什么几乎不用改动搬迁代码：倒置是**前面**做的

三个模块搬迁前只有三条跨模块引用，全部在更早的步骤里已经处理完毕：

| 原路径 | 何时处理 | 本步动作 |
|---|---|---|
| `crate::platform::poll_foreground_app()` / `clipboard_sequence()`（observe 2 处） | **M0** 把 `platform` 下沉到 `owo-agent-kernel` | 改成 `owo_agent_kernel::platform::`（2 行） |
| `crate::skill_health::{FailureMode, SkillHealth, ...}`（learn） | **M8** 把 `skill_health` 下沉到 `owo-agent-contracts` | 改成 `owo_agent_contracts::skill_health::`（1 行） |
| `crate::settings::ProactiveSettings`（learn） | 本步按 **M7 的规则**处理 | 类型本身随域搬进本 crate（见 13.3） |

三个模块之间的边（`memory ↔ observe`、`observe → learn`）属于 §5.1 意义上的**真环**，
但它们**同迁一个 crate**，边不再跨 crate 边界，因此同样零倒置——与 M3 的
`sandbox ↔ audit_chain` 是同一条判据（§6.1）。

### 13.3 `ProactiveSettings`：第二次应用"配置类型随域走"

M7 把 `McpServerConfig` 从 core 的 `mcp.rs` 搬到消费它的插件域，这次是同一个形状：

* **消费方决定归属**：`learn::ProactiveEngine::new(ProactiveSettings)` 与
  `apply_settings(ProactiveSettings)` 是这个类型唯一的行为性用法；
  core 的 `Settings` 只是把它当字段聚合（`pub proactive: ProactiveSettings`）。
* **处理方式**：类型 + `impl Default` + 6 个 `#[serde(default = "...")]` 用的
  默认值函数一起搬进 `owo-agent-memory::proactive_settings`；core 的 `settings.rs`
  改成 `pub use owo_agent_memory::ProactiveSettings;`。
* **调用方零改动**：`owo_agent_core::settings::ProactiveSettings`（含
  `owo_agent_core::settings::Settings { proactive, .. }` 的字段类型）全部照旧。
* 一个必须记下的细节：`#[serde(default = "path")]` 的 path **在类型定义处解析**，
  所以默认值函数必须随类型一起搬。`default_true` 在 core 的 settings.rs 里还被另外
  两个结构体用着，于是本 crate 自备一份等价实现（82 行里有 5 行是它）——
  这是有意的小重复，避免为一行布尔默认值在两侧 crate 之间造反向依赖。

### 13.5 验收证据（可复现）

| 验收项 | 命令 | 结果 | 证据 |
|---|---|---|---|
| 新 crate 独立编译 | `cargo check -p owo-agent-memory --all-targets` | **exit=0**，5 s | `docs/qa/logs/mk-m11-check-crate-*.log` |
| workspace 全目标编译 | `cargo check --workspace --all-targets`（`-j 1`） | **exit=0**，178 s（同时更新锁文件：新增成员） | `docs/qa/logs/mk-m11-check-ws-*.log` |
| 新 crate 自身测试 | `scripts/mk-tests-sharded.ps1 -Package owo-agent-memory -Tag m11-memory` | **exit=0**，**27/27 通过**（从 core 随代码搬来的单测） | `docs/qa/logs/mk-shard-m11-memory-lib-*.log` |
| 全量 core 测试 | `scripts/mk-tests-sharded.ps1 -Package owo-agent-core -Tag m11-core` | **exit=0**，**31/31 分片全绿**（`--lib` 336 passed / 2 ignored + 30 个集成目标） | `docs/qa/logs/mk-shard-m11-core-*.log` |
| 全量 server 测试 | 分批两次：`-Tag m11-server`（strict 档 27 分片）+ `-Tag m11-server-b -Skip <已过 27 个>`（normal 档 13 分片） | **exit=0，40/40 分片全绿**（`--lib` 63 + 39 个集成目标） | `docs/qa/logs/mk-shard-m11-server*-*.log` |
| 运行态 | `scripts/mk-smoke.ps1 -Tag m11-memory` | **18/18 PASS** | `docs/qa/evidence/mk-smoke-m11-memory-20260920-025512/report.json` |
| core 体量 | `git ls-tree` + 逐行统计 | **58 文件 / 43,352 行 → 53 文件 / 40,630 行**（−2,722 行）；新 crate 5 文件 / 2,573 行 | 与 §1 同口径 |

> 单测条数守恒核对：core `--lib` 由 363 降到 **336**（−27），新 crate 恰好 **27**；
> 测试没被删掉，只是随代码一起搬了家。

> server 分两批的原因（如实记录）：第一批 27 个分片用 `-PolicyMode strict`（该档要求
> 磁盘 ≥20 GB）；构建把卷压到 19.72 GB 后，strict 档的**磁盘门**开始持续拒绝启动剩余
> 分片（这与内存无关，日志里表现为"启动前被资源门拒绝"连续重试）。剩余 13 个分片改用
> `-PolicyMode normal`（磁盘 ≥6 GB）续跑，并用新增的 `-Skip` 参数跳过已通过的 27 个。
> 两批都**显式传了 `-j 1 -- --test-threads=1`**；门禁的注入逻辑是"只降不升"
> （`scripts/ci-shared.ps1:284-290` 的 `[Math]::Min($foundJobs, $Jobs)`），
> 因此两批的实际并发都是 1。**门禁阈值本身没有被改动。**

### 13.6 改动文件

| 文件 | 变更 |
|---|---|
| `crates/owo-agent-core/src/{memory,observe,learn}.rs` | `git mv` 到新 crate；`observe.rs` 2 处 `crate::platform::` → `owo_agent_kernel::platform::`；`learn.rs` 2 行 use 改路径（settings → 本 crate，skill_health → contracts） |
| `crates/owo-agent-memory/src/proactive_settings.rs` | 新文件：`ProactiveSettings` + `impl Default` + 7 个 serde 默认值函数（含随类型搬来的 6 个） |
| `crates/owo-agent-core/src/settings.rs` | 删除 `ProactiveSettings` 定义与 `impl Default`、6 个只有它用的 `default_*`；改为 `pub use owo_agent_memory::ProactiveSettings;` |
| `crates/owo-agent-core/src/lib.rs` | 删除 3 个 `pub mod`，改为 `pub use owo_agent_memory::{learn, memory, observe};` + 边界注释块 |
| `crates/owo-agent-memory/{Cargo.toml,src/lib.rs}` | 新 crate（边界文档 + glob re-export + "不得依赖 core"的约束） |
| `Cargo.toml` / core 的 `Cargo.toml` | 新增 workspace 成员与依赖；**server / cli / 集成测试未改一行** |


## 14. M12：`owo-agent-policy`（已完成）——Tool Host 的判定侧，与"权限不可绕过"的第一段

候选取舍见 §18；本节记录边界、结果与证据。

### 14.1 边界与依赖方向

| 模块 | 行数 | 内容 |
|---|---:|---|
| `permissions` | 1,055 | 授权判定核心：`Level` / `Policy` / `Approver` / `PermissionProfile` / `PermissionRequest`（grant 命中、spec 收紧、效应类别参与决策） |
| `grant_store` | 877 | 授权凭证存储：`GrantStore` / `GrantScope`（指纹稳定、按 workspace/host/任务域生效、可撤销、可过期） |
| `tool_effects` | 575 | 工具效应声明与矩阵：`EffectClass` / `ToolEffect`（内置矩阵 + MCP 注解降级 + 未声明即拒绝） |
| `permission_spec` | 564 | 四维权限规格（filesystem / command / network / persistence）+ `nearest_profile` 反推 |
| `tool_names` | 50 | 工具命名契约 `sanitize_tool_name`（**从 core 的 tools.rs 随域下沉**，见 14.3） |
| `lib.rs` | 38 | 边界文档 + glob re-export |

```text
kernel(M0) ─► tool-safety(M3) ─► plugins(M7) ─► mcp(M10) ─┐
                                                          ├─► owo-agent-policy ─► owo-agent-core ─► server/cli
```

### 14.2 四个模块为什么必须同迁：互相引用，且双向

```
permissions      → grant_store, permission_spec, tool_effects
permission_spec  → permissions
grant_store      → permissions
tool_effects     → permissions
```

八条引用**全部落在集合内部**：单独搬任何一个都会立刻与留在 core 的三个成环。
整组同迁之后这些边不再跨 crate 边界 → **零倒置**，与 M3（`sandbox ↔ audit_chain`）、
M11（`memory ↔ observe`）是同一条判据的第三次应用。

### 14.3 对外的两条出边：一条随域下沉，一条改绝对路径

| 出边 | 处置 | 理由 |
|---|---|---|
| `tool_effects → tools::sanitize_tool_name`（6 处） | 函数**随策略内核下沉**为 `tool_names::sanitize_tool_name`；core 的 `tools.rs` 改为 `pub(crate) use owo_agent_policy::tool_names::sanitize_tool_name;` | 这个命名函数**同时是权限判定的输入**：效应表按工具名查表、MCP 前缀按同一名字生成、内置矩阵按名字分类。两条消费链（工具注册表 / 效应权限表）必须同源，否则"登记名"与"执行名"会漂移——那正是"权限不可绕过"最怕的漏口。可见性保持 `pub(crate)`，core 的公共面不变 |
| `tool_effects → mcp::McpTool`（1 处） | 改绝对路径 `owo_agent_mcp::McpTool` | MCP 的 DTO 属 MCP 域（M10 已下沉），按「类型随域走」由消费方依赖域名。方向 `policy → mcp → plugins → tool-safety → kernel`，无环 |
| `tool_effects → owo_agent_plugins::McpServerConfig` | 无需改动 | M7 就已经是绝对路径 |

### 14.4 本步暴露的第三类可见性陷阱：`pub(crate)` 会**变成死代码**

M8 撞到的是 `pub(crate)` 跨 crate 后"不可见"（编译错误 `E0603`）。这次是它的兄弟形态：
`Policy::set_read_only_runtime` / `replace_runtime_deny` 原本是 `pub(crate)`，
**唯一调用方在 core 的 `agent.rs`**。搬进新 crate 后：

* 新 crate 内部只有测试调用它们；
* 于是 `check` 报 **`dead_code` warning**（不是 error）——
  "权限开关没人用"看起来像小事，实际意味着**运行时收紧/放宽只读的入口断了**。

处置：提权为 `pub` 并补 doc 注释说明为什么（调用方跨 crate 了）。

**教训（已入 §4 坑表）**：搬迁前扫 `pub(crate)` 不能只看"外部是否引用它"，
还要看 **"它的调用方是否与被搬的模块一起走"**——不一起走就要提权，
而且这次的表现是 warning 而不是 error，**只看"编译过了"会漏掉**。
以后每步的 `check` 必须逐条读 warning，不能只看 exit code。

### 14.5 验收证据（可复现）

| 验收项 | 命令 | 结果 | 证据 |
|---|---|---|---|
| 新 crate 独立编译 | `cargo check -p owo-agent-policy --all-targets` | **exit=0**（首次带 `dead_code` warning，见 14.4，已修） | `docs/qa/logs/mk-m12-check-crate-*.log` |
| workspace 全目标编译 | `cargo check --workspace --all-targets` | **exit=0**，108 s | `docs/qa/logs/mk-m12-check-ws-*.log` |
| 新 crate 自身测试 | `scripts/mk-tests-sharded.ps1 -Package owo-agent-policy -Tag m12-policy` | **exit=0**，**51/51 通过** | `docs/qa/logs/mk-shard-m12-policy-lib-*.log` |
| 全量 core 测试 | `scripts/mk-tests-sharded.ps1 -Package owo-agent-core -Tag m12-core` | **exit=0**，**31/31 分片全绿**（`--lib` 287 passed / 2 ignored + 30 个集成目标） | `docs/qa/logs/mk-shard-m12-core-*.log` |
| 全量 server 测试 | `scripts/mk-tests-sharded.ps1 -Package owo-agent-server -Tag m12-server` | **exit=0**，**40/40 分片全绿**（`--lib` 63 + 39 个集成目标；含权限相关的 `permissions_center_api_tests` 7、`permissions_profile_api_tests` 5、`production_security_contract_tests` 7） | `docs/qa/logs/mk-shard-m12-server-*.log` |
| 运行态 | `scripts/mk-smoke.ps1 -Tag m12-policy` | **18/18 PASS** | `docs/qa/evidence/mk-smoke-m12-policy-20260920-032821/report.json` |
| 依赖闭包 | `cargo tree -p owo-agent-policy -e normal` | 顶层只有 `mcp` / `plugins` / async-trait / chrono / serde / serde_json / sha2 / tracing / uuid；**core / kernel / contracts / workswarm / sherpa / ndarray / ort / rusqlite 均 0 次** | `docs/qa/logs/mk-m12-tree.log` |
| core 体量 | `git ls-tree` + 逐行统计 | **53 文件 / 40,630 行 → 49 文件 / 37,567 行**（−3,063 行）；新 crate 6 文件 / 3,159 行 | 与 §1 同口径 |

> 单测条数守恒核对：core `--lib` 由 336 降到 **287**（−49），新 crate **51** 条
> = 随代码搬来的 49 条 + 为 `tool_names` 新增的 2 条（"连字符必须保留"的回归断言，
> 见 14.3）。**测试没有被删，而是随代码搬家并顺带加密了契约。**

### 14.6 改动文件

| 文件 | 变更 |
|---|---|
| `crates/owo-agent-core/src/{permissions,permission_spec,grant_store,tool_effects}.rs` | `git mv` 到新 crate；`tool_effects.rs` 改 7 处路径（6 处命名函数 + 1 处 `McpTool`）；`permissions.rs` 2 个 `pub(crate)` 方法提权为 `pub` |
| `crates/owo-agent-policy/src/tool_names.rs` | 新文件（50 行）：`sanitize_tool_name` 及其 2 条单测（从 core 的 tools.rs 迁出，并补了"连字符必须保留"的回归断言） |
| `crates/owo-agent-core/src/tools.rs` | 删除 `pub(crate) fn sanitize_tool_name` 定义，改为 `pub(crate) use owo_agent_policy::tool_names::sanitize_tool_name;` |
| `crates/owo-agent-core/src/lib.rs` | 删除 4 个 `pub mod`，改为 `pub use owo_agent_policy::{grant_store, permission_spec, permissions, tool_effects};` + 边界注释块 |
| `crates/owo-agent-policy/{Cargo.toml,src/lib.rs}` | 新 crate（边界文档 + glob re-export + "不得依赖 core"的约束） |
| `Cargo.toml` / core 的 `Cargo.toml` | 新增 workspace 成员与依赖；**server / cli / 集成测试未改一行** |


## 15. M13：两个"归位"模块（已完成）——前两步的利息

候选取舍见 §18；本节记录边界、结果与证据。

### 15.1 这一步为什么几乎免费：出边目标刚被搬成了 crate

| 模块 | 行数 | 出边 | 出边目标现在在哪 | 本步动作 |
|---|---:|---|---|---|
| `mcp_health` | 532 | `crate::tool_effects::EffectClass` | **M12 已随 `owo-agent-policy` 下沉** | `git mv`，**内容零改写** |
| `share_skill` | 246 | `crate::learn::{...}` | **M11 已随 `owo-agent-memory` 下沉** | `git mv` + manifest 加 `zip` |

两个模块的 `crate::` 引用搬迁后**都落在新 crate 内部**，因此不需要改一行代码、
不需要任何倒置。这是 M11/M12 留下的利息：**先把被依赖的域搬走，后来者就变成零成本**。

### 15.2 域名归属：为什么不各开一个新 crate

* `mcp_health` → **`owo-agent-policy`**：它的判定依据是**效应类别**（只读可重试、
  写与执行不重试），与 `tool_effects`/`permissions` 是同一套判定语义。
  **不能放进 `owo-agent-mcp`**：`policy → mcp` 这条依赖已经存在（`McpTool`），反向会成环。
* `share_skill` → **`owo-agent-memory`**：它导出/导入的是 `learn` 的动作图与流程技能包，
  与学习域同一个数据形状族。

### 15.3 本步的新坑：内联路径扫描的**词表不全**

`mcp_health.rs:77` 用 `#[derive(thiserror::Error)]`（内联全限定路径），而当时的扫描词表
只有 `owo_agent_*/serde/chrono/uuid/sha2/zip/...`，**没有 thiserror** → 首次 `check`
报 `E0433`。处置：把 `thiserror`/`futures`/`once_cell`/`parking_lot` 补进词表，
并在 `owo-agent-policy` 的 manifest 里记档。§4 复用清单的"查可见性"一条同步改成
"用**完整词表**正则扫一遍"，不再依赖记忆。

### 15.4 验收证据（可复现）

| 验收项 | 命令 | 结果 | 证据 |
|---|---|---|---|
| workspace 全目标编译 | `cargo check --workspace --all-targets` | **exit=0**，96 s；**0 条 `dead_code`/`unused` warning**（M12 的教训生效） | `docs/qa/logs/mk-m13-check-ws2-*.log` |
| `owo-agent-policy` 测试 | `scripts/mk-tests-sharded.ps1 -Package owo-agent-policy -Tag m13-policy` | **exit=0**，**59/59**（51 → 59，+8 = `mcp_health` 单测随迁） | `docs/qa/logs/mk-shard-m13-policy-*.log` |
| `owo-agent-memory` 测试 | `... -Package owo-agent-memory -Tag m13-memory` | **exit=0**，**32/32**（27 → 32，+5 = `share_skill` 单测随迁） | `docs/qa/logs/mk-shard-m13-memory-*.log` |
| 全量 core 测试 | `... -Package owo-agent-core -Tag m13-core` | **exit=0**，**31/31 分片全绿**；`--lib` **287 → 274**（−13 = 8 + 5，逐条对上） | `docs/qa/logs/mk-shard-m13-core-*.log` |
| 全量 server 测试 | `... -Package owo-agent-server -Tag m13-server` | **exit=0**，**40/40 分片全绿** | `docs/qa/logs/mk-shard-m13-server-*.log` |
| 运行态 | `scripts/mk-smoke.ps1 -Tag m13-relocate` | **18/18 PASS** | `docs/qa/evidence/mk-smoke-m13-relocate-20260920-040022/report.json` |
| core 体量 | 逐行统计 | **49 文件 / 37,567 行 → 47 文件 / 36,787 行**（−780 = 532 + 246 + 接线） | 与 §1 同口径 |

### 15.5 改动文件

| 文件 | 变更 |
|---|---|
| `crates/owo-agent-core/src/mcp_health.rs` | `git mv` 到 `owo-agent-policy/src/`（**内容零改写**） |
| `crates/owo-agent-core/src/share_skill.rs` | `git mv` 到 `owo-agent-memory/src/`（**内容零改写**） |
| `crates/owo-agent-policy/{src/lib.rs,Cargo.toml}` | 加 `pub mod mcp_health` + re-export；manifest 补 `thiserror`（内联 derive 路径） |
| `crates/owo-agent-memory/{src/lib.rs,Cargo.toml}` | 加 `pub mod share_skill` + re-export；manifest 补 `zip`（`.owskill` 包读写） |
| `crates/owo-agent-core/src/lib.rs` | 删 2 个 `pub mod`，并入两条别名 re-export |
| 调用方 | **0 行改动**（`agent` / `tools` / server 全部走别名） |


## 17. M14：`owo-agent-perception`（已完成）——边界拿到了，成本没降（如实记录）

候选取舍见 §18；本节记录边界、结果与证据。

### 17.1 边界与结果

| 模块 | 行数 | 说明 |
|---|---:|---|
| `onnx_ocr` | 1,217 | ONNX Runtime 文字检测 + 识别 |
| `scene` | 800 | 场景图（UIA/OCR/视觉三源融合、稳定 id） |
| `perception` | 704 | 感知层 L0–L3 + 情境快照 |
| `vision` | 662 | 视觉通道（截图 → VLM 描述/定位/校验） |
| `locate` | 531 | 元素定位（稳定 id/名称/上下文/历史与模板先验） |
| `element_registry` | 527 | 元素注册表（跨帧稳定 id） |
| `paddle_ocr` | 389 | 云端 PaddleOCR 通道 |
| `ocr` | 372 | OCR 抽象与行分组 |
| `window_template` | 296 | 窗口模板 ROI |
| `stt` | 214 | 本地语音（Sherpa-ONNX，**可选 feature**） |
| `accessibility` | 158 | UIA 无障碍树 |
| `stt_settings` | 92 | `SttSettings` 随域搬入（M7/M11 之后第三次「配置类型随域走」） |
| `lib.rs` | 42 | 边界文档 + glob re-export |
| **合计** | **6,008** | 13 文件 |

唯一逃逸边 `stt → settings::SttSettings` 按配置类型随域走解决；群组内对 `platform::*`
的引用改为 `owo_agent_kernel::platform::*`（M0 已下沉）。**调用方零改动。**

### 17.2 验收证据（可复现）

| 验收项 | 命令 | 结果 | 证据 |
|---|---|---|---|
| workspace 全目标编译 | `cargo check --workspace --all-targets` | **exit=0**；0 条 `dead_code`/`unused`（首次带 3 条，来自随迁移失去调用方的 `default_stt_*`，已清） | `docs/qa/logs/mk-m14-check-ws3-*.log` |
| 新 crate 自身测试 | `scripts/mk-tests-sharded.ps1 -Package owo-agent-perception -Tag m14-perception` | **exit=0**，**66 passed / 1 ignored** | `docs/qa/logs/mk-shard-m14-perception-*.log` |
| 全量 core 测试 | `... -Package owo-agent-core -Tag m14-core` | **exit=0**，**31/31 分片全绿**；`--lib` **274 → 208**（−66，与随迁条数**精确对上**） | `docs/qa/logs/mk-shard-m14-core-*.log` |
| 全量 server 测试 | `... -Package owo-agent-server -Tag m14-server` | **exit=0**，**40/40 分片全绿** | `docs/qa/logs/mk-shard-m14-server-*.log` |
| 运行态 | `scripts/mk-smoke.ps1 -Tag m14-perception` | **18/18 PASS** | `docs/qa/evidence/mk-smoke-m14-perception-20260920-135006/report.json` |
| 依赖方向 | `cargo tree -p owo-agent-perception -e normal` | **core 出现 0 次**（感知不依赖权威运行时）；含 ort / ndarray / windows / reqwest / png | 本节 17.3 |
| STT 可关 | `cargo tree -p owo-agent-perception --no-default-features` | 输出中**无 sherpa-onnx** | 本节 17.3 |
| core 体量 | 逐行统计 | **47 文件 / 36,787 行 → 36 文件 / 30,891 行**（−5,896 行，−16%） | 与 §1 同口径 |

### 17.3 收益的诚实清单（ADR-002 §2.1 的结论）

| 项 | 结论 | 实测 |
|---|---|---|
| core 测试二进制不再链接 ONNX | ❌ **不成立** | `ort ← owo-agent-perception ← owo-agent-core`；exe **73→72 个、2.68→2.71 GB、中位数 28.7→30.5 MB** |
| 改 core 不再重编 ORT/Sherpa | ❌ **不算本步收益** | 迁移前它们就是独立 crate，任何时刻改 core 都不重编它们 |
| perception 可独立编译测试 | ✅ | 只编 6,008 行 + 1 个测试目标（90 s），不牵动 core 的 30 个测试目标 |
| STT 可整块关闭 | ✅ | `--no-default-features` 无 sherpa-onnx |
| core 源码变轻 | ✅ | −5,896 行（−16%） |
| A3 Worker 的前置就绪 | ✅ | perception 闭包中 core 出现 0 次 |

**教训：切边界 ≠ 降成本。** 判断某次拆分的成本收益，必须先问"**消费方在哪一侧**"：
只要 core 的 `executor`/`computer_use` 仍进程内调用感知，ORT 就一定留在链接链上。

### 17.4 本步踩的坑（第 4 次同一类，已用工具根治）

依赖清单前 4 步都靠"`use` 行 + 人工维护的内联路径词表"，于是 M5 漏 chrono/uuid、
M10 漏 tracing/uuid、M13 漏 thiserror、**M14 漏 tokio/tracing/windows-future**。
本步新增 `scripts/mk-deps-scan.ps1` 用机械方法取代词表：

```text
扫文件里所有 `ident::`（剥注释）→ 与 Cargo.lock 的真实包名取交集 → 与 manifest 已声明项比对
```

它当场就报出 10 项未声明（含被 doc 注释误报的项，剥注释后收敛到 9 项真实缺失）。
另有一处只有编译器能发现：`ocr.rs` 里 `IAsyncOperation::await` 需要 `windows-future`
提供 Future 实现——这不是 `use` 行，而是 **trait 来源**，因此脚本末尾仍写明
"不能替代编译器，feature 与 trait 来源要靠 check 兜底"。

### 17.5 改动文件

| 文件 | 变更 |
|---|---|
| `crates/owo-agent-core/src/{perception,ocr,onnx_ocr,paddle_ocr,stt,vision,scene,locate,element_registry,window_template,accessibility}.rs` | `git mv` 到新 crate；4 个文件共 6 处 `crate::platform::` → `owo_agent_kernel::platform::`；`stt.rs` 1 行 use 改指本 crate |
| `crates/owo-agent-perception/{Cargo.toml,src/lib.rs}` | 新 crate（分组 feature + 边界文档 + glob re-export） |
| `crates/owo-agent-perception/src/stt_settings.rs` | 新文件：`SttSettings` + `impl Default` + 5 个 serde 默认值函数 |
| `crates/owo-agent-core/src/settings.rs` | 删除 `SttSettings` 与 3 个只被它使用的 `default_*`；改为 `pub use owo_agent_perception::SttSettings;` |
| `crates/owo-agent-core/Cargo.toml` | **移除 ndarray / ort / sherpa-onnx / windows-future / windows-sys / png**；`windows` feature 从 9 项收到 6 项 |
| `crates/owo-agent-core/src/lib.rs` | 删 11 个 `pub mod`，加一条别名 re-export（35 个 `pub mod` 剩余） |
| `scripts/mk-deps-scan.ps1` | 新增：机械依赖扫描（根治词表漏项） |


## 19. M15：`owo-agent-executor` + `owo-agent-workflow`（已完成）——零倒置的闭合集

候选取舍见 §18；本节记录边界、结果与证据。

### 19.1 边界与结果

| 新 crate | 模块 | 行数 | 说明 |
|---|---|---:|---|
| `owo-agent-executor` | `executor` | 1,391 | 动作落地：UIA 定位、键鼠注入、窗口激活、屏幕捕获取词；`ExecReport`/`ExecStep` |
| `owo-agent-workflow` | `workflow` / `action_program` / `assert` | 2,917 | 工作流 DSL（校验/编译/审批/检查点回滚）、动作程序运行时（控制流/变量/断言）、断言 |
| **合计** | 4 模块 | **4,308** | 13 个源文件 |

**这是零倒置搬迁**：4 个模块的出边**全部**指向已抽出的 crate
（`memory`/`perception`/`kernel`/`contracts`），搬迁只需把 33 处相对路径改成绝对路径；
模块之间互相引用的边（`workflow → action_program/assert`、`action_program → assert`）
**同迁一个 crate**，因此也不跨 crate 边界。

依赖方向：`workflow → executor → {perception, memory, kernel}`，与指南 §4 的分工一致
（workflow 属 extensions，executor 属 tool-host）。

### 19.2 本步修的 6 处可见性收缩（同一模式第 5、6 次）

`check` 一次报出 6 处，全部是"`pub(crate)` 的调用方被搬走或被留在 core"：

| 条目 | 症状 |
|---|---|
| `parse_click_at` | `E0603: function is private`（调用方 `action_program` 迁到 workflow crate） |
| `scroll_at_screen` | `dead_code`（调用方 `computer_use` 留在 core） |
| `click_at_screen` / `launch_target` / `send_shortcut` / `send_unicode` | 同类（core 的 `computer_use` 依赖它们） |

后 4 个是我在修第 1 个之后**主动扫描**发现的（"新 crate 里的 `pub(crate)` 条目是否被 core 引用"），
没有等编译器逐个报。处置统一为提权 `pub` + doc 注释。

### 19.3 验收证据（可复现）

| 验收项 | 命令 | 结果 | 证据 |
|---|---|---|---|
| workspace 全目标编译 | `cargo check --workspace --all-targets` | **exit=0**，100 s；0 条 `dead_code`/`unused` | `docs/qa/logs/mk-m15-check-ws2-*.log` |
| `owo-agent-executor` 测试 | `scripts/mk-tests-sharded.ps1 -Package owo-agent-executor -Tag m15-executor` | **exit=0**，**14/14** | `docs/qa/logs/mk-shard-m15-executor-lib-*.log` |
| `owo-agent-workflow` 测试 | `... -Package owo-agent-workflow -Tag m15-workflow` | **exit=0**，**18/18** | `docs/qa/logs/mk-shard-m15-workflow-lib-*.log` |
| 全量 core 测试 | `... -Package owo-agent-core -Tag m15-core` | **exit=0**，**31/31 分片全绿**；`--lib` **208 → 176**（−32 = 14 + 18，精确对上） | `docs/qa/logs/mk-shard-m15-core-*.log` |
| 全量 server 测试 | 三批续跑（`m15-server` 13 + `m15-server-b` 13 + `m15-server-d` 14，用 `-Skip` 跳过已过项） | **exit=0，40/40 分片全绿**（`goal_api_tests` 另经直接复跑 32/32 验证） | `docs/qa/logs/mk-shard-m15-server*.log`、`mk-m15-goal-api-repro.log` |
| 运行态 | `scripts/mk-smoke.ps1 -Tag m15-exec-workflow` | **18/18 PASS** | `docs/qa/evidence/mk-smoke-m15-exec-workflow-*/report.json` |
| core 体量 | 逐行统计 | **36 文件 / 30,891 行 → 32 文件 / 26,655 行**（−4,236 行） | 与 §1 同口径 |

### 19.4 改动文件

| 文件 | 变更 |
|---|---|
| `crates/owo-agent-core/src/executor.rs` | `git mv` 到 `owo-agent-executor`；13 处路径改绝对（memory/perception/kernel）；6 个函数提权 `pub` |
| `crates/owo-agent-core/src/{workflow,action_program,assert}.rs` | `git mv` 到 `owo-agent-workflow`；20 处路径改绝对（executor/memory/perception/kernel/contracts） |
| `crates/owo-agent-executor/{Cargo.toml,src/lib.rs}` | 新 crate（边界文档 + glob re-export） |
| `crates/owo-agent-workflow/{Cargo.toml,src/lib.rs}` | 新 crate（边界文档 + glob re-export） |
| `crates/owo-agent-core/src/lib.rs` | 删 4 个 `pub mod`（35 → 31），加 2 条别名 re-export |
| `Cargo.toml` / core 的 `Cargo.toml` | 新增 workspace 成员与依赖；**server / cli / 集成测试未改一行** |
### 19.5 工具层卡死排查（如实记录，含一条失败的改进）

server 全量验收中途出现两次"分片零输出、长时间无结果"（`goal_api_tests` 1h43m、
`project_workspace_api_tests` 两次 20min）。排查过程与结论：

| 观察 | 证据 | 结论 |
|---|---|---|
| 卡死那次日志 **0 字节**（历次成功 987 B） | `mk-shard-m15-server-goal_api_tests-*.log` 0 B | 进程**没产出任何输出**，不是测试逻辑死锁 |
| 系统里没有本 lane 的 cargo/rustc/link/git/测试进程 | `Get-Process` 清点 | 没有孤儿占锁 |
| 同一测试**直接单跑** | `goal_api_tests` 32/32 in 8.27 s；`project_workspace_api_tests` 3/3 in 1.65 s | **测试本身没问题** |
| 只有走"分批器包一层 Start-Job + Wait-Job 做超时"时才必现 | 改动后 14 个分片**全部**零输出超时；回退直调后立刻全绿 | **是工具层故障**：Windows PowerShell 5.1 的 `Start-Job` 依赖命名管道做作业传输，在本机受限环境下会静默挂住 |

处置：**放弃用 Start-Job 实现分片超时**（该参数已删除，避免"看起来有超时其实没有"的误导），
把这条结论写进 `scripts/mk-tests-sharded.ps1` 的注释里，并保留人工排查判据：

```text
卡死时：日志 0 字节 + 无构建进程 → cargo 没起来（多为并行 lane 抢 target/ 锁）
        日志有内容            → 测试自身阻塞（此时才需要看测试代码）
```

教训：**"我加的保护"本身可能是故障源**。给工具加超时的第一次尝试反而把 14 个正常分片
变成超时失败；判断依据只能是"同一输入换路径重跑"的对照，而不是日志里的话。

## 18. 后续候选与取舍记录

M0–M3 已把 core 里"能结构性地零代价切下来"的部分用完：**§5.1 的 SCC 分析证明，整个 core
只有那 7 个模块满足零入边 + 零出边（M2 切 5 个、M3 切 2 个）**。因此 M4 起必须做
**真正的依赖倒置或整组搬迁**，不能再指望"搬文件 + re-export"零代价推进。

### M4 优先级 0（**已完成**）：三方事务边界契约测试

M3 把这条边界切成了三份（`change_set*` 在 extensions、`sandbox` 在 tool-safety、
实际写入在 core 的 `executor`/`tools`），交接点此前**没有任何专门测试**。已补
`crates/owo-agent-core/tests/execution_boundary_contract_tests.rs`，4 条测试全绿：

| 契约 | 断言 |
|---|---|
| 拒绝即不执行 | deny 名单命中 → 返回错误且**不进入 spawn**；工作区零文件产物。并用**裸进程**跑同一命令体做反向对照，证明该命令确实会写文件（否则断言是空转） |
| 拒绝必留收据 | 被拒绝的执行必须产生明确拒绝语义的沙箱事件（`SpawnRejected` / `UnsupportedIsolation`），且这些收据能汇入 `AuditChain` 并通过 `verify()` |
| 允许即可观测 | 真实 Job 内执行的写文件命令，其产物能被 `change_set::file_hash` 观察到（基线 None → 现在有哈希），与 `WorkspaceBaseSnapshot` 的"新建"判定一致 |
| 失败不半写 | 非 0 退出的命令不得留下最终产物 |

写这组测试本身踩了三个坑（都已记入 §4 坑表的同类条目）：
① 用**沙箱内**的命令做"会写文件"的对照 → 被沙箱正确地挡下（绝对路径写入被拒），对照失效；
   必须用裸进程做对照。
② `a && b` 与重定向混用在 `cmd` 里解析失败。
③ 给**不含空格**的路径加引号，`cmd` 把引号当字面量 →「文件名、目录名或卷标语法不正确」。

服务端侧的 `full_loop`（tracker → change_set → revert）仍由
`owo-agent-server/tests/v1_execution_safety_tests.rs` 覆盖；core 层这组补的是它下面的
沙箱与文件系统这一层。

### M8 追加的判据：候选清单要两路出（**M9 起生效**）

§5.1 的"零出边 + 零入边"筛选只覆盖了**有能力的模块**（service / 执行器 / 存储）。
M8 证明必须同时扫第二路：**只有数据形状的契约模块**（`struct`/`enum` + 序列化 + 纯函数）。
M8–M9 用同一次 `crate::` 扫描（含 `crate::{a, b}` 块与 `super::`）复查，得到：

| 模块 | 行数 | 出边 | 入边（谁引用它） | 归属域（指南 §3） |
|---|---:|---|---|---|
| `project_space_store` | 1,162 | **0** | `workswarm` | WorkSwarm 扩展 |
| `team_benefit` | 1,285 | **0** | `team_strategy`、`workswarm` | WorkSwarm 扩展 |
| `workswarm_output` | 400 | **0** | `artifact_pipeline`、`contract_worker`、`worker_profile`、`workswarm` | WorkSwarm 扩展 |
| `mcp` | 646 | **0** | `agent`、`tool_effects`、`tools` | Tool Host 的 MCP 接入层 |
| `accessibility` | 158 | **0** | `element_registry`、`executor`、`perception`、`scene`、`window_template` | Perception Worker |

（`mcp` 出边为 0 是 M3/M7 的副产品：它已经在用 `owo_agent_tool_safety::` 与
`owo_agent_plugins::` 的绝对路径，不再经 `crate::`。）

### M9 实际选择：WorkSwarm 契约与状态 → `owo-agent-workswarm`（3 个模块 2,847 行）

* 收益：一次搬走 2,847 行，且三者的入边**全部**可由 re-export 满足（调用方零改动）；
  更重要的是——指南 §3 把 `workswarm.rs`/`goal.rs`/`workflow.rs`/`team_*` 定档为
  `extensions/workswarm/`（默认关闭、可选加载）。**在它们还与 core 同处一个编译单元时，
  "可选加载"在物理上无法实现**（§3.2 已实测：feature 关不住一条会成环的边），
  所以先把**零出边**的契约/状态部分切成独立 crate，是这条迁移路线的前置条件。
* 代价：新 crate 依赖 `owo-agent-protocol` + `rusqlite`（bundled）+ `async-trait`，
  **不依赖 core**，因此零环风险；代价是 `rusqlite` 会在两个 crate 里被同时引用
  （同版本，Cargo 复用同一份编译产物，不增加构建负担）。
* 未被选中的理由（同批候选）：
  * `mcp`（646 行）属 Tool Host 的 MCP 段（指南 §9 A4 的第 5 段），
    但 `tools`/`executor` 仍在 core，单独搬 MCP 不改善"权限不可绕过"这条边界，
    更适合与 §9 A4 的后续段一起做；
  * `accessibility`（158 行）量太小、且属 Perception Worker（§9 A3 需先写 ADR），
    并入 M9 只会把两个域的决策混在一起。
### 候选 A：Perception Worker（指南 §9 A3，SLO 收益最大，但最贵）

* 收益：把 `ort`、`sherpa-onnx`、`ndarray`、Media_Ocr/UIA 从基础链搬走，
  直接兑现 §10 的"普通 Agent 改动不触发 ONNX 编译"。
* 代价：5 条出边全是**双向边**（真环）：
  * `computer_use` ↔ perception：`desktop_env`→`computer_use` 1 处 vs
    `computer_use`→perception 30 处；
  * `learn` ↔ `observe`：`observe`→`learn` 9 处；
  * `memory` ↔ `observe`：`memory`→`observe` 1 处；
  * `transition` ↔ `world_model`：`world_model`→`transition` 1 处；
  * `settings` ↔ `stt`：`stt`→`settings` 1 处。
* 处置：要么把 15 个模块整体搬（出边数从 5 降到 0，代价是 crate 变大且 `computer_use`
  这类"执行"能力会被错误地搬进感知 worker，违反 §3 的"动作执行和感知拆开"），
  要么在内核里放 `PerceptionProbe`/`TransitionSink` 之类的 trait 做倒置。
  **这是一次真正的架构决策，必须先写 ADR 再动手。**

### 候选 B：ProductEval devtool（入边为 0）

* 收益：入边 **0**，`eval`/`dataset_builder` 指向外部；`product_eval` 本身占 3,317 行，
  是 §1.1 点名的四个大文件之一。
* 代价：出边 7 条（`agent`、`gateway`、`session`、`tools`、`permissions`、
  `desktop_env`、`transition`），即新 crate 必须**依赖 `owo-agent-core`**。
* 关键约束：core 里还有 `#[path = "product_eval/workswarm_executor.rs"] pub mod
  product_eval_workswarm;`，且 `action_program`/`workflow` 反向引用 `eval`。
  为避免 core→eval 的环，必须用 **optional dependency + feature**（默认关闭）：

  ```toml
  [features]
  default = []
  product-eval = ["dep:owo-agent-product-eval"]
  ```

  这样默认构建把评测完全移出编译与启动链（正是指南 §9 对 ProductEval 的要求），
  而 CLI 的 `eval` 子命令与服务端 `product_eval_api` 显式开启该 feature。

### 候选 C：Tool Host 内核（Policy/Executor）

* 收益：指南 §2.2 的核心受信边界（`permissions`/`permission_spec`/`grant_store`/
  `sandbox`/`executor`/`change_set*`/`tool_effects`，8 模块 8,007 行），入边 16 个文件
  全部可由 re-export 满足。
* 代价：出边 8 条，其中 `tools`↔policy、`mcp`↔sandbox、`audit_chain`↔sandbox、
  `learn`↔executor、`ocr`/`scene`/`locate`/`accessibility`（executor 用）都是双向边。
  按 §9 A4 的迁移顺序（只读文件工具 → 写入+diff/revert → command/process →
  permission/grant/approval → MCP → browser/desktop action）逐段切，是**多步工作**，
  不适合作为 M1 的单个步骤。

### 候选 D：Fleet / WorkSwarm

指南 §1.3 已把它们定档为"暂停新增功能、降为 Advanced/Experimental、可选加载"。
它们的出边最多（15 条），且互相引用。**在 §9 A2（统一 Daemon）落地前动它们，只会
把环搬来搬去**；建议排到 A2/A7 之后。

### M1 实际选择：B（ProductEval）

零入边 + 一步可验收，实际执行结果见 §5。**结论：环依赖比预估严重**——原来以为
"optional dependency + feature" 足够，实际三条路都不成立，最终靠"独立 workspace +
薄门面 crate"才落地。这条经验已写入 §4 的坑表。

### M12 之后的实测排序（M13 起，用同一次 `crate::` 扫描算的，不是估计）

M12 之后 core 剩 49 文件 / 37,567 行。把"出边全部逃向已迁出的 crate"（= 可以直接搬）
与"逃向仍在 core 的模块"（= 必须先搬那个模块）分开看：

| 模块 | 行数 | 入边 | 出边（`crate::`） | 能否立刻搬 |
|---|---:|---:|---|---|
| ~~`mcp_health`~~ | 532 | 2 | `tool_effects` | ✅ **M13 已完成**（归位到 `owo-agent-policy`，见 §15） |
| ~~`share_skill`~~ | 246 | 0 | `learn` | ✅ **M13 已完成**（归位到 `owo-agent-memory`，见 §15） |
| `accessibility` | 158 | 5 | **无** | ⚠️ 能搬，但域名是 Perception（指南 §3），建议随 Perception 一步走，避免二次搬迁 |
| `schema_budget` | 253 | 1 | `tools`（仍在 core） | ❌ 等 `tools` |
| `workflow` | 1,461 | 0 | `action_program`、`assert`（仍在 core） | ❌ 等 `action_program`/`assert` |
| `action_program` | 902 | 1 | `assert` + `ocr`/`perception`/`scene`/`executor` | ❌ 等 Perception |
| `assert` | 525 | 2 | `ocr`/`perception`/`scene` | ❌ 等 Perception |
| `tools` | 1,132 | 7 | 11 条，含 `computer_use`/`session`/`subagent` | ❌ 多步（§9 A4 分段） |
| `executor` | 1,356 | 2 | `accessibility`/`element_registry`/`locate`/`ocr`/`scene` | ❌ 等 Perception |
| `computer_use` | 2,355 | 1 | 12 条，含 `executor`/`tools`/`vision` | ❌ 最后 |
| `trace` / `session` / `goal` | 163 / 663 / 1,639 | 0 / 7 / 4 | 多条指向 core 内部 | ❌ 属"总线型"，更可能留在 core 或被 Daemon 收编 |

### M15 候选的实测结论（M14 之后重新扫描）

M14 之后 core 剩 **36 文件 / 30,891 行**。重新扫描得到一组**完全闭合集**：

| 模块 | 行数 | `crate::` 出边 | 指向 |
|---|---:|---|---|
| `workflow` | 1,461 | `action_program`、`assert`、`audit`、`learn`、`skill_health` | 前两个同迁；后三个已是 kernel/memory/contracts |
| `action_program` | 902 | `assert`、`executor`、`learn`、`ocr`、`perception`、`scene`、`UiNode` | 同迁 + memory + **perception（M14 已抽出）** |
| `assert` | 525 | `ocr`、`perception`、`scene`、`UiNode` | 全部指向 **perception（M14 已抽出）** |
| `executor` | 1,356 | `accessibility`、`element_registry`、`locate`、`ocr`、`scene`、`learn`、`platform` | 全部指向 perception / memory / kernel |
| **合计** | **4,244** | — | **零 core 内部逃逸** |

因此 M15 可以整体搬迁，推荐拆成两个 crate（域名分开，且 `executor` 将来要进
`tool-host/crates/executor`）：

* `owo-agent-workflow` = `workflow` + `action_program` + `assert`（2,888 行，依赖 executor）；
* `owo-agent-executor` = `executor`（1,356 行，依赖 perception + memory + kernel）。

搬完之后 core 只剩 ~26,600 行，剩余的都是"总线型"模块（`agent` / `tools` / `computer_use` /
`session` / `goal` / `fleet` / `workswarm` / `worker_pool` / `trace` …），它们被多个域同时引用，
按 §9 A4 的分段逐步处理，或最终被 Daemon 收编。
结论（M13 的具体建议）：

1. ~~**M13 = 两个"归位"小步合并**~~ ✅ 已完成（见 §15）：778 行、零倒置、零新依赖。
2. **M14 = Perception Worker**：唯一能兑现指南 §10"普通 Agent 改动不触发 ONNX 编译"
   的一步，也是 §1 判据里"5 条出边全是双向边"的那个真环。必须先写 ADR（倒置 vs 整组
   搬迁），并且它必须是**独立 workspace**（§3.2 已实测：feature 关不住会成环的边）。
3. **M15+ = `action_program`/`assert`/`workflow`**：它们只有 Perception 一个前置。
4. **最后 = `executor`/`tools`/`computer_use`**（§9 A4 的分段），以及 `trace`/`session`/`goal`
   这类被多个域引用的总线型模块。

### 历史排序（M10 当时的判断，保留作对照）

实测把代价排序改写了三次（M1 的环比预估严重、M3 比 ADR 简单一半、M8 多出一整类边界），
所以只给方向、不给承诺：

1. **继续走"零出边契约/状态"这一路**（M8/M9 已证明它每次都能零调用方改动落地）：
   `mcp`（→ Tool Host 段）、`accessibility`（→ Perception，需 ADR）、
   `memory`/`observe`/`learn`（2,454 行，出边只有 `settings`/`skill_health`/`platform`）。
2. **再动带环的执行边界**：`workflow`（1,461 行，出边 `action_program`/`assert`/`learn`/
   `skill_health`）、Tool Host 其余部分（policy/grant/approval，§9 A4 分多段）。
3. **最后动 Perception Worker**（`ort`/`sherpa-onnx`/`ndarray` 出基础链，SLO 收益最大）——
   指南 §9 A3 要求它成为独立 workspace，必须先写 ADR。
4. Fleet / WorkSwarm 的**执行侧**（`workswarm.rs` 本体 4,328 行）排到 A2（统一 Daemon）
   之后，否则只是把环搬来搬去；M9 已经把它的契约侧先行落地，届时只需处理执行侧。

---
