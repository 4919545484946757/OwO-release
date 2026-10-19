# V1-R0 收口报告（第一路 · R0 契约同步、工程收口与总验收）

- 日期：2026-08-27（四路并行日二期 · V1 收口）
- 执行：第一路（本报告由第一路维护，最终验收段在三路交付后更新）
- 范围：OpenAPI / Rust 响应 / TS 类型三方契约同步；retry 冻结契约接线；一键检查脚本；脏工作树审计；统一验收
- 所有权边界：本路不修改 WorkSwarm 核心逻辑与任何 UI 文件；`workswarm.rs` / `workswarm_api.rs` 归第三路，`product_eval*` 归第二路，`desktop/web/**` 归第四路

## 1. 契约同步（上午，已完成）

### 1.1 ModelCandidate 治理字段三方口径（以 Rust serde wire 为权威）

| 字段 | Rust 真相（desktop_world_api.rs） | OpenAPI（lib.rs + 快照） | TS（schema.d.ts 再生成） |
|---|---|---|---|
| 注册入参 `provider_ref` | `RegisterCandidateBody.provider_ref: Option<CandidateProviderRef>` | `/model-candidates` 请求 `provider_ref` → `$ref CandidateProviderRef` | `provider_ref?: {type:"external",kind,locator} \| {type:"metadata_only"}` |
| 候选响应 `provider` | `ModelCandidate.provider: CandidateProviderRef`（serde tag=`type`，snake_case） | 组件 `ModelCandidate.provider` → `$ref CandidateProviderRef`（响应 wire 字段名为 `provider`，注册请求侧为 `provider_ref`，描述中显式注明） | `provider: CandidateProviderRef`（oneOf 双形态） |
| `calibration_summary` | `Option<CalibrationReport>`（仅晋升成功过的候选非空） | 组件 `ModelCandidate.calibration_summary`（allOf + nullable） | `calibration_summary: CalibrationReport \| null` |
| `gates` | 晋升响应 `json!({candidate, active, previous_active, samples, gates{min_shadow_samples, provider_wired{kind,locator}, calibration_summary, regression_check}})` | `/model-candidates/{id}/promote` 200 响应完整 schema（required 五字段） | 同构对象类型（编译期断言用例覆盖） |

新增 OpenAPI 组件：`ModelCandidate`、`CandidateProviderRef`、`CalibrationReport`（与 `owo_agent_core::world_model::CalibrationReport` 字段一一对应：samples/success_hit_rate/mean_calibration_error/mean_delta_jaccard/uncertainty_buckets[label,samples,hit_rate]）。

同步增补的响应状态码登记：注册 409（candidate_id 已存在）；晋升 400（ack=false/空 reason）、422（治理门控拒绝）；steer 400（retry 缺 step_id/未知 command）、404（未知团队/步骤）、409（运行中或目标已成功）。

### 1.2 retry 冻结契约（第三路实现，第一路接线同步）

```
POST /teams/{id}/steer
{"command":"retry","step_id":"builder","note":"修复输入后重试"}
```

- `command` 枚举：`continue | retry | steer | replace | cancel`（R2 追加 retry）
- 缺失/空/空白 `step_id` → 400（校验先于团队存在性）；未知团队/步骤 → 404；运行中或目标已成功 → 409（重复发送无额外副作用）
- 实现核对：`workswarm_api.rs::SteerHttpRequest::into_command`（400 语义）+ `workswarm.rs::apply_steer/steer_retry`（409/幂等语义）——仅核对，未越权修改

### 1.3 变更文件（本路认领范围内）

| 文件 | 变更 |
|---|---|
| `crates/owo-agent-server/src/lib.rs` | openapi_spec 四处：steer 枚举+字段描述+400/404；注册请求 provider_ref+200/409 响应 schema；晋升 200 完整 schema+400/422；components 新增 3 schema |
| `crates/owo-agent-server/tests/route_contract_tests.rs` | 新增 `steer_retry_contract_shape_is_frozen` 用例（锁 400/404/400 形状，不依赖业务实现） |
| `clients/ts/openapi.json`（gitignored 快照） | 与 lib.rs 语义一致的四处镜像（一次性 Node 同步脚本，自检 7/7） |
| `clients/ts/src/schema.d.ts` | `npm run generate:local` 再生成（非手工维护） |
| `clients/ts/tests/client.unit.test.ts` | 新增 2 条类型级契约断言（治理字段 + retry 枚举），任何字段漂移先在 tsc 编译期失败 |
| `AGENTS-COORD.md` | 四路认领登记 + retry 冻结契约留言 |
| `scripts/check-v1-r0.ps1`（新增） | 一键 R0 门禁（见 §4） |

## 2. 实际执行的命令与结果（上午阶段）

