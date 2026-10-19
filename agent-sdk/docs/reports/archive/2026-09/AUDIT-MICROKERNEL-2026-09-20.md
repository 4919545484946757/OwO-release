# 微内核拆分后的系统审计（2026-09-20）

> 范围：M0–M15 拆出的 16 个 crate 与 `owo-agent-core` 之间的**调用路径、可见性、
> 信息交互、审计不变式、编译/链接成本**。
> 方法：机械扫描（脚本可复现）+ 编译器的 warning/error + 逐条人工判定。
> 结论分三档：**已修（本步）／建议项（有证据与收益）／无问题（已核实，避免重复讨论）**。

---

## 0. 一句话结论

拆分的**结构**是健康的（依赖无环、调用方零改动、公共面等价），但审计发现
**三类真实问题**：① 内部调用仍走"别名桥"177 处，边界不自解释；
② 跨 crate 错误在边界上退化为字符串，结构化信息丢失；
③ `executor` 拆出后，"UI 动作必留收据"从**结构保证**退化为**约定**（当前只有 1 个调用方，
尚无真实绕过）。另量化出一个明确的提速目标：core 的 ORT 链接成本**由一个文件决定**。

---

## A. 调用路径与可见性

### A1（已修）`pub(crate)` 跨 crate 后不可达 / 变死代码 —— 本步 6 处

| 条目 | 症状 | 处置 |
|---|---|---|
| `executor::parse_click_at` | `E0603: function is private`（调用方 `action_program` 已迁到 workflow crate） | 提权 `pub` + doc 说明 |
| `executor::scroll_at_screen` | `dead_code` 警告（调用方 `computer_use` 在 core） | 提权 `pub` |
| `executor::{click_at_screen, launch_target, send_shortcut, send_unicode}` | 同类（core 的 `computer_use` 依赖它们的可见性） | 提权 `pub` |

**这是同一模式的第 5、6 次出现**（M8 `E0603`、M12 `dead_code`、M13 `thiserror`、M14 `dead_code`）。
处置方式已固化为两步：① 搬迁后逐条读 `check` 的 warning；② 对每个新 crate 跑一次
"`pub(crate)` 条目是否被其它 crate 的**限定路径**引用"扫描。

**扫描结果（精确版，限定路径匹配）**：当前所有 crate 中，
**0 处** 跨 crate 引用 `pub(crate)` 条目 —— 说明本步之后没有残留。

### A2（无问题，已核实）别名桥引用

`crate::audit::…` 这类写法在 core 里出现 **177 处**，覆盖 **30 个已迁出模块**：

| 模块 | 处数 | 模块 | 处数 | 模块 | 处数 |
|---|---:|---|---:|---|---:|
| `audit` | 19 | `mcp_health` | 8 | `lease` | 6 |
| `permissions` | 17 | `workswarm_output` | 7 | `cas_store` | 6 |
| `computer_task` | 15 | `capability` | 7 | `sandbox` / `storage_crypto` | 5 / 5 |
| `team_benefit` | 12 | `mcp` | 7 | `experience_store` / `error` / `deadline` | 4 各 |
| `platform` | 10 | `vision` | 9 | 其余 12 个模块 | 1–3 |
| `tool_effects` | 10 | | | | |

集中度：`computer_use.rs` 54 处、`agent.rs` 33、`tools.rs` 19、`workswarm.rs` 19。

**这不是 bug**（别名模块是拆分期的有意设计，见 `ARCH-MICROKERNEL.md` §0 手法），
但它是**质量与防回归的欠账**：源码里 `crate::permissions::Policy` 看起来像 core 自己的
东西，实际来自 `owo-agent-policy`。新提交者很容易把本该进 policy 的逻辑写进 core，
而编译器不会拦。

---

## B. 信息交互（跨 crate 边界传什么）

### B1（无问题，已核实）**没有**"字符串匹配错误信息"这类硬伤

我原本怀疑 server 靠 `err.contains("…")` 把 crate 错误映射成 HTTP 语义，实测：
**server 的 41 个 api 模块里 0 处** `err.contains(` / `to_string().contains(`。
错误码契约（`ErrorCode` / `error_codes_tests`）在 HTTP 层自成体系。

### B2（建议项）跨 crate 错误在边界退化为 `String`

| crate | `Result<_, String>` | `thiserror` 枚举 |
|---|---:|---:|
| `owo-agent-extensions` | 55 | 0 |
| `owo-agent-workflow` | 41 | 0 |
| `owo-agent-executor` | 34 | 0 |
| `owo-agent-contracts` | 25 | 0 |
| `owo-agent-memory` | 24 | 0 |
| `owo-agent-perception` | 23 | 0 |
| `owo-agent-kernel` | 15 | **4** |
| `owo-agent-policy` / `tool-safety` / `env` | 2 / 0 / 13 | **1 / 2 / 2** |

代价：调用方拿不到**错误类别**——无法区分"输入非法 / 前置缺失 / 环境不支持 / 权限拒绝"，
只能展示字符串。这会影响：① 重试策略（可重试 vs 不可重试）；② HTTP 错误码映射质量；
③ 测试断言的精确性（现在只能断言字符串包含）。

