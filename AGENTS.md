# AGENTS.md

> 适用：本仓库及其子目录。更新：2026-10-11。
> 本文件是开发约束与源码导航，功能状态以当前工作树和当次验证为准。

## 1. 工作范围与事实优先级

- 当前活跃开发目标是 `agent-sdk/`：Rust Agent SDK、共享 Daemon、CLI/TUI、HTTP API 和本地多 Agent 工作台。
- 用户已明确剔除历史输入法工程：本工作树不保留 C++ 输入法源码或本地备份，输入法由独立 OwO 主仓库维护。新业务代码放在 `agent-sdk/`；根目录规则文档等仅在任务明确涉及时修改。
- 2026-10-11 已从 `agent-sdk/` 活动源码、workspace 和锁文件剔除输入法 IPC 适配库；Agent CLI、Daemon、桌面客户端保持独立。历史 C++ 工程、输入法构建/安装脚本和本地归档均已删除，不得重新加入 Agent 构建、依赖或产品入口；运行 `agent-sdk/scripts/check-agent-boundary.py` 检查边界。
- 开始工作先读 `git status --short`，保留既有修改和未跟踪文件。不擅自 reset、clean、覆盖他人修改、清理构建产物或提交整个工作区。
- 证据顺序：当前源码与实际调用链 > 当前源码对应的运行/测试证据 > 最新专项设计 > 主技术文档 > README、历史验收和施工记录。源码能说明“怎么实现”，运行证据才能说明“本次是否可用”。
- 本文件的检查快照不是长期验收结论；后续改动必须重新核对涉及的事实。

阅读入口：

| 文件 | 用途 |
| --- | --- |
| `builGoal/主开发技术文档-Agent-SDK-v1.md` | 产品边界与整体设计；不可直接用其中旧日期的模块清单判定当前完成度 |
| `builGoal/Agent-SDK-后续任务实施指南-2026-09-18.md` | 功能闭环、Daemon、工具宿主、验收与资源红线，尤其 §2.4 |
| `builGoal/Agent-SDK-CLI优先-Codex式拆分-Electron前端-施工文档-2026-09-24.md` | CLI 与桌面前端演进 |
| `builGoal/权限档位与工具引入-设计开发文档-2026-09-30.md` | 权限档位、工具效果与安全边界 |
| `builGoal/子代理与蜂群-质量闭环-设计开发文档-2026-09-30.md` | 多 Agent、评审和交付质量 |
| `builGoal/Agent-SDK-远端engine取优合并记录-2026-10-01.md` | 最近合并范围与历史验证线索 |

`技术文档-AI智能体输入法.md` v0.6 保留为历史设计依据，不再作为唯一当前开发入口。文档之间有冲突时先核对源码；涉及产品范围变化，交由用户决定。

## 2. 当前架构与改动归属

主调用链：

```text
CLI turn / 默认 REPL / TUI / 桌面 HTTP 客户端
    → 共享 Daemon HTTP/SSE
    → Server 路由、会话、审批、任务生命周期
    → Core Agent / WorkSwarm
    → Policy + ToolHost + Audit
    → 文件 / Git / 命令 / MCP / Artifact
```

| 位置 | 职责与约束 |
| --- | --- |
| `crates/owo-agent-protocol`、`owo-agent-contracts` | 共享协议、DTO、基础契约；避免引入运行时业务和重型原生依赖 |
| `crates/owo-agent-client` | Daemon 发现、认证、HTTP/SSE、会话、回合和审批；仓库内部仅依赖 protocol，不得依赖 core/server/SQLite/MCP/感知或执行模块 |
| `crates/owo-agent-cli` | 命令和终端交互；`serve` 是宿主，`turn`、默认 REPL 和 TUI 经 Daemon；`repl --local` 是显式兼容路径，不作为新功能入口 |
| `crates/owo-agent-server` | HTTP 路由、认证、状态管理、事件、恢复和功能接线；修改路由必须同步路由契约 |
| `crates/owo-agent-core` | Agent loop、工具宿主、模型网关、任务编排与跨域组合；不要继续向 `tools.rs`、`agent/mod.rs` 堆叠不相关职责 |
| `owo-agent-kernel`、`owo-agent-policy`、`owo-agent-tool-safety` | 基础能力、权限和受信执行；下层不得反向依赖 core |
| `owo-agent-workswarm`、core 的 `workswarm/`、server 的 `workswarm_api/` | 分别承载协同契约/存储、协调运行和 HTTP 接线；角色写范围、租约、恢复与交付规则必须一致 |
| `owo-agent-perception`、`owo-agent-executor`、`owo-agent-workflow`、`owo-agent-env` | 感知、UI 执行、动作程序和环境模型；独立 crate 不等于已从默认构建依赖图卸载 |
| `owo-agent-memory`、`owo-agent-extensions`、`owo-agent-plugins`、`owo-agent-mcp` | 记忆学习、扩展服务、插件契约与 MCP 宿主 |
| `desktop/electron`、`desktop/tauri`、`desktop/web` | 多套现存桌面/浏览器表面；先确认用户实际使用的客户端与启动入口，再修改对应 UI |
| `clients/ts` | TypeScript 客户端与生成契约；API 修改需检查其同步需求 |
| `devtools/product-eval`、`owo-agent-eval-facade` | 独立评测 workspace 与兼容门面；server/cli 仍经门面依赖评测代码，不能宣称已完全退出生产构建 |

