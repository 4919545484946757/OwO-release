# ADR-001：抽出 Tool-Safety 内核（`audit_chain` + `sandbox`）

> 状态：**已实施**（M3，提交见 §7「实施结果」）
> 日期：2026-09-19
> 依据：`builGoal/Agent-SDK-后续任务实施指南-2026-09-18.md` §2.2（受信执行内核）、
> §2.4（不可拆散的事务边界）、§9 A4（抽取 Tool Host）、§13（Tool Host 权限不可绕过）；
> 事实基线：`docs/architecture/ARCH-MICROKERNEL.md` §5.1 的 SCC 分析。
> 前置：M0（`owo-agent-kernel`）、M1（`devtools/product-eval` + `eval-facade`）、
> M2（`owo-agent-extensions`）均已完成并验收。

> **实施结论（先说结果）**：原计划「先做接口倒置消除环、再整体搬迁」中的**前半步不必要**。
> 实测确认 `audit_chain ↔ sandbox` 的相互引用**只发生在这一对模块内部**，而两个模块是
> 一起搬进同一个新 crate 的，因此该边**不再跨越任何 crate 边界**，无需倒置。
> 实际实施 = 只做「整体搬迁 + 别名 re-export」，零接口改动、零调用方改动。
> 详见 §7；下面 §1–§6 保留当时的分析与决定，作为决策记录。


---

## 1. 背景与问题

`docs/architecture/ARCH-MICROKERNEL.md` §5.1 用 Tarjan 强连通分量把 `owo-agent-core` 的 76 个模块
压成 46 个分量。结论是：**M0/M1/M2 已经把 core 里所有"零入边 + 零出边"的模块切完了**
（M2 切走最后 5 个，4,493 行）。此后每一步都必须做**依赖倒置或整组搬迁**，不能再指望
"搬文件 + re-export"零代价推进。

在剩下的分量里，唯一一个**零出边**的成规模分量是：

```
[8] audit_chain(506 行) + sandbox(2,188 行) = 2,694 行
    出边（对 core 其它模块）：0
    对内核的依赖：audit_chain → credentials, storage_crypto；sandbox → 无
    入边：mcp、plugin
```

它正对应指南要隔离的**受信执行边界**：

* §2.2：「Policy 与 Executor 必须在同一受信边界，不能让主 Agent 自批自执行」；
* §13 完成定义：「Tool Host 权限不可绕过」；
* §2.4 第 3 条：权限判定、capability、执行和审计收据必须同属一个拥有者。

因此把 `sandbox`（OS 级执行沙箱：Job Object / AppContainer / 资源上限 / kill-on-close）
与 `audit_chain`（防篡改审计链）放在同一个 crate，是**方向正确且必须**的一步。

## 2. 唯一阻塞点（实测）

两者**互相引用**，构成一个 2 模块真环：

```rust
// crates/owo-agent-core/src/audit_chain.rs:14
use crate::sandbox::SandboxAuditLog;

// crates/owo-agent-core/src/sandbox.rs:637
chain: &mut crate::audit_chain::AuditChain,
```

* `audit_chain::append_sandbox_log(&mut self, log: &SandboxAuditLog, actor: &str) -> usize`
  —— 链**知道**沙箱类型（把 `SandboxAuditEvent` 按 `sandbox.<kind>` 写成审计记录）；
* `sandbox` 内部某处把 `SandboxAuditLog` 汇入 `AuditChain` —— 沙箱**知道**链。

在同一个 crate 里这完全合法，一旦分 crate 就必然成环。这与 M1 的
"core ↔ devtools"环不同：那是**包之间**的环，可以用独立 workspace + 门面解决；
这里是**两个模块之间的概念环**，只能靠接口倒置消除。

## 3. 候选方案

### 方案 A（推荐）：沙箱审计汇出改为注入式 sink，`audit_chain` 不再认识沙箱类型

把"把沙箱事件写成审计记录"的职责从 `audit_chain` 挪到调用点（core 侧）：

```rust
// 新 crate：sandbox 侧只暴露数据与一个中性汇出口
pub trait SandboxAuditSink {
    fn record(&mut self, event: &SandboxAuditEvent, actor: &str) -> usize;
}
// sandbox 自己不再 import audit_chain；core 侧提供适配器
impl SandboxAuditSink for AuditChain { /* 原来的 append_sandbox_log 逻辑搬到这里 */ }
```

* 优点：环彻底消失；`sandbox` 与 `audit_chain` 之间不再有概念依赖；不扩大内核 API；
  审计记录格式（`sandbox.<kind>`、`tool = sandbox:<name>`）保持字节级不变，
  指南 §5.3 的 `policy.db` / 审计收据语义不受影响。
* 代价：`append_sandbox_log` 从 `AuditChain` 的固有方法变成 core 侧适配器的实现；
  需要改动 `audit_chain` 的少量调用点与 `audit_chain_tests` 的断言位置。