**建议第一步（窄口径）**：只改 server 直接消费的 4 条边界
（`perception_api` / `workflow_api` / `computer_api` / `memory_api`），
让它们返回带类别的错误；**不动**内部实现。收益可量化：这 4 条边界的
HTTP 错误码覆盖度、以及"同一失败在两条路径上是否给出同一错误码"的一致性测试。

---

## C. 审计与安全不变式

### C1（建议项，本步最重要的发现）拆出 `executor` 后，"动作必留收据"从结构保证退化为约定

* 事实：`owo-agent-executor` 内部 **0 处**提及审计；真正写审计的是 core 的
  `computer_use.rs`（34 处）。
* 事实：直接调用 executor 落地函数的**只有 `computer_use.rs`**（22 处调用点），
  所以**当前不存在真实绕过**。
* 风险：crate 边界一旦打开，"先调 `click_at_screen` 再补审计"就成了可能；
  这与指南 §2.4（权限判定与执行与审计不可拆散）和 §13（权限不可绕过）相冲突。

**建议**（按代价从低到高）：

1. **契约测试**：加一条"UI 动作必留收据"的端到端测试（与 M4 的
   `execution_boundary_contract_tests` 同型，但覆盖 UI 动作而非沙箱命令）；
2. **类型层收口**：让 executor 的落地函数要求一个能力凭证/审计 sink 参数
   （如 `&dyn ExecReceiptSink`），使"无收据的执行"在类型上不可表达；
3. 长期：`executor` 进 Tool Host 时，与 `owo-agent-policy` 的裁决结果绑定
   （指南 §9 A4 的"权限不可绕过"落点）。

---

## D. 编译 / 链接成本（提速）

### D1（建议项，唯一明确的提速目标）core 的 ORT 链接成本由**一个文件**决定

* 事实：`core` 里引用感知模块（`ocr` / `perception` / `scene` / `vision` /
  `element_registry` / `paddle_ocr` / `accessibility` …）的文件**只有
  `computer_use.rs`**（15 处）。
* 推论：只要 `computer_use` 不再进程内调用感知，core 的 30 个集成测试二进制就会
  彻底脱离 ONNX/Sherpa 的静态链接（M14 实测这部分**没有**改善：73→72 个 exe、
  2.68→2.71 GB、中位数 28.7→30.5 MB）。
* 阻塞：`computer_use ↔ tools` 是真环（`tools` 也用 `computer_use` 的类型），
  必须先按指南 §9 A4 的分段解开（executor/tools 段）。

### D2（无问题）分片后的编译单元收益已经拿到

`cargo test -p owo-agent-perception` 只编译 6,008 行 + 1 个测试目标（90 s），
不牵动 core 的 30 个测试目标；`-p owo-agent-executor` 43 s；`-p owo-agent-workflow` 12 s。

---

## E. 生产代码 panic 风险（质量）

非测试代码里的 `unwrap()` / `expect()` 统计：

| crate | `unwrap()` | `expect()` | 备注 |
|---|---:|---:|---|
| `owo-agent-workswarm` | 25 | 0 | 最高，建议逐个确认 |
| `owo-agent-env` | 1 | 15 | `expect` 集中在 desktop_env 的解析 |
| `owo-agent-perception` | 11 | 2 | |
| `owo-agent-extensions` | 8 | 5 | |
| `owo-agent-kernel` | 4 | 0 | |
| `owo-agent-tool-safety` | 0 | 1 | |
| 其余 7 个 crate | 0 | 0 | ✅ |

守护进程不应该因为一条畸形输入 panic。按"每个 unwrap 都能证明安全"的标准逐个审计，
优先 `workswarm`（25）与 `env`（15 expect）。

---

## F. 优先级建议（按 收益 ÷ 代价 排序）

| 优先级 | 项 | 类型 | 代价 | 收益 |
|---|---|---|---|---|
| **P0** | C1-1：UI 动作"必留收据"契约测试 | 安全 | 低（1 个测试文件） | 把不变式拉回可验证 |
| **P0** | E：`workswarm` 25 处 unwrap 审计 | 质量 | 低 | 去掉守护进程 panic 面 |
| **P1** | A2/O1：core 内部 177 处别名桥改绝对路径 | 质量/防回归 | 中（纯机械，10 个文件） | 边界自解释；别名层可退化为"仅对外兼容" |
| **P1** | D1：解开 `computer_use ↔ tools` 环并迁出 `computer_use` | 提速 | 高（§9 A4 一段） | core 彻底脱离 ONNX 链接（唯一明确的提速项） |
| **P2** | B2：4 条 server 边界的结构化错误 | 质量 | 中 | 错误码覆盖度 + 重试语义 |

---

## G. 复现命令

```powershell
# 可见性审计（跨 crate 引用 pub(crate) 条目）
#   见本文 A1 的脚本；判据：0 处
# 别名桥统计
#   扫描 core 源码里 `crate::<已迁出模块>` 的出现次数（本文 A2 表；判据：当前 177 处）
# 依赖清单完整性（每个 crate 搬完就跑）
& scripts\mk-deps-scan.ps1 -Path crates/owo-agent-<x>/src -Manifest crates/owo-agent-<x>/Cargo.toml
# 编译/链接成本
cargo tree -p owo-agent-core -e normal -i ort        # 目前：ort ← perception ← core
Get-ChildItem target\debug\deps -Filter *.exe | Measure-Object Length -Sum
```
