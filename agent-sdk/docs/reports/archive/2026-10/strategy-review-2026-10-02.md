# OwO Agent SDK 战略审查报告：现状、Codex 差距与能力路线图

> 审查日期：2026-10-02
> 审查范围：`agent-sdk/`（22 crate + 独立 workspace 的 product-eval；392 个 .rs / 179,376 行 Rust + 67 个 JS / 29,139 行 Web）
> 方法：三路并行代码实证调查（工具能力 / 多代理与验证 / 界面与产物）+ Codex 官方文档与 2026 公开报道联网核对
> 证据纪律：所有现状判断均标注代码路径与行号；Codex 侧为公开文档/官方博客/第三方评测，标注来源性质
> 参照：本文接续 `code-review-2026-10-02.md`（工程规范审查），本文只做**能力与战略**审查

---

## 0. 执行摘要（一页读懂）

### 0.1 一句话结论

**OwO 的"骨架密度"已超过 Codex，"肉"却缺在三个地方：产物能力（画图/表格/PPT 全空）、无人值守长程任务（自动化只是提醒器）、界面收敛（5.4 万行死代码 + 38 项契约测试红）。**

这不是"功能少"，而是"已建成的部分没接线、没验证"——这是一个**完工度问题，不是架构问题**，也因此修复成本远低于新建。

### 0.2 五项关键判断

| # | 判断 | 依据强度 |
|---|---|---|
| 1 | **多代理能力已达 Codex 同一量级**，甚至更深（epoch 代次、critic 复核、wilson_ci95 统计显著性门） | 强（源码级） |
| 2 | **产物能力是最大空洞**：画图 0 实现、PPT 0 实现、表格/文档/PDF 仅"提示词空壳" | 强（源码级） |
| 3 | **"彻夜长程任务"差最后一公里**：`AutomationAction` 只有 `Reminder` 变体，不能驱动 Agent 回合；而重试退避的成熟实现就在同一 crate 的 `cloud_exec.rs` 里，没打通 | 强（源码级） |
| 4 | **界面是"未接线"而非"未实现"**：20 个文件 5.4 万行从未被 index.html 引入，导致 22 项测试失败 | 强（源码级） |
| 5 | **安全内核 2,754 行零测试**（`owo-agent-tool-safety`），是全仓唯一 0 测试的实质模块 | 强（源码级） |

### 0.3 与 Codex 的总体差距画像

```
              骨架（架构完备度）        肉（能力兑现度）
Codex         ████████████ 95%          ████████░░ 80%
OwO           ███████████░ 92%          ████░░░░░░ 38%
```

OwO 在**内核设计上**（沙箱三档降级、审计链、场景图融合、WorkSwarm 统计门）已经达到甚至超过 Codex 水准，
但在**用户可感知的能力**（产物生成、长程自动化、界面收敛）上大幅落后。

**根因判断**：OwO 走的是"先建内核、后接血肉"的路线，且**没有收尾阶段**。
19.9 万行代码，测试 1,749 个（密度不低），但**死代码与未验证模块占相当比例**——
说明工程投入放在了"新建"而非"接通与验证"。

---

## 1. 能力现状总表

### 1.1 工具能力（45 个内置工具）

| 域 | 状态 | 关键事实 |
|---|---|---|
| 文件读写/搜索 | ✅ 完整 | read/write/edit/multi_edit/apply_patch/list/search/grep 共 9 个 |
| Shell 执行 | ✅ 完整 | 沙箱 + 审批 + 60s 超时 + CPU/内存/进程数三重限额 |
| Git | ⚠️ **只读** | 仅 status/diff/log 三个；**提交/暂存/推送无专用工具**，须回落 `run_command` |
| 网页抓取 | ✅ 完整 | web_fetch（HTML→纯文本）+ web_search（DuckDuckGo，可换源） |
| 浏览器自动化 | ✅ 完整 | 9 个工具（navigate/search/snapshot/click/type/press/screenshot/download/close），**依赖 Node + Playwright + 系统 Edge**，非原生 CDP |
| OCR/视觉 | ✅ **强于预期** | 双 OCR 引擎（Windows Media.Ocr + ONNX/PaddleOCR）；`vision_verify`/`vision_ground` 视觉断言与定位；OCR×视觉交叉验证（冲突打 0.6 折） |
| 桌面控制 | ✅ 完整但**默认关闭** | 9 个工具（click/type/key/shortcut/launch/scroll/wait/wait_until/activate）+ 免 IME UTF-16 注入；**仅 Windows** |
| 子代理 | ✅ **强于 Codex** | explore / subagent（**返回后自动 critic 复核，未过按意见返工一次**）/ fan_out_subagents（JoinSet 真并行 + 6 态终态 + 失败隔离 + 预算硬停） |
| MCP | ✅ **完整** | stdio+HTTP 双传输、命名空间隔离、schema 预算压缩、per-server 熔断、**热注册热卸载** |
| **图像生成** | ❌ **0 实现** | 全仓 0 命中（`sim_browser.rs:205` 的 `generate_images` 是测试桩，造纯色块） |
| **表格 xlsx** | ❌ **空壳** | 技能包仅 20 行提示词，SKILL.md 未指定任何脚本/库 |
| **文档 docx** | ❌ **空壳** | 同上；`tests/run_tests.py` 测的是第三方 `python-docx` 本身 |
| **PPT** | ❌ **完全缺失** | 连技能包目录都不存在 |
| **PDF** | ❌ **空壳** | 20 行提示词，未指定工具 |
| TTS | ❌ 无 | 只有 STT（Sherpa-ONNX SenseVoice） |

