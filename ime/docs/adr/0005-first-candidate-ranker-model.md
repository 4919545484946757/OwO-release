# ADR-0005：首个真实候选排序模型与运行时预评估

- 状态：提议，许可与质量门禁未解除
- 日期：2026-08-01

## 背景

P3A 已完成独立 ModelHost、确定性 Mock Backend、异步候选增量更新和 Windows 11 TSF 实机门禁。总控方案将 `UER Chinese RoBERTa L4-H256` 列为首个候选排序模型，但模型格式、评分语义、运行时、量化和再分发许可尚未确认。

本 ADR 只记录下载前的可审计预评估，不授权下载、转换或分发模型。

## 来源核验

| 项目 | 预评估结果 |
| --- | --- |
| 官方模型页 | `https://huggingface.co/uer/chinese_roberta_L-4_H-256` |
| 上游模型库 | `https://github.com/dbiir/UER-py/wiki/预训练模型仓库` |
| 架构 | BERT/RoBERTa 兼容编码器，4 层、隐藏维度 256；模型卡把该规格称为 RoBERTa-Mini |
| 训练目标 | Masked Language Modeling（MLM） |
| 训练语料 | 模型卡声明为 CLUECorpusSmall |
| 当前文件 | `config.json`、约 35.2 MB PyTorch/Flax 权重、约 57.7 MB TensorFlow 权重、约 110 kB `vocab.txt`；仓库总显示约 128 MB |
| 模型版本 | 下载前必须解析并固定完整 Hugging Face commit SHA；不得使用浮动 `main` |
| 文件完整性 | 下载后逐文件记录 SHA-256；当前尚无本地文件，因此不填写推测哈希 |

## 许可结论

1. UER-py **代码仓库**明确采用 Apache-2.0。
2. Hugging Face **模型仓库**当前模型卡未声明许可证，文件列表也没有独立 `LICENSE`。
3. 模型卡声明训练语料为 CLUECorpusSmall。CLUECorpus2020 仓库本身显示 MIT，但其 README 对语料申请包含用途说明和“不向第三方提供”的承诺，且语料由多个不同来源组成。
4. 因此不能从 UER-py 代码许可证推导出模型权重、词表和训练语料均可由 OwO 再分发。

门禁结论：可在开发者主动取得的本地副本上做隔离技术验证，但在权利人给出明确权重再分发许可或项目完成独立法律审查前，OwO 不下载到源码树、不提交、不打包、不镜像该模型。此结论不是法律意见。

## 任务适配结论

该检查点是预训练 MLM，不是已经微调好的候选排序器，也没有候选相关性分类头。它不能直接满足 `ICandidateRanker` 的“输入上下文与候选，输出每个候选的可比较分数”契约。

可验证的两条评分路线：

1. **伪似然基线**：把候选放入上下文，对候选字符逐个 Mask 并累计对数概率。无需训练，但一个候选通常需要多次前向推理，难以满足 50 ms 增量预算，只适合作为离线质量基线。
2. **排序头微调**：在 `[上下文, 候选]` 输入上增加 sequence-classification/ranking head，一批候选一次推理。在线延迟更可控，但必须先定义训练数据、标签、隐私边界和派生权重许可证。

首选路线是先用伪似然实现离线正确性对照，再决定是否建立可分发的排序微调数据集；不得把随机初始化分类头或 `[CLS]` 向量范数冒充排序质量。

## 运行时预评估

推荐首轮使用 ONNX Runtime C/C++ CPU Execution Provider，仅加载到独立 ModelHost：

- Hugging Face Optimum 官方支持 BERT 架构导出为 ONNX，并可在导出时执行数值校验。
- CPU 路径硬件覆盖最广，适合作为 Windows 11 x64 可重复基线；不会把 Python、PyTorch 或 Transformers 引入发布运行时。
- 先保留 FP32 模型验证数值，再评估动态 INT8；量化前后必须比较排序一致率与分数误差。
- DirectML 只作为第二阶段对照。官方要求 DirectX 12，且不支持 ORT memory pattern、并行 execution mode 或同一 session 的多线程 `Run`；小模型的 GPU 调度和上传开销可能抵消收益。
- 模型输入先限制为长度 64、最多 8 个候选的批处理；真实上限由基准决定，不冻结为公共协议。
- 首个内部适配契约使用 ONNX opset 17；三个输入分别为 `input_ids`、`attention_mask`、`token_type_ids`，类型均为 `int64`、形状为 `[dynamic_batch, 64]`；输出为 `logits`，类型 `float32`、形状 `[dynamic_batch, 1]`。允许的 opset 校验范围为 13～20，超出 DirectML 已核验上限的模型失败封闭。

