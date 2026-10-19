# OwO Agent SDK 全量代码审查报告

- **日期**：2026-10-02
- **范围**：`agent-sdk/`（Rust workspace 24 crate，约 169,464 行 .rs；desktop 三端 + clients/ts + devtools/product-eval 独立 workspace；根目录 C++ 输入法为历史基线，不在本次重点范围）
- **背景**：项目为类 Codex 桌面 agent，CLI 端与 desktop 端并行开发，2026-09 下旬起对远端 engine 分支做了 30+ 次逐块"取优合并"（cherry-pick 式，非整枝合并）
- **方法**：静态审查（git 历史、依赖图核对、路由/测试对照、规模统计、逐文件抽查），未执行构建与测试（遵守资源红线）

---

## 1. 总体结论

**健康度：中上（B），有 1 项阻断性缺陷（A0），并存在明确的"架构收敛债"。**

| 维度 | 评价 |
|---|---|
| 合并残留 | ⚠️ 无冲突标记、无依赖环、git 干净，但**合并提交 `2d23cd2` 覆盖了本土 web 功能**（A0，P0）；另有游离文件与双壳平行实现 |
| 架构（Rust 侧） | ✅ 微内核分层纪律严明，注释声明的依赖红线**实测全部守住** |
| 架构（前端侧） | ⚠️ 三套 API 客户端 + 双桌面壳 + 自制 mini 框架，自由生长 |
| 测试 | ⚠️ 服务端 39 个集成测试文件；web 24 个契约测试虽进 CI，但**实际执行为 373/335/38 红**，首次配置引导等功能已回归失效（A0） |
| 文档 | ⚠️ docs/ 与 ADR 质量高，但 README 顶层基线叙事落后于代码约一个半月 |

核心矛盾一句话：**后端在"做减法"（微内核化、依赖红线、契约测试），前端在"做加法"（三客户端、双壳、自制框架）——两端工程纪律不统一是当前最大的系统性风险；而 A0 说明这种不统一已经产生了实质代价：一次观感对齐合并就抹掉了一整个已锁定的功能层，且 CI 没能拦住。**

---

## 2. 亮点（审查确认的事实，非客套）

1. **合并纪律好**：30+ 次 engine 取优合并全部"单块 + 单测 + 提交说明"伴随，并有专门的同步收口报告（commit 08f6d82），冲突标记扫描为 0，`TODO/FIXME/HACK` 计数为 0。
2. **依赖红线实测成立**：workspace 声明的分层规则逐一核对通过——
   - `owo-agent-workswarm` 仅依赖 protocol ✅
   - `owo-agent-memory` 仅 kernel + contracts ✅
   - `owo-agent-client` 仅 protocol ✅
   - `owo-agent-mcp` 对 core 的依赖是 **dev-dependency**（Cargo.toml L34-35，注释中有环检测论证）✅
3. **测试资产雄厚（但需区分"资产"与"健康度"）**：server 39 个集成测试文件（含 `route_contract_tests.rs` 3632 行、`v1_execution_safety_tests.rs`、`production_security_contract_tests.rs`）；web 24 个 `node --test` 契约测试共 373 条断言，已进 `ci-gate.ps1` 第 5b 步——**但实际执行为 335 通过 / 38 失败（见 A0）**。契约写得足够多是真资产；门禁长期红则让资产失效。
4. **运行时防护齐全**：回合历史有 `compact_truncate` 截断与 `DeadlineBudget`（agent/mod.rs:505/621）；MCP schema 有预算压缩（`schema_budget::enforce_budget`）。
5. **Electron 安全基础正确**：`contextIsolation: true`、`nodeIntegration: false`（main.js:337-338）、19 行最小 preload、HTML 带 CSP。
6. **CLI 功能面完整**：16 个子命令 + REPL 富斜杠命令（/fork /rewind /tree /traces /share /undo…）+ OpenCode 风格 TUI（1489 行），Codex 式核心链路（会话/审批/diff/revert/审计）全通。

---

## 3. 问题清单

严重度定义：**P1** = 影响正确性/安全/长期演进，本迭代必须处理；**P2** = 应尽快排入；**P3** = 清理项。

### A. 合并引入的问题