* 风险：低。审计链的**验证**逻辑（HMAC 分段链、anchor、verify）完全不动，只动"谁负责
  把沙箱事件转成 AuditRecord"。

### 方案 B：把 `SandboxAuditEvent` / `SandboxEventKind` 等类型下沉到 `owo-agent-kernel`

* 优点：改动更机械（只是移类型），环消失。
* 缺点：内核开始承载"沙箱"概念，与 §2.2 的边界划分相悖——内核应当只放"被多个边界
  共用的稳定原语"，而不是"某个执行器的数据类型"。**否决。**

### 方案 C：整组搬迁 + 让 core 依赖新 crate（`owo-agent-tool-safety`）

* 即：先做方案 A 消除环，再把 `sandbox` + `audit_chain` 一起 `git mv` 到新 crate，
  core 添加别名 re-export（与 M0/M2 同手法），入边 `mcp`/`plugin` 由 re-export 满足。
* 这是方案 A 的执行形态，不是替代方案。**采纳。**

## 4. 决定

采用 **A + C**：

1. 先用注入式 sink 打断 `audit_chain ↔ sandbox` 环（方案 A）；
2. 再按 M0/M2 的既有手法把两者整体迁到新 crate `owo-agent-tool-safety`
   （依赖 `owo-agent-kernel`；反向依赖为零）；
3. core 保留同名别名模块 + 顶层 `pub use`，因此 `mcp`、`plugin` 与所有集成测试
   **零改动**；
4. 新 crate 是否最终演进为指南 §4 的独立 `services/tool-host` workspace，
   留到 §9 A4（Tool Host 抽取）再决定——本 ADR 只负责把**受信执行内核的边界**立起来。

### 命名的取舍

暂命名为 `owo-agent-tool-safety` 而不是直接叫 `owo-agent-tool-host`：按指南 §4，
Tool Host 是一个**独立 workspace 的进程**（含 policy/grant/approval/executor/mcp-host
四组 crate）。当前迁移只是把"沙箱执行 + 审计收据"这两块从 core 拿出来，
尚未包含 policy/grant/approval，故不用 `tool-host` 之名，避免名不副实。

## 5. 验收标准（实施时逐条落证据）

| # | 标准 | 判据 |
|---|---|---|
| 1 | 环已消除 | `audit_chain.rs` 中不再出现 `sandbox`（注释除外）；`sandbox.rs` 中不再出现 `audit_chain` |
| 2 | 新 crate 零反向依赖 | `cargo tree -p owo-agent-tool-safety` 中 `owo-agent-core`/`owo-agent-server` 出现 0 次 |
| 3 | 调用方零改动 | `git status` 显示 `owo-agent-server`、`owo-agent-cli` 未改动 |
| 4 | 编译 | `check --workspace --all-targets` exit=0（经 `Invoke-CiCargo`，完整档 `-j 1`） |
| 5 | 安全契约不回归 | `cargo test -p owo-agent-core --locked` exit=0，且 `sandbox_tests`、`os_sandbox_integration_tests`、`production_security_contract_tests`、`audit_chain_tests` 全绿 |
| 6 | 审计链语义不变 | 审计记录格式与 `audit_chain_tests` 的篡改检出矩阵不变；`verify` 仍能检出改字段/删记录/重排/伪造插入/篡改锚点 |
| 7 | 运行态 | `scripts/mk-smoke.ps1` 全绿（含 `audit.event_emitted` 与 no-orphan） |
| 8 | 资源合规 | 全部 cargo 经 `Invoke-CiCargo`；必要时按允许清单清理 `incremental`/`*.pdb` |

## 6. 影响与不做的事

* **不做**：本次不动 `permissions`/`permission_spec`/`grant_store`/`tool_effects`
  （它们与 `agent`/`tools` 同处 18 模块环团 `[16]`，属 §9 A4 后续步骤）。
* **不做**：不把沙箱执行改成跨进程。当前仍是同进程受信内核；跨进程 RPC
  （指南 §5.2 的 length-prefixed JSON-RPC）是 §9 A4 的课题。
* **必须复核**：`change_set` / `change_set_store`（M2 已迁到 `owo-agent-extensions`）
  与 `sandbox` 共同承担 §2.4 第 4 条"写前快照 / 写入 / diff / revert"边界。
  迁完 M3 后要明确：**快照与恢复的状态机在 extensions，执行的隔离与审计在 tool-safety，
  实际文件写入仍在 core 的 executor/tools** —— 三者的交接点必须有测试覆盖，
  否则就是"把一个事务边界切成了三份"。这是 M3 之后第一个要补的契约测试。

---

## 7. 实施结果（M3 已完成）

### 7.1 实际做了什么（比 ADR 原计划更简单）

