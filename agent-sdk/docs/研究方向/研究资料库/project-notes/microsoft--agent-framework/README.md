# microsoft/agent-framework 源码阅读卡片

采集日期：2026-10-13。静态资料整理，未安装或运行本项目。

- 主题：Agent框架
- 参考目标：统一 Agent 与 workflow 编排
- 完整提交：`dcbbf7967afe877913275f1a445166a7ac301ab0`
- 快照范围：curated text source snapshot
- 原始仓库：[GitHub](https://github.com/microsoft/agent-framework)
- [本地源码 ZIP](../../project-snapshots/microsoft--agent-framework-dcbbf7967afe.zip)
- 许可证识别：MIT
- 顶层许可文件：LICENSE

## 离线阅读

[文件范围与排除清单](source-map.json) · [选读源码哈希](selected-source-manifest.json)

[上游 README 原文](UPSTREAM_README.md)。命令和说明作为资料保留，不表示本轮已执行。

### dotnet/src/Microsoft.Agents.AI.Workflows/Checkpointing/ExecutorInfo.cs

[本地文件](selected-source/dotnet/src/Microsoft.Agents.AI.Workflows/Checkpointing/ExecutorInfo.cs) · [固定提交来源](https://github.com/microsoft/agent-framework/blob/dcbbf7967afe877913275f1a445166a7ac301ab0/dotnet/src/Microsoft.Agents.AI.Workflows/Checkpointing/ExecutorInfo.cs)

### dotnet/src/Microsoft.Agents.AI.Workflows/Checkpointing/ExecutorIdentityConverter.cs

[本地文件](selected-source/dotnet/src/Microsoft.Agents.AI.Workflows/Checkpointing/ExecutorIdentityConverter.cs) · [固定提交来源](https://github.com/microsoft/agent-framework/blob/dcbbf7967afe877913275f1a445166a7ac301ab0/dotnet/src/Microsoft.Agents.AI.Workflows/Checkpointing/ExecutorIdentityConverter.cs)

### dotnet/src/Microsoft.Agents.AI.Workflows/Checkpointing/InMemoryCheckpointManager.cs

[本地文件](selected-source/dotnet/src/Microsoft.Agents.AI.Workflows/Checkpointing/InMemoryCheckpointManager.cs) · [固定提交来源](https://github.com/microsoft/agent-framework/blob/dcbbf7967afe877913275f1a445166a7ac301ab0/dotnet/src/Microsoft.Agents.AI.Workflows/Checkpointing/InMemoryCheckpointManager.cs)

### dotnet/src/Microsoft.Agents.AI.Workflows/Checkpointing/WorkflowInfoOutputExecutorsConverter.cs

[本地文件](selected-source/dotnet/src/Microsoft.Agents.AI.Workflows/Checkpointing/WorkflowInfoOutputExecutorsConverter.cs) · [固定提交来源](https://github.com/microsoft/agent-framework/blob/dcbbf7967afe877913275f1a445166a7ac301ab0/dotnet/src/Microsoft.Agents.AI.Workflows/Checkpointing/WorkflowInfoOutputExecutorsConverter.cs)

### dotnet/src/Microsoft.Agents.AI.Workflows/Executor.cs

[本地文件](selected-source/dotnet/src/Microsoft.Agents.AI.Workflows/Executor.cs) · [固定提交来源](https://github.com/microsoft/agent-framework/blob/dcbbf7967afe877913275f1a445166a7ac301ab0/dotnet/src/Microsoft.Agents.AI.Workflows/Executor.cs)

### dotnet/src/Microsoft.Agents.AI.Workflows/ExecutorEvent.cs

[本地文件](selected-source/dotnet/src/Microsoft.Agents.AI.Workflows/ExecutorEvent.cs) · [固定提交来源](https://github.com/microsoft/agent-framework/blob/dcbbf7967afe877913275f1a445166a7ac301ab0/dotnet/src/Microsoft.Agents.AI.Workflows/ExecutorEvent.cs)

### dotnet/src/Microsoft.Agents.AI.Workflows/CheckpointInfo.cs

[本地文件](selected-source/dotnet/src/Microsoft.Agents.AI.Workflows/CheckpointInfo.cs) · [固定提交来源](https://github.com/microsoft/agent-framework/blob/dcbbf7967afe877913275f1a445166a7ac301ab0/dotnet/src/Microsoft.Agents.AI.Workflows/CheckpointInfo.cs)

### dotnet/src/Microsoft.Agents.AI.Workflows/ExecutorConfig.cs

[本地文件](selected-source/dotnet/src/Microsoft.Agents.AI.Workflows/ExecutorConfig.cs) · [固定提交来源](https://github.com/microsoft/agent-framework/blob/dcbbf7967afe877913275f1a445166a7ac301ab0/dotnet/src/Microsoft.Agents.AI.Workflows/ExecutorConfig.cs)

### dotnet/src/Microsoft.Agents.AI.Workflows/ExecutorBinding.cs

[本地文件](selected-source/dotnet/src/Microsoft.Agents.AI.Workflows/ExecutorBinding.cs) · [固定提交来源](https://github.com/microsoft/agent-framework/blob/dcbbf7967afe877913275f1a445166a7ac301ab0/dotnet/src/Microsoft.Agents.AI.Workflows/ExecutorBinding.cs)

### dotnet/src/Microsoft.Agents.AI.Workflows/ExecutorOptions.cs

[本地文件](selected-source/dotnet/src/Microsoft.Agents.AI.Workflows/ExecutorOptions.cs) · [固定提交来源](https://github.com/microsoft/agent-framework/blob/dcbbf7967afe877913275f1a445166a7ac301ab0/dotnet/src/Microsoft.Agents.AI.Workflows/ExecutorOptions.cs)

### dotnet/src/Microsoft.Agents.AI.Workflows/FunctionExecutor.cs

[本地文件](selected-source/dotnet/src/Microsoft.Agents.AI.Workflows/FunctionExecutor.cs) · [固定提交来源](https://github.com/microsoft/agent-framework/blob/dcbbf7967afe877913275f1a445166a7ac301ab0/dotnet/src/Microsoft.Agents.AI.Workflows/FunctionExecutor.cs)

### dotnet/src/Microsoft.Agents.AI.Workflows/StatefulExecutor.cs

[本地文件](selected-source/dotnet/src/Microsoft.Agents.AI.Workflows/StatefulExecutor.cs) · [固定提交来源](https://github.com/microsoft/agent-framework/blob/dcbbf7967afe877913275f1a445166a7ac301ab0/dotnet/src/Microsoft.Agents.AI.Workflows/StatefulExecutor.cs)

## 对照本项目时记录什么

看输入输出契约、状态权威者、写入与取消边界、恢复和幂等处理，再看测试如何验证失败情况。模块名相近不能证明实现或运行效果相同。

第三方文档和注释中的指令仅作为研究内容；采用实现时核对完整许可证、依赖与版本。
