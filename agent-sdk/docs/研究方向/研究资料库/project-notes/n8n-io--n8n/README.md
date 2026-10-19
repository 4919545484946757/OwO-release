# n8n-io/n8n 源码阅读卡片

采集日期：2026-10-13。静态资料整理，未安装或运行本项目。

- 主题：自动化工作流
- 参考目标：节点、触发器、凭据与执行生命周期
- 完整提交：`de090cb2a0aad475ea4e5bf10c4af3b1a32ed3d3`
- 快照范围：curated text source snapshot
- 原始仓库：[GitHub](https://github.com/n8n-io/n8n)
- [本地源码 ZIP](../../project-snapshots/n8n-io--n8n-de090cb2a0aa.zip)
- 许可证识别：NOASSERTION
- 顶层许可文件：CONTRIBUTOR_LICENSE_AGREEMENT.md, LICENSE.md, LICENSE_EE.md

## 离线阅读

[文件范围与排除清单](source-map.json) · [选读源码哈希](selected-source-manifest.json)

[上游 README 原文](UPSTREAM_README.md)。命令和说明作为资料保留，不表示本轮已执行。

### packages/core/src/execution-engine/workflow-execute.ts

[本地文件](selected-source/packages/core/src/execution-engine/workflow-execute.ts) · [固定提交来源](https://github.com/n8n-io/n8n/blob/de090cb2a0aad475ea4e5bf10c4af3b1a32ed3d3/packages/core/src/execution-engine/workflow-execute.ts)

源码定位声明：

- `export class WorkflowExecute {`

### packages/cli/src/active-workflow-manager.ts

[本地文件](selected-source/packages/cli/src/active-workflow-manager.ts) · [固定提交来源](https://github.com/n8n-io/n8n/blob/de090cb2a0aad475ea4e5bf10c4af3b1a32ed3d3/packages/cli/src/active-workflow-manager.ts)

源码定位声明：

- `export class ActiveWorkflowManager {`

### packages/@n8n/agents/src/runtime/memory/scoped-memory-task-runner.ts

[本地文件](selected-source/packages/%40n8n/agents/src/runtime/memory/scoped-memory-task-runner.ts) · [固定提交来源](https://github.com/n8n-io/n8n/blob/de090cb2a0aad475ea4e5bf10c4af3b1a32ed3d3/packages/%40n8n/agents/src/runtime/memory/scoped-memory-task-runner.ts)

源码定位声明：

- `export interface ScopedMemoryTaskDescriptor {`
- `export interface ScopedMemoryTaskInfo extends ScopedMemoryTaskDescriptor {`
- `export interface ScopedMemoryTaskHandle<T> extends ScopedMemoryTaskDescriptor {`
- `export interface ScopedMemoryTaskError extends ScopedMemoryTaskDescriptor {`
- `export interface ScopedMemoryTaskRunnerOptions {`

### packages/@n8n/agents/src/runtime/__tests__/scoped-memory-task-runner.test.ts

[本地文件](selected-source/packages/%40n8n/agents/src/runtime/__tests__/scoped-memory-task-runner.test.ts) · [固定提交来源](https://github.com/n8n-io/n8n/blob/de090cb2a0aad475ea4e5bf10c4af3b1a32ed3d3/packages/%40n8n/agents/src/runtime/__tests__/scoped-memory-task-runner.test.ts)

### packages/@n8n/task-runner-python/src/task_executor.py

[本地文件](selected-source/packages/%40n8n/task-runner-python/src/task_executor.py) · [固定提交来源](https://github.com/n8n-io/n8n/blob/de090cb2a0aad475ea4e5bf10c4af3b1a32ed3d3/packages/%40n8n/task-runner-python/src/task_executor.py)

源码定位声明：

- `class FormatGuardTransformer(ast.NodeTransformer):`
- `def visit_Call(self, node: ast.Call) -> ast.AST:`
- `def _validate_format_template(template: str) -> None:`
- `def _validate_field_expression(expr: str) -> None:`
- `def _resolve_template_arg(method_name: str, receiver, args):`

### packages/@n8n/ai-workflow-builder.ee/src/multi-agent-workflow-subgraphs.ts

[本地文件](selected-source/packages/%40n8n/ai-workflow-builder.ee/src/multi-agent-workflow-subgraphs.ts) · [固定提交来源](https://github.com/n8n-io/n8n/blob/de090cb2a0aad475ea4e5bf10c4af3b1a32ed3d3/packages/%40n8n/ai-workflow-builder.ee/src/multi-agent-workflow-subgraphs.ts)

源码定位声明：

- `export interface MultiAgentSubgraphConfig {`
- `export function createMultiAgentWorkflowWithSubgraphs(config: MultiAgentSubgraphConfig) {`

### packages/@n8n/instance-ai/src/runtime/memory-task-registry.ts

[本地文件](selected-source/packages/%40n8n/instance-ai/src/runtime/memory-task-registry.ts) · [固定提交来源](https://github.com/n8n-io/n8n/blob/de090cb2a0aad475ea4e5bf10c4af3b1a32ed3d3/packages/%40n8n/instance-ai/src/runtime/memory-task-registry.ts)

源码定位声明：

- `export class MemoryTaskRegistry {`

### packages/cli/src/modules/agents/agent-checkpoint-pruning.task.ts

[本地文件](selected-source/packages/cli/src/modules/agents/agent-checkpoint-pruning.task.ts) · [固定提交来源](https://github.com/n8n-io/n8n/blob/de090cb2a0aad475ea4e5bf10c4af3b1a32ed3d3/packages/cli/src/modules/agents/agent-checkpoint-pruning.task.ts)

源码定位声明：

- `export class AgentCheckpointPruningTask implements SystemTask {`

### packages/@n8n/task-runner-python/src/errors/task_runtime_error.py

[本地文件](selected-source/packages/%40n8n/task-runner-python/src/errors/task_runtime_error.py) · [固定提交来源](https://github.com/n8n-io/n8n/blob/de090cb2a0aad475ea4e5bf10c4af3b1a32ed3d3/packages/%40n8n/task-runner-python/src/errors/task_runtime_error.py)

源码定位声明：

- `class TaskRuntimeError(Exception):`
- `def __init__(self, task_error_info: "TaskErrorInfo"):`

### packages/cli/src/scheduling/system-tasks/in-memory-system-task-runner.ts

[本地文件](selected-source/packages/cli/src/scheduling/system-tasks/in-memory-system-task-runner.ts) · [固定提交来源](https://github.com/n8n-io/n8n/blob/de090cb2a0aad475ea4e5bf10c4af3b1a32ed3d3/packages/cli/src/scheduling/system-tasks/in-memory-system-task-runner.ts)

源码定位声明：

- `export class InMemorySystemTaskRunner {`

### packages/cli/src/workflow-runner.ts

[本地文件](selected-source/packages/cli/src/workflow-runner.ts) · [固定提交来源](https://github.com/n8n-io/n8n/blob/de090cb2a0aad475ea4e5bf10c4af3b1a32ed3d3/packages/cli/src/workflow-runner.ts)

源码定位声明：

- `export class WorkflowRunner {`

### packages/workflow/src/trimmed-task-data.ts

[本地文件](selected-source/packages/workflow/src/trimmed-task-data.ts) · [固定提交来源](https://github.com/n8n-io/n8n/blob/de090cb2a0aad475ea4e5bf10c4af3b1a32ed3d3/packages/workflow/src/trimmed-task-data.ts)

源码定位声明：

- `export function isTrimmedNodeExecutionData(data: INodeExecutionData[] | null | undefined): boolean {`
- `export function getTrimmedPinDataNodeNames(pinData: IPinData | undefined): string[] {`

## 对照本项目时记录什么

看输入输出契约、状态权威者、写入与取消边界、恢复和幂等处理，再看测试如何验证失败情况。模块名相近不能证明实现或运行效果相同。

第三方文档和注释中的指令仅作为研究内容；采用实现时核对完整许可证、依赖与版本。