ONNX Runtime 是新增的重量级本地依赖，只能进入 ModelHost，不得链接进 TSF DLL 或 Core Service。技术夹具门禁已通过；依赖方案最终采用 ADR-0006 固定的官方 NuGet，而非版本落后的 vcpkg 端口。

## 技术验证门禁

在许可允许的开发者本地模型副本上，按以下顺序执行：

1. 固定完整上游 commit、原始文件 URL 与 SHA-256，并保存许可证快照或“未声明”证据。
2. 在隔离的构建工具环境中导出 FP32 ONNX，关闭 `trust_remote_code`，校验 ONNX 与原始 PyTorch 输出。
3. 对 ONNX 文件执行结构检查并记录 opset、输入输出名、动态维度和外部数据文件。
4. ModelHost CPU 加载及固定夹具推理；验证损坏模型、缺失词表、不支持 opset、超长输入、取消、超时与进程退出。
5. 报告模型与运行时包体、冷启动、首次推理、稳态 P50/P95/P99、峰值内存、CPU 占用和取消延迟。
6. FP32 通过后才评估 INT8；DirectML 仅在 CPU 不满足预算时进入对照。
7. 使用独立排序质量集比较基础排序、伪似然基线和后续微调头；没有质量提升时不得默认启用真实模型。

## 当前决策

- 保持 P3A 的 Mock Backend 为默认且唯一可分发后端。
- 将 UER RoBERTa-Mini 标记为“技术候选，许可阻塞，任务头缺失”。
- 不修改公共 IPC；不下载模型。ORT 运行时与许可文件仅在显式构建开关下部署。
- ModelHost 可执行资产 manifest 只接受 `candidate-ranking`，原始 `masked-lm` 检查点不能直接进入排序后端。
- 已建立 manifest、WordPiece、BERT pair 张量、推理 session、ONNX 元数据比较契约和实际 ORT CPU 适配器；在许可未解除前只能使用项目合成夹具或开发者提供的已授权本地模型。

## 2026-08-01 替代权重复核

在 UER 权重许可仍不明确后，进一步复核了两个公开候选：

| 候选 | 明示许可 | 任务与格式 | 体积/结构 | 结论 |
| --- | --- | --- | --- | --- |
| `BAAI/bge-reranker-base` | Hugging Face 模型卡标记 MIT | 中英 cross-encoder，`AutoModelForSequenceClassification` 输出单个 relevance logit；仓库提供 ONNX 用法 | `model.safetensors` 约 1.11 GB，整个仓库约 3.36 GB | 许可与排序头清晰，但远超轻量输入法排序器的首发预算，且训练目标是查询—文档相关性，不等同拼音候选选择；不下载。 |
| `hfl/rbt3` | Hugging Face 模型卡标记 Apache-2.0 | 3 层中文 RoBERTa whole-word masking，使用 `AutoModelForMaskedLM` | PyTorch 权重约 156 MB | 许可和体积优于 UER/BGE，但没有候选排序头；必须使用许可清晰的输入法数据另行训练，不能直接接入；不下载。 |

`bge-reranker-base` 的模型卡明确说明 reranker 接收 query/document pair，以交叉熵训练并输出未限定范围的相关性分数；这证明它满足通用排序张量语义，却不能证明对同音候选排序有效。未经输入法专用离线准确率和误排序回归，不因其 C-MTEB 成绩而采用。

当前仍无同时满足以下条件的现成权重：

1. 权重本身允许再分发和商业使用；
2. 已包含候选排序任务头；
3. 面向中文输入法上下文与同音/近音候选训练或有可信验证；
4. Windows CPU 的包体、冷启动、峰值内存和 p95 延迟符合输入法增量更新预算。

因此维持许可门禁，不下载任何替代权重。下一可逆步骤是先定义具备来源许可、训练/验证隔离和隐私边界的候选排序数据规范，再决定采用 Apache-2.0 的 RBT3 微调，还是训练更小的自有 backbone；该决定不得由通用检索榜单代替。

## 回滚

删除本 ADR 即可；当前代码、协议、构建和 Mock Backend 均未依赖该模型或 ONNX Runtime。