| 计划（§4） | 实际 |
|---|---|
| 先用注入式 `SandboxAuditSink` 打断环（方案 A） | **不做**。实测环只在 `audit_chain` 与 `sandbox` 之间，两者同迁一个 crate 后该边不再跨 crate 边界，倒置纯属多余改动 |
| 再整体搬迁（方案 C） | ✅ 做了：`git mv` 两个模块到 `crates/owo-agent-tool-safety/` |
| core 保留别名 re-export | ✅ 做了：`pub use owo_agent_tool_safety::{audit_chain, sandbox};` |

代价面比预期小得多：`audit_chain.rs` 只需把 3 处内核引用改指
（`crate::credentials` → `owo_agent_kernel::credentials`；`crate::storage_crypto` ×2 同理），
`sandbox.rs` **一行未改**。

### 7.2 为什么原判断偏保守

ADR §2 把"两个模块互相引用"直接当成"必须倒置"，没有先问一个更基本的问题：
**这条边在搬迁后是否会跨越 crate 边界？** 只有当两个模块被分到不同 crate 时才需要倒置。
M3 的实际教训应写进 §4 的复用清单：

> 判定"是否需要接口倒置"的正确顺序是：
> ① 列出该模块的全部出边；② 判断这些目标是否与它**同迁**；
> ③ 只有"不同迁、且目标反向引用它"的边，才是必须倒置的真环。

### 7.3 额外发现（有价值，值得记）

`docs/architecture/ARCH-MICROKERNEL.md` §5.1 的分量清单把 `[8] audit_chain + sandbox` 记为"零出边
（对 core）"，但更精确的事实是：`sandbox` 的 Windows 部分是**裸 FFI**
（`extern "system"` + `#[link(name = "kernel32"/"advapi32"/"ntdll")]`），
**完全不使用 `windows` / `windows-sys` crate**。这意味着新 crate 的依赖闭包只有
`owo-agent-kernel` + serde/serde_json/chrono/uuid/sha2/thiserror，连 Windows 绑定依赖都不需要。
对指南 §10「普通 Agent 改动不触发原生重链」是直接利好。

### 7.4 验收证据

| # | 标准（§5） | 结果 |
|---|---|---|
| 1 | 环不再跨 crate | `sandbox` 只剩 `crate::audit_chain`、`audit_chain` 只剩 `crate::sandbox`；两者同 crate，无跨边界环 |
| 2 | 新 crate 零反向依赖 | `cargo tree -p owo-agent-tool-safety`：`owo-agent-core` / `owo-agent-server` / `sherpa` / `ndarray` / `rusqlite` 出现 **0 次**；`owo-agent-*` 只出现 kernel 与自身 |
| 3 | 调用方零改动 | `git status`：`mcp.rs` / `plugin.rs` / `tools.rs` / server / cli **全部未改动** |
| 4 | 编译 | `check --workspace --all-targets` exit=0（157 s，`Invoke-CiCargo -j 1`） |
| 5 | 安全契约不回归 | `cargo test -p owo-agent-core` exit=0（320 s）；`sandbox_tests`(26)、`os_sandbox_integration_tests`(25)、`production_security_contract_tests`(12)、`audit_chain` 篡改矩阵全绿 |
| 6 | 审计链语义不变 | 同上——`audit_chain_detects_any_tampering`、`encrypted_audit_export_restore_and_tamper_rejected`、`audit_export_never_leaks_managed_key_or_secret`、`audit_managed_key_reuses_and_verifies_across_restart`、`sandbox_event_kind_labels_for_audit_chain` 全部通过 |
| 7 | 运行态 | `mk-smoke.ps1 -Tag m3-tool-safety` **18/18 PASS**，含审计事件落盘与无孤儿进程 |
| 8 | 资源合规 | 全部 cargo 经 `Invoke-CiCargo`；完整档 `-j 1`。server 全量测试 995 s（含冷链），为本次最长单项 |

### 7.5 三方事务边界的缺口：**已闭合**

§6 末尾点明的三方事务边界契约测试，已在 M4 补齐：
`crates/owo-agent-core/tests/execution_boundary_contract_tests.rs`（4 条，全绿）：

1. **拒绝即不执行**：deny 名单命中 → 返回错误、不进入 spawn、工作区零文件产物；
   并用**裸进程**跑同一命令体做反向对照，证明该命令确实会写文件（避免断言空转）。
2. **拒绝必留收据**：被拒绝的执行必须产生明确拒绝语义的沙箱事件，且能汇入
   `AuditChain` 并通过 `verify()`。
3. **允许即可观测**：真实 Job 内写出的文件能被 `change_set::file_hash` 观察到
   （基线 `None` → 有哈希），与 `WorkspaceBaseSnapshot` 的"新建"判定一致。
4. **失败不半写**：非 0 退出的命令不得留下最终产物。

服务端侧的 `full_loop`（tracker → change_set → revert）仍由
`owo-agent-server/tests/v1_execution_safety_tests.rs` 覆盖；core 这组补的是它下面的
沙箱与文件系统这一层。指南 §2.4 第 4 条由此在两层都有断言。