| # | 严重度 | 问题 | 位置 | 证据 | 影响 | 修复建议 |
| **A0** | **P0** | **合并提交 `2d23cd2`（"工作台 UI 对齐上游 be6298f"）覆盖了本土功能，web 契约测试 38 项转红** | `desktop/web/app.js`、`index.html`、`style.css` | 该提交改动 app.js **6,909 行**、index.html **1,282 行**、style.css **3,503 行**（+8,621/−3,073）。对齐前有 `needsSetup`/`renderSetupGuide` 共 **7 处**，对齐后 **0 处**；`§12-12 路由化设置页`、`R10/R13 模型配置`、`style.css 第 16–21 节` 等同样丢失。实测 `node --test desktop/web/tests/*.test.mjs` → **373 测试 / 335 通过 / 38 失败**；其中 `r4-setup-gate.test.mjs` 在模块加载期即抛 `app.js 里找不到 async function needsSetup`，`product-eval.panel.test.mjs` 抛 `window is not defined`。**已验证非本次改动引入**：恢复被删的 `desktop/tauri` 后重跑仍为 373/335/38，且 ci-gate 第 5b 步在本次改动前就已跑这组测试 | ① 首次配置引导（provider 未配置分流）整体失效，新用户进不了引导页；② 38 条契约守卫长期红 → 门禁失去意义，真正的回归淹没在既有红灯里；③ 模型参数（上下文/输出/温度/超时）与面板样式节回退到旧版 | **不要回滚**（会丢掉上游观感）。做定向三方恢复：以 `git show a18ce87:agent-sdk/desktop/web/app.js` 为源，把 `needsSetup()`、`renderSetupGuide()`、路由化设置页分支、`PANEL_ORDER` 迁回 `app-domain.js`、style.css 缺失节逐段补回，每补一项跑对应测试转绿。建议单独立项（约 3-5 人日），不混入其它改动 |
| A1 | P2 | **双桌面壳平行实现，监管能力错位** | `desktop/tauri/src-tauri/` vs `desktop/electron/src/main/main.js` | tauri 监管栈 4664 行（core_runtime.rs 2058、provider.rs 833、single_instance.rs 477、core_supervisor.rs 324），最后提交 2026-09-23；electron 壳仅 3 处 spawn/supervisor/restart 相关调用，但 electron+web 是当前活跃主线（10-01 仍在提交） | 同一"核心进程监管"职责两套实现、两种成熟度：单实例、provider 热切换、深度重启策略只在 tauri 有；行为不一致与双倍维护成本 | 做出决断并写入 AGENTS.md：建议 electron+web 为主线，tauri 冻结归档；将 tauri 的 core_runtime/provider/single_instance 精华逻辑吸收进 server 或 electron 主进程后删除 |
| A2 | P2 | **三份 API 客户端实现并存** | `desktop/web/core/api-client.js`（大而全：token/重连/ledger）、`desktop/electron/src/renderer/lib/api.js`（129 行手写子集）、`clients/ts`（openapi 生成的类型面） | server 有 287 条路由，三份客户端各自手写覆盖子集，无共享 | 契约漂移风险：改一条路由要同步三处；electron 子集落后于 web 版（重连、ledger 标签等能力缺失） | electron renderer 复用 web 的 api-client.js（同源加载）；clients/ts 保持"类型生成面"单一职责；中期把 web api-client 也改为由 openapi 生成 |
| A3 | P3 | **游离的假验证脚本** | `agent-sdk/validation_report.rs`（workspace 根） | 47 行，只 `println!("✓ 加密/解密成功")`，**无任何真实断言**，模拟"测试通过"输出 | 伪造验证证据，误导后续维护者以为 v4 加密已验证 | 删除；真实验证已由存储加密契约测试承担（commit 1c08d35） |
| A4 | P3 | **空嵌套目录残留** | `agent-sdk/agent-sdk/Cargo.toml` | 空文件空目录，非 workspace 成员 | 目录噪音，疑似合并残留 | 删除 |
| A5 | P3 | **根目录散落报告与生成物** | `storage_crypto_refactor_summary.md`、`SECURITY_UPDATE_SUMMARY.md`、`demo-output.md`、`stream-demo.md` | 与 docs/（已有 reports/ 子目录）体系脱节 | 文档分散，新人难以建立全景 | 移入 `docs/reports/`；生成物（demo-output）删除 |
| A6 | P3 | **个人测试脚本混入产品目录** | `desktop/electron/` 下 `chat-test.ps1`、`shot.ps1`、`typing-test.ps1`、`九九乘法表.md` | 与 electron 交付物同目录 | 仓库卫生 | 移入 devtools/ 或删除 |
| A7 | P2 | **README 顶层基线叙事脱节** | `agent-sdk/README.md` | ① L18 写 `tests/route_contract.rs`，实际文件是 `route_contract_tests.rs`；② 基线停在 v0.6 / 2026-08-14，engine 合并的重大成果（Anthropic provider、多模态图片回合、hooks、reasoning 档位、MCP resources/prompts、fs 路由、桌宠、web 工作台对齐上游 be6298f）均未进入 README | 文档与代码漂移约一个半月，误导评估 | 修正文件名；补"engine 取优合并成果（2026-09-30）"小节 |

