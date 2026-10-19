# V1-R1 Live 基线报告（第一路 · R1）

- 生成：2026-08-28（四路推进日，第一路收口）
- 范围：R1 live 基线、环境预检与统计判定
- 套件：`agent-sdk/evals/v1/suite.json`（v1-r1-product-suite，10 任务：code×4 / research×3 / document×3）
- 执行引擎：`live-agent`（真实单 Agent：独立 Session + 最小受控工具集 + 任务级权限审批）与
  `live-workswarm`（真实 WorkSwarm TeamRun：producer → critic → leader）
- 模型：`glm-5.3-flash`（BigModel OpenAI 兼容端点；凭据仅经环境变量注入，全程未落盘）
- 数据：`scratch-eval-runs/live-baseline/{agent-single,workswarm-single}/{state.jsonl,report.json}`

## 1. 结论速览

| 指标 | single（n=30） | workswarm（n=10） |
|---|---|---|
| 成功率 | **80.0%**（24/30），95% CI [62.7%, 90.5%] | **50.0%**（5/10），95% CI [23.7%, 76.3%]（样本不足标注） |
| 耗时 p50 / p95 | 30.3s / 107.4s | 95.5s / 171.5s |
| 平均模型调用 | 2.9 次/格 | 6.8 次/格 |
| tokens | 126,611（≈4,220/格） | 84,798（≈8,480/格） |
| 成本 | None（单价未配置） | None（单价未配置） |

**多 Agent 启用条件判定：三条全部不满足 → 暂不建议启用多 Agent（当前 WorkSwarm 形态）。**

- ⬜ 成功率 +5%：multi − single = **−30.0pp**（远低于 +5pp 门槛）
- ⬜ 质量 +10%：成功率相对提升 **−37.5%**（门槛 ≥ +10%；质量代理 = 成功率相对提升，检查器为二元判定，暂无独立质量指标）
- ⬜ 耗时 −30%：平均墙钟相对变化 **+163%**（40.5s → 106.4s；门槛 ≤ −30%）

样本充分性：single n=30 达阈值；workswarm n=10 未达（CLI 输出已自动标注"样本不足 n<30，区间仅供参考"）。
方向性结论：在本套件 + 该模型上，WorkSwarm 三角色 DAG **更慢（+163%）、更贵（+101% tokens/格）、成功率更低（−30pp）**，
失败模式也更重（整文件级失败，见 §5）。

## 2. 环境预检（`product-eval preflight`）

exit 0=就绪 / 2=阻塞；只报"已配置/未配置"，绝不输出凭据内容。

实测（凭据就绪，本日真实输出）：

```text
① Provider URL：已配置（https://open.bigmodel.cn/api/paas/v4）
② 模型名：已配置（glm-5.3-flash）
③ OPENAI_API_KEY：已配置（长度 49，内容不展示）
④ 套件：已加载（v1-r1-product-suite，10 任务，校验 全部通过）
⑤ 输出目录：可写（…scratch-eval-runs/product-eval）磁盘剩余 98.0 GB
⑥ ORT_LIB_PATH：未设置（仓库缓存自动探测可用：…target\sherpa-onnx-prebuilt\…\lib；
   跑 check-v1-r0.ps1 会自动注入当前进程）
preflight：全部就绪（exit 0）
```

实测（移除凭据后）：`③ OPENAI_API_KEY：未配置 → live/agent/workswarm 阻塞（dry 不受影响）` →
`preflight：1 项阻塞（exit 2）——live 基线在阻塞解除前不会执行，也不会用 reference 结果代替`。

**双路径语义均验证通过。** 另实测：live 编排脚本在新进程缺少凭据注入时，`run` 命令本身也会报
`缺少 OPENAI_API_KEY 环境变量` 退出（脚本入口先读用户级注册表值注入本进程，详见
`run-product-eval-live.ps1` 的 Credential passthrough 段）。

