# ADR-002：抽出感知（Perception）内核，并把"按需 Worker"推迟到 Daemon 之后

* 状态：**已实施（M14）** —— 但 §2.1 记录了实施中发现的、与原计划不符的一条重要更正
* 日期：2026-09-20
* 相关：指南 §2.2 / §3（`ocr.rs`、`onnx_ocr.rs`、`stt.rs`、`accessibility.rs`、`vision.rs` → Perception Worker）、
  §9 A3（Perception Worker 是独立 workspace）、§10（资源红线：普通 Agent 改动不得触发 ONNX 编译）、
  `docs/architecture/ARCH-MICROKERNEL.md` §16（M14 候选）、ADR-001（Tool-Safety 的环处置）

---

## 1. 背景与实测数据（不是估计）

感知群组共 **11 个模块 5,870 行**，全部仍在 `owo-agent-core`：

| 模块 | 行数 | 群组内出边 | 重依赖 |
|---|---:|---|---|
| `onnx_ocr` | 1,217 | `ocr`、`platform` | **ort**、ndarray |
| `perception` | 704 | `accessibility`、`ocr`、`platform` | chrono / uuid / serde |
| `vision` | 662 | `ocr`、`paddle_ocr`、`platform` | base64、**png**、**reqwest**、serde_json |
| `scene` | 800 | `accessibility`、`element_registry`、`ocr`、`perception` | chrono / serde |
| `locate` | 531 | `element_registry`、`scene` | chrono / serde |
| `element_registry` | 527 | `accessibility`、`ocr` | chrono / serde |
| `paddle_ocr` | 389 | `ocr`、`onnx_ocr`、`vision` | reqwest、serde_json |
| `ocr` | 372 | `onnx_ocr`、`platform` | futures、**windows** |
| `window_template` | 296 | `accessibility`、`ocr` | chrono / serde_json |
| `stt` | 214 | **`settings`（core）** | **sherpa-onnx** |
| `accessibility` | 158 | 无 | **windows**（UIA） |

**关键测量结果：这个群组是一个近乎闭合的集合。**

* 群组内所有 `crate::` 引用中，指向群组外的只有两类：
  1. `platform::{capture_screen, capture_screen_region, render_text_bmp, poll_foreground_app}`
     —— **M0 已经下沉到 `owo-agent-kernel`**，搬迁时只要改绝对路径即可；
  2. `stt → settings::SttSettings` —— **唯一真正的逃逸边**（1 个类型）。
* 这与指南 §1 的早期判断（"5 条出边全是双向边"）**不一致**，原因是 M0/M5/M11 三步
  把 `platform`、`skill_health`、`learn` 等目标陆续搬走了——群组的出边被前面的步骤
  "消耗"掉了。这正是 `ARCH-MICROKERNEL.md` §15.1 记下的那条经验：**先把被依赖的域搬走，
  后来者就是零成本**。
* 群组入边全部来自仍在 core 的 `action_program` / `assert` / `computer_use` / `executor`，
  以及 server 的 `perception_api`(25) / `desktop_api`(6) / `locate_api`(3) /
  `workflow_backend`(2) / `session_api`(1)，都能由 core 的别名 re-export 满足。

## 2. 问题

1. core 直接依赖 `ort`、`sherpa-onnx`、`ndarray`、`png`、`windows`，于是
   **core 的 30 个集成测试二进制每一个都要静态链接 ORT/Sherpa**。本会话实测：
   每次分片测试都要重新链接 8–16 s，`target/debug/deps/*.exe` 累积到 **29.35 GB**。
2. 指南 §10 的红线是"普通 Agent 改动不得触发 ONNX 编译"。
3. 指南 §3 要求 `ocr.rs`/`onnx_ocr.rs`/`stt.rs`/`accessibility.rs`/`vision.rs` 归
   Perception Worker，§9 A3 要求它成为**独立 workspace**。

### 2.1 实施中的更正（2026-09-20，实施后回填，必须读）

计划里我写的是"切出去之后 core 的测试二进制不再链接 ONNX"。**这个断言是错的**，
实施后实测：`cargo tree -p owo-agent-core -e normal -i ort` 显示

```text
ort v2.0.0-rc.13
└── owo-agent-perception
    └── owo-agent-core
```

原因：core 的 `executor` / `computer_use` / `action_program` / `assert` **仍在进程内
调用感知**，所以 core 依然依赖本 crate，链接链上依然有 ORT/Sherpa。

**实施后逐项实测，本步的真实收益与"不是收益的东西"如下（这一段是本 ADR 最重要的部分）**：