> ⚠️ **必须正视的交付风险**：documents/pdf/spreadsheets 三个技能包 manifest 声明的 tools 是
> `["read_file","write_file","run_command"]`，实际全靠 `run_command` 调**宿主机 python 库**。
> 若用户机器没装 python-docx/openpyxl/reportlab，**能力链在用户回合中途断裂**，
> 而失败点不可控、不具备产品级可交付性。

### 1.2 桌面操控（仅 Windows）

| 模块 | 行数 | 能力 |
|---|---|---|
| perception | 6,011 | UIA 无障碍树（前台+后台双路）、OCR 双引擎、场景图融合（跨帧统一事实源）、语义锚点定位、元素注册表（稳定 ID）、窗口模板匹配、本地 STT |
| executor | 1,392 | 动作图执行引擎（含**敏感词熔断**：password/支付/密码/验证码 → keyword_breaks）、免 IME UTF-16 注入、组合键、坐标点击、滚轮 |

**平台限制（硬伤）**：`executor`/`perception`/`platform` 全部依赖 `cfg(windows)` + Win32 API，
非 Windows 平台所有函数是**返回空值的 stub**（`platform.rs` 24 处）。
`run_command` 还硬编码 `cmd /C`，跨平台场景直接阻塞。
→ 唯一跨平台的是浏览器自动化，但被 `cmd /C` 拖累。

### 1.3 自动化（★ 最大错位点）

| 维度 | 现状 | 证据 |
|---|---|---|
| 定时触发 | ✅ 3 种 schedule（OneShot/Interval/Daily）+ 每秒调度循环 | `automation.rs:13-17`, `lib.rs:1134` |
| 执行记录 | ✅ `AutomationRun` 500 条上限 + 审计 + 结构化日志 | `automation.rs:25-35` |
| 增删改查/启停 | ✅ REST 6 路由 | `lib.rs:566-585` |
| **动作类型** | ❌ **只有 `Reminder{text}` 一个变体** | `automation.rs:19-23` |
| 驱动 Agent 回合 | ❌ 不调用 Agent | `automation.rs` 全文无 Agent 调用 |
| **失败重试** | ❌ 仅 `record_run("failed")`，**无重试/退避/max_retries** | `lib.rs:1163-1171` |
| 循环迭代 | ❌ 无"多轮迭代"概念 | — |
| 结果汇总 | ⚠️ 有逐次记录，无聚合报告 | `automation.rs:259-266` |

**讽刺点**：`automation.rs` 所在的同一个 crate（`owo-agent-extensions`）里，
`cloud_exec.rs` **有完整的长程任务编排**——`CloudScheduler` 状态机、指数退避 `backoff_delay(base, retry_count)`
（封顶 60s，`:850-851`）、`max_retries` 默认 2（`:894-907`）、`Retrying{retry_count}` 进度事件、断线重连 `POLL_RETRY_MAX=4`。
**能力齐备，只是与本地定时触发器没有打通。**

### 1.4 界面（20 个文件 5.4 万行死代码）

`index.html` 只加载 22 个脚本。**完全未被加载**的目录：

| 目录 | 文件数 | 缺失的功能 |
|---|---|---|
| `core/` | 8 | 统一 API 客户端、事件流、文件夹选择器、路由、恢复控制器、SSE 回放 |
| `views/` | 5 | **状态条、诊断台账、设置面板、首次配置引导页** |
| `permissions/` | 4 | **权限中心**整套（api/controller/domain/view） |
| `shell/` | 3 | 连接管理、Markdown 渲染、语音 |

**并且 `PANEL_ORDER` 双定义且内容不一致**：
- `app.js:4898`（16 项，含 automations）← **运行时生效**
- `app-domain.js:1431`（17 项，不含 automations）← 权威源但未加载

`needsSetup()` / `renderSetupGuide()` 在全部 22 个已加载脚本中**均无定义**，而 6 个测试文件断言它们必须存在。

### 1.5 验证状态

| 指标 | 数值 |
|---|---|
| 主 workspace 测试 | **1,682**（`#[test]` 953 + `#[tokio::test]` 729） |
| product-eval（独立 workspace） | 67（**零 CI 覆盖**） |
| 全仓合计 | **1,749** |
| 契约测试 | `route_contract_tests.rs` 2,000+ 行，三重校验（路由可达/openapi 双一致/Router 构建）+ 6 条历史回归清单 |
| web 契约 | 24 个测试文件 → **373/335/38**（38 红） |