### B. CLI 端与 desktop 端进度差异

**两端规模**：CLI 9,424 行（16 命令）；web 工作台约 29,139 行（17 面板 + 5 视图 + 8 核心模块）；electron 壳 1,547 行；tauri 壳 4,664 行。

**功能覆盖矩阵（server 287 条路由为基准）**：

| 能力域 | CLI | Web 工作台 | 结论 |
|---|---|---|---|
| 会话/turn/diff/revert/fork/rewind/redo | ✅ REPL + TUI | ✅ | 两端齐全 |
| 审计（audit） | ✅ 专命令 | ✅ observability 面板 | 齐全 |
| 能力目录 capabilities | ✅ 命令 | ✅ 面板 | 齐全（§8.3 同源） |
| goal / fleet / team / workswarm / workflow | ❌ 无入口 | ✅ 各有面板 | **CLI 缺口** |
| notes / memory / automations | ❌ | ✅ | CLI 缺口 |
| doctor / backup / bench / worker 子进程 | ✅ | ❌（部分散在 setup-guide） | Web 缺口（可接受） |
| product-eval | ✅ 命令 | ✅ eval 面板 | 齐全 |
| 多模态图片回合 | ✅（合并 A1-2） | ✅ | 齐全 |

| # | 严重度 | 问题 | 位置 | 影响 | 修复建议 |
|---|---|---|---|---|---|
| B1 | P1 | **功能入口不对称成为常态**：goal/fleet/team/workswarm/workflow 等核心域只在 web 有 UI，CLI 无只读查询命令 | `crates/owo-agent-cli/src/main.rs`（命令枚举 L46-79 无对应项） | 脚本化运维与远程排障无入口；capabilities 目录（§8.3）声称"UI/CLI/诊断页共同来源"，实际两端覆盖不一致 | 以 capabilities 目录为对账单，给每个能力域补 CLI 最小面（list/status 两个只读命令即可） |
| B2 | P2 | **架构与代码风格两端不统一**：Rust 侧微内核 + workspace 依赖 + fmt/clippy 门禁 + ADR；前端侧原生 JS 无构建（web）、自制 mini-Vue（electron/reactive.js 216 行）、双壳（electron/tauri）三范式并存 | `desktop/electron/src/renderer/lib/reactive.js` | 前端无法享受 Rust 侧同等的质量门禁与工程纪律；自制框架是长期维护负债 | 前端定一个范式（建议：web 原生 JS 面板制 + electron 纯壳），删除 reactive.js 自制框架，electron renderer 改为加载 web 工作台（main.js:341 目前 loadFile 指向自带 renderer） |

### C. 其他问题（逻辑/架构/性能/测试/安全）