| 项 | 结论 | 证据 |
|---|---|---|
| core 测试二进制不再链接 ONNX | ❌ **不成立** | `cargo tree -p owo-agent-core -i ort` → `ort ← owo-agent-perception ← owo-agent-core`；实测 exe 体积 **73 → 72 个、2.68 → 2.71 GB、中位数 28.7 → 30.5 MB**，基本持平 |
| "改 core 不再重编 ORT/Sherpa" | ❌ **不算本步收益** | 迁移前 ort/sherpa 本来就是独立 crate，改 core 也不会重编它们。这条在任何时候都成立，与本步无关 |
| perception 可独立编译与测试 | ✅ **成立（真收益）** | `cargo test -p owo-agent-perception` 只编译 6,008 行感知代码 + 1 个测试目标（90 s），完全不牵动 core 的 30 个集成测试目标 |
| STT 可整块关掉 | ✅ **成立** | `cargo tree -p owo-agent-perception --no-default-features` 的输出里**没有 sherpa-onnx**（ort/ndarray/windows 仍在，见下） |
| core 源码体量下降 | ✅ **成立** | 47 文件 / 36,787 行 → **36 文件 / 30,891 行**（−5,896 行，−16%） |
| 边界就绪（§9 A3 Worker 的前置） | ✅ **成立** | `cargo tree -p owo-agent-perception` 中 **core 出现 0 次**，即感知不依赖权威运行时——它已经可以独立成进程 |

**结论：本步拿到的是"边界 + 独立可测 + core 变轻"，不是"链接/磁盘节省"**。
后者的前提是"谁还在进程内用被切出去的东西"——只要 core 的 `executor`/`computer_use`
仍进程内调用感知，ORT 就必然留在链接链上。要真正拿到它，需要 §9 A3 的按需 Worker，
或者把 `executor`/`computer_use` 也搬走（§9 A4 分段）。
**这条教训比本步的收益更值钱：切边界 ≠ 降成本，必须先问"消费方在哪一侧"。**

这条更正写在这里而不是悄悄改掉原文，是因为它是本步最重要的事实：**边界切干净 ≠ 成本下降**，
成本下降取决于"谁还在进程内用被切出去的东西"。

## 3. 决策

### 3.1 M14 做：抽出 `owo-agent-perception`（workspace 成员）

* 11 个模块整体迁入新 crate（`git mv`，保留历史）；
* 唯一逃逸边 `stt → settings::SttSettings` 按 **M7/M11 已经用过两次的规则**处理：
  **配置类型随域走**——`SttSettings` + 它的 `impl Default` + 2 个 serde 默认值函数
  随 `stt` 迁入感知内核，core 的 `settings.rs` 用 `pub use` 转出
  （`owo_agent_core::settings::SttSettings` 与 `Settings.stt` 字段类型均不变）；
* 群组内对 `platform::*` 的引用改为 `owo_agent_kernel::platform::*`（M0 已下沉）；
* core 保留同名别名模块 + 顶层 `pub use`，**调用方零改动**；
* 新 crate 的依赖分三组，且**全部可选化**（`[features] ocr / stt / paddle`，默认全开）：
  `ort`+`ndarray`（OCR）、`sherpa-onnx`（STT）、`reqwest`（云端 OCR/视觉）、
  `windows`（UIA）、`png`、`base64`、`futures`、`chrono`、`serde`、`serde_json`、`uuid`。
  这样"只做桌面自动化、不要语音"的构建可以关掉 `stt`，不必解析 Sherpa。

### 3.2 M14 不做（并说明为什么）

* **不**把感知做成独立 workspace：指南 §9 A3 的最终形态是"按需 Worker"。但独立
  workspace 意味着 core/server **不能**直接 `use` 它，必须先有 worker 的 IPC 契约与
  生命周期管理（指南 §2.4 第 6 条：壳对子进程的生命周期）。现在做只会把 API 面挡掉。
* **不**改进程模型：server 的 `perception_api` 仍在进程内调用。等指南 §9 A2（统一
  Daemon）落地后，再按 §9 A3 把它改成"按需原生 Worker"，那时本 crate 直接成为 worker
  的二进制入口即可（这也是为什么本步**先**把边界切干净）。

## 4. 备选方案与取舍

| 方案 | 收益 | 代价 | 结论 |
|---|---|---|---|
| **A. 整体迁入一个新 crate（选定）** | 一次性把 ORT/Sherpa/ndarray 从 core 的依赖与链接里摘掉；零倒置（除 1 个配置类型）；调用方零改动 | 新 crate 较大（5,870 行）；feature 组合需要设计 | ✅ 采用 |
| B. 拆成 `perception` + `stt` 两个 crate | 不要语音时可完全不解析 Sherpa | 两个 crate + 两条 re-export 链；`stt` 只占 214 行 | ❌ 用 feature 达到同样效果，成本更低 |
| C. 现在就做独立 workspace + worker 进程 | 完全兑现 §9 A3 与 §10 | 需要 IPC 契约、进程生命周期、故障与重启语义；没有 A2 的 Daemon 就没有归属 | ❌ 推迟到 A2 之后 |
| D. 只把 `onnx_ocr`/`stt` 两个重依赖模块搬走 | 改动最小 | `perception`/`vision`/`paddle_ocr`/`scene` 仍留在 core，且会反向依赖新 crate → 产生新环 | ❌ 会造环 |