不得在新客户端再创建第二套 Agent、会话库、MCP 连接或工具注册表。已有兼容实现需要显式标识，迁移时保持原有数据和错误语义。拆包须检查实际依赖图与 feature，禁止仅凭“搬到另一个 crate”认定隔离完成。

## 3. 快速源码检查快照（2026-10-02）

范围：workspace 清单、代表性入口、模型网关、权限/工具宿主、SSE、WorkSwarm 和对应测试源码。此次仅重写说明文件，未编译 Rust、未运行测试、未调用云端模型、未验收桌面或发布产物。

总体判断：已有实质性的 Agent 与多 Agent 实现，契约测试和安全约束较完整；代码仍在迁移整合期，维护性、依赖隔离和运行闭环存在缺口。功能与声明部分匹配，不宜称为“全流程全面交付”。

| 能力/质量项 | 当前源码证据 | 判定与后续重点 |
| --- | --- | --- |
| 共享 Daemon | CLI `commands/turn.rs`、默认 `commands/repl.rs` 分派和 `tui.rs` 使用客户端；client 有依赖守卫 | 默认 CLI 路径已接入；保留 `--local` 兼容路径。跨客户端真实共享与恢复仍需当次运行验证 |
| 工具、审批与审计 | core `tools.rs` 的 `ToolHostService`，policy 模块，server 审批路由及权限测试 | 有真实实现；不能以接口存在代替越权、撤权、取消和恢复验收 |
| 多 Agent 与交付 | core `workswarm/`、server `workswarm_api/`、Artifact/Review/Human Inbox、恢复测试 | 有编排与质量闭环代码；当前工作树包含相关未提交修改及未跟踪 `write_lease.rs`，应先验证再认定完成 |
| 模型流式输出 | `gateway/provider.rs` 在收到 chunk 时调用 `consume_stream_buffer` 并转发回调 | 已有增量处理，不能沿用历史“全部缓冲后回放”的判断；首字延迟、取消与错误恢复需运行验证 |
| 服务启动 | CLI `commands/serve.rs` 在监听前 await MCP；`support.rs::connect_mcp_clients` 已并发连接并设置全局 3 秒上限 | 已有限时降级，不属于无限挂起；仍可能给 ready 增加约 3 秒等待，需验证坏 MCP 下的完整启动时延与状态诊断 |
| SSE 资源边界 | Turn 队列有 128 事件/1 MiB 上限；`event_stream.rs`、`sse.rs` HTTP 转发仍使用 unbounded channel | 不能宣称所有 SSE 都有背压；慢客户端可能在转发层积压，需端到端有界与断连回收验证 |
| 流式用量与预算 | `gateway/stream.rs::parse_sse_payload` 在读 usage 前要求 `/choices/0/delta` 存在；现有 usage 单测使用非空 choices | 输入 `{"choices":[],"usage":{...}}` 会提前返回 None，漏记该帧用量并影响依赖用量的预算判断；需真实格式回归测试 |
| 架构维护性 | `tools.rs` 约 3,765 行，`agent/mod.rs` 约 1,776 行，server `lib.rs` 约 1,533 行 | 拆包已有进展，但工具与装配仍集中；优先按职责拆分并保留行为契约，不为拆分继续增加门面层 |
| 重依赖与评测隔离 | core 直接依赖 perception，后者默认启用 STT 并依赖 ORT；server/cli 依赖 eval-facade | 仅凭 core 清单不再直接列 ORT，不能证明传递依赖或最终链接已消除；按实际 feature/构建结果判断 |
| 能力成熟度与旧文档 | server `capabilities.rs` 当前登记 5 个 Beta、2 个 Experimental、0 个 Stable；README 仍沿用 v0.6 和旧验收声明 | 登记表也只覆盖部分能力；文档不可自动把有模块/测试的功能提升为 Stable，README 的旧测试路径也需按现文件核对 |
| 桌面自治与跨机 | 感知/执行/环境模块真实存在；world-model 有不可用回退，fleet 协议有证书预留字段 | 属于需单独验证的扩展面；模拟、协议骨架和 fallback 不构成通用桌面自治或跨机生产验收 |