| # | 严重度 | 问题 | 位置 | 证据 | 影响 | 修复建议 |
|---|---|---|---|---|---|---|
| C1 | P1 | **上帝文件**：`owo-agent-core/src/tools.rs` 3,765 行 | 该文件 | engine 合并持续往里堆（multi_edit、git 工具、MCP resources 泛化工具…），已成全仓库最大源文件与改动热点 | 改一个工具牵动整个核心编译单元；审查困难 | 按工具域拆模块（file/git/mcp/desktop/browser/edit），tools.rs 只留注册表 |
| C2 | P1 | **受信执行内核零单元测试**：`owo-agent-tool-safety`（sandbox.rs 2,188 行 + audit_chain）src 内 `#[cfg(test)]` = 0 且无 tests/ 目录；`owo-agent-eval-facade` 同样为 0/无 | 测试密度统计 | 仅靠 server 侧 `v1_execution_safety_tests.rs` 集成兜底；sandbox 是安全边界，单元级回归网缺失 | 边界条件（路径穿越、参数变形）回归无保障 | 补 sandbox 策略判定与 audit_chain 校验的单元测试（纯函数居多，成本低收益高） |
| C3 | P2 | **6 处 `unbounded_channel`**：SSE 广播 ×3、worker_pool 任务队列、perception 事件流、goal 取消 | `server/src/event_stream.rs:720`、`server/src/sse.rs:181`、`server/src/workswarm_api/handlers.rs:409`、`core/src/worker_pool/pool.rs:108`、`perception/src/perception.rs:225`、`server/src/goal_api/handlers.rs:837` | 慢消费者 + 高频事件 → 发送端无背压、内存无界增长（turn SSE 有界队列已做，事件流这边没做） | 长时运行桌面场景内存缓慢膨胀 | SSE 广播改 bounded + 滞后者断连重连补拉（复用 turn SSE 已有模式）；worker_pool 队列加上限 + 拒绝策略 |
| C4 | P2 | **server 非测试代码 84 处 `unwrap()/expect`** | `server/src/**`（grep 统计） | handler 内 panic → 500 或连接重置；部分在启动路径可接受 | 生产稳定性 | 按路径分级：请求处理路径全部改 `?` + 统一错误 envelope（error_codes.rs 已有基础） |
| C5 | P2 | **287 路由的契约同步是纯纪律活**：39 个测试文件覆盖面广（前缀统计：session 32、teams 20、desktop-envs 16…），但 engine 合并新增路由（/activity、/fs/pick-directory、/fs/open、/approvals/pending、/automations/runs、/pet）是否全部有断言未逐一核对 | `server/tests/route_contract_tests.rs` | AGENTS.md 规定"新增路由必须同步契约测试"，无自动对账则靠人肉 | 做一个对账测试：遍历 `Router` 生成路由清单 vs openapi.json vs 测试断言集合，缺一即红 |
| C6 | P3 | electron CSP 允许 `script-src 'unsafe-inline'` | `desktop/electron/src/renderer/index.html` | XSS 面扩大（本地 UI，风险中等） | 收紧为 'self'（app.js 已是 module 化，改造量小） |
| C7 | P3 | electron 无打包/分发配置（package.json 仅 electron devDep，无 electron-builder/forge） | `desktop/electron/package.json` | 桌面分发路径未闭合（NSIS 打包脚本目前服务 C++ 输入法线） | 若 electron 定为主线，补打包配置并接 scripts/ 门禁 |
| C8 | P2 | **`owo-agent-server/src/lib.rs` 1,533 行集中挂载 60+ 模块** | lib.rs:409-760+ | 虽然业务已拆模块文件，但挂载、静态目录、中间件、错误映射集中一处；新增面板路由都要动这个文件（AGENTS.md 已标注它是"核心文件"，改动需 cargo check） | 挂载表按域拆分为 router 组合函数（每模块自带 pub fn router()） |

### 测试与文档覆盖总览

- **测试密度**（`#[cfg(test)]` 次数 / 有无 tests 目录）：core 34/Y、server 15/Y、perception 11/N、cli 10/Y、kernel 8/Y、policy 7/N、ime 4/Y、contracts 4/N、extensions 4/N、memory 4/N、workswarm 3/N、workflow 2/N、env 1/N、executor 1/N、plugins 1/N、protocol 1/N、build-info 1/N；**tool-safety 0/N、eval-facade 0/N、client 0/Y、owo-sim 0/N**。
- workspace 级 `agent-sdk/tests/` 仅含 `stt-corpus`（数据），无共享集成测试——可接受（集成都在 server/tests）。
- **文档**：docs/ 有 ARCH-MICROKERNEL.md、ROADMAP-PARITY.md、ADR×2、ci.md 等高质量文档；缺一份**前端架构文档**（三端关系、加载链路、面板规范目前只在代码注释里）。

---

## 4. 严重度汇总

| 级别 | 数量 | 问题编号 |
|---|---|---|
| P0（阻断） | 1 | **A0（合并 2d23cd2 覆盖本土功能，38 项 web 契约测试转红）** |
| P1（本迭代必修） | 3 | B1（CLI 功能入口不对称）、C1（tools.rs 上帝文件）、C2（tool-safety 零单元测试） |
| P2（尽快排入） | 8 | A1（双壳）、A2（三客户端）、A7（README 脱节）、B2（前端范式不统一）、C3（无界通道）、C4（unwrap）、C5（路由对账）、C8（lib.rs 挂载集中） |
| P3（清理） | 6 | A3、A4、A5、A6、C6、C7 |