## 5. 后果

* **正面（已获得）**：编译单元隔离——改 perception 不再重编 core，改 core 不再重编
  ORT/Sherpa 这些上游 crate；感知成为可独立演进的编译边界。
* **未获得（见 §2.1）**：core 的测试二进制仍链接 ONNX，链接时间与磁盘占用**与迁移前持平**。
  原计划里"显著下降"的预期**不成立**；要拿到它必须做 §9 A3 的按需 Worker。
* **正面**：感知成为可独立演进的边界（UIA/OCR/VLM 的迭代不再牵动 core 编译单元）。
* **负面**：多一次 `git mv` 级别的代码移动；server 仍依赖新 crate（`perception_api` 等），
  因此 **server 的测试二进制仍会链接 ORT**——真正的"按需"要到 A2/A3 之后。
* **风险**：`feature` 默认全开时行为与今天完全一致；但如果某个消费方用
  `default-features = false`，必须显式打开它需要的能力，否则编译期就会报缺符号
  （这是刻意的：不允许静默降级）。

## 6. 验收标准（M14 落地时必须全部给出证据）

```text
1. cargo check --workspace --all-targets            exit=0，且 0 条 dead_code/unused warning
2. cargo tree -p owo-agent-perception              含 ort/sherpa/windows，且不含 owo-agent-core
   ※ 原第 2 条写的是"core 闭包里 ort/sherpa 为 0 次"——**该条已作废**（见 §2.1：
     core 仍进程内使用感知，ort 必然在闭包里）。改为核验"新 crate 不反向依赖 core"。
3. 新 crate 自身测试                                 全绿（onnx_ocr/vision/scene/locate/element_registry/
                                                    window_template/accessibility 的单测随迁）
4. core 全量测试（分批 31 分片）                     全绿；lib 条数减少 = 随迁条数（逐条对上）
5. server 全量测试（分批 40 分片）                   全绿
6. scripts/mk-smoke.ps1                             18/18 PASS
7. 编译单元隔离（本步真实收益）                      记录"改 perception 不重编 core /
                                                    改 core 不重编 ort|sherpa"的实测证据
8. 链接成本（本步**未**获得，如实记录）              记录 core 测试二进制体积与链接耗时实测值，
                                                    并标注"与迁移前持平，待 §9 A3 Worker 才改善"
9. 文档                                             ARCH-MICROKERNEL.md 新增 §17（M14），
                                                    本 ADR 状态改为"已实施"，并保留 §2.1 更正
```

## 7. 实施顺序（M14 内部）

1. `git mv` 11 个模块到 `crates/owo-agent-perception/src/`；
2. `stt → settings::SttSettings` 按"配置类型随域走"迁出（core 反向 `pub use`）；
3. 群组内 `crate::platform::` 改 `owo_agent_kernel::platform::`；
4. 写新 crate 的 `lib.rs`（边界文档 + glob re-export）与 `Cargo.toml`（分组 feature）；
5. core：删 11 个 `pub mod`，加别名 re-export；
6. 跑 §6 的 9 项验收并落证据。

## 8. 基线（迁移前实测，2026-09-20，供验收第 8 项对比）

| 指标 | 迁移前 |
|---|---|
| `cargo tree -p owo-agent-core` 中的重依赖 | `ort` ×1、`sherpa-onnx` ×1、`ndarray` ×2、`windows` ×1、`png` ×1 |
| `target/debug/deps/*.exe`（core + server 测试二进制） | 73 个 / **2.68 GB**，中位数 **28.7 MB**，最大 **69.4 MB** |
| core 分片测试的单目标链接耗时（本会话 M13 日志实测） | **8–16 s / 目标**（每个 core 集成测试目标都要静态链接 ORT/Sherpa） |
| core 源码体量 | 47 文件 / 36,787 行（其中感知群组 11 模块 5,870 行） |

> 迁移后必须重新测这三项并写进 `ARCH-MICROKERNEL.md` §17。**预期**：core 的闭包不再出现
> `ort`/`sherpa-onnx`/`ndarray`/`windows`；core 集成测试目标的链接耗时与体积显著下降
> （server 侧因为仍直接用 `perception_api` 等，预计不变——这一点必须在结论里如实说明，
> 不能把 server 的改善算作本步收益）。
