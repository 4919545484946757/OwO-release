# OwO Agent IPC 连接器

这是 OwO 维护的高权限本机 IPC 桥接插件。它不会执行 Agent 返回的自然语言，职责仅限于：
收集用户明确确认的请求、最小化应用与局部上下文、脱敏、连接本机第三方 Agent、展示结构化结果并返回用户选择。

启用前必须在设置中心阅读权限与隐私告知并明确同意。完整使用与第三方接入规范见：

- `docs/plugins/agent-ipc-privacy.md`
- `docs/plugins/agent-ipc-integration.md`
- `docs/plugins/agent-ipc-mock.md`

开发构建目标为 `owo_agent_ipc_plugin`。发行打包时必须把该目标复制为
`bin/owo-agent-ipc-plugin.exe`，不要从源码目录直接运行。