**零测试 / 未进 CI 的高风险模块**：

| 模块 | 规模 | 风险 | 说明 |
|---|---|---|---|
| **`owo-agent-tool-safety`** | 2,754 行 | **最高** | 沙箱 AppContainer/LowIL/Job Object 三级降级 + audit_chain。**全仓唯一 0 测试的实质模块**，Windows 专属代码在 Linux CI 上根本不编译 |
| `cloud_exec.rs` | 1,316 行 | 高 | 长程云执行编排，0 内联测试 |
| `owo-sim` | 1,598 行 | 中 | sim_qq 1,327 行，0 测试，且无任何 crate 依赖它 |
| `fleet_transport.rs` | 887 行 | 中高 | 跨机传输（Lease/fencing/心跳），内联 0 测试 |
| **product-eval live E2E** | 137 行 | 中高 | **唯一验证多代理真实收益的测试，`#[ignore]` 且不在 CI** |
| `workswarm/coord_*.rs` | 3 大件 2,300 行 | 中 | 17 文件中仅 `tests.rs` 有内联测试，其余靠 26 个集成测试间接覆盖 |
| CI 三步骤 | — | 中 | `root-manifest` / `permission-ctor` / `forbidden-files` **永不执行**（`-Step` 子串匹配无人命中），密钥文件防护形同虚设 |
| Linux 门禁 | — | 中 | `continue-on-error: true`，编译失败长期无人拦 |
| tokio worker_threads | — | 中 | `new_multi_thread()` 未设上限，运行时并发不受 CI 的 `-j 1` 约束 |
| 6 处 `unbounded_channel` | — | 中 | `worker_pool/pool.rs:108`（调度）、`perception.rs:225`（订阅者）为真无界，无背压 |

**必须肯定的部分**（避免只列问题）：
- 并行编排测试是真并行断言，不是 mock 计数：`subagent.rs:299-307` 断言 3×300ms < 750ms 且峰值并发 ≥2；`:325-330` 断言 `max_parallel=1` 时峰值恰为 1
- 资源红线（§2.4）是**真强制**：内存门 + `-j 1` + 退出码 137，且 `resource-selftest` 把"把限制并发的代码写错了"本身变成回归测试
- SQLite 事件回放有**显式 opt-in**（`store.rs:36-39` 默认返回错误而非假装支持）——接口设计诚实

---

## 2. 与 Codex 的功能差距

### 2.1 多代理：OwO 已达同一量级，甚至更深

| 维度 | Codex | OwO | 判定 |
|---|---|---|---|
| 子代理派生 | ✅ `~/.codex/agents/*.toml` 自定义 + 内置 default/worker/explorer | ✅ `explore`/`subagent`/`fan_out_subagents` + WorkSwarm 组队（≤5） | **平** |
| 嵌套深度控制 | ✅ `max_depth`（默认 1） | ✅ `MAX_SUBAGENT_DEPTH=2` | 平 |
| 并发上限 | ✅ `max_threads`（默认 6） | ✅ `max_parallel`（默认 4，clamp 1-4） | 平 |
| 独立上下文 | ✅ | ✅ 每子代理独立 `Session::new` | 平 |
| 失败隔离 | 部分 | ✅ 单 worker 失败不影响其余，6 态终态枚举 | **OwO 更强** |
| **结果复核** | 自动 reviewer agent（gate 风险审批） | ✅ **subagent 返回后自动 critic 复核 + 有界返工** | **OwO 更深** |
| **协作拓扑** | 独立 task，无 inter-agent 通信 | ✅ 阶段接力 + epoch 代次 + steer 五态（continue/steer/replace/cancel/retry） | **OwO 更强** |
| **统计显著性门** | 无 | ✅ `wilson_ci95` + 配对对照 + `gate_auto` 冻结门槛 | **OwO 独有** |
| **worktree 隔离** | ✅ | ❌ 全仓 `grep worktree` 0 命中 | **Codex 领先** |
| 跨机多 agent | cloud sandbox | ⚠️ `fleet_transport` 有 Lease/fencing 但 0 内联测试 | 未验证 |

> 结论：**OwO 的多代理编排设计深度已超过 Codex**（critic 复核、epoch 代次、wilson 统计门都是 Codex 没有的），
> 缺的是 worktree 隔离（界面维度）与真实收益的评测验证。

### 2.2 沙箱与安全

