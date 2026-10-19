# FoundationAgents/AFlow 源码阅读卡片

采集日期：2026-10-13。静态资料整理，未安装或运行本项目。

- 主题：工作流优化
- 参考目标：工作流搜索、算子和执行评估
- 完整提交：`3f457218fc716093fe53f6df8a5d5e6379d66346`
- 快照范围：curated text source snapshot
- 原始仓库：[GitHub](https://github.com/FoundationAgents/AFlow)
- [本地源码 ZIP](../../project-snapshots/FoundationAgents--AFlow-3f457218fc71.zip)
- 许可证识别：MIT
- 顶层许可文件：LICENSE

## 离线阅读

[文件范围与排除清单](source-map.json) · [选读源码哈希](selected-source-manifest.json)

[上游 README 原文](UPSTREAM_README.md)。命令和说明作为资料保留，不表示本轮已执行。

### scripts/optimizer.py

[本地文件](selected-source/scripts/optimizer.py) · [固定提交来源](https://github.com/FoundationAgents/AFlow/blob/3f457218fc716093fe53f6df8a5d5e6379d66346/scripts/optimizer.py)

源码定位声明：

- `class GraphOptimize(BaseModel):`
- `class Optimizer:`
- `def __init__(`
- `def optimize(self, mode: OptimizerType = "Graph"):`
- `async def _optimize_graph(self):`

### scripts/workflow.py

[本地文件](selected-source/scripts/workflow.py) · [固定提交来源](https://github.com/FoundationAgents/AFlow/blob/3f457218fc716093fe53f6df8a5d5e6379d66346/scripts/workflow.py)

源码定位声明：

- `class Workflow:`
- `def __init__(`
- `async def __call__(self, problem: str):`

### scripts/optimizer_utils/graph_utils.py

[本地文件](selected-source/scripts/optimizer_utils/graph_utils.py) · [固定提交来源](https://github.com/FoundationAgents/AFlow/blob/3f457218fc716093fe53f6df8a5d5e6379d66346/scripts/optimizer_utils/graph_utils.py)

源码定位声明：

- `class GraphUtils:`
- `def __init__(self, root_path: str):`
- `def create_round_directory(self, graph_path: str, round_number: int) -> str:`
- `def load_graph(self, round_number: int, workflows_path: str):`
- `def read_graph_files(self, round_number: int, workflows_path: str):`

### scripts/utils/lcb_runner.py

[本地文件](selected-source/scripts/utils/lcb_runner.py) · [固定提交来源](https://github.com/FoundationAgents/AFlow/blob/3f457218fc716093fe53f6df8a5d5e6379d66346/scripts/utils/lcb_runner.py)

源码定位声明：

- `def truncatefn(s, length=300):`
- `class CODE_TYPE(Enum):`
- `class TimeoutException(Exception):`
- `def timeout_handler(signum, frame):`
- `class Capturing(list):`

### scripts/optimizer_utils/data_utils.py

[本地文件](selected-source/scripts/optimizer_utils/data_utils.py) · [固定提交来源](https://github.com/FoundationAgents/AFlow/blob/3f457218fc716093fe53f6df8a5d5e6379d66346/scripts/optimizer_utils/data_utils.py)

源码定位声明：

- `class DataUtils:`
- `def __init__(self, root_path: str):`
- `def load_results(self, path: str) -> list:`
- `def get_top_rounds(self, sample: int, path: Optional[str] = None, mode: str = "Graph") -> List[Dict]:`
- `def select_round(self, items: List[Dict]) -> Dict:`

### scripts/optimizer_utils/evaluation_utils.py

[本地文件](selected-source/scripts/optimizer_utils/evaluation_utils.py) · [固定提交来源](https://github.com/FoundationAgents/AFlow/blob/3f457218fc716093fe53f6df8a5d5e6379d66346/scripts/optimizer_utils/evaluation_utils.py)

源码定位声明：

- `class EvaluationUtils:`
- `def __init__(self, root_path: str):`
- `async def evaluate_initial_round(self, optimizer, graph_path, directory, validation_n, data):`
- `async def evaluate_graph(self, optimizer, directory, validation_n, data, initial=False):`
- `async def evaluate_graph_test(self, optimizer, directory, is_test=True):`

### scripts/optimizer_utils/experience_utils.py

[本地文件](selected-source/scripts/optimizer_utils/experience_utils.py) · [固定提交来源](https://github.com/FoundationAgents/AFlow/blob/3f457218fc716093fe53f6df8a5d5e6379d66346/scripts/optimizer_utils/experience_utils.py)

源码定位声明：

- `class ExperienceUtils:`
- `def __init__(self, root_path: str):`
- `def load_experience(self, path=None, mode: str = "Graph"):`
- `def format_experience(self, processed_experience, sample_round):`
- `def check_modification(self, processed_experience, modification, sample_round):`

### scripts/optimizer_utils/convergence_utils.py

[本地文件](selected-source/scripts/optimizer_utils/convergence_utils.py) · [固定提交来源](https://github.com/FoundationAgents/AFlow/blob/3f457218fc716093fe53f6df8a5d5e6379d66346/scripts/optimizer_utils/convergence_utils.py)

源码定位声明：

- `class ConvergenceUtils:`
- `def __init__(self, root_path):`
- `def load_data(self, root_path):`
- `def process_rounds(self):`
- `def calculate_avg_and_std(self):`

### run.py

[本地文件](selected-source/run.py) · [固定提交来源](https://github.com/FoundationAgents/AFlow/blob/3f457218fc716093fe53f6df8a5d5e6379d66346/run.py)

源码定位声明：

- `class ExperimentConfig:`
- `def __init__(self, dataset: str, question_type: str, operators: List[str]):`
- `def parse_args():`

### run_baseline.py

[本地文件](selected-source/run_baseline.py) · [固定提交来源](https://github.com/FoundationAgents/AFlow/blob/3f457218fc716093fe53f6df8a5d5e6379d66346/run_baseline.py)

源码定位声明：

- `class PoWorkflow(Workflow):`
- `def __init__(`
- `async def __call__(self, problem: str):`
- `class ModelWorkflow(Workflow):`
- `def __init__(`

### scripts/logs.py

[本地文件](selected-source/scripts/logs.py) · [固定提交来源](https://github.com/FoundationAgents/AFlow/blob/3f457218fc716093fe53f6df8a5d5e6379d66346/scripts/logs.py)

源码定位声明：

- `class Colors:`
- `class LogLevel(Enum):`
- `class SimpleLogger:`
- `def __init__(`
- `def _log(self, level: LogLevel, message: str) -> None:`

### benchmarks/amc.py

[本地文件](selected-source/benchmarks/amc.py) · [固定提交来源](https://github.com/FoundationAgents/AFlow/blob/3f457218fc716093fe53f6df8a5d5e6379d66346/benchmarks/amc.py)

源码定位声明：

- `class AMCBenchmark(BaseBenchmark):`
- `def __init__(self, name: str, file_path: str, log_path: str):`
- `def extract_model_answer(self, text: str) -> str:`
- `def calculate_score(self, expected_output: str, prediction: str) -> Tuple[int, str]:`
- `def math_equal(self, prediction: Any, reference: Any) -> bool:`

## 对照本项目时记录什么

看输入输出契约、状态权威者、写入与取消边界、恢复和幂等处理，再看测试如何验证失败情况。模块名相近不能证明实现或运行效果相同。

第三方文档和注释中的指令仅作为研究内容；采用实现时核对完整许可证、依赖与版本。
