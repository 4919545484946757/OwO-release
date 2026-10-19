# google/adk-python 源码阅读卡片

采集日期：2026-10-13。静态资料整理，未安装或运行本项目。

- 主题：Agent框架
- 参考目标：Sequential/Parallel/Loop Agent 与会话
- 完整提交：`c213adf02d491726037b9dcceecbc4106dbd2aa6`
- 快照范围：curated text source snapshot
- 原始仓库：[GitHub](https://github.com/google/adk-python)
- [本地源码 ZIP](../../project-snapshots/google--adk-python-c213adf02d49.zip)
- 许可证识别：Apache-2.0
- 顶层许可文件：LICENSE

## 离线阅读

[文件范围与排除清单](source-map.json) · [选读源码哈希](selected-source-manifest.json)

[上游 README 原文](UPSTREAM_README.md)。命令和说明作为资料保留，不表示本轮已执行。

### src/google/adk/cli/agent_graph.py

[本地文件](selected-source/src/google/adk/cli/agent_graph.py) · [固定提交来源](https://github.com/google/adk-python/blob/c213adf02d491726037b9dcceecbc4106dbd2aa6/src/google/adk/cli/agent_graph.py)

源码定位声明：

- `async def build_graph(`
- `def get_node_name(tool_or_agent: Union[BaseAgent, BaseTool]) -> str:`
- `def get_node_caption(tool_or_agent: Union[BaseAgent, BaseTool]) -> str:`
- `def get_node_shape(tool_or_agent: Union[BaseAgent, BaseTool]) -> str:`
- `def should_build_agent_cluster(`

### src/google/adk/workflow/_graph.py

[本地文件](selected-source/src/google/adk/workflow/_graph.py) · [固定提交来源](https://github.com/google/adk-python/blob/c213adf02d491726037b9dcceecbc4106dbd2aa6/src/google/adk/workflow/_graph.py)

源码定位声明：

- `class Edge(BaseModel):`
- `class Graph(BaseModel):`
- `def from_edge_items(cls, edge_items: list[EdgeItem]) -> Graph:`
- `def model_post_init(self, context: Any) -> None:`
- `def get_next_pending_nodes(`

### src/google/adk/cli/agent_test_runner.py

[本地文件](selected-source/src/google/adk/cli/agent_test_runner.py) · [固定提交来源](https://github.com/google/adk-python/blob/c213adf02d491726037b9dcceecbc4106dbd2aa6/src/google/adk/cli/agent_test_runner.py)

源码定位声明：

- `def get_test_files(`
- `class MockModel(BaseLlm):`
- `def create(cls, contents: list[types.Content]):`
- `def supported_models(cls) -> list[str]:`
- `async def generate_content_async(`

### src/google/adk/workflow/_node_runner.py

[本地文件](selected-source/src/google/adk/workflow/_node_runner.py) · [固定提交来源](https://github.com/google/adk-python/blob/c213adf02d491726037b9dcceecbc4106dbd2aa6/src/google/adk/workflow/_node_runner.py)

源码定位声明：

- `def _has_non_output_content(event: Event) -> bool:`
- `class NodeRunner:`
- `def __init__(`
- `def run_id(self) -> str:`
- `async def run(`

### src/google/adk/agents/langgraph_agent.py

[本地文件](selected-source/src/google/adk/agents/langgraph_agent.py) · [固定提交来源](https://github.com/google/adk-python/blob/c213adf02d491726037b9dcceecbc4106dbd2aa6/src/google/adk/agents/langgraph_agent.py)

源码定位声明：

- `def _get_thread_id(app_name: str, user_id: str, session_id: str) -> str:`
- `def _get_last_human_messages(`
- `class LangGraphAgent(BaseAgent):`
- `async def _run_async_impl(`
- `def _get_messages(`

### src/google/adk/workflow/_llm_agent_wrapper.py

[本地文件](selected-source/src/google/adk/workflow/_llm_agent_wrapper.py) · [固定提交来源](https://github.com/google/adk-python/blob/c213adf02d491726037b9dcceecbc4106dbd2aa6/src/google/adk/workflow/_llm_agent_wrapper.py)

源码定位声明：

- `def _extract_finish_task_fc(event: Event) -> types.FunctionCall | None:`
- `def _extract_task_delegation_fcs(`
- `def _event_has_eager_tool_calls(`
- `async def _drain_pending_tool_response_events(`
- `def _find_unresolved_task_delegations(`

### src/google/adk/workflow/_node_runner_utils.py

[本地文件](selected-source/src/google/adk/workflow/_node_runner_utils.py) · [固定提交来源](https://github.com/google/adk-python/blob/c213adf02d491726037b9dcceecbc4106dbd2aa6/src/google/adk/workflow/_node_runner_utils.py)

源码定位声明：

- `async def run_node_async(`
- `async def _run() -> AsyncGenerator[Event, None]:`
- `async def _drive_root_node() -> None:`

### src/google/adk/optimization/agent_optimizer.py

[本地文件](selected-source/src/google/adk/optimization/agent_optimizer.py) · [固定提交来源](https://github.com/google/adk-python/blob/c213adf02d491726037b9dcceecbc4106dbd2aa6/src/google/adk/optimization/agent_optimizer.py)

源码定位声明：

- `class AgentOptimizer(ABC, Generic[SamplingResultT, AgentWithScoresT]):`
- `async def optimize(`

### src/google/adk/workflow/_dynamic_node_scheduler.py

[本地文件](selected-source/src/google/adk/workflow/_dynamic_node_scheduler.py) · [固定提交来源](https://github.com/google/adk-python/blob/c213adf02d491726037b9dcceecbc4106dbd2aa6/src/google/adk/workflow/_dynamic_node_scheduler.py)

源码定位声明：

- `class DynamicNodeRun:`
- `class DynamicNodeState:`
- `def next_run_id(self, node_name: str, parent_path: str = '') -> str:`
- `def reset_for_retry(self, parent_path: str) -> None:`
- `def get_dynamic_tasks(self) -> list[asyncio.Task[Context]]:`

### src/google/adk/optimization/gepa_root_agent_optimizer.py

[本地文件](selected-source/src/google/adk/optimization/gepa_root_agent_optimizer.py) · [固定提交来源](https://github.com/google/adk-python/blob/c213adf02d491726037b9dcceecbc4106dbd2aa6/src/google/adk/optimization/gepa_root_agent_optimizer.py)

源码定位声明：

- `class GEPARootAgentOptimizerConfig(BaseModel):`
- `class GEPARootAgentOptimizerResult(OptimizerResult[AgentWithScores]):`
- `def _update_skill_toolset(`
- `def _create_agent_from_candidate(`
- `def _create_agent_gepa_adapter_class() -> type[Any]:`

### src/google/adk/optimization/gepa_root_agent_prompt_optimizer.py

[本地文件](selected-source/src/google/adk/optimization/gepa_root_agent_prompt_optimizer.py) · [固定提交来源](https://github.com/google/adk-python/blob/c213adf02d491726037b9dcceecbc4106dbd2aa6/src/google/adk/optimization/gepa_root_agent_prompt_optimizer.py)

源码定位声明：

- `class GEPARootAgentPromptOptimizerConfig(BaseModel):`
- `class GEPARootAgentPromptOptimizerResult(OptimizerResult[AgentWithScores]):`
- `def _create_agent_gepa_adapter_class() -> type[Any]:`
- `class _AgentGEPAAdapter(  # type: ignore[misc]`
- `def __init__(`

### src/google/adk/code_executors/agent_engine_sandbox_code_executor.py

[本地文件](selected-source/src/google/adk/code_executors/agent_engine_sandbox_code_executor.py) · [固定提交来源](https://github.com/google/adk-python/blob/c213adf02d491726037b9dcceecbc4106dbd2aa6/src/google/adk/code_executors/agent_engine_sandbox_code_executor.py)

源码定位声明：

- `class AgentEngineSandboxCodeExecutor(BaseCodeExecutor):`
- `def __init__(`
- `def execute_code(`
- `def _get_api_client(self) -> agentplatform.Client:`
- `def _get_project_id_and_location_from_resource_name(`

## 对照本项目时记录什么

看输入输出契约、状态权威者、写入与取消边界、恢复和幂等处理，再看测试如何验证失败情况。模块名相近不能证明实现或运行效果相同。

第三方文档和注释中的指令仅作为研究内容；采用实现时核对完整许可证、依赖与版本。