| 维度 | Codex | OwO | 判定 |
|---|---|---|---|
| 沙箱形态 | 内核级 Seatbelt(Linux)/Landlock(macOS)/Windows sandbox | AppContainer / LowIL / Job Object **三级降级** | 平（降级策略更细） |
| 网络隔离 | 默认可关 | `run_command` 有 `OWO_CLOUD_ENABLED` 开关 | 平 |
| 权限 | 沙箱(read-only/workspace-write/danger-full-access) × 审批(on-request/never/untrusted) **两个独立维度** | 4 类副作用（Read/Write/Execute/Inject）+ deny-by-default | 平 |
| MCP 副作用 | readOnlyHint | ✅ 需"server+tool+schema hash"宿主可信声明，否则降级为 deny | **OwO 更严** |
| **沙箱测试覆盖** | 有 | ❌ **2,754 行 0 测试** | **OwO 严重落后** |

> ⚠️ 这是最需要立刻补的：**安全内核的代码质量可能没问题，但"零测试"意味着任何一次重构都可能静默削弱隔离**。

### 2.3 长程任务与"无人值守"：OwO 明显落后

| 维度 | Codex | OwO | 判定 |
|---|---|---|---|
| 定时唤醒 | ✅ automations 可为**自己排程**，跨天/跨周自动继续 | ⚠️ 只有提醒器 | **Codex 大幅领先** |
| 复用已有会话线程 | ✅ automations 可 re-use conversation threads | ❌ | Codex 领先 |
| 失败重试/退避 | ✅ | ❌（但 `cloud_exec` 里有现成实现） | **OwO 可低成本补齐** |
| 目标驱动持续执行 | ✅ `/goal` 持久执行循环 | ⚠️ `core/src/goal/` 1,648 行存在但非自动化驱动 | 部分 |
| 主动建议下一步 | ✅ 结合项目+插件+memory 主动提议 | ⚠️ `proactive` 模块存在（weekly/daily threshold、cooldown、daily_cap） | 部分 |
| 记忆（跨会话经验） | ✅ memory preview（偏好/纠正/耗时经验） | ✅ `owo-agent-memory` 2,822 行 + `/learn/*` + experience_store | **OwO 更强** |

> **OwO 的记忆内核比 Codex 更完整**（experience_store + learn/record + skill_health），
> 缺的是"记忆 → 主动驱动长程任务"这最后一跳的接线。

### 2.4 产物与电脑操控

| 维度 | Codex | OwO | 判定 |
|---|---|---|---|
| 电脑操控 | ✅ background computer use（own cursor，多 agent 并行不干扰，**macOS 先行**） | ✅ Windows UIA+OCR+SendInput 9 工具 | **平**（平台不同：Codex macOS 先行，OwO Windows 独占） |
| 图像生成 | ✅ gpt-image-1.5 | ❌ 0 实现 | **Codex 领先** |
| in-app browser + 页面评论 | ✅ | ⚠️ 有 browser 技能（纯提示词） | Codex 领先 |
| Office 产物 | ✅ 侧栏富预览 PDF/spreadsheet/slides/docs | ❌ 四个全空壳 | **Codex 大幅领先** |
| 文件富预览 | ✅ | ❌ `index.html` 0 命中 | Codex 领先 |

### 2.5 生态差距（结构性，最大劣势）

| 维度 | Codex/Claude | OwO | 判定 |
|---|---|---|---|
| **插件生态** | Codex 90+ 官方插件（Atlassian Rovo/CircleCI/GitLab/Neon/Remotion…）；Claude 3,000+ MCP 集成 + plugin marketplace | ⚠️ 有 MCP 宿主 + `.owskill` 打包，但**无市场/分发/安装器** | **Codex 数量级领先** |
| **社区规模** | Codex 90k+ stars；Claude 161k+ | 无（内网项目） | 不可比 |
| **模型兼容** | Codex = OpenAI only；Claude = Anthropic only | ✅ **7 家 provider**（bigmodel/openai/deepseek/ollama/moonshot/qwen/zhipu 等）+ 自定义端点 | **OwO 领先**（BYOK 多模型是 Codex/Claude 都没有的） |
| **MCP 标准** | ✅ | ✅ 完整（stdio+HTTP） | 平 |
| **AGENTS.md** | ✅ 事实标准（Linux 基金会托管），Codex 首倡 | ✅ `AGENTS.md` 顶层生效 | **平（OwO 已对齐）** |
| **钩子/生命周期** | Claude 17+ hook 事件；Codex 较新 | ⚠️ 无 hook 系统（仅权限策略 + 审计） | **落后** |
| **IDE/移动端** | Codex 6 个表面（CLI/IDE/Cloud/ChatGPT app/mobile/Chrome ext） | ⚠️ 仅 Electron 桌面壳 + web | **Codex 领先** |
| 远程/SSH devbox | ✅ app-server + SSH | ❌ | Codex 领先 |
| **协议开放性** | Codex CLI 开源 Apache 2.0 | ⚠️ **GPL-3.0-only** | ⚠️ 商业闭源集成有阻力（第三方向要遵守 copyleft） |

> ⚠️ **GPL-3.0-only 值得决策层注意**：企业要把它嵌进闭源产品会受 copyleft 传染，
> 这会直接影响生态扩展路径（Codex Apache 2.0 无此约束）。

