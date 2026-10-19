# strands-agents/harness-sdk 源码阅读卡片

采集日期：2026-10-13。静态资料整理，未安装或运行本项目。

- 主题：Agent框架
- 参考目标：工具执行、状态和协作
- 完整提交：`dc03b1c127ee6314f42ae35f4d51641a496edd12`
- 快照范围：curated text source snapshot
- 原始仓库：[GitHub](https://github.com/strands-agents/sdk-python)
- [本地源码 ZIP](../../project-snapshots/strands-agents--harness-sdk-dc03b1c127ee.zip)
- 许可证识别：Apache-2.0
- 顶层许可文件：LICENSE.APACHE, NOTICE

## 离线阅读

[文件范围与排除清单](source-map.json) · [选读源码哈希](selected-source-manifest.json)

[上游 README 原文](UPSTREAM_README.md)。命令和说明作为资料保留，不表示本轮已执行。

### strands-ts/src/multiagent/graph.ts

[本地文件](selected-source/strands-ts/src/multiagent/graph.ts) · [固定提交来源](https://github.com/strands-agents/harness-sdk/blob/dc03b1c127ee6314f42ae35f4d51641a496edd12/strands-ts/src/multiagent/graph.ts)

源码定位声明：

- `export interface GraphConfig {`
- `export interface GraphOptions extends GraphConfig {`
- `export class Graph implements MultiAgent {`

### strands-cli/src/tui/permissions/policy.ts

[本地文件](selected-source/strands-cli/src/tui/permissions/policy.ts) · [固定提交来源](https://github.com/strands-agents/harness-sdk/blob/dc03b1c127ee6314f42ae35f4d51641a496edd12/strands-cli/src/tui/permissions/policy.ts)

源码定位声明：

- `export const DEFAULT_CEDAR_POLICIES = `
- `export class ToolPermissionBroker {`
- `export class CedarPermissions extends InterventionHandler {`
- `export function pathIsWithinWorkspace(value: unknown, workspace: string): boolean {`

### strands-py/src/strands/multiagent/graph.py

[本地文件](selected-source/strands-py/src/strands/multiagent/graph.py) · [固定提交来源](https://github.com/strands-agents/harness-sdk/blob/dc03b1c127ee6314f42ae35f4d51641a496edd12/strands-py/src/strands/multiagent/graph.py)

源码定位声明：

- `class EdgeConditionWithContext(Protocol):`
- `def __call__(self, state: "GraphState", *, invocation_state: dict[str, Any], **kwargs: Any) -> bool:`
- `def _is_context_condition(condition: EdgeCondition) -> TypeGuard[EdgeConditionWithContext]:`
- `class GraphState:`
- `def should_continue(`

### strands-ts/src/multiagent/__tests__/graph.test.ts

[本地文件](selected-source/strands-ts/src/multiagent/__tests__/graph.test.ts) · [固定提交来源](https://github.com/strands-agents/harness-sdk/blob/dc03b1c127ee6314f42ae35f4d51641a496edd12/strands-ts/src/multiagent/__tests__/graph.test.ts)

### strands-py/src/strands/background_tasks/_runtime.py

[本地文件](selected-source/strands-py/src/strands/background_tasks/_runtime.py) · [固定提交来源](https://github.com/strands-agents/harness-sdk/blob/dc03b1c127ee6314f42ae35f4d51641a496edd12/strands-py/src/strands/background_tasks/_runtime.py)

源码定位声明：

- `class _BackgroundTaskRuntime:`
- `def __init__(self) -> None:`
- `async def run(self, operation: Callable[[], _T | Awaitable[_T]]) -> _T:`
- `def _ensure_started(self) -> asyncio.AbstractEventLoop:`
- `def _run_loop(self) -> None:`

### strands-ts/src/agent/__tests__/agent.checkpoint.test.ts

[本地文件](selected-source/strands-ts/src/agent/__tests__/agent.checkpoint.test.ts) · [固定提交来源](https://github.com/strands-agents/harness-sdk/blob/dc03b1c127ee6314f42ae35f4d51641a496edd12/strands-ts/src/agent/__tests__/agent.checkpoint.test.ts)

### strands-ts/src/multiagent/__tests__/graph.tracer.test.ts

[本地文件](selected-source/strands-ts/src/multiagent/__tests__/graph.tracer.test.ts) · [固定提交来源](https://github.com/strands-agents/harness-sdk/blob/dc03b1c127ee6314f42ae35f4d51641a496edd12/strands-ts/src/multiagent/__tests__/graph.tracer.test.ts)

### strands-ts/src/multiagent/__tests__/graph.invocation-state.test.ts

[本地文件](selected-source/strands-ts/src/multiagent/__tests__/graph.invocation-state.test.ts) · [固定提交来源](https://github.com/strands-agents/harness-sdk/blob/dc03b1c127ee6314f42ae35f4d51641a496edd12/strands-ts/src/multiagent/__tests__/graph.invocation-state.test.ts)

### harness-ts/src/agent.ts

[本地文件](selected-source/harness-ts/src/agent.ts) · [固定提交来源](https://github.com/strands-agents/harness-sdk/blob/dc03b1c127ee6314f42ae35f4d51641a496edd12/harness-ts/src/agent.ts)

源码定位声明：

- `export interface HarnessAgentOptions extends Omit<`
- `export async function createHarness(options: HarnessAgentOptions = {}): Promise<Agent> {`

### harness-ts/src/memory.ts

[本地文件](selected-source/harness-ts/src/memory.ts) · [固定提交来源](https://github.com/strands-agents/harness-sdk/blob/dc03b1c127ee6314f42ae35f4d51641a496edd12/harness-ts/src/memory.ts)

源码定位声明：

- `export interface ResolveMemoryOptions {`
- `export async function resolveMemory(options: ResolveMemoryOptions = {}): Promise<MemoryManager> {`

### strands-py/src/strands/multiagent/a2a/executor.py

[本地文件](selected-source/strands-py/src/strands/multiagent/a2a/executor.py) · [固定提交来源](https://github.com/strands-agents/harness-sdk/blob/dc03b1c127ee6314f42ae35f4d51641a496edd12/strands-py/src/strands/multiagent/a2a/executor.py)

源码定位声明：

- `def _is_interrupt_resume(prompt: _AgentPrompt) -> bool:`
- `def _jsonable(value: Any) -> Any:`
- `class _StreamState:`
- `class _ContextEntry:`
- `class StrandsA2AExecutor(AgentExecutor):`

### harness-ts/src/types/agent.ts

[本地文件](selected-source/harness-ts/src/types/agent.ts) · [固定提交来源](https://github.com/strands-agents/harness-sdk/blob/dc03b1c127ee6314f42ae35f4d51641a496edd12/harness-ts/src/types/agent.ts)

源码定位声明：

- `export interface WebFetchConfig {`
- `export interface SessionConfig {`
- `export interface MemoryConfig {`

## 对照本项目时记录什么

看输入输出契约、状态权威者、写入与取消边界、恢复和幂等处理，再看测试如何验证失败情况。模块名相近不能证明实现或运行效果相同。

第三方文档和注释中的指令仅作为研究内容；采用实现时核对完整许可证、依赖与版本。
