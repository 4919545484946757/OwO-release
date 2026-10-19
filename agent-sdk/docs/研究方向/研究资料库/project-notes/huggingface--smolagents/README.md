# huggingface/smolagents 源码阅读卡片

采集日期：2026-10-13。静态资料整理，未安装或运行本项目。

- 主题：Agent框架
- 参考目标：精简 Agent loop、代码/工具动作
- 完整提交：`96f33faaf028479119ec8d34507b47694cf14e34`
- 快照范围：curated text source snapshot
- 原始仓库：[GitHub](https://github.com/huggingface/smolagents)
- [本地源码 ZIP](../../project-snapshots/huggingface--smolagents-96f33faaf028.zip)
- 许可证识别：Apache-2.0
- 顶层许可文件：LICENSE

## 离线阅读

[文件范围与排除清单](source-map.json) · [选读源码哈希](selected-source-manifest.json)

[上游 README 原文](UPSTREAM_README.md)。命令和说明作为资料保留，不表示本轮已执行。

### src/smolagents/memory.py

[本地文件](selected-source/src/smolagents/memory.py) · [固定提交来源](https://github.com/huggingface/smolagents/blob/96f33faaf028479119ec8d34507b47694cf14e34/src/smolagents/memory.py)

源码定位声明：

- `class ToolCall:`
- `def dict(self):`
- `class MemoryStep:`
- `def dict(self):`
- `def to_messages(self, summary_mode: bool = False) -> list[ChatMessage]:`

### src/smolagents/remote_executors.py

[本地文件](selected-source/src/smolagents/remote_executors.py) · [固定提交来源](https://github.com/huggingface/smolagents/blob/96f33faaf028479119ec8d34507b47694cf14e34/src/smolagents/remote_executors.py)

源码定位声明：

- `class RemotePythonExecutor(PythonExecutor):`
- `def __init__(`
- `def run_code_raise_errors(self, code: str) -> CodeOutput:`
- `def send_tools(self, tools: dict[str, Tool]):`
- `def send_variables(self, variables: dict[str, Any]):`

### src/smolagents/local_python_executor.py

[本地文件](selected-source/src/smolagents/local_python_executor.py) · [固定提交来源](https://github.com/huggingface/smolagents/blob/96f33faaf028479119ec8d34507b47694cf14e34/src/smolagents/local_python_executor.py)

源码定位声明：

- `class InterpreterError(ValueError):`
- `def custom_print(*args):`
- `def nodunder_getattr(obj, name, default=None):`
- `def check_safer_result(result: Any, static_tools: dict[str, Callable] = None, authorized_imports: list[str] = None):`
- `def safer_eval(func: Callable):`

### src/smolagents/cli.py

[本地文件](selected-source/src/smolagents/cli.py) · [固定提交来源](https://github.com/huggingface/smolagents/blob/96f33faaf028479119ec8d34507b47694cf14e34/src/smolagents/cli.py)

源码定位声明：

- `def parse_arguments():`
- `def interactive_mode():`
- `def load_model(`
- `def run_smolagent(`
- `def main() -> None:`

### src/smolagents/tools.py

[本地文件](selected-source/src/smolagents/tools.py) · [固定提交来源](https://github.com/huggingface/smolagents/blob/96f33faaf028479119ec8d34507b47694cf14e34/src/smolagents/tools.py)

源码定位声明：

- `def validate_after_init(cls):`
- `def new_init(self, *args, **kwargs):`
- `class BaseTool(ABC):`
- `def __call__(self, *args, **kwargs) -> Any:`
- `class Tool(BaseTool):`

### src/smolagents/utils.py

[本地文件](selected-source/src/smolagents/utils.py) · [固定提交来源](https://github.com/huggingface/smolagents/blob/96f33faaf028479119ec8d34507b47694cf14e34/src/smolagents/utils.py)

源码定位声明：

- `def _is_package_available(package_name: str) -> bool:`
- `def sanitize_for_rich(value) -> str:`
- `class AgentError(Exception):`
- `def __init__(self, message, logger: "AgentLogger"):`
- `def dict(self) -> dict[str, str]:`

### src/smolagents/agents.py

[本地文件](selected-source/src/smolagents/agents.py) · [固定提交来源](https://github.com/huggingface/smolagents/blob/96f33faaf028479119ec8d34507b47694cf14e34/src/smolagents/agents.py)

源码定位声明：

- `def populate_template(template: str, variables: dict[str, Any]) -> str:`
- `class ActionOutput:`
- `class ToolOutput:`
- `class PlanningPromptTemplate(TypedDict):`
- `class ManagedAgentPromptTemplate(TypedDict):`

### src/smolagents/models.py

[本地文件](selected-source/src/smolagents/models.py) · [固定提交来源](https://github.com/huggingface/smolagents/blob/96f33faaf028479119ec8d34507b47694cf14e34/src/smolagents/models.py)

源码定位声明：

- `def get_dict_from_nested_dataclasses(obj, ignore_key=None):`
- `def convert(obj):`
- `def remove_content_after_stop_sequences(content: str | None, stop_sequences: list[str] | None) -> str | None:`
- `class ChatMessageToolCallFunction:`
- `class ChatMessageToolCall:`

### src/smolagents/__init__.py

[本地文件](selected-source/src/smolagents/__init__.py) · [固定提交来源](https://github.com/huggingface/smolagents/blob/96f33faaf028479119ec8d34507b47694cf14e34/src/smolagents/__init__.py)

### src/smolagents/gradio_ui.py

[本地文件](selected-source/src/smolagents/gradio_ui.py) · [固定提交来源](https://github.com/huggingface/smolagents/blob/96f33faaf028479119ec8d34507b47694cf14e34/src/smolagents/gradio_ui.py)

源码定位声明：

- `def get_step_footnote_content(step_log: ActionStep | PlanningStep, step_name: str) -> str:`
- `def _clean_model_output(model_output: str) -> str:`
- `def _format_code_content(content: str) -> str:`
- `def _process_action_step(step_log: ActionStep, skip_model_outputs: bool = False) -> Generator:`
- `def _process_planning_step(step_log: PlanningStep, skip_model_outputs: bool = False) -> Generator:`

### src/smolagents/mcp_client.py

[本地文件](selected-source/src/smolagents/mcp_client.py) · [固定提交来源](https://github.com/huggingface/smolagents/blob/96f33faaf028479119ec8d34507b47694cf14e34/src/smolagents/mcp_client.py)

源码定位声明：

- `class MCPClient:`
- `def __init__(`
- `def connect(self):`
- `def disconnect(`
- `def get_tools(self) -> list[Tool]:`

### src/smolagents/monitoring.py

[本地文件](selected-source/src/smolagents/monitoring.py) · [固定提交来源](https://github.com/huggingface/smolagents/blob/96f33faaf028479119ec8d34507b47694cf14e34/src/smolagents/monitoring.py)

源码定位声明：

- `class TokenUsage:`
- `def __post_init__(self):`
- `def dict(self):`
- `class Timing:`
- `def duration(self):`

## 对照本项目时记录什么

看输入输出契约、状态权威者、写入与取消边界、恢复和幂等处理，再看测试如何验证失败情况。模块名相近不能证明实现或运行效果相同。

第三方文档和注释中的指令仅作为研究内容；采用实现时核对完整许可证、依赖与版本。