---

## 3. 界面差距

### 3.1 现状优点（必须承认）

| 能力 | 状态 |
|---|---|
| 面板生态 | ✅ **17 个 OwoPanels**（capabilities/action-center/notes/automations/plugin-market/workflow/goal/team/eval/observability/memory/command/fleet/workswarm/project-launcher/project-history/about）—— **比 Codex app 的面板更宽** |
| 感知/学习区 | ✅ 情景感知 + 主动建议 + 情景记忆 + 审计日志 + 12 处 tool-card 学习闭环 —— **OwO 独有** |
| 深色模式 | ✅ 手动 + `prefers-color-scheme` 跟随系统 |
| 响应式 | ✅ 6 个断点（1000/900/1120/820/1180/860px） |
| 减弱动效 | ✅ `prefers-reduced-motion` |
| a11y | ⚠️ 51 处 aria/18 处 role，**但流式消息区无 `aria-live`，无 focus trap** |

### 3.2 与 Codex app 的差距

| Codex app | OwO | 差距性质 |
|---|---|---|
| 并行 thread 可视化（多栏/切换 Tab） | ⚠️ 状态层支持多会话（`activeTurns`/`pendingApprovals` Map），**但用户无法同时看见两个会话** | **需新建视图层** |
| worktree 切换 | ❌ | 需新增 |
| 多终端 tab | ❌（设置页有"集成终端"文案但无终端 UI） | 需新建 |
| 文件富预览（PDF/表格/PPT/文档） | ❌ | 需新建（依赖产物能力） |
| summary pane（计划/来源/产物汇总） | ❌ | 需新建 |
| in-app browser + 页面评论 | ❌ | 需新建 |
| PR review 评论流 | ❌ 0 命中 | 需新建 |
| 移动端 | ❌ 最小断点 820px，三栏固定 | 需新建 |

### 3.3 38 项失败测试的归因（界面的真实病因）

| 归因 | 失败数 | 含义 |
|---|---|---|
| **① 死代码未接线** | **22** | 首屏引导、状态条、权限中心、诊断台账、API 客户端、SSE 回放、文件夹选择器 **7 块功能代码写好了但从未 `<script>` 引入** |
| ② PANEL_ORDER/style.css 分节未收尾 | 7 | style.css 缺第 16-21 节（inline 换行/中断徽标/表格包裹/Launcher 布局/ph-*/评审闭环） |
| ③ 断言与实现漂移 | 7 | 审批条缺 `data-scope="once"`；3 处 UI 文案泄漏内部字段名；两处 `needsSetup` |
| 整文件加载失败 | 2 | `product-eval.panel.test.mjs`、`r4-setup-gate.test.mjs`（0 通过/1 失败） |

> **关键洞察**：修一行 `<script>` 标签（把 core/ views/ permissions/ 引入 index.html）
> 可能一次性解决 **20+ 个失败**，这是全项目**投入产出比最高的单点修复**。

---

## 4. 未验证功能清单（按风险排序）

| 风险 | 模块/功能 | 规模 | 缺什么 |
|---|---|---|---|
| 🔴 **最高** | 沙箱 OS 隔离（`owo-agent-tool-safety`） | 2,754 行 | 0 测试；Windows 代码在 Linux CI 不编译 |
| 🔴 高 | `cloud_exec.rs` 长程云执行 | 1,316 行 | 0 内联测试 |
| 🟠 中高 | `fleet_transport.rs` 跨机传输 | 887 行 | 0 内联测试 |
| 🟠 中高 | **product-eval live E2E**（single vs WorkSwarm） | 137 行 | `#[ignore]` + 不在 CI + 文档命令包名写错 |
| 🟠 中高 | **product-eval 整个评测面** | 59 测试 | **零 CI 覆盖** |
| 🟠 中 | tokio worker_threads 无上限 | — | 运行时并发不受 CI 约束 |
| 🟠 中 | 跨 team 全局并发上限缺失 | — | 多 team 无背压 |
| 🟠 中 | 6 处 unbounded_channel | — | `pool.rs:108`/`perception.rs:225` 真无界 |
| 🟠 中 | CI 三步骤永不执行 | — | `permission-ctor`/`forbidden-files` 密钥防护失效 |
| 🟠 中 | Linux 门禁不阻断 | — | `continue-on-error: true` |
| 🟡 中 | 产物能力（xlsx/docx/pdf/PPT） | — | **写了提示词但从未真实验证过产出质量** |
| 🟡 中 | Windows 桌面操控 E2E | — | sim 面存在，真实桌面未系统验证 |
| 🟡 中低 | `owo-sim` 1,598 行 | — | 0 测试且无人依赖 |
| 🟡 中低 | README「尚未实现」章节 | — | 与实际 SQLite 已实现矛盾 |

---

## 5. 建议补充的功能（按投入产出比排序）

### P0 — 先把已有的接通与验证（低成本、高回报）

