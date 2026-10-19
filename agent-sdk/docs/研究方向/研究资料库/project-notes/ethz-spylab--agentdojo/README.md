# ethz-spylab/agentdojo 源码阅读卡片

采集日期：2026-10-13。静态资料整理，未安装或运行本项目。

- 主题：安全评测
- 参考目标：工具注入、策略与攻击防御
- 完整提交：`089ed468cf3ed0322acc66b0211f26d9d90dbf60`
- 快照范围：curated text source snapshot
- 原始仓库：[GitHub](https://github.com/ethz-spylab/agentdojo)
- [本地源码 ZIP](../../project-snapshots/ethz-spylab--agentdojo-089ed468cf3e.zip)
- 许可证识别：未获取可确认的 SPDX；查看保留的许可原文
- 顶层许可文件：LICENSE

## 离线阅读

[文件范围与排除清单](source-map.json) · [选读源码哈希](selected-source-manifest.json)

[上游 README 原文](UPSTREAM_README.md)。命令和说明作为资料保留，不表示本轮已执行。

### src/agentdojo/base_tasks.py

[本地文件](selected-source/src/agentdojo/base_tasks.py) · [固定提交来源](https://github.com/ethz-spylab/agentdojo/blob/089ed468cf3ed0322acc66b0211f26d9d90dbf60/src/agentdojo/base_tasks.py)

源码定位声明：

- `class TaskDifficulty(enum.Enum):`
- `class BaseUserTask(abc.ABC, Generic[Env]):`
- `def init_environment(environment: Env) -> Env:`
- `def ground_truth(self, pre_environment: Env) -> list[FunctionCall]:`
- `def utility(`

### src/agentdojo/functions_runtime.py

[本地文件](selected-source/src/agentdojo/functions_runtime.py) · [固定提交来源](https://github.com/ethz-spylab/agentdojo/blob/089ed468cf3ed0322acc66b0211f26d9d90dbf60/src/agentdojo/functions_runtime.py)

源码定位声明：

- `class TaskEnvironment(BaseModel):`
- `class EmptyEnv(TaskEnvironment):`
- `class Depends:`
- `def extract_dep_from_env(self, env: BaseModel) -> BaseModel:`
- `def __repr__(self) -> str:`

### src/agentdojo/task_suite/__init__.py

[本地文件](selected-source/src/agentdojo/task_suite/__init__.py) · [固定提交来源](https://github.com/ethz-spylab/agentdojo/blob/089ed468cf3ed0322acc66b0211f26d9d90dbf60/src/agentdojo/task_suite/__init__.py)

### src/agentdojo/task_suite/task_suite.py

[本地文件](selected-source/src/agentdojo/task_suite/task_suite.py) · [固定提交来源](https://github.com/ethz-spylab/agentdojo/blob/089ed468cf3ed0322acc66b0211f26d9d90dbf60/src/agentdojo/task_suite/task_suite.py)

源码定位声明：

- `class InjectionVector(BaseModel):`
- `def get_version_compatible_items(`
- `def messages_to_dict(messages: Sequence[ChatMessage]) -> list[dict[str, str]]:`
- `def transform(msg: ChatMessage):`
- `def functions_stack_trace_from_messages(`

### src/agentdojo/task_suite/load_suites.py

[本地文件](selected-source/src/agentdojo/task_suite/load_suites.py) · [固定提交来源](https://github.com/ethz-spylab/agentdojo/blob/089ed468cf3ed0322acc66b0211f26d9d90dbf60/src/agentdojo/task_suite/load_suites.py)

源码定位声明：

- `def register_suite(suite: TaskSuite, version: str) -> None:`
- `def get_suites(benchmark_version: str) -> dict[str, TaskSuite]:`
- `def get_suite(benchmark_version: str, suite_name: str) -> TaskSuite:`

### src/agentdojo/task_suite/task_combinators.py

[本地文件](selected-source/src/agentdojo/task_suite/task_combinators.py) · [固定提交来源](https://github.com/ethz-spylab/agentdojo/blob/089ed468cf3ed0322acc66b0211f26d9d90dbf60/src/agentdojo/task_suite/task_combinators.py)

源码定位声明：

- `class TaskCombinator(Generic[TASK, ENV]):`
- `def __init__(self, task_suite: "TaskSuite"):`
- `def create_combined_task(`
- `def update_combined_task(`
- `def _create_combined_task(`

### src/agentdojo/types.py

[本地文件](selected-source/src/agentdojo/types.py) · [固定提交来源](https://github.com/ethz-spylab/agentdojo/blob/089ed468cf3ed0322acc66b0211f26d9d90dbf60/src/agentdojo/types.py)

源码定位声明：

- `class TextContentBlock(TypedDict, total=False):`
- `class ThinkingContentBlock(TypedDict, total=False):`
- `class RedactedThinkingContentBlock(TypedDict, total=False):`
- `def text_content_block_from_string(content: str) -> TextContentBlock:`
- `class ChatUserMessage(TypedDict, total=False):`

### src/agentdojo/models.py

[本地文件](selected-source/src/agentdojo/models.py) · [固定提交来源](https://github.com/ethz-spylab/agentdojo/blob/089ed468cf3ed0322acc66b0211f26d9d90dbf60/src/agentdojo/models.py)

源码定位声明：

- `class ModelsEnum(StrEnum):`

### src/agentdojo/logging.py

[本地文件](selected-source/src/agentdojo/logging.py) · [固定提交来源](https://github.com/ethz-spylab/agentdojo/blob/089ed468cf3ed0322acc66b0211f26d9d90dbf60/src/agentdojo/logging.py)

源码定位声明：

- `class Logger:`
- `def __enter__(self):`
- `def __exit__(self, exc_type, exc_value, traceback):`
- `def get():`
- `class NullLogger(Logger):`

### src/agentdojo/strenum.py

[本地文件](selected-source/src/agentdojo/strenum.py) · [固定提交来源](https://github.com/ethz-spylab/agentdojo/blob/089ed468cf3ed0322acc66b0211f26d9d90dbf60/src/agentdojo/strenum.py)

源码定位声明：

- `class StrEnum(str, enum.Enum):`
- `def __str__(self) -> str:`
- `def __repr__(self) -> str:`

### src/agentdojo/__init__.py

[本地文件](selected-source/src/agentdojo/__init__.py) · [固定提交来源](https://github.com/ethz-spylab/agentdojo/blob/089ed468cf3ed0322acc66b0211f26d9d90dbf60/src/agentdojo/__init__.py)

### src/agentdojo/ast_utils.py

[本地文件](selected-source/src/agentdojo/ast_utils.py) · [固定提交来源](https://github.com/ethz-spylab/agentdojo/blob/089ed468cf3ed0322acc66b0211f26d9d90dbf60/src/agentdojo/ast_utils.py)

源码定位声明：

- `class ASTParsingError(Exception): ...`
- `def create_python_function_from_tool_call(func_call_dict: FunctionCall) -> str:`
- `def parse_arg_value(arg_value: ast.expr | None) -> FunctionCallArgTypes:`
- `def parse_tool_call_from_ast(call: ast.expr) -> FunctionCall:`
- `def parse_tool_calls_from_python_function(model_output: str) -> list[FunctionCall]:`

## 对照本项目时记录什么

看输入输出契约、状态权威者、写入与取消边界、恢复和幂等处理，再看测试如何验证失败情况。模块名相近不能证明实现或运行效果相同。

第三方文档和注释中的指令仅作为研究内容；采用实现时核对完整许可证、依赖与版本。