| # | 命令 | 结果 |
|---|---|---|
| 1 | 快照同步脚本自检 | 7/7 通过 |
| 2 | `npm run generate:local`（clients/ts） | ✅ openapi-typescript 7.13.0，320ms |
| 3 | `npm run typecheck` | ✅ exit 0 |
| 4 | `npm run build`（含 postbuild copy-schema） | ✅ exit 0 |
| 5 | `npm run test:unit` | ✅ 5/5（3 原有 + 2 新增契约断言） |
| 6 | `cargo test -p owo-agent-server --test route_contract_tests` | 12 条用例：11 条 ok；新增 `steer_retry_contract_shape_is_frozen` 首轮 >60s 未归（原因见下），单跑复现 **0.15s ok**（400×3→404→400 逐条符合冻结契约） |
| 7 | `cargo fmt --all -- --check` | ⚠️ exit 1：违规**全部位于第三路 WIP 文件**（workswarm.rs / workswarm_api.rs / workswarm_recovery_tests.rs / workswarm_recovery_api_tests.rs）；第一路文件零违规。按文件认领规则不代改，最终门禁复验 |
| 8 | `npm test`（含 2 条集成用例） | ⚠️ 3+2 通过、2 条集成用例 ECONNREFUSED（需活服 127.0.0.1:4097），最终门禁起服复验 |

关于第 6 项首轮挂起的定性：当晚第三路正在实时保存其独占文件（recovery 测试与 workswarm_api 改动陆续落盘），首轮二进制编入其中间态快照；以当前磁盘状态单跑该用例 0.15s 通过，且其余 11 条（含 openapi 快照双向一致性、全契约路径可达、401/429/CORS/SSE 豁免）全部 ok。全量 12/12 在最终门禁复验确认。

## 3. 脏工作树分类审计（2026-08-27 22:1x 快照）

### R0（本路）
`server/lib.rs`(openapi 部分)、`route_contract_tests.rs`、`schema.d.ts`、`client.unit.test.ts`、`sdk.test.ts`（昨日第二路修复的 auth 自举，保留）、`AGENTS-COORD.md`、`scripts/check-v1-r0.ps1`、`docs/reports/archive/baselines/v1-r0-closeout.md`、`.gitignore`（昨日收口新增忽略规则，保留）、`agent-sdk/Cargo.lock`（依赖变更产物）、`server/Cargo.toml`（sha2.workspace 一行，昨日第二路登记）

### R1 相关
`desktop_world_api.rs` + `desktop_world_api_tests.rs`（昨日第二路已完成，今日复验）；今日第二路新起：`core/product_eval.rs`（进行中，待其交付）、`core/lib.rs`、`cli/main.rs`

### R2（第三路，进行中）
`core/workswarm.rs`、`core/tests/workswarm_tests.rs`、`workswarm_recovery_tests.rs`、`server/workswarm_api.rs`、`server/tests/workswarm_api_tests.rs`、`workswarm_recovery_api_tests.rs`

### 桌面 UI（第四路待开工；含昨日面板成果）
`desktop/web/panels/workswarm.panel.js`、`style.css`、`app.js`、`index.html`

### 此前迭代遗留（历日已测绿、未提交，随本次一并收口复验）
`core/{goal,gateway,experience_store,fleet_transport,node_agent,worker_pool,world_model,transition,desktop_env,dataset_builder,execution_target,fleet_node_protocol,project_space_store}.rs` 及对应测试、`protocol/lib.rs`、`goal_api.rs` + tests、`fleet_api.rs` + tests、`cli/main.rs`、`cli/worker_child.rs` + tests、`desktop_env/execution_target/transition/world_model_tests.rs`

### 无关内容（不入 agent-sdk 交付）
`skills/`、`skills-main/`、`skills-main.zip`（外部技能资料）、`builGoal/*.md` 两份文档（仓库级设计文档，是否入库由用户决定）

### 违禁物扫描结果
- `git ls-files --others --exclude-standard` 按模式 `\.(db|pid|log|sqlite3?|tmp|key|pem)$|\.env|scratch|dev-glm\.local|target/` 过滤 → **0 命中**
- `.gitignore` 覆盖确认：`target/`、`/agent-sdk/.owo-agent/`、`scratch-*/`、`baseline_*.log`、`gate-results-*.log`、`.check_err.log`、`dev-glm.local.ps1` 均在规则内
- 全量源码 diff 凭据模式扫描（`sk-…` 长串 / api_key|token|secret|password 赋值）→ **0 命中**；凭据红线（仅环境变量）未破

## 4. 一键检查脚本 `scripts/check-v1-r0.ps1`

顺序执行：`cargo fmt --all -- --check` → `cargo check core/server/cli` → server 定向测试（route_contract/desktop_world/workswarm/goal + recovery 若在场）→ cli `worker_child_tests` → core `workswarm_tests`（+recovery 若在场）→ TS typecheck/build/unit → web `node --check` 全量 JS + `node --test tests/*.test.mjs`（第四路交付后自动纳入）。支持 `-SkipCargo/-SkipTs/-SkipWeb`；失败步骤输出尾部日志，汇总表 + `R0 GATE: PASS/FAIL` 退出码 0/1。冒烟验证通过（web 段 12 文件 0 失败；tests 目录缺席时明确跳过并标注第四路待交付）。

## 5. 未纳入交付的文件（明确排除）

