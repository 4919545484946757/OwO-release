# OwO Agent IPC 连接器

当前插件版本：`1.0.1`

这是 OwO 维护的高权限本机 IPC 桥接插件。它不会执行 Agent 返回的自然语言，职责仅限于：
收集用户明确确认的请求、最小化应用与局部上下文、脱敏、连接本机第三方 Agent、展示结构化结果并返回用户选择。

启用前必须在设置中心阅读权限与隐私告知并明确同意。完整使用与第三方接入规范见：

- `docs/plugins/agent-ipc-privacy.md`
- `docs/plugins/agent-ipc-integration.md`
- `docs/plugins/agent-ipc-mock.md`

开发构建目标为 `owo_agent_ipc_plugin`。发行打包时必须把该目标复制为
`bin/owo-agent-ipc-plugin.exe`，不要从源码目录直接运行。

使用小写 `v` 作为预编辑串首字符进入 Agent 模式。`v` 与后续内容以 `v +` 分隔显示，
不参与拼音解析。候选完整消耗请求时可自动发送；Enter 或 Space 可显式确认。删除已选
候选时先恢复其原拼音，只有明确删除开头的 `v` 才退出尚未结束的 Agent 会话。

该连接器不包含模型或 Agent 工具运行时，也不会执行返回的自然语言。第三方 Agent
必须按照接入规范提供本机命名管道服务，并在自身可信界面确认高风险系统操作。
