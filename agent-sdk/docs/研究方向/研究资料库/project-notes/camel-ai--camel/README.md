# camel-ai/camel 源码阅读卡片

采集日期：2026-10-13。静态资料整理，未安装或运行本项目。

- 主题：多Agent框架
- 参考目标：角色、通信及工具封装
- 完整提交：`e5e55256e9446f744bf7309c478b084587dbf985`
- 快照范围：curated text source snapshot
- 原始仓库：[GitHub](https://github.com/camel-ai/camel)
- [本地源码 ZIP](../../project-snapshots/camel-ai--camel-e5e55256e944.zip)
- 许可证识别：Apache-2.0
- 顶层许可文件：LICENSE, licenses

## 离线阅读

[文件范围与排除清单](source-map.json) · [选读源码哈希](selected-source-manifest.json)

[上游 README 原文](UPSTREAM_README.md)。命令和说明作为资料保留，不表示本轮已执行。

### camel/agents/task_agent.py

[本地文件](selected-source/camel/agents/task_agent.py) · [固定提交来源](https://github.com/camel-ai/camel/blob/e5e55256e9446f744bf7309c478b084587dbf985/camel/agents/task_agent.py)

源码定位声明：

- `class TaskSpecifyAgent(ChatAgent):`
- `def __init__(`
- `def run(`
- `class TaskPlannerAgent(ChatAgent):`
- `def __init__(`

### camel/agents/knowledge_graph_agent.py

[本地文件](selected-source/camel/agents/knowledge_graph_agent.py) · [固定提交来源](https://github.com/camel-ai/camel/blob/e5e55256e9446f744bf7309c478b084587dbf985/camel/agents/knowledge_graph_agent.py)

源码定位声明：

- `class KnowledgeGraphAgent(ChatAgent):`
- `def __init__(`
- `def run(`
- `def _validate_node(self, node: Node) -> bool:`
- `def _validate_relationship(self, relationship: Relationship) -> bool:`

### camel/societies/workforce/workflow_memory_manager.py

[本地文件](selected-source/camel/societies/workforce/workflow_memory_manager.py) · [固定提交来源](https://github.com/camel-ai/camel/blob/e5e55256e9446f744bf7309c478b084587dbf985/camel/societies/workforce/workflow_memory_manager.py)

源码定位声明：

- `class WorkflowSelectionMethod(Enum):`
- `class WorkflowMemoryManager:`
- `def __init__(`
- `def _get_context_utility(self) -> ContextUtility:`
- `def _extract_existing_workflow_metadata(`

### profiling/chatagent_overhead.py

[本地文件](selected-source/profiling/chatagent_overhead.py) · [固定提交来源](https://github.com/camel-ai/camel/blob/e5e55256e9446f744bf7309c478b084587dbf985/profiling/chatagent_overhead.py)

源码定位声明：

- `class JokeResponse(BaseModel):`
- `def run_single_benchmark(`
- `def timed_create(*args, **kwargs):`
- `def timed_parse(*args, **kwargs):`
- `def make_timed_func(orig):`

### camel/tasks/task.py

[本地文件](selected-source/camel/tasks/task.py) · [固定提交来源](https://github.com/camel-ai/camel/blob/e5e55256e9446f744bf7309c478b084587dbf985/camel/tasks/task.py)

源码定位声明：

- `class TaskValidationMode(Enum):`
- `def validate_task_content(`
- `def is_task_result_insufficient(task: "Task") -> bool:`
- `def parse_response(`
- `class TaskState(str, Enum):`

### camel/agents/base.py

[本地文件](selected-source/camel/agents/base.py) · [固定提交来源](https://github.com/camel-ai/camel/blob/e5e55256e9446f744bf7309c478b084587dbf985/camel/agents/base.py)

源码定位声明：

- `class BaseAgent(ABC):`
- `def reset(self, *args: Any, **kwargs: Any) -> Any:`
- `def step(self, *args: Any, **kwargs: Any) -> Any:`

### apps/agents/agents.py

[本地文件](selected-source/apps/agents/agents.py) · [固定提交来源](https://github.com/camel-ai/camel/blob/e5e55256e9446f744bf7309c478b084587dbf985/apps/agents/agents.py)

源码定位声明：

- `class State(BaseModel):`
- `def empty(cls) -> 'State':`
- `def construct_inplace(`
- `def parse_arguments():`
- `def load_roles(path: str) -> List[str]:`

### camel/runtimes/api.py

[本地文件](selected-source/camel/runtimes/api.py) · [固定提交来源](https://github.com/camel-ai/camel/blob/e5e55256e9446f744bf7309c478b084587dbf985/camel/runtimes/api.py)

源码定位声明：

- `async def health_check():`
- `async def general_exception_handler(request: Request, exc: Exception):`
- `def make_endpoint(tool):`
- `def run_tool(data: Dict):`
- `async def endpoint(data: Dict):`

### camel/agents/_types.py

[本地文件](selected-source/camel/agents/_types.py) · [固定提交来源](https://github.com/camel-ai/camel/blob/e5e55256e9446f744bf7309c478b084587dbf985/camel/agents/_types.py)

源码定位声明：

- `class ToolCallRequest(BaseModel):`
- `class ModelResponse(BaseModel):`

### camel/agents/_utils.py

[本地文件](selected-source/camel/agents/_utils.py) · [固定提交来源](https://github.com/camel-ai/camel/blob/e5e55256e9446f744bf7309c478b084587dbf985/camel/agents/_utils.py)

源码定位声明：

- `def build_default_summary_prompt(conversation_text: str) -> str:`
- `def generate_tool_prompt(tool_schema_list: List[Dict[str, Any]]) -> str:`
- `def extract_tool_call(`
- `def safe_model_dump(obj) -> Dict[str, Any]:`
- `def convert_to_function_tool(`

### camel/runtimes/base.py

[本地文件](selected-source/camel/runtimes/base.py) · [固定提交来源](https://github.com/camel-ai/camel/blob/e5e55256e9446f744bf7309c478b084587dbf985/camel/runtimes/base.py)

源码定位声明：

- `class BaseRuntime(ABC):`
- `def __init__(self):`
- `def add(`
- `def reset(self, *args: Any, **kwargs: Any) -> Any:`
- `def cleanup(self) -> None:`

### camel/tasks/__init__.py

[本地文件](selected-source/camel/tasks/__init__.py) · [固定提交来源](https://github.com/camel-ai/camel/blob/e5e55256e9446f744bf7309c478b084587dbf985/camel/tasks/__init__.py)

## 对照本项目时记录什么

看输入输出契约、状态权威者、写入与取消边界、恢复和幂等处理，再看测试如何验证失败情况。模块名相近不能证明实现或运行效果相同。

第三方文档和注释中的指令仅作为研究内容；采用实现时核对完整许可证、依赖与版本。