上述问题只作本次审查记录，没有在本次顺带修复。处理顺序：先补流式用量格式和 SSE 资源边界的回归验证，再验证 MCP 限时降级、收敛默认构建依赖与文档成熟度；大型模块拆分跟随具体功能边界推进。

## 4. 安全与功能契约

- 权限默认 deny：未获策略授权的操作不得执行。当前 `Workspace` 档允许宿主验证的工作区只读操作，写入、命令、联网、UI 控制、越界及破坏性操作按实际档位/规则审批；不要把“默认 deny”误写成所有读取都必须人工确认。
- 主 Agent 不得给自己授权。审批、策略判定与执行分离；所有工具执行经过统一宿主，检查工具、参数、范围、授权有效期与撤销状态。
- 保持 deny 规则、审计、秘密脱敏、取消和高风险确认；不得用 `--no-approval`、trusted/unrestricted 档或模拟执行结果替代正式验收。
- M1 基础能力持续有效：会话、审计、diff/revert、工具权限。相关行为修改必须带有意义的契约测试。
- HTTP 新增/修改/删除路由时同步 `agent-sdk/crates/owo-agent-server/tests/route_contract_tests.rs`，并检查 OpenAPI、协议 DTO、TS 客户端与实际 UI 调用。测试文件存在不等于测试已通过。
- 多 Agent 写操作必须遵守实际角色写范围与资源租约；恢复、重试、steer、Human 提交和评审不能绕过原有权限或幂等校验。
- 功能验收至少包括：调用入口 → 输入/权限 → 执行与状态变化 → 可读输出/Artifact → 审计 → 失败/取消/恢复。接口返回 200、模块存在、模拟通过和历史绿色报告不能单独判定完成。
- 标记 Stable 前需核对入口、权限、契约测试和诊断，并给出同版本真实运行证据。Beta/Experimental 与未配置状态必须对用户明确可见。

## 5. Windows PowerShell、写文件与编码

- 兼容 Windows PowerShell 5.1；禁止 `&&`、`||`、bash 变量替换和 bash 文件重定向。依赖操作分步执行；需要组合时使用合法 PowerShell 语法。
- 搜索优先 `rg` / `rg --files`；PowerShell 下通配文件用 `rg -g '*.rs' 目录`，不要把未展开的 `目录\\*.rs` 当作文件参数。
- 创建或修改文件一律由 Python 完成；禁止用 `Set-Content`、`Out-File`、echo 或重定向写文件。含中文/引号/Markdown 的内容使用可靠的 UTF-8 传输或编码载荷，避免经 GBK 控制台中转。
- 源码与文档使用 UTF-8；`.ps1` 必须 UTF-8 BOM，前三字节为 `239,187,191`。写后用 `Get-Content -Encoding UTF8` 核对；提交前抽查 diff 和相关编码门禁。
- 路径使用 Windows 格式，含空格时加引号。禁止跨 shell 拼装删除/移动命令；递归删除前校验绝对目标位于用户指定范围。
- 后台辅助进程用隐藏窗口启动；需要真实 GUI 验收时另行确认实际可见窗口与交互结果。

## 6. Rust 资源红线与验证

资源策略唯一实现：`agent-sdk/scripts/ci-shared.ps1`。优先使用 `ci-gate.ps1` 或 `Invoke-CiCargo`，不得另抄一套资源/原生依赖探测逻辑。

强制规则：

1. 任何手写 Cargo 命令前，先点源 `scripts/resolve-ort.ps1` 并执行 `Resolve-OwoOrtEnv`；只注入当前进程，不写用户/机器级 ORT 配置。缺失时先修前置条件，防止 ort-sys/sherpa-onnx-sys 静默下载挂起。
2. 定向构建/测试最多 `-j 2`、`RUST_TEST_THREADS=2`。完整 workspace、完整 core、release、原生依赖重链一律 `-j 1`、测试线程 1。禁止依赖 Cargo 默认并发、同时运行两组 Cargo、为提速调高并发。
3. 可用物理内存 <6 GB 或已用 ≥80% 时不得启动构建。构建卷剩余 <6 GB（定向）或 <20 GB（完整/release/原生重链）时不得启动。运行中保持资源守护。
4. 资源不足报告 `resource_limited` 与实际退出码；不擅自清理文件。可再生产物包括 incremental/PDB，但清理须在已授权范围内，并确认没有正在运行的构建。
5. 长任务保留逐行日志与约 30 秒心跳；禁止 `--quiet` 加 `Select-Object -Last` 隐藏进度，禁止靠加 timeout 替代并发限制。保留真实退出码，区分失败、超时、资源拒绝与未运行。

定向验证示例（在根目录开始；替换成受影响的测试）：

