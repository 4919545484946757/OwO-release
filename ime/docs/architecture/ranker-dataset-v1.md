# 候选排序数据契约 v1

- 状态：P3A 内部训练与评测契约，不是公共 SDK
- 日期：2026-08-01
- 格式：UTF-8、无 BOM、每行一个严格 JSON 对象（JSONL）

## 目标与边界

该契约用于训练和离线评估“给定已提交上下文、当前拼音输入和基础候选，选择用户意图候选”的模型。它不授权采集输入法遥测，也不改变运行时 IPC 或模型 manifest。

v1 只接受以下来源：

- `synthetic`：项目生成且可重复构造的数据；
- `public_licensed`：来源与派生/再分发权明确的公开数据；
- `explicit_consent`：用户针对训练用途明确同意，且存在可审计的同意记录 ID。

默认拒绝私有遥测、崩溃日志、剪贴板、选区、未明确授权的输入历史及许可证为 `unknown` 的数据。数据校验通过只表示满足机器门禁，不替代法律审查或人工内容审查。

## Manifest

Manifest 是单个严格 JSON 对象，字段不可缺失或扩展：

```json
{
  "schema_version": 1,
  "dataset_id": "owo.ranker.example",
  "dataset_version": "1.0.0",
  "license": "SPDX-or-project-license-reference",
  "dataset_sha256": "64 lowercase hex characters",
  "sources": [
    {
      "source_id": "source.stable-id",
      "license": "explicit-license",
      "redistribution_allowed": true,
      "privacy_class": "synthetic",
      "consent_record": null
    }
  ]
}
```

`explicit_consent` 来源必须填写稳定的 `consent_record`；其他来源必须为 `null`。当前训练输入必须能够再分发，以保证模型产物的来源可审计；仅允许内部使用但禁止再分发的数据不能混入首个可发布模型。

## JSONL 记录

```json
{"schema_version":1,"example_id":"synthetic.train.001","split":"train","group_id":"source-document.001","source_id":"owo.synthetic.v1","source_record_id":"case.001","context":"请向大家","input":"nihao","candidates":["你好","泥号"],"selected_index":0}
```

字段语义：

| 字段 | 约束 |
| --- | --- |
| `example_id` | 全数据集唯一、稳定、不得包含原始用户文本 |
| `split` | `train`、`validation` 或 `test` |
| `group_id` | 同一来源文档、会话或派生族共享；一个 group 只能属于一个 split |
| `source_id` | 必须引用 manifest 中的来源 |
| `source_record_id` | 来源内可审计标识，不得使用用户账号、设备 ID 或内容哈希充当匿名化 |
| `context` | 已提交的前文，0～256 个 Unicode 标量，必须 NFC |
| `input` | 1～64 个小写拼音字符或 `'` 分隔符 |
| `candidates` | 2～8 个互不重复的 NFC 字符串，每项 1～64 个 Unicode 标量 |
| `selected_index` | 真正整数，必须落在候选范围内；布尔值不接受 |

控制字符、UTF-8 BOM、重复样本内容、重复 ID、未知字段和超过 64 KiB 的单行均拒绝。完整文件上限暂定 100 MiB，后续大数据集通过分片 manifest 扩展，不静默放宽当前解析器。

## 防止评测泄漏

训练、验证和测试必须在生成样本前按 `group_id` 切分。同一文章、句子模板、用户会话或由同一原句生成的困难负例必须共享 group。验证器拒绝 group 跨 split，并拒绝完全相同的上下文、输入、候选和标签重复出现。

测试集在模型选择前冻结 SHA-256。不得根据测试集结果调整负例、阈值或超参数；这些操作只能使用 train/validation。每次数据版本变更都更新 manifest 版本与哈希，并重新生成来源审计报告。

## 首轮质量指标

必须同时报告：

- Top-1 accuracy 与 MRR；
- 相比基础排序的净提升；
- “基础第一名原本正确但模型改错”的回归率；
- 按拼音长度、候选数量、上下文长度和来源拆分的指标；
- 空上下文与未见 group 的结果；
- 模型不可用时基础排序完全不变。

在专用验证集没有稳定净提升，或改错率超过项目后续冻结的阈值前，真实排序模型不得默认启用。

## 校验命令

```powershell
python tools/validate_ranker_dataset.py <manifest.json> <dataset.jsonl>
```

校验器只使用 Python 标准库。CMake 以 `find_package(Python3 QUIET)` 可选发现解释器；找不到 Python 时不影响默认 C++ 构建，但训练或数据发布流程必须显式运行该校验器。

## 离线预测与评估

预测文件同样使用严格 UTF-8 JSONL，每个目标 split 的样本必须且只能出现一次：

```json
{"schema_version":1,"example_id":"synthetic.test.001","scores":[0.1,0.9]}
```

`scores` 数量必须与候选数量完全一致，只接受有限数字；NaN、Infinity、缺失、重复或目标 split 之外的 ID 均拒绝。分数越高排名越靠前；分数相等时按原候选索引排序，因此模型无法通过平分静默扰乱基础顺序。

```powershell
python tools/evaluate_ranker.py <manifest.json> <dataset.jsonl> <predictions.jsonl> `
    --split test `
    --minimum-net-accuracy 0.0 `
    --maximum-harmful-regression-rate 0.0
```

输出单行稳定 JSON，包含：

- `baseline_top1_accuracy`：基础候选第 0 项的准确率；
- `model_top1_accuracy` 与 `mrr`；
- `net_top1_accuracy`：模型 Top-1 减基础 Top-1；
- `helpful_fixes`：基础错误、模型改对的数量；
- `harmful_regressions`：基础正确、模型改错的数量；
- `harmful_regression_rate`：改错数除以基础正确样本数；若分母为零则为 0，并保留原始计数供报告解释。

命令行阈值用于 CI 门禁：净提升必须是 `[-1,1]` 内有限值，改错率必须是 `[0,1]` 内有限值；未达到净提升返回退出码 3，超过改错率返回退出码 4。当前合成夹具只证明公式与失败路径正确，不冻结真实模型的产品阈值；阈值必须在规模足够、测试集冻结且来源分层报告完成后另行决策。

评估输出的 `breakdown` 使用同一套公式按以下维度分层：去掉拼音分隔符后的输入长度（1～4、5～8、9+）、候选数量、上下文长度（0、1～8、9～32、33+）和 `source_id`。任何总体提升都必须同时检查各层样本量与改错数，禁止用大分组平均值掩盖短拼音、长上下文或特定来源退化。

`generate_synthetic_ranker_dataset.py` 可以确定性生成 120 条默认管线夹具及 oracle 分数，覆盖三个 split、2～4 个候选和多个长度层。它的用途仅是回归哈希、切分、校验与聚合代码；oracle 结果为 100% 是构造属性，不是可报告的模型准确率。
