# 远端 engine 分支同步报告（2026-10-01）

## 0. 结论

- 同步对象：`https://github.com/3311930677/LingXi-Suite` 的 **engine 分支**（`be6298f`）。
- 同步状态：**已完成**。实质代码/行为缺口为 0；engine 侧 169 个本地没有的文件已逐一归类，其中 3 篇设计文档本次补入，其余 166 个按“本地项目为主”保留不回移（清单与理由见 §5）。
- 原则：**以本仓库为主，取优合并**；凭据只经环境变量（AGENTS.md 红线）；只改 `agent-sdk/**`。
- 范围外：`desktop` 分支（独立工程树：Tauri overlay + 桌宠皮肤 + IME crates，177 个文件）与 `main` 分支（仅 README 导航）不在本次指令范围；根目录 OwO C++/`include/`/`apps/settings_center` 等改动按 AGENTS.md「不修改 OwO 输入法代码」不动。

## 1. 同步方法（怎么合并的）

1. **差异定界**：`git fetch lingxi` 后，对 engine 的 `agent-sdk/**` 与本地逐文件比对 blob 哈希，分成三类：
   - 同哈希（已一致）；
   - engine-only（本地无该路径，169 个）；
   - 内容不同（同一路径两边都改过，90 个）。
2. **符号级取优**：对 engine-only 与内容不同的 Rust 文件，做 `pub fn/struct/enum` 符号比对（先剥注释再比对；本地 crate 已微内核拆分，按同名文件/整 crate 合并文本比对），找出「engine 有而本地没有的公开符号」，逐个判定：真缺口 / 本地等价物 / 本地超集 / 架构差异。
3. **路由面契约**：提取双方服务端全部 `route(...)` 注册清单比对（本地 285 条 vs engine 127 条）。engine 独有路由仅 `/permissions/rules`、`/permissions/rules/remove` 两条。
4. **逐项落位**：
   - 真缺口 → 移植（保持本地 API 面，例如 `run_turn_with_images` 适配本地 `run_turn_inner` 入口）；
   - 本地等价 → 记录等价物（如 REPL 补全、审批 scope、auth token）；
   - 本地超集/更优 → 不回移，记录理由；
   - 上游前端/资产/脚本 → 本地实现优先（见 §5）。
5. **契约测试随功能提交**：路由类改动同步 `openapi.rs` + `clients/ts/openapi.json`（脚本 `scripts/regenerate-openapi-snapshot.ps1`）+ `tests/route_contract_tests.rs`；core 行为改动补单元/契约测试。
6. **资源红线**：所有构建先 `. scripts\resolve-ort.ps1; Resolve-OwoOrtEnv`，定向测试 `-j 2`、`--test-threads<=2`，`route_contract_tests` 强制 `--test-threads=1`；未在仓库写入任何凭据。

## 2. 本轮同步提交

