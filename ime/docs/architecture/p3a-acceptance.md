# 阶段验收报告：P3A 模型基础

- 日期：2026-08-01
- 结论：工程基础通过；真实候选排序权重因外部许可与任务适配未达标而保持关闭

## 已完成

- 独立 ModelHost v1 进程与版本化命名管道协议，支持请求 ID、模型 ID、预算、状态、取消、超时、诊断和协议关停。
- Core Service 先返回基础候选，模型结果只以 request/generation 匹配的异步增量更新进入 TSF；宿主缺失、崩溃和超时不影响基础输入。
- 确定性 Mock、可替换 `IInferenceSession`、BERT pair 编码、严格模型 manifest、流式 SHA-256、ONNX opset/张量契约。
- ONNX Runtime CPU 1.28.0 仅动态链接 ModelHost；默认构建关闭 ORT，TSF 与 Core Service 不依赖或加载 ORT。
- 项目合成 ONNX 的真实 Session 加载、元数据核验、推理、取消、超时、非法批次和命名管道实进程闭环。
- ORT 版本、NuGet SHA-512、DLL SHA-256、LICENSE、ThirdPartyNotices 和发布目录完整性门禁。
- 候选排序数据 JSONL v1、来源许可/隐私/哈希/切分校验、严格预测格式、Top-1/MRR/净提升/改错率与分层报告。
- 确定性 120 条合成管线夹具，覆盖 train/validation/test、候选数量、拼音长度和上下文长度；明确不作为模型质量证据。
- Windows 11 记事本实机验证基础候选先返回、Mock 智能结果增量更新、快速改写时旧 generation 不覆盖新输入。

## 未完成与原因

- 未集成 UER Chinese RoBERTa L4-H256：上游模型仓库未明确权重再分发许可，且检查点是 MLM、没有候选排序头。
- 未采用 BGE reranker：许可和排序头清晰，但权重约 1.11 GB，训练目标为查询—文档相关性，未经输入法质量验证且不符合首发轻量预算。
- 未采用 HFL RBT3：Apache-2.0 与体积相对可接受，但仍缺候选排序头；只有合成数据时训练会产生误导性质量结论。
- 未冻结真实模型的准确率、改错率、冷启动、内存和延迟阈值：必须在许可清晰且规模足够的冻结数据集与真实权重上决策。
- 文本联想和高级重排只保留可替换接口，未引入 MiniCPM4 或 Qwen3-Reranker 权重；P3 不要求一次打包三个完整模型。

## 实际变更范围

- `include/owo/model/`、`src/model/`：后端、调度、资产、tokenizer、session 与协议契约。
- `apps/model_host/`、`apps/model_shell/`：独立宿主与诊断客户端。
- `include/owo/ipc/`、`src/ipc/`、`apps/core_service/`、`src/tsf/`：异步增量候选数据流。
- `tools/`、`benchmarks/`、`tests/model/`、`tests/data/`：依赖获取、合成 ONNX、数据校验、质量评估和性能测试。
- `docs/adr/0005-*`、`0006-*`、`docs/benchmarks/` 与 P3 文档：可审计决策与真实结果。

## 构建命令与结果

```powershell
cmake --build --preset windows-release --target owo_model_host
cmake --build --preset windows-release-ort --target owo_model_host owo_onnxruntime_session_tests
```

- 默认 Release 关键目标构建成功。
- ORT Release 关键目标构建成功。

## 测试命令与结果

```powershell
ctest --preset windows-release --output-on-failure
ctest --preset windows-release-ort --output-on-failure
```

- 默认 Release：24/24 通过。
- ORT Release：26/26 通过。
- ModelHost + shell 真实进程：返回“泥号 / 你好”，排序、shutdown ack 与宿主退出码均为 0。

## 性能或资源数据

合成 589 字节 ONNX、Release、AMD Ryzen 9 9955HX：

- 资产校验：1.11～1.25 ms。
- 首个 ORT Session 创建：44.77～50.47 ms（不含进入 `main` 前的 DLL loader 时间）。
- 推理后进程峰值工作集：30.88～31.02 MB。
- 批次 1/2/4/8 的 p95：7.5～9.4 μs。

这些数据只验证运行时和测量管线，不代表真实 BERT。

## 已知缺陷

- 真实权重不可用时产品仍使用 P2 基础排序；这是显式降级，不是隐藏的 Mock 质量替代。
- 当前 ModelHost 服务循环串行处理请求，适合首个候选排序器；多模型并发需在真实负载出现后重新设计。
- Python 数据工具是可选离线工具；找不到 Python 时不阻塞默认 C++ 构建，但训练/发布数据必须单独执行校验。
- 一次既有合成 session 时间敏感测试在完整套件中瞬时失败，随后单测及连续两轮全套均通过；未复现，继续保留观察，不放宽断言。

## 安全与权限检查

- 模型运行时未进入 TSF 或 Core Service；PE 依赖表确认 `OwO.TSF.dll` 不引用 `onnxruntime.dll`。
- 模型资产限制在 manifest 同目录，拒绝绝对路径、目录穿越、重解析逃逸、错误哈希、错误 opset 和错误张量契约。
- 默认日志不保存完整用户输入。
- 数据门禁拒绝私有遥测、未知许可、不可再分发来源、跨 split 泄漏、BOM、非法 UTF-8、控制字符与代理码点。
- 未下载、提交或分发许可不明确的模型权重。

## 文档更新

- ADR-0005：模型许可、任务头与替代权重复核。
- ADR-0006：ORT 依赖、供应链与部署边界。
- `ranker-dataset-v1.md`：数据、预测、评估与隐私契约。
- `p3a-onnxruntime-2026-08-01.md`：真实基准环境、方法和限制。

## 回滚方式

- 默认 `OWO_ENABLE_ONNXRUNTIME=OFF`；关闭可选开关即可回到无 ORT 构建。
- Core Service 不指定 `--model-host` 即完全使用基础候选路径。
- 数据与评估工具不参与运行时，可独立移除而不影响输入法。

## 下一阶段候选方案

进入 P3B：先实现不依赖 WinUI 3 的版本化配置 Schema、严格解析、原子持久化、上一有效版本恢复和热加载契约；CLI/CTest 通过后再接独立设置中心 UI。

## 需要开发者决定的事项

- 真实排序模型仍维持“许可与质量阻塞”。未来若取得明确授权的权重或数据，再恢复该门禁，不阻塞 P3B。
- WinUI 3 的具体视觉语言与安装部署方式在配置服务稳定后再冻结；当前不作永久 UI 决策。
