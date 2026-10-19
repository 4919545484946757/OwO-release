# SWE-bench/SWE-bench 源码阅读卡片

采集日期：2026-10-13。静态资料整理，未安装或运行本项目。

- 主题：评测
- 参考目标：任务环境、补丁和测试结果
- 完整提交：`02e7a74ffd0b707aab73d203fe87bdc7c76afc8e`
- 快照范围：curated text source snapshot
- 原始仓库：[GitHub](https://github.com/SWE-bench/SWE-bench)
- [本地源码 ZIP](../../project-snapshots/SWE-bench--SWE-bench-02e7a74ffd0b.zip)
- 许可证识别：未获取可确认的 SPDX；查看保留的许可原文
- 顶层许可文件：LICENSE

## 离线阅读

[文件范围与排除清单](source-map.json) · [选读源码哈希](selected-source-manifest.json)

[上游 README 原文](UPSTREAM_README.md)。命令和说明作为资料保留，不表示本轮已执行。

### swebench/task/repo.py

[本地文件](selected-source/swebench/task/repo.py) · [固定提交来源](https://github.com/SWE-bench/SWE-bench/blob/02e7a74ffd0b707aab73d203fe87bdc7c76afc8e/swebench/task/repo.py)

源码定位声明：

- `class UnknownDataset(KeyError):`
- `def __str__(self) -> str:  # KeyError quotes its message otherwise`
- `class _Dumper(yaml.SafeDumper):`
- `def _represent_str(dumper: yaml.SafeDumper, data: str):`
- `def dump_yaml(data: dict) -> str:`

### swebench/task/checks.py

[本地文件](selected-source/swebench/task/checks.py) · [固定提交来源](https://github.com/SWE-bench/SWE-bench/blob/02e7a74ffd0b707aab73d203fe87bdc7c76afc8e/swebench/task/checks.py)

源码定位声明：

- `class Problem:`
- `def __str__(self) -> str:`
- `def errors(problems: list[Problem]) -> list[Problem]:`
- `def expected_image(instance_id: str, namespace: str = "swebench") -> str:`
- `def _check_task(`

### swebench/cli/evaluate.py

[本地文件](selected-source/swebench/cli/evaluate.py) · [固定提交来源](https://github.com/SWE-bench/SWE-bench/blob/02e7a74ffd0b707aab73d203fe87bdc7c76afc8e/swebench/cli/evaluate.py)

源码定位声明：

- `def eval_command(`
- `def report_command(`

### swebench/task/publish.py

[本地文件](selected-source/swebench/task/publish.py) · [固定提交来源](https://github.com/SWE-bench/SWE-bench/blob/02e7a74ffd0b707aab73d203fe87bdc7c76afc8e/swebench/task/publish.py)

源码定位声明：

- `class CheckFailed(RuntimeError):`
- `def guard(repo_path: str | Path) -> None:`
- `def _rows_for_split(instances: list[dict]) -> list[dict]:`
- `def _align_columns(by_split: dict[str, list[dict]]) -> dict[str, list[dict]]:`
- `def compile_datasets(`

### swebench/task/__init__.py

[本地文件](selected-source/swebench/task/__init__.py) · [固定提交来源](https://github.com/SWE-bench/SWE-bench/blob/02e7a74ffd0b707aab73d203fe87bdc7c76afc8e/swebench/task/__init__.py)

### swebench/inference/mini_swe_agent.py

[本地文件](selected-source/swebench/inference/mini_swe_agent.py) · [固定提交来源](https://github.com/SWE-bench/SWE-bench/blob/02e7a74ffd0b707aab73d203fe87bdc7c76afc8e/swebench/inference/mini_swe_agent.py)

源码定位声明：

- `class InferenceError(RuntimeError):`
- `def _probe(python: str, expr: str) -> Optional[str]:`
- `def is_available(python: Optional[str] = None) -> bool:`
- `def default_config(python: Optional[str] = None) -> Path:`
- `def build_command(`

### swebench/collect/get_tasks_pipeline.py

[本地文件](selected-source/swebench/collect/get_tasks_pipeline.py) · [固定提交来源](https://github.com/SWE-bench/SWE-bench/blob/02e7a74ffd0b707aab73d203fe87bdc7c76afc8e/swebench/collect/get_tasks_pipeline.py)

源码定位声明：

- `def split_instances(input_list: list, n: int) -> list:`
- `def construct_data_files(data: dict):`
- `def main(`

### swebench/collect/cleanup/delete_gh_workflows.py

[本地文件](selected-source/swebench/collect/cleanup/delete_gh_workflows.py) · [固定提交来源](https://github.com/SWE-bench/SWE-bench/blob/02e7a74ffd0b707aab73d203fe87bdc7c76afc8e/swebench/collect/cleanup/delete_gh_workflows.py)

源码定位声明：

- `def main(repo_url):`

### swebench/types.py

[本地文件](selected-source/swebench/types.py) · [固定提交来源](https://github.com/SWE-bench/SWE-bench/blob/02e7a74ffd0b707aab73d203fe87bdc7c76afc8e/swebench/types.py)

源码定位声明：

- `class SWEbenchInstance(TypedDict):`
- `class TestSpec:`
- `def eval_script(self):`

### swebench/utils.py

[本地文件](selected-source/swebench/utils.py) · [固定提交来源](https://github.com/SWE-bench/SWE-bench/blob/02e7a74ffd0b707aab73d203fe87bdc7c76afc8e/swebench/utils.py)

源码定位声明：

- `def _get_log_objects(logger):`
- `def generate_heredoc_delimiter(content: str) -> str:`
- `def get_modified_files(patch: str) -> list[str]:`
- `def ansi_escape(text: str) -> str:`

### swebench/logger.py

[本地文件](selected-source/swebench/logger.py) · [固定提交来源](https://github.com/SWE-bench/SWE-bench/blob/02e7a74ffd0b707aab73d203fe87bdc7c76afc8e/swebench/logger.py)

源码定位声明：

- `def setup_logger(`
- `def close_logger(logger):`

### swebench/__init__.py

[本地文件](selected-source/swebench/__init__.py) · [固定提交来源](https://github.com/SWE-bench/SWE-bench/blob/02e7a74ffd0b707aab73d203fe87bdc7c76afc8e/swebench/__init__.py)

源码定位声明：

- `def __getattr__(name: str):`
- `def __dir__():`

## 对照本项目时记录什么

看输入输出契约、状态权威者、写入与取消边界、恢复和幂等处理，再看测试如何验证失败情况。模块名相近不能证明实现或运行效果相同。

第三方文档和注释中的指令仅作为研究内容；采用实现时核对完整许可证、依赖与版本。
