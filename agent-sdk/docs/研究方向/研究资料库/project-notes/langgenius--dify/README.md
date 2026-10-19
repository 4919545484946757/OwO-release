# langgenius/dify 源码阅读卡片

采集日期：2026-10-13。静态资料整理，未安装或运行本项目。

- 主题：自动化工作流
- 参考目标：图编排、节点结果与人机交互
- 完整提交：`8a237c6daf522f6bc11205a7d27f59aa48d790c6`
- 快照范围：curated text source snapshot
- 原始仓库：[GitHub](https://github.com/langgenius/dify)
- [本地源码 ZIP](../../project-snapshots/langgenius--dify-8a237c6daf52.zip)
- 许可证识别：NOASSERTION
- 顶层许可文件：LICENSE

## 离线阅读

[文件范围与排除清单](source-map.json) · [选读源码哈希](selected-source-manifest.json)

[上游 README 原文](UPSTREAM_README.md)。命令和说明作为资料保留，不表示本轮已执行。

### dify-agent/src/dify_agent/runtime/runner.py

[本地文件](selected-source/dify-agent/src/dify_agent/runtime/runner.py) · [固定提交来源](https://github.com/langgenius/dify/blob/8a237c6daf522f6bc11205a7d27f59aa48d790c6/dify-agent/src/dify_agent/runtime/runner.py)

源码定位声明：

- `class _HasAccumulatedUsage(Protocol):`
- `def accumulated_usage(self) -> LLMUsage | None: ...`
- `class AgentRunValidationError(ValueError):`
- `def _run_failed_error_payload(exc: Exception) -> tuple[str, RunFailureType | None, str | None]:`
- `def _extract_agent_message_delta(event: AgentStreamEvent) -> str | None:`

### dify-agent/src/dify_agent/runtime/run_scheduler.py

[本地文件](selected-source/dify-agent/src/dify_agent/runtime/run_scheduler.py) · [固定提交来源](https://github.com/langgenius/dify/blob/8a237c6daf522f6bc11205a7d27f59aa48d790c6/dify-agent/src/dify_agent/runtime/run_scheduler.py)

源码定位声明：

- `class SchedulerStoppingError(RuntimeError):`
- `class RunCancellationConflictError(RuntimeError):`
- `class RunStore(RunEventSink, Protocol):`
- `async def create_run(self) -> RunRecord:`
- `async def request_cancellation(self, run_id: str, request: CancelRunRequest) -> RunStatus:`

### dify-agent/src/dify_agent/runtime/command_runner.py

[本地文件](selected-source/dify-agent/src/dify_agent/runtime/command_runner.py) · [固定提交来源](https://github.com/langgenius/dify/blob/8a237c6daf522f6bc11205a7d27f59aa48d790c6/dify-agent/src/dify_agent/runtime/command_runner.py)

源码定位声明：

- `async def execute_complete_with_commands(`
- `def _remaining_time(deadline: float) -> float:`

### dify-agent/src/shellctl/shared/runtime.py

[本地文件](selected-source/dify-agent/src/shellctl/shared/runtime.py) · [固定提交来源](https://github.com/langgenius/dify/blob/8a237c6daf522f6bc11205a7d27f59aa48d790c6/dify-agent/src/shellctl/shared/runtime.py)

源码定位声明：

- `def utc_now() -> datetime:`
- `def format_timestamp(value: datetime | None = None) -> str:`
- `def parse_timestamp(value: str) -> datetime:`
- `def is_terminal_status(status: JobStatusName) -> bool:`
- `def generate_job_id(*, now: datetime | None = None) -> str:`

### dify-agent/src/dify_agent/runtime/context.py

[本地文件](selected-source/dify-agent/src/dify_agent/runtime/context.py) · [固定提交来源](https://github.com/langgenius/dify/blob/8a237c6daf522f6bc11205a7d27f59aa48d790c6/dify-agent/src/dify_agent/runtime/context.py)

源码定位声明：

- `class LayerData(TypedDict):`
- `class Services:`
- `class RunResources:`
- `class Deps:`

### dify-agent/src/dify_agent/runtime/modules.py

[本地文件](selected-source/dify-agent/src/dify_agent/runtime/modules.py) · [固定提交来源](https://github.com/langgenius/dify/blob/8a237c6daf522f6bc11205a7d27f59aa48d790c6/dify-agent/src/dify_agent/runtime/modules.py)

源码定位声明：

- `def load_modules(`
- `def create_modules(`
- `def snapshot_modules(deps: Deps, *, registry: Mapping[str, ModuleType] = REGISTRY) -> SessionSnapshot:`

### dify-agent/src/dify_agent/runtime/compaction.py

[本地文件](selected-source/dify-agent/src/dify_agent/runtime/compaction.py) · [固定提交来源](https://github.com/langgenius/dify/blob/8a237c6daf522f6bc11205a7d27f59aa48d790c6/dify-agent/src/dify_agent/runtime/compaction.py)

源码定位声明：

- `def build_compaction_capability(`

### dify-agent/src/dify_agent/runtime/event_sink.py

[本地文件](selected-source/dify-agent/src/dify_agent/runtime/event_sink.py) · [固定提交来源](https://github.com/langgenius/dify/blob/8a237c6daf522f6bc11205a7d27f59aa48d790c6/dify-agent/src/dify_agent/runtime/event_sink.py)

源码定位声明：

- `class RunFinalizationResult:`
- `class RunEventSink(Protocol):`
- `async def append_event(self, event: NonTerminalRunEvent) -> str:`
- `async def finalize_run(self, event: TerminalRunEvent) -> RunFinalizationResult:`
- `class InMemoryRunEventSink:`

### dify-agent/src/dify_agent/runtime_backend/e2b.py

[本地文件](selected-source/dify-agent/src/dify_agent/runtime_backend/e2b.py) · [固定提交来源](https://github.com/langgenius/dify/blob/8a237c6daf522f6bc11205a7d27f59aa48d790c6/dify-agent/src/dify_agent/runtime_backend/e2b.py)

源码定位声明：

- `class _E2BControlPlaneNotFoundError(RuntimeError):`
- `class _E2BControlPlaneCapacityExhaustedError(RuntimeError):`
- `class _E2BFileEntry(Protocol):`
- `class _E2BFileSystem(Protocol):`
- `async def make_dir(self, path: str) -> bool: ...`

### dify-agent/src/dify_agent/runtime/cancellation.py

[本地文件](selected-source/dify-agent/src/dify_agent/runtime/cancellation.py) · [固定提交来源](https://github.com/langgenius/dify/blob/8a237c6daf522f6bc11205a7d27f59aa48d790c6/dify-agent/src/dify_agent/runtime/cancellation.py)

源码定位声明：

- `class RunCancellationIntent(BaseModel):`

### dify-agent/src/dify_agent/runtime/agent_factory.py

[本地文件](selected-source/dify-agent/src/dify_agent/runtime/agent_factory.py) · [固定提交来源](https://github.com/langgenius/dify/blob/8a237c6daf522f6bc11205a7d27f59aa48d790c6/dify-agent/src/dify_agent/runtime/agent_factory.py)

源码定位声明：

- `def create_agent(`
- `def normalize_user_input(user_prompts: Sequence[UserContent]) -> str | Sequence[UserContent]:`

### dify-agent/src/dify_agent/runtime/observability.py

[本地文件](selected-source/dify-agent/src/dify_agent/runtime/observability.py) · [固定提交来源](https://github.com/langgenius/dify/blob/8a237c6daf522f6bc11205a7d27f59aa48d790c6/dify-agent/src/dify_agent/runtime/observability.py)

源码定位声明：

- `def dify_run_attributes(execution_context: DifyExecutionContextLayerConfig) -> tuple[tuple[str, str], ...]:`
- `class IsolatedTracerProvider(TracerProvider):`
- `def get_tracer(`
- `class _IsolatedTracer(Tracer):`
- `def _parent_context(self, context: Context | None) -> Context | None:`

## 对照本项目时记录什么

看输入输出契约、状态权威者、写入与取消边界、恢复和幂等处理，再看测试如何验证失败情况。模块名相近不能证明实现或运行效果相同。

第三方文档和注释中的指令仅作为研究内容；采用实现时核对完整许可证、依赖与版本。
