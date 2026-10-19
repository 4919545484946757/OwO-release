# microsoft/autogen 源码阅读卡片

采集日期：2026-10-13。静态资料整理，未安装或运行本项目。

- 主题：Agent框架
- 参考目标：A programming framework for agentic AI
- 完整提交：`027ecf0a379bcc1d09956d46d12d44a3ad9cee14`
- 快照范围：prior batch source snapshot
- 原始仓库：[GitHub](https://github.com/microsoft/autogen)
- [本地源码 ZIP](../../project-snapshots/AutoGen-027ecf0a379b.zip)
- 许可证识别：CC-BY-4.0
- 顶层许可文件：LICENSE, LICENSE-CODE

## 离线阅读

[文件范围与排除清单](source-map.json) · [选读源码哈希](selected-source-manifest.json)

[上游 README 原文](UPSTREAM_README.md)。命令和说明作为资料保留，不表示本轮已执行。

### python/packages/autogen-agentchat/src/autogen_agentchat/tools/_task_runner_tool.py

[本地文件](selected-source/python/packages/autogen-agentchat/src/autogen_agentchat/tools/_task_runner_tool.py) · [固定提交来源](https://github.com/microsoft/autogen/blob/027ecf0a379bcc1d09956d46d12d44a3ad9cee14/python/packages/autogen-agentchat/src/autogen_agentchat/tools/_task_runner_tool.py)

源码定位声明：

- `class TaskRunnerToolArgs(BaseModel):`
- `class TaskRunnerTool(BaseStreamTool[TaskRunnerToolArgs, BaseAgentEvent | BaseChatMessage, TaskResult], ABC):`
- `def __init__(`
- `async def run(self, args: TaskRunnerToolArgs, cancellation_token: CancellationToken) -> TaskResult:`
- `async def run_stream(`

### dotnet/src/Microsoft.AutoGen/Contracts/IAgentRuntime.cs

[本地文件](selected-source/dotnet/src/Microsoft.AutoGen/Contracts/IAgentRuntime.cs) · [固定提交来源](https://github.com/microsoft/autogen/blob/027ecf0a379bcc1d09956d46d12d44a3ad9cee14/dotnet/src/Microsoft.AutoGen/Contracts/IAgentRuntime.cs)

### dotnet/src/Microsoft.AutoGen/Core.Grpc/GrpcAgentRuntime.cs

[本地文件](selected-source/dotnet/src/Microsoft.AutoGen/Core.Grpc/GrpcAgentRuntime.cs) · [固定提交来源](https://github.com/microsoft/autogen/blob/027ecf0a379bcc1d09956d46d12d44a3ad9cee14/dotnet/src/Microsoft.AutoGen/Core.Grpc/GrpcAgentRuntime.cs)

### dotnet/src/Microsoft.AutoGen/Core/AgentRuntimeExtensions.cs

[本地文件](selected-source/dotnet/src/Microsoft.AutoGen/Core/AgentRuntimeExtensions.cs) · [固定提交来源](https://github.com/microsoft/autogen/blob/027ecf0a379bcc1d09956d46d12d44a3ad9cee14/dotnet/src/Microsoft.AutoGen/Core/AgentRuntimeExtensions.cs)

### dotnet/src/Microsoft.AutoGen/Core.Grpc/IAgentRuntimeExtensions.cs

[本地文件](selected-source/dotnet/src/Microsoft.AutoGen/Core.Grpc/IAgentRuntimeExtensions.cs) · [固定提交来源](https://github.com/microsoft/autogen/blob/027ecf0a379bcc1d09956d46d12d44a3ad9cee14/dotnet/src/Microsoft.AutoGen/Core.Grpc/IAgentRuntimeExtensions.cs)

### dotnet/src/Microsoft.AutoGen/AgentChat/Abstractions/Tasks.cs

[本地文件](selected-source/dotnet/src/Microsoft.AutoGen/AgentChat/Abstractions/Tasks.cs) · [固定提交来源](https://github.com/microsoft/autogen/blob/027ecf0a379bcc1d09956d46d12d44a3ad9cee14/dotnet/src/Microsoft.AutoGen/AgentChat/Abstractions/Tasks.cs)

### python/packages/autogen-core/src/autogen_core/_agent_runtime.py

[本地文件](selected-source/python/packages/autogen-core/src/autogen_core/_agent_runtime.py) · [固定提交来源](https://github.com/microsoft/autogen/blob/027ecf0a379bcc1d09956d46d12d44a3ad9cee14/python/packages/autogen-core/src/autogen_core/_agent_runtime.py)

源码定位声明：

- `class AgentRuntime(Protocol):`
- `async def send_message(`
- `async def publish_message(`
- `async def register_factory(`
- `class MyMessage:`

### python/samples/task_centric_memory/chat_with_teachable_agent.py

[本地文件](selected-source/python/samples/task_centric_memory/chat_with_teachable_agent.py) · [固定提交来源](https://github.com/microsoft/autogen/blob/027ecf0a379bcc1d09956d46d12d44a3ad9cee14/python/samples/task_centric_memory/chat_with_teachable_agent.py)

源码定位声明：

- `async def main():`

### python/packages/autogen-core/src/autogen_core/_single_threaded_agent_runtime.py

[本地文件](selected-source/python/packages/autogen-core/src/autogen_core/_single_threaded_agent_runtime.py) · [固定提交来源](https://github.com/microsoft/autogen/blob/027ecf0a379bcc1d09956d46d12d44a3ad9cee14/python/packages/autogen-core/src/autogen_core/_single_threaded_agent_runtime.py)

源码定位声明：

- `class PublishMessageEnvelope:`
- `class SendMessageEnvelope:`
- `class ResponseMessageEnvelope:`
- `class RunContext:`
- `def __init__(self, runtime: SingleThreadedAgentRuntime) -> None:`

### dotnet/src/Microsoft.AutoGen/RuntimeGateway.Grpc/Abstractions/AgentsRegistryState.cs

[本地文件](selected-source/dotnet/src/Microsoft.AutoGen/RuntimeGateway.Grpc/Abstractions/AgentsRegistryState.cs) · [固定提交来源](https://github.com/microsoft/autogen/blob/027ecf0a379bcc1d09956d46d12d44a3ad9cee14/dotnet/src/Microsoft.AutoGen/RuntimeGateway.Grpc/Abstractions/AgentsRegistryState.cs)

### dotnet/src/Microsoft.AutoGen/RuntimeGateway.Grpc/Services/AgentWorkerHostingExtensions.cs

[本地文件](selected-source/dotnet/src/Microsoft.AutoGen/RuntimeGateway.Grpc/Services/AgentWorkerHostingExtensions.cs) · [固定提交来源](https://github.com/microsoft/autogen/blob/027ecf0a379bcc1d09956d46d12d44a3ad9cee14/dotnet/src/Microsoft.AutoGen/RuntimeGateway.Grpc/Services/AgentWorkerHostingExtensions.cs)

### dotnet/src/AutoGen.Gemini/GeminiChatAgent.cs

[本地文件](selected-source/dotnet/src/AutoGen.Gemini/GeminiChatAgent.cs) · [固定提交来源](https://github.com/microsoft/autogen/blob/027ecf0a379bcc1d09956d46d12d44a3ad9cee14/dotnet/src/AutoGen.Gemini/GeminiChatAgent.cs)

## 对照本项目时记录什么

看输入输出契约、状态权威者、写入与取消边界、恢复和幂等处理，再看测试如何验证失败情况。模块名相近不能证明实现或运行效果相同。

第三方文档和注释中的指令仅作为研究内容；采用实现时核对完整许可证、依赖与版本。