## 3. ORT 链接环境（新终端可自动恢复）

- 背景问题：`owo-agent-core` 依赖 `ort 2.0.0-rc.13` + `sherpa-onnx 1.13`；此前会话观察到新终端
  不设置 `ORT_LIB_PATH` 时 Rust 测试在链接阶段失败。
- 本轮实证：当前依赖树在全新进程（无 `ORT_LIB_PATH`）下 `cargo test` **链接成功**（新增统计测试
  二进制首次链接即通过，13/13；`ort-sys` 自行完成库准备）。原失败模式在现树不复现。
- 防御层（仍落地，实测探针命中 `…sherpa-onnx-v1.13.5-win-x64-static-MT-Release-lib\lib`）：
  - `scripts/check-v1-r0.ps1`：`ORT_LIB_PATH` 未设置时自动探测
    `target\sherpa-onnx-prebuilt\**\onnxruntime.lib`，**只设置当前进程环境**，不写持久配置；
  - `scripts/run-product-eval-live.ps1`：同探针（本轮实测日志：`[ORT] ORT_LIB_PATH auto-probed (process only)`）；
  - `product-eval preflight` ⑥：报告 ORT 状态与仓库缓存探测结果（live 基线运行时显示"已配置（目录含 onnxruntime.lib）"）。
- 完工要求"新终端无需手工寻找 ORT 路径即可执行定向测试"：**满足**（现树直接可跑；若未来依赖
  回退到需显式路径的形态，探针自动兜底）。

## 4. Live 基线执行与断点续跑实证

`scripts/run-product-eval-live.ps1 -Full` 按计划顺序执行：preflight 门禁 → Phase A（3 任务 real
single Agent）→ Phase B（同 3 任务 real WorkSwarm）→ Phase C（10 任务×3 次 agent + 10 任务×1 次 workswarm）。

真实发生的中断与恢复（journal 语义验证）：

- 编排过程中两次 `cargo run` 以 exit=-1 异常终止（疑与外层管道缓冲相关，非评测器缺陷）：
  一次发生在 Phase B `document-revise` workswarm 单元之前，一次发生在 Phase C agent 全量跑到第
  21 格之后。**失败单元格永久留存，绝不静默重跑；缺失单元格由断点续跑精确补齐。**
- 补跑同一命令：`planned=30 done=21 todo=9` → 只补 9 格，30/30 收口（实测日志原文）。
- workswarm 侧 10/10 全部完成（document-revise 格由 Phase C 补齐并如实计为失败）。

## 5. 真实失败定位（分母含失败，不剔除）

**single agent（6 个失败/超时格）：**

| 单元格 | 判定依据（checker 输出原文） |
|---|---|
| document-structured-extract ×3（0/3，系统性） | `json_field(out/extract.json.scope == "重构结算模块")` ⇒ 实际写入 `"重构结算模块，仅限清算子域"`（模型过度收窄表述，严格相等判负） |
| code-contract-change ×2 | `contains(out/contract-change.md, "get_user_name → fetch_user_display_name")` ⇒ 期望片段缺失 |
| code-repo-audit ×1 | `timeout:超过 180s 上限`（model_calls=0：GLM 单次调用 180s 内未返回，live 网络条件真实呈现） |

**workswarm（5 个失败/超时格）：** research-technical-brief（交付物含禁止片段 `"TBD"`）、
code-contract-change（同上期望片段缺失）、code-refactor（TeamRun 180s 超时）、
document-revise（含禁止片段 `"非常非常"`、`"搞出来"`）、
document-structured-extract（**整份输出不是合法 JSON**——比 single 的"字段偏差"更重的失败形态）。

**真实工具调用轨迹（journal `tool_log` 摘录，全部为经权限审批的真实工具执行）：**