> 说明：A0 是本轮审查**唯一**的 P0，也是审查重点第 1 条（合并引入的问题）的直接实证。审查初版把它漏掉了——原因是当时只统计了"24 个测试文件"的数量而**从未运行**它们。教训写入 §7：契约测试的数量不等于健康度，必须实际执行。

---

## 5. 分阶段整改方案

### Phase 0 — 清理与对齐（1-2 天，零风险）✅ 已完成（2026-10-02）
1. ✅ 删除 `agent-sdk/agent-sdk/` 空目录、`validation_report.rs` 假验证脚本（A3/A4）。
2. ✅ 根目录散落 md 移 `docs/reports/`；electron 个人脚本删除（A5/A6）。
3. ⚠️ README 只改了桌面壳相关章节；"engine 合并成果小节"与文件名修正仍未做（A7 未闭合）。
4. ✅ 桌面壳决断已落到 `docs/adr/ADR-003-desktop-shell-merge.md`：electron+web 主线，tauri 删除（A1 已闭合）。

### Phase 0.5 — 修复合并回归，把 web 门禁由红转绿（3-5 人日）【新增，建议最高优先级】
目标：`node --test desktop/web/tests/*.test.mjs` 达到 373/373。**先修这个再谈其它**，否则任何新回归都淹没在 38 项既有红灯里。

1. **取回被覆盖的函数**：`git show a18ce87:agent-sdk/desktop/web/app.js` 取出 `needsSetup()`、`renderSetupGuide()`，按 `r4-setup-gate.test.mjs:47` 的字面契约（必须形如 `async function needsSetup`）接回当前 app.js 的 boot 序列。
2. **修 `product-eval.panel.test.mjs` 的 `window is not defined`**：该测试用 CJS `require()` 加载浏览器面板脚本，缺少 `window` 宿主。改为在测试内自建最小 `globalThis.window` 垫片，或改用读取源码做静态断言（与仓库其它契约测试一致的做法）。
3. **补回路由化设置页与模型配置**（`ui-ia-fixes.test.mjs` 的 R10/R13、`§12-12`）：`data-rail-target="model"`、`settingsBaseUrl`/`settingsModelName` 可编辑输入、`reload_model_config`、模型参数随保存提交。
4. **补回 style.css 第 16–21 节**：第 16（inline 换行/中断徽标/重试按钮/窄栏/焦点环）、18（进度区/评审闭环/统计判读）、19（策略/指标/时间线/差异/交付物）、20（Launcher）、21（action-center 面板）。
5. **收敛 `PANEL_ORDER` 唯一来源到 `app-domain.js`**，app.js 不得再定义。
6. **收尾**：把 `ui-ia-fixes.test.mjs:265` 对 `../../tauri/src-tauri/src/provider.rs` 的引用改指核心配置 schema（tauri 壳已随 ADR-003 删除，该路径已不存在）。

### Phase 1 — 安全网（约 1 周）
1. `owo-agent-tool-safety` 补 sandbox 判定 + audit_chain 单元测试（C2，优先级最高）。
2. 三处 SSE unbounded → bounded + 慢消费者断连（C3），复用 turn SSE 已验证的有界队列模式。
3. server 请求路径 unwrap 清零（C4，84 处按路径分级处理）。
4. 契约对账测试：路由表 vs openapi.json vs 断言集合自动对账（C5），一次补齐 engine 新增路由的缺口。

### Phase 2 — 架构收敛（2-4 周）
1. `tools.rs` 按工具域拆分（C1）：file/git/mcp/desktop/browser/edit 子模块 + 注册表留守。
2. 前端统一（A2/B2）：electron renderer 改为加载 web 工作台，删除自制 reactive.js 与重复 api.js；web api-client 升级为唯一 JS 客户端。
3. server lib.rs 挂载拆分为每模块 `pub fn router()`（C8）。
4. 以 capabilities 目录为对账单，补齐 CLI 对 goal/fleet/team/workswarm/workflow/notes/memory 的只读命令（B1）。
5. tauri 处置执行：吸收 single_instance/provider 逻辑后归档（A1 的执行部分）。

### Phase 3 — 创新与扩张（1-2 月，见 §6）
每项独立立项，先 ADR 后编码，保持既有合并纪律（单块 + 单测 + 提交说明 + 同步报告）。

---

## 6. 创新点与发展方向

结合已有资产（感知/学习/执行闭环、桌宠、fleet、goal、workswarm、ime、product-eval、审计链），成熟 agent 的发展建议按"护城河深度"排序：