| 提交 | 内容 | 验证 |
|---|---|---|
| `773a218` | `/pet`、`/pet-assets` 静态路由 + 全局 no-store（engine `be6298f` 取优） | route 契约 33/33 |
| `9036891` | `ToolResult.preview` 全链路（core → SSE → CLI） | core/cli |
| `fa9e8dd` | 步数耗尽收尾总结 + 空回答兜底摘要 | loop_tests 契约更新 |
| `1594bdb` | `multi_edit` 批量替换工具（原子 + Write 级策略） | policy 计数 |
| `cd65280` | `fan_out_subagents` 并行只读子代理（含取消桥） | core |
| `c2c7307` | `git_status` / `git_diff` / `git_log` 只读工具 | core |
| `ff64788` | hooks A2-1：5 类生命周期事件 + settings/agent/server/CLI 全链路 | core/server |
| `1a0fe7c` | `ReasoningDelta` / `PlanUpdate` / `TurnStats` 事件全链路（provider 流式重构） | protocol/core/cli |
| `3312b76` | 脏历史归一 + CJK token 估算 + A4-3 预算硬裁剪兜底 | core |
| `69b9e50` | A8 路由：`/activity`、桌宠显隐 `/desktop/pet[/report]`、`/approvals/pending`、`/automations/runs` | route 契约 |
| `7c6f811` | `/fs/pick-directory` 原生选目录、`/fs/open` 工作区内开文件 | route 契约 |
| `cdd0b10` | ask_user 全链路：question 通道 + 工具 + SSE 事件 + `/session/{id}/answer/{question_id}` | core/policy/route 36/36 |
| `3a98e1a` | Anthropic 原生 provider（A1-1）+ `build_model_http_client`（NO_PROXY，A1-4）+ `OWO_PROVIDER` 选择 | core 235 |
| `1c08d35` | MCP resources/prompts 泛化工具全链路（A2-2）+ 存储加密契约测试（P0-1，移植到 kernel） | mcp 15/15；kernel 36+11 |
| `39c977e` | A1-2 多模态回合 `run_turn_with_images` + DeferredProvider（provider 热重读）+ 两处历史测试缺陷修复 | core 238、server lib 71、route 36 |
| `71958cf` | CLI B4：`turn` 提示词支持 stdin（`--prompt -` / 管道省略） | cli bins 45 |
| `d81c515` | 推理档位：`settings.reasoning_effort` → `OWO_REASONING_EFFORT` → 请求体按需下发 | core 240 |
| `9bb3ca8` | B2：TUI transcript 去 Markdown 标记（`strip_markdown`） | cli bins 46 |

（另：`builGoal/Agent-SDK-远端engine取优合并记录-2026-10-01.md` 为同批次的合并记录文档。）

## 3. 关键取优点（与上游不同但更强的实现）

- **多模态回合**：engine 的 `run_turn_with_images` 直接进 Agent；本地统一为 `run_turn_inner`，`run_turn`/`run_turn_with_asker`/`run_turn_with_images` 三入口共用，空图片时行为与旧版逐字节一致；服务端图片附件转 base64 data URL（>5MB 拒绝），文本附件维持路径注入。
- **延迟 Provider**：engine 的 `DeferredProvider` 全量移植，但**密钥仍只来自环境变量**（不采纳上游把 key 写入加密 settings 的 `set_provider_override` 路径）；新增 `provider_ready()` 与 `ResilientProvider::from_deferred()`，serve 未配置不阻塞启动、设置页换模型对新回合即时生效。
- **Anthropic 通道**：engine 只实现 `complete`/`complete_stream_with_reasoning`；本地额外适配 `complete_stream_with_model` 与 `complete_stream_with_reasoning_and_model`，会话级模型覆盖不丢。
- **MCP 泛化工具**：engine 每个 server 注册 2 个工具；本地保持一致（`{server}_read_resource` / `{server}_get_prompt`），副作用按“未声明注解的 MCP 工具”登记为 Execute（deny-by-default），不因名字带 read 自动降权。
- **推理档位**：非法取值（含空串/大小写混写归一后非法）一律不下发 `reasoning_effort`，避免不支持该字段的 OpenAI 兼容端点 400。

## 4. 验证基线（当前 HEAD `9bb3ca8`）

| 套件 | 结果 |
|---|---|
| `owo-agent-core --lib` | 240 passed |
| `owo-agent-server --lib` | 71 passed |
| `owo-agent-server --test route_contract_tests`（`--test-threads=1`） | 36 passed |
| `owo-agent-cli --bins` | 46 passed |
| `owo-agent-mcp --test mcp_tests` | 15 passed（含 A2-2 两项） |
| `owo-agent-kernel` | 36 lib + 11 crypto_contract |
| `owo-agent-policy --lib` | 63 passed |
| 其余（observability 27、cloud_sse 6、turn_api 4、protocol 2、extensions 22） | 通过 |
| `cargo fmt --all --check` / 改动 crate `clippy --all-targets` | 干净 |
| OpenAPI 快照 | `clients/ts/openapi.json` paths=283，双向一致 |
| 工作树 | clean（本报告与 3 篇文档为本次新增） |

## 5. engine 侧“有但本地不回移”清单（本地项目为主）