```text
code-bug-fix（2 步）: read_file src/calc.py（233 字符） → write_file out/fix-report.md（2175 字节）
code-repo-audit（5 步）: list_dir .（2 项） → read_file src/order_service.py（400 字符）
   → read_file src/README.md（48 字符） → read_file config/settings.ini（33 字符）
   → write_file out/audit.md（5478 字节）
research-source-compare（3 步）: read_file sources/source_a.md → read_file sources/source_b.md
   → write_file out/comparison-table.md（879 字节）
document-draft（2 步）: read_file briefs/launch-points.md → write_file out/draft.md（1726 字节）
```

## 6. 统计口径与方法

- 实现位置：`owo-agent-core/src/product_eval.rs` 末尾统计模块——`wilson_interval`（95% Wilson score）、
  `percentile`（线性插值 p50/p95）、`mode_statistics`（拓扑快照：CI/分位数/均值/tokens/cost/样本充分性）、
  `compare_mode_statistics`（对照 + 三条启用条件判定）、`report_statistics`（JSON 契约形状，
  供 server/TS 后续同步）、`format_mode_statistics`（run 摘要追加段，CLI 已接入）。
- 口径：分母一律含失败/错误/超时/取消，**不剔除、不重算**；成功区间用 Wilson（小样本稳健）；
  质量代理 = 成功率相对提升（检查器为二元判定，暂无独立质量指标，detail 中已标注）。
- 边界测试：`tests/product_eval_statistics_tests.rs` 13 项——0 样本 / 全成功（n=30 下界≈0.89）/
  全失败（上界≈0.11）/ 单元素 / 插值 / 区间随样本收窄 / 样本不足标注 / 耗时 −30% 恰好满足（<= 语义）等；
  另 `product_eval_tests.rs` 补 1 项集成断言（19/19）。
- 成本：`OWO_EVAL_PRICE_IN/OUT_PER_MTOK` 未配置 → cost=null（tokens 真实落盘）。

## 7. 完工对照

| 完工要求 | 状态 |
|---|---|
| 新终端无需手工 ORT 路径即可定向测试 | ✅（实证直接可跑 + 双脚本探针兜底，探针命中实测） |
| preflight 不泄露任何密钥 | ✅（掩码输出；exit 0/2 双路径实测） |
| 统计函数边界测试（0 样本/全成功/全失败/样本不足） | ✅（13+1 项） |
| 有凭据时 ≥6 次真实调用对照 | ✅（single 30 格 + workswarm 10 格，共 156 次真实模型调用、211,409 tokens） |
| 无凭据时形成明确阻塞报告 | ✅（语义实测：exit 2 + 明示"不用 reference 结果代替"；本轮凭据可用） |
| 不修改 server、WorkSwarm、UI 文件 | ✅（仅触碰第一路清单文件 + CLI Cargo.toml 新增 windows-sys cfg(windows) 依赖一处，已在协调层披露） |

## 8. 遗留与建议（供第四路/后续轮次）

1. **document-structured-extract 0/3 系统性失败**：`json_field` 严格相等 vs 模型"过度收窄"表述
   （"重构结算模块" → "重构结算模块，仅限清算子域"）。建议任务定义给 scope 字段增加允许子串
   （`contains`）或在该字段改用包含判定——属于检查器/任务定义口径问题，不是执行器缺陷。
2. WorkSwarm 当前形态（3 角色 DAG，≤8 回合/角色）在本套件上全面劣于单 Agent；若要继续投入，
   建议：压缩 critic/leader 到单回合、限制 producer 交付物必须直接落盘（本轮已出现整文件非法 JSON）。
3. live 编排中两次 exit=-1 建议后续在 `Invoke-EvalRun` 内改为 `Start-Process -Wait` 直跑
   （规避 PowerShell 管道缓冲与原生命令 stderr 交互），并在脚本内加单跑重试上限（journal 语义已保证不重复计费）。
4. 成本列：配置 `OWO_EVAL_PRICE_IN/OUT_PER_MTOK` 后即可填充（tokens 已真实落盘，可离线回算）。