```powershell
cd agent-sdk
. scripts\resolve-ort.ps1
Resolve-OwoOrtEnv -Quiet | Out-Null
. scripts\ci-shared.ps1
$ciExit = Invoke-CiCargo -Arguments @('test', '-p', 'owo-agent-core', '--test', 'gateway_tests', '--locked', '-j', '2', '--', '--test-threads=2') -Cwd (Get-Location).Path -PolicyMode normal -PassThru
if ($ciExit -ne 0) { throw ('测试退出码：{0}' -f $ciExit) }
```

完整验证仅在改动范围和交付要求需要时运行，使用 strict 档：

```powershell
$ciExit = Invoke-CiCargo -Arguments @('test', '--workspace', '--locked', '-j', '1', '--', '--test-threads=1') -Cwd (Get-Location).Path -PolicyMode strict -PassThru
if ($ciExit -ne 0) { throw ('测试退出码：{0}' -f $ciExit) }
```

- Rust 改动保持 `cargo fmt --check` 与受影响范围 clippy 干净；先做受影响 crate check，再跑 1～3 组相关定向契约测试。修改 server `src/lib.rs` 等核心装配处必须验证 cargo check。
- 文档改动核对路径、内容、UTF-8 和 `git diff --check`，不为纯文档修改启动全量 Rust 编译。
- Tauri 壳与 `devtools/product-eval` 为独立 workspace；相关改动需分别验证，主 workspace 的绿色结果不能覆盖它们。
- 交付 EXE/安装包时核对源码身份、dirty 状态、build ID、核心/sidecar 哈希和真实进程；证明双击启动、可见窗口、核心健康与真实交互。旧二进制冒烟不能用于当前代码验收。

## 7. 模型凭据与运行配置

- 模型凭据只经环境变量注入，用户级环境变量可从 Windows 注册表读取后注入当前进程。禁止把密钥写入源码、配置、仓库文档、日志、测试 fixture 或对话。
- 不把“当前机器已设置”“固定密钥长度”或“某模型已获得服务权限”写成通用仓库事实。排障只显示存在性与长度，不读取或回显密钥正文。
- 当前代码默认值来自 `agent-sdk/crates/owo-agent-core/src/gateway/config.rs`：`https://open.bigmodel.cn/api/paas/v4` 和 `glm-5.3-flash`。这是本地配置事实，不代表远端可用或实际回合必定使用它；还需检查环境、设置和请求覆盖。
- `OPENAI_BASE_URL` / `OPENAI_MODEL` 可覆盖默认值；本地兼容端点按代码允许空密钥。模型未配置时维持诊断和设置入口，返回明确 `provider/not_configured`。
- `OWO_CLOUD_ENABLED=false` 是模型调用出境开关，不能据此宣称所有 MCP/浏览器/下载网络都被关闭。
- 其他常用变量：`OWO_AGENT_DATA`（数据根）、`OWO_HTTP_PROXY` / `HTTPS_PROXY`（代理）、`OWO_MODEL_FAST` / `OWO_MODEL_VISION`（模型档位）、`OWO_MCP_SCHEMA_BUDGET_BYTES`（schema 阈值）。
- 普通 Agent 会话默认不设模型轮数和工具调用总数上限；OWO_AGENT_MAX_MODEL_TURNS、OWO_AGENT_MAX_TOOL_CALLS 可设正数作为显式上限。子代理、Team Worker 与评测仍按各自任务预算单独限额。
- 成本估算用 `OWO_EVAL_PRICE_IN_PER_MTOK` / `OWO_EVAL_PRICE_OUT_PER_MTOK`；未设置单价、缺失 usage 或只获得模拟结果时，不能填报真实成本。
- 本次仅审查文档，无需云端调用。需要真实模型评测时控制轮数/预算，并保留脱敏用量与错误证据。

子进程标准注入示例：

```powershell
$modelKey = [Environment]::GetEnvironmentVariable('OPENAI_API_KEY', 'User')
if ($modelKey) { $env:OPENAI_API_KEY = $modelKey }
Write-Host ('SET={0} LEN={1}' -f [bool]$env:OPENAI_API_KEY, $env:OPENAI_API_KEY.Length)
```

## 8. 协作与完成报告

- 并行 Agent 工作仅在用户或适用指令要求时启动；启动后按 `AGENTS-COORD.md` 认领文件，同一文件同一时间只允许一个 Agent 修改。
- 单人任务不为形式化协作写入巨大协调日志。遇到他人修改先读取并合并理解，不能覆盖。
- 完成报告说明实际改了什么、验证了什么、哪些问题仍未解决。明确区分静态审查、测试通过、真实模型执行、桌面验收和发布验收。
- 不把设计稿、mock/reference 结果、未提交代码、测试文件数量或历史报告当作功能全部完成的证明。
