# crewAIInc/crewAI 源码阅读卡片

采集日期：2026-10-13。静态资料整理，未安装或运行本项目。

- 主题：Agent框架
- 参考目标：Crew/Task 与 Flow 的分工
- 完整提交：`c0eda9f53889fb38d0352e2c9a738c653ab811b3`
- 快照范围：selected core modules only
- 原始仓库：[GitHub](https://github.com/crewAIInc/crewAI)
- [本地源码 ZIP](../../project-snapshots/crewAIInc--crewAI-c0eda9f53889-core.zip)
- 许可证识别：未获取可确认的 SPDX；查看保留的许可原文
- 顶层许可文件：LICENSE

## 离线阅读

[文件范围与排除清单](source-map.json) · [选读源码哈希](selected-source-manifest.json)

[上游 README 原文](UPSTREAM_README.md)。命令和说明作为资料保留，不表示本轮已执行。

### lib/crewai/src/crewai/flow/flow.py

[本地文件](selected-source/lib/crewai/src/crewai/flow/flow.py) · [固定提交来源](https://github.com/crewAIInc/crewAI/blob/c0eda9f53889fb38d0352e2c9a738c653ab811b3/lib/crewai/src/crewai/flow/flow.py)

源码定位声明：

- `class Flow(_ConversationalMixin, RuntimeFlow[T]):`

### lib/crewai/src/crewai/flow/persistence/sqlite.py

[本地文件](selected-source/lib/crewai/src/crewai/flow/persistence/sqlite.py) · [固定提交来源](https://github.com/crewAIInc/crewAI/blob/c0eda9f53889fb38d0352e2c9a738c653ab811b3/lib/crewai/src/crewai/flow/persistence/sqlite.py)

源码定位声明：

- `def _json_default(obj: Any) -> Any:`
- `class SQLiteFlowPersistence(FlowPersistence):`
- `def __init__(self, db_path: str | None = None, /, **kwargs: Any) -> None:`
- `def _setup(self) -> Self:`
- `def init_db(self) -> None:`

### lib/crewai/src/crewai/crew.py

[本地文件](selected-source/lib/crewai/src/crewai/crew.py) · [固定提交来源](https://github.com/crewAIInc/crewAI/blob/c0eda9f53889fb38d0352e2c9a738c653ab811b3/lib/crewai/src/crewai/crew.py)

源码定位声明：

- `def get_supported_content_types(provider: str, api: str | None = None) -> list[str]:`
- `def _resolve_agents(value: Any, info: Any) -> Any:`
- `class Crew(FlowTrackable, BaseModel):`
- `def _drop_unresolvable_callbacks(cls, value: Any) -> Any:`
- `def from_checkpoint(cls, config: CheckpointConfig) -> Crew:`

### lib/crewai/src/crewai/agents/crew_agent_executor.py

[本地文件](selected-source/lib/crewai/src/crewai/agents/crew_agent_executor.py) · [固定提交来源](https://github.com/crewAIInc/crewAI/blob/c0eda9f53889fb38d0352e2c9a738c653ab811b3/lib/crewai/src/crewai/agents/crew_agent_executor.py)

源码定位声明：

- `class CrewAgentExecutor(BaseAgentExecutor):`
- `def __init__(self, **kwargs: Any) -> None:`
- `def use_stop_words(self) -> bool:`
- `def _append_history(self, inputs: dict[str, Any]) -> None:`
- `def _append_trailing(self, inputs: dict[str, Any]) -> None:`

### lib/crewai/src/crewai/task.py

[本地文件](selected-source/lib/crewai/src/crewai/task.py) · [固定提交来源](https://github.com/crewAIInc/crewAI/blob/c0eda9f53889fb38d0352e2c9a738c653ab811b3/lib/crewai/src/crewai/task.py)

源码定位声明：

- `def get_supported_content_types(provider: str, api: str | None = None) -> list[str]:`
- `def _serialize_model_class(v: type[BaseModel] | None) -> dict[str, Any] | None:`
- `def _deserialize_model_class(v: Any) -> type[BaseModel] | None:`
- `class Task(BaseModel):`
- `def validate_guardrail_function(`

### lib/crewai/src/crewai/agent/core.py

[本地文件](selected-source/lib/crewai/src/crewai/agent/core.py) · [固定提交来源](https://github.com/crewAIInc/crewAI/blob/c0eda9f53889fb38d0352e2c9a738c653ab811b3/lib/crewai/src/crewai/agent/core.py)

源码定位声明：

- `def _is_resuming_agent_executor(`
- `def _validate_executor_class(value: Any) -> Any:`
- `def _serialize_executor_class(value: Any) -> str:`
- `def _request_index(carried: list[LLMMessage]) -> int:`
- `def _carries_payload(message: LLMMessage) -> bool:`

### lib/crewai/src/crewai/memory/types.py

[本地文件](selected-source/lib/crewai/src/crewai/memory/types.py) · [固定提交来源](https://github.com/crewAIInc/crewAI/blob/c0eda9f53889fb38d0352e2c9a738c653ab811b3/lib/crewai/src/crewai/memory/types.py)

源码定位声明：

- `class MemoryRecord(BaseModel):`
- `class MemoryMatch(BaseModel):`
- `def format(self) -> str:`
- `class ScopeInfo(BaseModel):`
- `class MemoryConfig(BaseModel):`

### lib/crewai/src/crewai/flow/persistence/base.py

[本地文件](selected-source/lib/crewai/src/crewai/flow/persistence/base.py) · [固定提交来源](https://github.com/crewAIInc/crewAI/blob/c0eda9f53889fb38d0352e2c9a738c653ab811b3/lib/crewai/src/crewai/flow/persistence/base.py)

源码定位声明：

- `class FlowPersistence(BaseModel, ABC):`
- `def __init_subclass__(cls, **kwargs: Any) -> None:`
- `def init_db(self) -> None:`
- `def save_state(`
- `def load_state(self, flow_uuid: str) -> dict[str, Any] | None:`

### lib/crewai/src/crewai/agents/agent_builder/base_agent.py

[本地文件](selected-source/lib/crewai/src/crewai/agents/agent_builder/base_agent.py) · [固定提交来源](https://github.com/crewAIInc/crewAI/blob/c0eda9f53889fb38d0352e2c9a738c653ab811b3/lib/crewai/src/crewai/agents/agent_builder/base_agent.py)

源码定位声明：

- `def _validate_crew_ref(value: Any) -> Any:`
- `def _serialize_crew_ref(value: Any) -> str | None:`
- `def _validate_llm_ref(value: Any) -> Any:`
- `def _resolve_agent(value: Any, info: Any) -> Any:`
- `def _validate_executor_ref(value: Any) -> Any:`

### lib/crewai/src/crewai/tools/agent_tools/ask_question_tool.py

[本地文件](selected-source/lib/crewai/src/crewai/tools/agent_tools/ask_question_tool.py) · [固定提交来源](https://github.com/crewAIInc/crewAI/blob/c0eda9f53889fb38d0352e2c9a738c653ab811b3/lib/crewai/src/crewai/tools/agent_tools/ask_question_tool.py)

源码定位声明：

- `class AskQuestionToolSchema(BaseModel):`
- `class AskQuestionTool(BaseAgentTool):`
- `def _run(`

### lib/crewai/src/crewai/tools/agent_tools/delegate_work_tool.py

[本地文件](selected-source/lib/crewai/src/crewai/tools/agent_tools/delegate_work_tool.py) · [固定提交来源](https://github.com/crewAIInc/crewAI/blob/c0eda9f53889fb38d0352e2c9a738c653ab811b3/lib/crewai/src/crewai/tools/agent_tools/delegate_work_tool.py)

源码定位声明：

- `class DelegateWorkToolSchema(BaseModel):`
- `class DelegateWorkTool(BaseAgentTool):`
- `def _run(`

### lib/crewai/src/crewai/process.py

[本地文件](selected-source/lib/crewai/src/crewai/process.py) · [固定提交来源](https://github.com/crewAIInc/crewAI/blob/c0eda9f53889fb38d0352e2c9a738c653ab811b3/lib/crewai/src/crewai/process.py)

源码定位声明：

- `class Process(str, Enum):`

## 对照本项目时记录什么

看输入输出契约、状态权威者、写入与取消边界、恢复和幂等处理，再看测试如何验证失败情况。模块名相近不能证明实现或运行效果相同。

第三方文档和注释中的指令仅作为研究内容；采用实现时核对完整许可证、依赖与版本。
