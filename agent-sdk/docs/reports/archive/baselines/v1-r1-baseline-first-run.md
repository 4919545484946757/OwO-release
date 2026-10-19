# V1-R1 首份产品评测基线（第四路 · ProductEval HTTP API 真实链路）

- 日期：2026-08-28（四路并行日三期 · 下午）
- 执行：第四路（ProductEval HTTP API、契约接线与统一验收）
- 性质：**真实链路运行记录**（非模拟数据）；reference 全链路实测 + 重启中断实测 + 契约负例实测；live 对照为**已记录阻塞**（见 §5，不伪造）
- 冻结契约：`AGENTS-COORD.md` 留言区「第四路（三期开工）」

## 1. 执行环境

| 项 | 值 |
|---|---|
| 服务器 | `cargo run -p owo-agent-cli -- serve --port 4096 --workspace T:\…\agent-sdk`（真实 axum 服务，bearer 保护面） |
| 数据根 | `OWO_AGENT_DATA=scratch-ws-ui/pe-data`（gitignored scratch；运行目录 `product_eval/runs/`） |
| 凭据 | **环境无 OPENAI_API_KEY**（凭据红线：仅环境变量，本会话未配置真实凭据；以占位串启动服务——echo/generative worker 不会调用模型，reference 路径零模型调用） |
| 套件 | `evals/v1/suite.json`（10 任务：code×4 / research×3 / document×3），suite_hash `25bf1777…61ca` |

## 2. 契约负例实测（真实 HTTP）

| 请求 | 实测 | 冻结契约 |
|---|---|---|
| POST `/product-eval/runs`（无 Authorization） | **401** | 未认证拒绝 ✓ |
| POST `modes:["team"]`（未知拓扑） | **400** | 未知 mode → 400 ✓ |
| POST 缺 `modes` 字段 | **422** | 结构错误 → 422 ✓ |

## 3. reference 全链路真实运行（免模型）

- 受理：`POST /product-eval/runs` `{"suite":"v1","execution":"reference","modes":["single","workswarm"],"repetitions":1}` → **202** `{"run_id":"eval-e172fc4f020e46cfa4610aed232e163e","status":"queued"}`
- 轮询详情至终态：**completed，进度 20/20**（10 任务 × 2 拓扑 × 1 次）
- 聚合指标（`report.metrics`）：

| runs_total | passed | failed | errors | timeouts | cancelled | success_rate | mean_wall_ms | total_model_calls | total_tokens |
|---|---|---|---|---|---|---|---|---|---|
| 20 | 20 | 0 | 0 | 0 | 0 | 1.0 | 6.75 | 0 | null |

- `per_case` 20 行（每 case × mode 一行）：全部 passed，单/多拓扑 mean_wall_ms 2–4ms，`mean_model_calls=0`、`total_tokens=null`（reference 免模型，符合契约口径）。
- 每行 `artifact_refs` 指向沙盒内相对路径（如 `code-repo-audit` multi → `out/audit.md`）；`failed_steps` 空。
- **`tool_log` 字段**（收口阶段新增）：`ProductEvalRun`/journal 增加真实工具调用轨迹（`#[serde(default)]`，旧记录反序列化为空数组，向后兼容）；本基线为 reference 回放，`tool_log` 均为空；live 运行时由单 Agent 执行器填写（工具+实参摘要+结果）。OpenAPI/TS 已同步该字段（契约用例含 `tool_log: []`）。
- **语义说明**：reference = 参考输出回放 + 检查器自检（harness 确定性验证），证明任务/权限/检查器/矩阵/报告链路真实可达；**该 20/20 不是模型质量结论**，模型质量结论只能来自 live 对照（§5）。
- 原始报告快照：`scratch-ws-ui/pe-baseline-reference.json`（hub.json + report.json 原样）。

## 4. 重启中断语义实测（真实杀服/重启）

1. 发起 `repetitions=20` 长跑（400 单元格）→ `eval-7ac9fb79cb474f4cb48b1ebf68da991e`，杀服前快照 `running 44/400`；
2. 直接 kill 服务器进程（非优雅停止）→ 重启；
3. 重启后实测：`status=interrupted`、`progress=205/400`（journal 与 report.json 保留已完成单元格）、`finished_at` 落账、`error="服务重启：运行被中断（不自动重跑；报告含已完成单元格）"`（磁盘 hub.json UTF-8 无损）；
4. 之前的 completed 运行原样可查（20/20 + 完整报告）；`GET /product-eval/runs` 列出 2 条，状态 `interrupted, completed`，created_at 倒序。

（取消语义的幂等性——终态后取消零副作用、live 慢执行器取消立即 cancelled 且收尾不覆盖——由 `product_eval_api_tests.rs` 10 条用例锁定，其中 `cancel_marks_running_run_cancelled_immediately_and_keeps_partial_report` 使用真实 CaseExecutor 契约桩。）

## 5. live 真实对照：**阻塞（已记录，不伪造）**

- **阻塞条件**：本会话环境无任何模型凭据（`OPENAI_API_KEY`/`ANTHROPIC_API_KEY` 均未配置；凭据红线禁止把真实凭据写入代码/配置/提交）。
- **已就绪部分**：
  - live 工厂已接线（server 编译通过）：`reference → ReferenceDryExecutor`；`live → ModeDispatchExecutor{single → 第一路 SingleAgentExecutor，multi → 第二路 WorkSwarmExecutor}`；Provider 经 core `build_live_provider` 统一构建（模型解析：OPENAI_MODEL > 内置默认 GLM）。第二路适配器以 crate 根模块名注册（`#[path = "product_eval/workswarm_executor.rs"] pub mod product_eval_workswarm;`，物理文件归位 product_eval/ 目录、模块树避免与第一路双写）。
  - 工厂失败路径有测试锁定（`live_factory_error_fails_run_without_fabricating_results`）：运行进 `failed`、不带报告、零模型调用。
  - 第二路 `WorkSwarmExecutor`（45KB）与第三路 UI 面板文件已在磁盘交付。
- **解除条件**：在环境中导出真实 `OPENAI_API_KEY`（可选 `OPENAI_MODEL`/`OPENAI_BASE_URL`）后重启服务，`POST /product-eval/runs {"execution":"live",...}` 即可产出首份单 Agent vs WorkSwarm 真实对照；结果将进入本报告的后续小节，**不回填、不伪造**。

## 6. 交接与验收对应

| 验收项（三期） | 状态 |
|---|---|
| reference 免模型完成 API 全链路 | ✅ §3（202→completed 20/20，真实服务） |
| 未认证请求被拒绝 | ✅ §2（401） |
| 非法 suite/mode/repetitions 明确 400/422 | ✅ §2 + 契约测试 `product_eval_contract_shapes_are_frozen`（13/13×2 轮） |
| 重启读历史并标记 unfinished 为 interrupted | ✅ §4（真实杀服/重启） |
| 取消语义（协作令牌+立即 cancelled+幂等） | ✅ 单元测试 10/10 |
| OpenAPI/TS/服务端/UI 字段一致 | ✅ 快照+schema.d.ts 再生成 + TS unit 7/7（类型级断言含 workswarm≡multi 口径）；UI 消费契约已冻结并通告 |
| live 首份真实单/多对照 | ⛔ 阻塞（无凭据；解除条件见 §5，不伪造） |
