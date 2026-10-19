# stanfordnlp/dspy 源码阅读卡片

采集日期：2026-10-13。静态资料整理，未安装或运行本项目。

- 主题：工作流优化
- 参考目标：类型化模块、优化器、指标和轨迹
- 完整提交：`099982b3c3eec9fc4ec4796d49915af42367a253`
- 快照范围：curated text source snapshot
- 原始仓库：[GitHub](https://github.com/stanfordnlp/dspy)
- [本地源码 ZIP](../../project-snapshots/stanfordnlp--dspy-099982b3c3ee.zip)
- 许可证识别：未获取可确认的 SPDX；查看保留的许可原文
- 顶层许可文件：LICENSE

## 离线阅读

[文件范围与排除清单](source-map.json) · [选读源码哈希](selected-source-manifest.json)

[上游 README 原文](UPSTREAM_README.md)。命令和说明作为资料保留，不表示本轮已执行。

### dspy/evaluate/metrics.py

[本地文件](selected-source/dspy/evaluate/metrics.py) · [固定提交来源](https://github.com/stanfordnlp/dspy/blob/099982b3c3eec9fc4ec4796d49915af42367a253/dspy/evaluate/metrics.py)

源码定位声明：

- `def EM(prediction, answers_list):  # noqa: N802`
- `def F1(prediction, answers_list):  # noqa: N802`
- `def HotPotF1(prediction, answers_list):  # noqa: N802`
- `def normalize_text(s):`
- `def remove_articles(text):`

### dspy/evaluate/__init__.py

[本地文件](selected-source/dspy/evaluate/__init__.py) · [固定提交来源](https://github.com/stanfordnlp/dspy/blob/099982b3c3eec9fc4ec4796d49915af42367a253/dspy/evaluate/__init__.py)

### dspy/evaluate/evaluate.py

[本地文件](selected-source/dspy/evaluate/evaluate.py) · [固定提交来源](https://github.com/stanfordnlp/dspy/blob/099982b3c3eec9fc4ec4796d49915af42367a253/dspy/evaluate/evaluate.py)

源码定位声明：

- `def display(obj: Any):`
- `def HTML(x: str) -> str:  # noqa: N802`
- `class EvaluationResult(Prediction):`
- `def __init__(self, score: float, results: list[tuple["dspy.Example", "dspy.Example", Any]]):`
- `def __repr__(self):`

### dspy/primitives/runner.js

[本地文件](selected-source/dspy/primitives/runner.js) · [固定提交来源](https://github.com/stanfordnlp/dspy/blob/099982b3c3eec9fc4ec4796d49915af42367a253/dspy/primitives/runner.js)

源码定位声明：

- `def last_exception_args():`
- `class _DSPyFinalOutput(BaseException):`
- `def SUBMIT(output):`
- `def ${toolName}(${signature}):`
- `def SUBMIT(output):`

### dspy/evaluate/auto_evaluation.py

[本地文件](selected-source/dspy/evaluate/auto_evaluation.py) · [固定提交来源](https://github.com/stanfordnlp/dspy/blob/099982b3c3eec9fc4ec4796d49915af42367a253/dspy/evaluate/auto_evaluation.py)

源码定位声明：

- `class SemanticRecallPrecision(Signature):`
- `class DecompositionalSemanticRecallPrecision(Signature):`
- `def f1_score(precision, recall):`
- `class SemanticF1(Module):`
- `def __init__(self, threshold=0.66, decompositional=False):`

### dspy/teleprompt/copro_optimizer.py

[本地文件](selected-source/dspy/teleprompt/copro_optimizer.py) · [固定提交来源](https://github.com/stanfordnlp/dspy/blob/099982b3c3eec9fc4ec4796d49915af42367a253/dspy/teleprompt/copro_optimizer.py)

源码定位声明：

- `class BasicGenerateInstruction(Signature):`
- `class GenerateInstructionGivenAttempts(dspy.Signature):`
- `class COPRO(Teleprompter):`
- `def __init__(`
- `def _check_candidates_equal(self, candidate1, candidate2):`

### dspy/teleprompt/avatar_optimizer.py

[本地文件](selected-source/dspy/teleprompt/avatar_optimizer.py) · [固定提交来源](https://github.com/stanfordnlp/dspy/blob/099982b3c3eec9fc4ec4796d49915af42367a253/dspy/teleprompt/avatar_optimizer.py)

源码定位声明：

- `class EvalResult(BaseModel):`
- `class Comparator(dspy.Signature):`
- `class FeedbackBasedInstruction(dspy.Signature):`
- `class AvatarOptimizer(Teleprompter):`
- `def __init__(`

### dspy/teleprompt/mipro_optimizer_v2.py

[本地文件](selected-source/dspy/teleprompt/mipro_optimizer_v2.py) · [固定提交来源](https://github.com/stanfordnlp/dspy/blob/099982b3c3eec9fc4ec4796d49915af42367a253/dspy/teleprompt/mipro_optimizer_v2.py)

源码定位声明：

- `def _import_optuna():`
- `class MIPROv2(Teleprompter):`
- `def __init__(`
- `def compile(`
- `def _set_random_seeds(self, seed):`

### dspy/lm15.py

[本地文件](selected-source/dspy/lm15.py) · [固定提交来源](https://github.com/stanfordnlp/dspy/blob/099982b3c3eec9fc4ec4796d49915af42367a253/dspy/lm15.py)

源码定位声明：

- `class ModelSupport:`
- `def __post_init__(self) -> None:`
- `def over(self, other: "ModelSupport | None") -> "ModelSupport":`
- `class RegisteredProvider:`
- `def id(self) -> str:`

### dspy/__init__.py

[本地文件](selected-source/dspy/__init__.py) · [固定提交来源](https://github.com/stanfordnlp/dspy/blob/099982b3c3eec9fc4ec4796d49915af42367a253/dspy/__init__.py)

源码定位声明：

- `def __getattr__(name):`

### dspy/__metadata__.py

[本地文件](selected-source/dspy/__metadata__.py) · [固定提交来源](https://github.com/stanfordnlp/dspy/blob/099982b3c3eec9fc4ec4796d49915af42367a253/dspy/__metadata__.py)

### scripts/update_vendored_lm15.py

[本地文件](selected-source/scripts/update_vendored_lm15.py) · [固定提交来源](https://github.com/stanfordnlp/dspy/blob/099982b3c3eec9fc4ec4796d49915af42367a253/scripts/update_vendored_lm15.py)

源码定位声明：

- `def git(*args: str, cwd: Path = REPO_ROOT) -> str:`
- `def main() -> int:`

## 对照本项目时记录什么

看输入输出契约、状态权威者、写入与取消边界、恢复和幂等处理，再看测试如何验证失败情况。模块名相近不能证明实现或运行效果相同。

第三方文档和注释中的指令仅作为研究内容；采用实现时核对完整许可证、依赖与版本。
