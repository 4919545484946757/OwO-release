# pydantic/pydantic-ai 源码阅读卡片

采集日期：2026-10-13。静态资料整理，未安装或运行本项目。

- 主题：Agent框架
- 参考目标：类型化工具、输出验证与 durable 集成
- 完整提交：`0a28fc079668fa03040f0a3024f3bf1f1b63e819`
- 快照范围：curated text source snapshot
- 原始仓库：[GitHub](https://github.com/pydantic/pydantic-ai)
- [本地源码 ZIP](../../project-snapshots/pydantic--pydantic-ai-0a28fc079668.zip)
- 许可证识别：MIT
- 顶层许可文件：LICENSE

## 离线阅读

[文件范围与排除清单](source-map.json) · [选读源码哈希](selected-source-manifest.json)

[上游 README 原文](UPSTREAM_README.md)。命令和说明作为资料保留，不表示本轮已执行。

### src/pydantic_clai2/pydantic_clai2/runtime/tasks.py

[本地文件](selected-source/src/pydantic_clai2/pydantic_clai2/runtime/tasks.py) · [固定提交来源](https://github.com/pydantic/pydantic-ai/blob/0a28fc079668fa03040f0a3024f3bf1f1b63e819/src/pydantic_clai2/pydantic_clai2/runtime/tasks.py)

源码定位声明：

- `def specialists() -> dict[str, SubAgent[object]]:`
- `def task_tree(records: Sequence[DelegationTask]) -> list[tuple[int, DelegationTask]]:`
- `def visit(record: DelegationTask, depth: int) -> None:`
- `def task_row(event: AgentStreamEvent) -> Text | None:`
- `class Tasks:`

### src/pydantic_ai_harness/pydantic_ai_harness/subagents/_tasks.py

[本地文件](selected-source/src/pydantic_ai_harness/pydantic_ai_harness/subagents/_tasks.py) · [固定提交来源](https://github.com/pydantic/pydantic-ai/blob/0a28fc079668fa03040f0a3024f3bf1f1b63e819/src/pydantic_ai_harness/pydantic_ai_harness/subagents/_tasks.py)

源码定位声明：

- `class DelegationTask:`
- `def report(self) -> SystemPromptPart:`
- `class DelegationTaskEvent:`
- `class DelegationTasks:`
- `def __init__(`

### src/pydantic_clai2/pydantic_clai2/runtime/agent_instrumentation.py

[本地文件](selected-source/src/pydantic_clai2/pydantic_clai2/runtime/agent_instrumentation.py) · [固定提交来源](https://github.com/pydantic/pydantic-ai/blob/0a28fc079668fa03040f0a3024f3bf1f1b63e819/src/pydantic_clai2/pydantic_clai2/runtime/agent_instrumentation.py)

源码定位声明：

- `class _Claims:`
- `def current() -> InstrumentationSettings | bool:`
- `def tracer() -> Tracer | None:`
- `def claim(settings: InstrumentationSettings) -> None:`
- `def release(settings: InstrumentationSettings) -> None:`

### pydantic_ai_slim/pydantic_ai/_agent_graph.py

[本地文件](selected-source/pydantic_ai_slim/pydantic_ai/_agent_graph.py) · [固定提交来源](https://github.com/pydantic/pydantic-ai/blob/0a28fc079668fa03040f0a3024f3bf1f1b63e819/pydantic_ai_slim/pydantic_ai/_agent_graph.py)

源码定位声明：

- `def set_agent_graph_sleep(sleep_func: AgentGraphSleepFunc) -> Generator[None]:`
- `async def durable_sleep(seconds: float) -> None:`
- `async def _agent_graph_sleep(delay: float) -> None:`
- `async def _with_event_stream_buffer(`
- `async def _cancel_task(task: Task[Any]) -> None:`

### src/pydantic_clai2/pydantic_clai2/mcp/_runtime.py

[本地文件](selected-source/src/pydantic_clai2/pydantic_clai2/mcp/_runtime.py) · [固定提交来源](https://github.com/pydantic/pydantic-ai/blob/0a28fc079668fa03040f0a3024f3bf1f1b63e819/src/pydantic_clai2/pydantic_clai2/mcp/_runtime.py)

源码定位声明：

- `class ServerEntry:`
- `def origin(self) -> tuple[Source, Path | None, str]:`
- `class _Connection:`
- `def _shape(server: Server) -> Server:`
- `class MCPServers:`

### src/pydantic_clai2/pydantic_clai2/runtime/forks.py

[本地文件](selected-source/src/pydantic_clai2/pydantic_clai2/runtime/forks.py) · [固定提交来源](https://github.com/pydantic/pydantic-ai/blob/0a28fc079668fa03040f0a3024f3bf1f1b63e819/src/pydantic_clai2/pydantic_clai2/runtime/forks.py)

源码定位声明：

- `class ForkRecord:`
- `def tag(self) -> str:`
- `async def _ignore(event: HostEvent) -> None:`
- `def _activity_style(activity: str) -> str:`
- `def parse_fork_args(text: str) -> tuple[str | None, str]:`

### src/pydantic_clai2/pydantic_clai2/runtime/launch.py

[本地文件](selected-source/src/pydantic_clai2/pydantic_clai2/runtime/launch.py) · [固定提交来源](https://github.com/pydantic/pydantic-ai/blob/0a28fc079668fa03040f0a3024f3bf1f1b63e819/src/pydantic_clai2/pydantic_clai2/runtime/launch.py)

源码定位声明：

- `def launch_session_id(*, resume: str | None, session_id: str | None, fork: bool) -> str | None:`

### src/pydantic_clai2/pydantic_clai2/cli/agent_import.py

[本地文件](selected-source/src/pydantic_clai2/pydantic_clai2/cli/agent_import.py) · [固定提交来源](https://github.com/pydantic/pydantic-ai/blob/0a28fc079668fa03040f0a3024f3bf1f1b63e819/src/pydantic_clai2/pydantic_clai2/cli/agent_import.py)

源码定位声明：

- `def import_agent(path: str) -> AbstractAgent[None, object]:`

### src/pydantic_clai2/pydantic_clai2/runtime/__init__.py

[本地文件](selected-source/src/pydantic_clai2/pydantic_clai2/runtime/__init__.py) · [固定提交来源](https://github.com/pydantic/pydantic-ai/blob/0a28fc079668fa03040f0a3024f3bf1f1b63e819/src/pydantic_clai2/pydantic_clai2/runtime/__init__.py)

### src/pydantic_clai2/pydantic_clai2/runtime/_session.py

[本地文件](selected-source/src/pydantic_clai2/pydantic_clai2/runtime/_session.py) · [固定提交来源](https://github.com/pydantic/pydantic-ai/blob/0a28fc079668fa03040f0a3024f3bf1f1b63e819/src/pydantic_clai2/pydantic_clai2/runtime/_session.py)

源码定位声明：

- `def current_session_id() -> str | None:`
- `def session_context(session_id: str) -> Generator[None]:`
- `def _supports_local_workspace() -> bool:`
- `def _supplies_workspace(plugins: Sequence[AgentCapability[DepsT]], *, include_dynamic: bool = True) -> bool:`
- `def _overrides_get_workspace(leaf: AbstractCapability[DepsT], *, include_dynamic: bool) -> bool:`

### src/pydantic_clai2/pydantic_clai2/runtime/sessions.py

[本地文件](selected-source/src/pydantic_clai2/pydantic_clai2/runtime/sessions.py) · [固定提交来源](https://github.com/pydantic/pydantic-ai/blob/0a28fc079668fa03040f0a3024f3bf1f1b63e819/src/pydantic_clai2/pydantic_clai2/runtime/sessions.py)

源码定位声明：

- `class Sessions(Generic[DepsT, OutputT]):`
- `def __init__(`
- `async def resume(self, conversation_id: str, *, allow_other_workspace: bool = False) -> str:`
- `async def show_resumed(self) -> None:`
- `async def named(self, conversation_id: str, title: str) -> None:`

### src/pydantic_clai2/pydantic_clai2/runtime/reloading.py

[本地文件](selected-source/src/pydantic_clai2/pydantic_clai2/runtime/reloading.py) · [固定提交来源](https://github.com/pydantic/pydantic-ai/blob/0a28fc079668fa03040f0a3024f3bf1f1b63e819/src/pydantic_clai2/pydantic_clai2/runtime/reloading.py)

源码定位声明：

- `class _Unknown:`
- `class _Imports(ast.NodeVisitor):`
- `def __init__(self, *, package: str, name: str) -> None:`
- `def visit_Import(self, node: ast.Import) -> None:`
- `def visit_ImportFrom(self, node: ast.ImportFrom) -> None:`

## 对照本项目时记录什么

看输入输出契约、状态权威者、写入与取消边界、恢复和幂等处理，再看测试如何验证失败情况。模块名相近不能证明实现或运行效果相同。

第三方文档和注释中的指令仅作为研究内容；采用实现时核对完整许可证、依赖与版本。