| engine 内容 | 本地替代/保留原因 |
|---|---|
| `permissions.rs` + `/permissions/rules`、`/permissions/rules/remove` | `owo-agent-policy` 的 GrantStore（参数指纹 + scope + 过期）为超集，且审批与主 Agent 分离；规则表语义无法无损映射，不提供假适配层 |
| `plan_tools.rs` | `todo` 工具 + `PlanUpdate` 事件已覆盖，另立计划状态会产生第二状态源 |
| `web_tools.rs` | `tools.rs` 内 `web_fetch`/`web_search` 已具且带宿主校验 |
| `loop_guard.rs` | 执行循环内联守卫（重复调用/步数/空转）已覆盖 |
| `tokenizer.rs`（tiktoken-rs） | 本地 CJK 感知启发式估算 + 预算兜底；不引入新原生依赖 |
| `desktop/web` 前端（app.js/panels/style/index/provider-presets/tauri） | 本地自有 views/core/shell 架构且持续演进的 UI 为主 |
| 48 个桌宠皮肤（约 28 MB） | 皮肤由桌面 overlay 自带并经 `OWO_PET_ASSETS_DIR` 注入；engine 副本路径与本仓库回落路径不一致，属无消费者的重复资产 |
| `about.panel.js` / `automations.panel.js` | 本地 UI 不引用这两个上游面板 |
| `scripts/acceptance.ps1` 等 4 个脚本 | 依赖未采纳的测试面/IME 路线或属上游评测辅助；本地以 `scripts/ci-*` 为准 |
| `agent-sdk/AGENTS.md`、`agent-sdk/.gitignore` | 本地治理文件为准 |
| engine 单文件 core/server 模块（`agent.rs`、`gateway.rs`、`mcp.rs` 等 61 个路径） | 本地已微内核拆分为 `owo-agent-kernel`/`-extensions`/`-mcp`/`-policy` 等 crate 或目录模块，为超集 |
| engine-only 测试（loop_guard/plan_tool/permission_rules/tokenizer_accuracy/tool_efficiency 等） | 对应功能已判定不回移；crypto_contract 已移植到 kernel、eval_tests 已在 devtools/product-eval |
| `evals/**`（36）、`dist/**`（5） | 评测/打包产物，本地有各自的评测链与打包脚本 |
| 根目录 C++/`include/`/`apps/settings_center` 改动 | AGENTS.md：不修改 OwO 输入法代码 |

本次补入的 3 篇 engine 设计文档：`docs/plans/archive/2026-09/Agent 能力发展方案.md`、`docs/plans/archive/2026-09/效能核心改造方案.md`、`docs/plans/product/桌宠功能审核与接入规划.md`。

## 6. 复核方式（可复现）

```powershell
cd agent-sdk
git fetch lingxi --prune

# 1) 文件级：engine 与本地逐文件哈希对比（只需读 git，不动工作树）
#    engine-only / 内容不同 两张清单
# 2) 符号级：对 engine 的 crates/**/*.rs 提取 pub fn/struct/enum，
#    与本地同名文件（或整 crate）比对，输出 engine-only 符号清单
# 3) 路由面：提取双方 route(...) 清单做差集
# 4) 回归：
. scripts\resolve-ort.ps1; Resolve-OwoOrtEnv -Quiet | Out-Null
$env:CARGO_BUILD_JOBS = "2"; $env:RUST_TEST_THREADS = "2"
cargo test -p owo-agent-core --lib --locked -j 2 -- --test-threads=2
cargo test -p owo-agent-server --test route_contract_tests --locked -j 2 -- --test-threads=1
cargo test -p owo-agent-cli --bins --locked -j 2 -- --test-threads=2
```

（差异与符号审计脚本本次放在临时目录 `%TEMP%\opencode\`，未入库；需要固化时可移入 `agent-sdk/scripts/`。）

## 7. 后续（不在本次范围）

- `lingxi/desktop` 分支（177 文件）：独立 `LingXi-DesktopAgent` 工程树（Tauri overlay、`owo-bridge`/`assistant-*` crates、docs/CI）。若要做桌宠端到端，建议只导 `apps/overlay` + `crates/owo-bridge` + 相关文档，并保持根目录 OwO 工程不动。
- `lingxi/main`：仅 README 仓库导航（19 行），如需可直接取文档。