| # | 事项 | 工作量 | 预期收益 |
|---|---|---|---|
| **1** | **接死代码**：`index.html` 引入 `core/`+`views/`+`permissions/`+`shell/`，收敛 `PANEL_ORDER` 唯一源到 `app-domain.js` | 0.5-1 人日 | 一次性修掉 20+ 项契约测试失败；状态条/权限中心/引导页/诊断台账**白得** |
| **2** | **补 style.css 第 16-21 节** | 0.5 人日 | 修掉 7 项样式守卫失败 |
| **3** | **沙箱内核补测试**（Windows Job Object/AppContainer 真实 spawn + audit_chain 校验） | 2-3 人日 | 消除**最高风险**的零覆盖 |
| **4** | **product-eval 进 nightly**（至少跑 59 个 mock 测试）+ 修正 live 测试文档命令 | 0.5 人日 | 恢复评测面的回归价值 |
| **5** | **CI 三步骤接入 PR**（`permission-ctor`/`forbidden-files`/`root-manifest`） | 0.5 人日 | 密钥/`.db`/`.log` 误入防护生效 |
| **6** | **Linux 门禁改阻断**（或至少标注为 allow-failure 并建 issue 跟踪） | 0.5 人日 | 长期编译健康 |

### P1 — 补齐"最后一公里"（中等成本、你点名的高价值）

| # | 事项 | 工作量 | 说明 |
|---|---|---|---|
| **7** | **通电"无人值守长程任务"** ★最高价值 | 3-5 人日 | `AutomationAction` 增加 `RunAgent{goal, max_iters}` 变体；`start_automation_loop`（`lib.rs:1145-1173`）调 Agent 回合；**直接复用 `cloud_exec.rs:850` 的 `backoff_delay` + `max_retries` + `Retrying{}` 进度事件** |
| **8** | **产物能力原生化** | 5-8 人日 | 见 §6 专题 |
| **9** | **worktree 隔离** | 3-4 人日 | 多 agent 并行改同一仓库不冲突（Codex 已有） |
| **10** | **并行 thread 可视化** | 2-3 人日 | 多栏并排/切换 Tab（状态层已支持，只差视图） |
| **11** | **git 写操作工具化**（commit/stage/push/checkout） | 1-2 人日 | 避免每次回落 `run_command` |
| **12** | **runtime 收敛**：`worker_threads` 上限 + 跨 team 全局信号量 + `unbounded_channel` 有界化 | 1-2 人日 | 资源可预期 |
| **13** | **文件富预览**（PDF/表格/PPT/文档） | 2-3 人日 | 依赖 #8 |

### P2 — 生态与规模化（长期）

| # | 事项 | 工作量 | 说明 |
|---|---|---|---|
| 14 | **插件市场**：技能/MCP 一键安装（对标 Codex 90+ 插件） | 10-15 人日 | 最大生态短板 |
| 15 | **钩子系统**（17+ 生命周期事件，对标 Claude Code） | 5-8 人日 | 可编程的确定性护栏 |
| 16 | **多终端 tab** | 2-3 人日 | Codex app 已有 |
| 17 | **in-app browser + 页面评论** | 3-4 人日 | 前端迭代体验 |
| 18 | **PR review 评论流 + GitHub Action** | 3-4 人日 | 开发流程闭环 |
| 19 | **macOS/Linux 平台支持** | 15+ 人日 | 解除单平台锁定 |
| 20 | **许可证决策**（GPL-3.0-only vs Apache/MIT） | 决策 | 影响生态扩展路径 |
| 21 | **移动端** | 10+ 人日 | 对标 Codex mobile |
| 22 | 远程 SSH devbox / app-server | 5-8 人日 | Codex 已有 |

---

## 6. 专题：能力兑现路线（你最关心的四项）

### 6.1 画图（图像生成）

**现状**：**0 实现**。全仓 `image_gen`/`generate_image`/`dall`/`stable_diffusion` 0 命中。

**建议路径**（三档，按投入递增）：

| 档 | 方案 | 工作量 | 说明 |
|---|---|---|---|
| A | **接云端 API 工具** | 1-2 人日 | 新增 `image_generate` 工具，走 `/v1/images/generations`，复用现有 provider 解析（`canonicalProvider`/`effectiveBaseUrl`）。缺点：需 BYOK 图生图 key |
| B | **本地 ONNX 扩散模型** | 5-8 人日 | 项目已有完整 ONNX 运行时（ORT 2.0-rc.13）+ sherpa-onnx 打包链路，模型分发是现成的（`download-onnx-ocr-models.ps1` 可仿照） |
| C | A+B 混合 | 6-10 人日 | 本地优先、云端兜底，**建议 B 先做**——项目已有 ONNX 分发基础设施，边际成本低 |

### 6.2 表格 / 文档 / PPT

**现状**：表格/文档/PDF 是**提示词空壳**（各 20 行 SKILL.md，未指定任何脚本/库），**PPT 连目录都没有**。

