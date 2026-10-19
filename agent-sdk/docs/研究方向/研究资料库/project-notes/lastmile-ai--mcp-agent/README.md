# lastmile-ai/mcp-agent 源码阅读卡片

采集日期：2026-10-13。静态资料整理，未安装或运行本项目。

- 主题：MCP工作流
- 参考目标：MCP 工具组合与 workflow
- 完整提交：`f62d849350816588b1c6294e7914bbe4d8b84072`
- 快照范围：curated text source snapshot
- 原始仓库：[GitHub](https://github.com/lastmile-ai/mcp-agent)
- [本地源码 ZIP](../../project-snapshots/lastmile-ai--mcp-agent-f62d84935081.zip)
- 许可证识别：未获取可确认的 SPDX；查看保留的许可原文
- 顶层许可文件：LICENSE

## 离线阅读

[文件范围与排除清单](source-map.json) · [选读源码哈希](selected-source-manifest.json)

[上游 README 原文](UPSTREAM_README.md)。命令和说明作为资料保留，不表示本轮已执行。

### src/mcp_agent/executor/workflow_task.py

[本地文件](selected-source/src/mcp_agent/executor/workflow_task.py) · [固定提交来源](https://github.com/lastmile-ai/mcp-agent/blob/f62d849350816588b1c6294e7914bbe4d8b84072/src/mcp_agent/executor/workflow_task.py)

源码定位声明：

- `class GlobalWorkflowTaskRegistry:`
- `def __new__(cls):`
- `def register_task(self, func: Callable, metadata: Dict[str, Any]):`
- `def get_all_tasks(self) -> List[tuple]:`
- `def clear(self):`

### src/mcp_agent/workflows/deep_orchestrator/task_executor.py

[本地文件](selected-source/src/mcp_agent/workflows/deep_orchestrator/task_executor.py) · [固定提交来源](https://github.com/lastmile-ai/mcp-agent/blob/f62d849350816588b1c6294e7914bbe4d8b84072/src/mcp_agent/workflows/deep_orchestrator/task_executor.py)

源码定位声明：

- `class TaskExecutor:`
- `def __init__(`
- `def set_budget_callback(self, update_budget_tokens: Callable[[int], None]):`
- `async def execute_step(`
- `async def execute_task(`

### src/mcp_agent/executor/workflow.py

[本地文件](selected-source/src/mcp_agent/executor/workflow.py) · [固定提交来源](https://github.com/lastmile-ai/mcp-agent/blob/f62d849350816588b1c6294e7914bbe4d8b84072/src/mcp_agent/executor/workflow.py)

源码定位声明：

- `class WorkflowState(BaseModel):`
- `def record_error(self, error: Exception) -> None:`
- `class WorkflowResult(BaseModel, Generic[T]):`
- `class WorkflowExecution(BaseModel):`
- `class Workflow(ABC, Generic[T], ContextDependent):`

### src/mcp_agent/executor/task_registry.py

[本地文件](selected-source/src/mcp_agent/executor/task_registry.py) · [固定提交来源](https://github.com/lastmile-ai/mcp-agent/blob/f62d849350816588b1c6294e7914bbe4d8b84072/src/mcp_agent/executor/task_registry.py)

源码定位声明：

- `class ActivityRegistry:`
- `def __init__(self):`
- `def register(`
- `def get_activity(self, name: str) -> Callable:`
- `def get_metadata(self, name: str) -> Dict[str, Any]:`

### src/mcp_agent/executor/workflow_signal.py

[本地文件](selected-source/src/mcp_agent/executor/workflow_signal.py) · [固定提交来源](https://github.com/lastmile-ai/mcp-agent/blob/f62d849350816588b1c6294e7914bbe4d8b84072/src/mcp_agent/executor/workflow_signal.py)

源码定位声明：

- `class Signal(BaseModel, Generic[SignalValueT]):`
- `class SignalRegistration(BaseModel):`
- `class SignalHandler(Protocol, Generic[SignalValueT]):`
- `async def signal(self, signal: Signal[SignalValueT]) -> None:`
- `async def wait_for_signal(`

### src/mcp_agent/executor/workflow_registry.py

[本地文件](selected-source/src/mcp_agent/executor/workflow_registry.py) · [固定提交来源](https://github.com/lastmile-ai/mcp-agent/blob/f62d849350816588b1c6294e7914bbe4d8b84072/src/mcp_agent/executor/workflow_registry.py)

源码定位声明：

- `class WorkflowRunsPage(BaseModel):`
- `class WorkflowRegistry(ABC):`
- `def __init__(self):`
- `async def register(`
- `async def unregister(self, run_id: str, workflow_id: str | None = None) -> None:`

### src/mcp_agent/executor/temporal/workflow_signal.py

[本地文件](selected-source/src/mcp_agent/executor/temporal/workflow_signal.py) · [固定提交来源](https://github.com/lastmile-ai/mcp-agent/blob/f62d849350816588b1c6294e7914bbe4d8b84072/src/mcp_agent/executor/temporal/workflow_signal.py)

源码定位声明：

- `class TemporalSignalHandler(BaseSignalHandler[SignalValueT]):`
- `def __init__(self, executor: Optional["TemporalExecutor"] = None) -> None:`
- `def attach_to_workflow(self, wf_instance: "Workflow") -> None:`
- `async def wait_for_signal(`
- `def on_signal(self, signal_name: str):`

### src/mcp_agent/workflows/deep_orchestrator/memory.py

[本地文件](selected-source/src/mcp_agent/workflows/deep_orchestrator/memory.py) · [固定提交来源](https://github.com/lastmile-ai/mcp-agent/blob/f62d849350816588b1c6294e7914bbe4d8b84072/src/mcp_agent/workflows/deep_orchestrator/memory.py)

源码定位声明：

- `class WorkspaceMemory:`
- `def __init__(`
- `def save_artifact(`
- `def get_artifact(self, name: str) -> Optional[str]:`
- `def add_knowledge(self, item: KnowledgeItem) -> None:`

### src/mcp_agent/workflows/deep_orchestrator/policy.py

[本地文件](selected-source/src/mcp_agent/workflows/deep_orchestrator/policy.py) · [固定提交来源](https://github.com/lastmile-ai/mcp-agent/blob/f62d849350816588b1c6294e7914bbe4d8b84072/src/mcp_agent/workflows/deep_orchestrator/policy.py)

源码定位声明：

- `class PolicyEngine:`
- `def __init__(`
- `def decide_action(`
- `def record_success(self) -> None:`
- `def record_failure(self) -> None:`

### src/mcp_agent/executor/temporal/workflow_registry.py

[本地文件](selected-source/src/mcp_agent/executor/temporal/workflow_registry.py) · [固定提交来源](https://github.com/lastmile-ai/mcp-agent/blob/f62d849350816588b1c6294e7914bbe4d8b84072/src/mcp_agent/executor/temporal/workflow_registry.py)

源码定位声明：

- `class TemporalWorkflowRegistry(WorkflowRegistry):`
- `def __init__(self, executor: "TemporalExecutor"):`
- `async def register(`
- `async def unregister(self, run_id: str, workflow_id: str | None = None) -> None:`
- `async def get_workflow(`

### src/mcp_agent/executor/temporal/interactive_workflow.py

[本地文件](selected-source/src/mcp_agent/executor/temporal/interactive_workflow.py) · [固定提交来源](https://github.com/lastmile-ai/mcp-agent/blob/f62d849350816588b1c6294e7914bbe4d8b84072/src/mcp_agent/executor/temporal/interactive_workflow.py)

源码定位声明：

- `class HumanResponse:`
- `class InteractiveWorkflow(Workflow[T], Generic[T]):`
- `class MyWorkflow(InteractiveWorkflow):`
- `async def run(self, input: str) -> WorkflowResult[str]:`
- `def __init__(self, *args, **kwargs) -> None:`

### src/mcp_agent/workflows/evaluator_optimizer/__init__.py

[本地文件](selected-source/src/mcp_agent/workflows/evaluator_optimizer/__init__.py) · [固定提交来源](https://github.com/lastmile-ai/mcp-agent/blob/f62d849350816588b1c6294e7914bbe4d8b84072/src/mcp_agent/workflows/evaluator_optimizer/__init__.py)

## 对照本项目时记录什么

看输入输出契约、状态权威者、写入与取消边界、恢复和幂等处理，再看测试如何验证失败情况。模块名相近不能证明实现或运行效果相同。

第三方文档和注释中的指令仅作为研究内容；采用实现时核对完整许可证、依赖与版本。