- `skills-main.zip`、`skills/`、`skills-main/`（外部资料，与 agent-sdk 无关）
- `builGoal/` 两份未跟踪文档（仓库级设计文档，待用户决定；不含凭据）
- `target/`、`node_modules/`、`dist/`、`dist-tests/`（构建产物，已忽略）
- `clients/ts/openapi.json`（gitignored 本地快照，`include_str!` 与 codegen 的共同源头，属构建输入非交付物）

## 6. 最终验收清单（三路交付后由本路执行并回填）

> 所有权说明：三期（2026-08-28）`docs/reports/archive/baselines/v1-r0-closeout.md` 归第四路独占（见当日认领表）；以下回填由第四路以当日真实门禁结果执行。

- [x] `pwsh scripts/check-v1-r0.ps1` 全绿 —— **三期最终门禁 13/13 PASS**（2026-08-28）：fmt --all 清零；cargo check core/server/cli；server 定向套件 5 套（route_contract 13 条、desktop_world、workswarm_api、goal、**product_eval_api 10 条**——脚本已自动纳入）；cli worker_child；core workswarm + recovery + **single_agent + workswarm 适配器**（脚本自动纳入）；TS typecheck/build/unit(7/7)；web node --check 12 文件 + node --test（第三路已解决活服依赖，0.4s 干净退出，不再挂起）
- [x] `npm test` 集成用例 —— **9/9**（7 单元 + 2 集成：health、创建会话并列出，对 4097 活服、含 ProductEval 全接线新二进制）
- [x] 第二路 `product_eval` 交付后：core/cli 编译面与既有套件无回归 —— 门禁 cargo check + core 四套测试全绿；server 侧由第四路完成 live 接线（ModeDispatchExecutor，single→第一路 / multi→第二路 crate 根 `#[path]` 注册）
- [x] 失败项与处理记录（见 §7）

## 7. 三期收口记录（2026-08-28 · 第四路）

### 7.1 过程中的失败项与处理

| # | 现象 | 根因 | 处理 |
|---|---|---|---|
| 1 | 上午门禁 cargo check/test 全 101 | 各路 WIP 中间态（core 缺 `single_agent.rs`、`workswarm_executor.rs` 半行） | 属并行预期；各路落盘后自愈，最终门禁复验 |
| 2 | 上午门禁 web `node --test` 段挂起烧 CPU | 面板测试活服依赖（第三路域） | 第三路交付版 0.4s 干净退出，最终门禁绿 |
| 3 | `product_eval_contract_shapes_are_frozen` 全量跑 429、单跑过 | 限流用例 `OWO_API_RPM_GLOBAL=5` 进程级泄漏进并行用例的 AppState（rate_limit 全局桶 5 rpm） | 第四路（该测试文件当日归第四路）：test_state 与限流用例共用 STATE_ENV_LOCK 串行化 + test_state 构建前 remove_var；13/13×2 轮稳定 |
| 4 | `cargo fmt --all -- --check` 违规 27 文件 | 24 个历史遗留文件（历日未提交迭代）+ 第四路 3 文件误用 `--edition 2024` 手工 rustfmt（crate 实为 2021） | 我方 3 文件改用 `cargo fmt` 修正；24 个无主历史文件按统一验收职责机械 `cargo fmt --all`（零语义变更，测试全绿复验），登记于此备案 |
| 5 | 门禁首轮 cargo 全线 E0063 `missing tool_log` ×5 | 第一路 `tool_log` 重构中间态 | 等待落盘后复跑，最终门禁绿 |
| 6 | live 真实对照无法执行 | 环境无模型凭据（OPENAI_API_KEY 未配置；凭据红线禁止落盘） | **阻塞已记录，不伪造**：reference 全链路真实基线已产出（`docs/reports/archive/baselines/v1-r1-baseline-first-run.md`：20/20、401/400/422 负例、真实杀服重启 interrupted 语义）；解除条件 = 导出真实凭据后一条 POST |
| 7 | `tool_log` 字段跨文件披露修复后 wire 漂移风险 | 第一路为 `RawExecOutcome`/`ProductEvalRun` 新增 `tool_log: Vec<String>`（`#[serde(default)]` 向后兼容），涉及第二路/测试夹具共 5 处机械补行（用户披露：WorkSwarm 逻辑零改动） | 第四路完成契约同步：OpenAPI `ProductEvalRun` 组件 + 快照 + schema.d.ts 再生成 + TS 用例补 `tool_log: []`；typecheck 0、TS unit 7/7、API 10/10、契约 13/13 复验绿 |

### 7.2 三期新增验收结论（第四路域）

- ProductEval HTTP API 四路由 + 取消幂等 + 重启 interrupted + 工厂失败不伪造：单元 10/10、契约 13/13×2、真实服务器全链路实测（详见 `v1-r1-baseline-first-run.md` §6 对应表）。
- OpenAPI/TS/服务端/UI 字段一致：快照与 schema.d.ts 由脚本同步再生成（含 ProductEvalReport/Run/Key/Metrics/CaseModeMetrics/RunSummary/Error 组件）；TS unit 7/7 类型级锁 `workswarm≡multi` 口径；第三路 eval.panel.js 抽查与冻结契约一致（请求体六字段、六态、`agent_mode multi→workswarm` 展示映射）。