**关键设计决策**：现有 `zip` crate 已在用（`.owskill` 打包 `share_skill.rs:25`、备份 `backup.rs:155`），
但**从未构造过 OOXML**（无 `[Content_Types].xml` / `xl/workbook.xml` / `word/document.xml`）。

| 方案 | 说明 | 工作量 | 风险 |
|---|---|---|---|
| **A. 内置确定性脚本 + manifest 声明依赖** | 把 python 脚本写进技能包目录，SKILL.md **明确指定命令**（不再让模型猜），`manifest.json` 的 `tools` 补 `run_command` 权限与依赖声明；`/skill/verify` 增加"依赖存在性"检查 | **1-2 人日** | 低。**立即消除"交付断裂"风险** |
| **B. Rust 原生 OOXML 写入** | 用 `zip` crate + 手写 XML 写 xlsx/docx（模板固定场景够用）；`docx-rs`/`rust_xlsxwriter` 可评估 | 5-8 人日 | 中。样式/公式支持有限 |
| **C. PPT 全新** | pptx 的 OOXML 复杂度最高（slide/layout/master/主题） | 3-5 人日（单独） | 中高。**建议先做"Markdown/HTML → pptx"** 窄场景 |

> **强烈建议：先做 A（2 人日内可交付）**。它把"能力取决于用户机器有没有装库"
> 这个**不可控的产品级风险**降级为"可控的依赖检查"。

### 6.3 操控电脑

**现状**：**Windows 上能力已相当完整**（UIA 双路 + OCR 双引擎 + 场景图融合 + 语义锚点 + 9 个控制工具 + 免 IME 注入 + 敏感词熔断）。
**真正的差距是三个**：

| 差距 | 现状 | 建议 |
|---|---|---|
| **单平台锁定** | macOS/Linux 全是 stub | 中期（15+ 人日），短期可用"仅 Windows"作明确产品边界 |
| **默认关闭** | `desktop_control: bool = false`（`settings.rs:135`） | 做**首次配置引导页**（正好是死代码里现成的 `setup-guide.view.js`）显式征询授权，**把关闭变成"已知且同意"** |
| **无真实 E2E 验证** | 只有 sim 面，真实桌面未系统验证 | 补 Windows E2E（可用现有 `computer-use-e2e.py`） |

### 6.4 Agent 驱动 Agent（multi-agent）★

**现状**：**已实现且比 Codex 更深**（见 §2.1）。你想要的这个能力，项目里已经有了。

**真正缺的不是"能不能"，而是"验证与收敛"**：

| 缺什么 | 建议 |
|---|---|
| **worktree 隔离** | 3-4 人日。多 agent 并行改同一仓库的**唯一真实障碍**（Codex 已有） |
| **live E2E 收益验证** | 唯一验证 single vs WorkSwarm 三分类对照的测试是 `#[ignore]` 且不在 CI。**没有这个数据，worktree 做完也不敢上** |
| **跨 team 全局并发上限** | 多 team 场景无背压 |
| **界面并行 thread 视图** | 状态层已支持，差视图层（2-3 人日） |

---

## 7. 路线图建议（按阶段）

```
阶段 0：接通与止血（1-2 周，低风险高回报）
  ├─ 接死代码（index.html 补 script 标签）        → 修 20+ 测试失败
  ├─ 补 style.css 16-21 节                        → 修 7 项
  ├─ CI 三步骤接入 PR                              → 密钥防护生效
  └─ product-eval 进 nightly                       → 评测面有回归

阶段 1：验证补齐（2-3 周，消除风险）
  ├─ 沙箱内核补测试（2,754 行）                     → 消除最高风险
  ├─ cloud_exec 补测试（1,316 行）
  ├─ linux 门禁改阻断
  └─ runtime 收敛（worker_threads/信号量/无界队列）

阶段 2：长程与产物（3-5 周，你的核心诉求）
  ├─ 通电无人值守长程任务（复用 cloud_exec 退避）    ★ 最高价值
  ├─ 产物能力 A 方案（脚本+依赖校验）               → 消除交付断裂
  ├─ git 写操作工具化
  └─ 文件富预览

阶段 3：并行规模化（4-6 周）
  ├─ worktree 隔离
  ├─ 并行 thread 可视化
  ├─ live E2E 收益验证 → 决定 WorkSwarm 投产
  └─ 多终端 tab / in-app browser

阶段 4：生态（长期）
  ├─ 插件市场
  ├─ 钩子系统
  ├─ 许可证决策
  └─ 平台扩展（macOS/Linux）
```

---

## 8. 关键风险与决策建议