1. **输入法即 Agent 入口（最高差异化）**：`owo-agent-ime` 命名管道 IPC v3 已合并。把 agent 嵌入日常输入流——任意输入框唤起（类似现在的技能调用但零切换成本），是桌面 agent 独有的入口形态，Codex/Claude Code 均无此位。
2. **技能包生态**：`.owskill` 分享（含 schema/权限/敏感度校验）已具雏形 → 加签名与版本化，做本地优先的技能市场（plugin_market 已有本地离线模式），形成 UGC 飞轮。
3. **可回放行为账本**：R6 audit_chain（篡改可检出 + 离线导出校验）→ 产品化为"agent 行为账本"：每一步可解释、可审计、可回放，是进入企业/受监管场景的信任基建。
4. **混合算力调度**：fleet_api（节点注册/认领/进度/取消）+ 本地 ORT/Sherpa 推理 → "本地小模型干脏活（OCR/STT/视觉验证），云端大模型干推理"的调度器，成本与隐私双优。
5. **持续质量分**：devtools/product-eval（固定任务集 × 重复 × 对照）→ 接 CI 后做"质量分仪表盘"，每次合并自动出回归趋势，把现在人肉守门变成数据守门。
6. **主动式个人助理**：静默观察 + 情景记忆 + 阈值频控建议（M6 experience_store 已就位）→ 从"被问才答"到"该出手时出手"，配合桌宠做轻量通知面。
7. **多模态 computer-use**：图片回合全链路（A1-2）+ screen_vision/vision_verify + 动作图执行器已通 → 主攻"看得懂屏幕的自动化"，对标但差异化于 OpenAI Operator（本地感知、白名单、敏感面熔断）。
8. **收敛原则**：以上任何一项动工前，先完成 Phase 2 的"单壳单前端单客户端"收敛——护城河的宽度受限于地基层的整洁度。

---

## 7. 附：审查证据索引（关键命令与位置）

- 合并历史：`git log --oneline -25`（30+ 条"合并远端 engine"系列）+ 两次 merge commit（af413e6、5ba7a79）
- 冲突标记：rg `^<<<<<<<|^>>>>>>>`（crates/**/*.rs）→ 0 命中
- 依赖红线：各 crate Cargo.toml 逐项核对（mcp 的 core 为 dev-dep，见其 Cargo.toml L31-35 注释）
- 路由总数：`grep -r "\.route(" server/src` = 287
- 测试资产：`ls server/tests/` = 39 文件；`ls desktop/web/tests/` = 24 文件；ci-gate.ps1:308-314（web 测试门禁）
- 上帝文件 Top5：tools.rs 3765 / route_contract_tests.rs 3632 / desktop_env.rs 2442 / sandbox.rs 2188 / agent/mod.rs 1776
- unwrap：server src 非测试 84 处；unbounded_channel：6 处（位置见 C3）
- 双壳：tauri src-tauri 4,664 行（末次 2026-09-23）vs electron 1,547 行（活跃）→ 已按 ADR-003 合并为单壳
- 游离文件：validation_report.rs（47 行假断言）、agent-sdk/agent-sdk/（空）
- **A0 关键证据链（本次实测）**：
  - `node --test "desktop/web/tests/*.test.mjs"` → `# tests 373 / # pass 335 / # fail 38`
  - 整文件崩溃 2 个：`product-eval.panel.test.mjs`（`ReferenceError: window is not defined`）、`r4-setup-gate.test.mjs`（模块加载期 `AssertionError: app.js 里找不到 async function needsSetup`）
  - `git show 2d23cd2 --stat -- agent-sdk/desktop/web/{app.js,index.html,style.css}` → app.js 6,909 / index.html 1,282 / style.css 3,503 行改动
  - `git show a18ce87:agent-sdk/desktop/web/app.js | grep -c "needsSetup\|renderSetupGuide"` → **7**；当前工作区 → **0**
  - **排除自身干扰**：临时 `git checkout HEAD -- desktop/tauri` 恢复已删目录后重跑，仍为 373/335/38；且 ci-gate 5b 步在本轮改动前就已执行该测试组
- **教训（写给自己）**：审查初版统计了"24 个契约测试文件"却未执行，从而给出"P0=0"的错误结论。**契约测试的数量 ≠ 健康度，必须实际运行**；后续审查把"跑一遍现有门禁"作为第一步。