| 风险 | 影响 | 建议 |
|---|---|---|
| 🔴 沙箱零测试 | 一次重构可能静默削弱隔离，且**无法被 CI 发现** | 阶段 1 立刻补 |
| 🔴 产物能力交付断裂 | 用户回合中途失败，不可控 | 阶段 2 方案 A（2 人日） |
| 🟠 死代码规模 5.4 万行 | 认知负担、测试全红、误导新人 | 阶段 0 一次性接线 |
| 🟠 product-eval 零 CI | 无法证明多代理有收益，WorkSwarm 投产无依据 | 阶段 0+1 |
| 🟠 GPL-3.0-only | 企业闭源集成受 copyleft 传染，限制生态 | 决策层尽早定 |
| 🟡 Windows 单平台 | 用户群体天花板 | 明确产品边界，中期规划 |
| 🟡 界面 38 红 | 用户信任度 | 阶段 0 |

---

## 9. 值得肯定的部分（避免报告只列问题）

1. **内核设计已达国际一流水平**：沙箱三级降级、审计链、场景图多源融合（冲突打 0.6 折）、
   敏感词熔断、epoch 代次防污染——这些**不是 Codex 全部具备**的。
2. **多代理编排深度超过 Codex**：critic 复核 + 有界返工、wilson_ci95 统计门、
   steer 五态、6 态终态枚举、失败隔离。
3. **多模型 BYOK 是差异化优势**：7 家 provider + 自定义端点，Codex/Claude 各锁自家模型。
4. **记忆与学习内核比 Codex 更完整**：experience_store + learn/record + skill_health + proactive。
5. **感知/学习界面比 Codex 更宽**：17 个面板 + 情景感知 + 主动建议，Codex app 无对应物。
6. **测试密度真实**：1,749 个测试，且并行测试是真并行断言（不是 mock 计数）。
7. **工程纪律意识强**：资源红线（内存门/-j 1/退出码 137）是真强制，连"把限制并发的代码写错了"
   都做成了回归测试（`resource-selftest`）。
8. **接口设计诚实**：SQLite 事件回放显式 opt-in，默认返回错误而非假装支持。

---

## 10. 附录：证据索引

| 结论 | 证据 |
|---|---|
| 45 个工具/19 默认 | `core/src/tools.rs:431-463` |
| 副作用 4 分类 + deny-by-default | `policy/src/tool_effects.rs:299-327` |
| 桌面控制默认关闭 | `core/src/settings.rs:135` |
| subagent critic 复核 | `core/src/tools.rs:2720-2760` |
| fan_out JoinSet 并行 | `core/src/fleet/fanout.rs:148-297` |
| `MAX_SUBAGENT_DEPTH=2` | `core/src/subagent.rs:21` |
| WorkSwarm 执行侧 4,591 行 | `core/src/workswarm/`（17 文件） |
| wilson_ci95 统计门 | `workswarm/src/team_benefit.rs:203-728` |
| 自动化只有 Reminder | `extensions/src/automation.rs:19-23` |
| cloud_exec 退避实现 | `extensions/src/cloud_exec.rs:850-851, 894-907` |
| 感知 13 模块 6,011 行 | `crates/owo-agent-perception/src/` |
| 敏感词熔断 | `executor/src/executor.rs:42-53` |
| 平台 cfg(windows) stub | `kernel/src/platform.rs`（24 处 cfg，非 Win 返回空） |
| 沙箱 2,754 行 0 测试 | `crates/owo-agent-tool-safety/src/sandbox.rs`, `audit_chain.rs` |
| 死代码 20 文件 | `desktop/web/{core,views,permissions,shell}/`（index.html 无 script 引用） |
| PANEL_ORDER 双定义 | `app.js:4898` vs `app-domain.js:1431` |
| 三个空壳技能包 | `skills/{documents,spreadsheets,pdf}/SKILL.md`（各 20-24 行） |
| 无图像生成 | 全仓 grep 0 命中；`sim_browser.rs:205` 为测试桩 |
| TS SDK 仅 3 方法 | `clients/ts/src/index.ts:34,39,44`（vs schema 273 路径） |
| 38 项测试失败 | `node --test desktop/web/tests` → 373/335/38 |
| 1,749 测试总数 | grep `#[test]` 986 + `#[tokio::test]` 763（排除 target） |

**Codex 侧来源**（联网核对，2026-10-02）：
- Codex subagent 能力与配置项：`developers.openai.com/codex/subagents`（经搜索摘要，官方页直连返回 Forbidden）
- 2026 能力更新（电脑操控/图像生成/90+ 插件/automations 长程唤醒/memory/in-app browser）：`openai.com/index/codex-for-almost-everything`
- 沙箱与审批双设置、模型分层（GPT-5.6 Sol/Terra/Luna）、多表面（6 个）、4 个 agent 审查维度：`tectack.org` 2026 指南
- 与 Claude Code 逐维度对比（生态、hooks、长程任务、token 效率）：`firecrawl.dev`、`ainative.to`、`rulesell.com`

> 声明：Codex 侧数据来自官方博客与第三方评测，**可能滞后于实际产品**。
> 建议以 `developers.openai.com/codex` 为最终事实源复核后再做投资决策。
