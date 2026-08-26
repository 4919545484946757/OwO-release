# 模拟第三方 Agent 测试工具

`owo_agent_mock` 是只用于本机开发测试的第三方 Agent 示例。它与 OwO 通过默认命名管道通信，不加载进输入法、Core 或插件进程。

## 启动

双击项目根目录的 `start-agent-mock.cmd`。脚本会构建并启动模拟服务，控制台保持打开期间服务有效；按 Ctrl+C 可停止。

默认管道：

```text
\\.\pipe\OwO.Agent.External.v1
```

控制台会显示收到的输入、应用信息、局部上下文、会话 ID、动作和指令 ID。测试时不要输入真实密码、令牌、支付信息或其他秘密。

## 首次准备

1. 在设置中心安装 `org.owo.agent-ipc-1.0.1.owopkg`。
2. 阅读 Agent IPC 权限与隐私说明，勾选明确同意后启用。
3. 保持模拟 Agent 控制台运行。
4. 在新的输入窗口中使用 OwO 输入法。

## 建议测试流程

输入：

```text
vbangwozhaowenjian
```

先用数字键或鼠标选择拼音候选，确认文字只暂存在 `v +` 后面。完整消耗请求后会自动
发送；也可在尚有拼音时按 Enter 或 Space 显式发送。

模拟 Agent 首页会返回：

1. `回显本次请求`：返回实际收到的用户请求并完成会话；
2. `显示收到的应用上下文`：显示收到的程序名、窗口类和局部上下文；
3. `模拟执行中状态`：先返回 `executing`，下一次轮询返回 `completed`；
4. `模拟删除文件（高风险测试）`：标记为高风险，OwO 应拒绝在候选框直接选择。

下一页包含“模拟完成任务”和“模拟 Agent 错误”。按 Escape、删除开头 `v` 或关闭候选框时，控制台应收到相同 `session_id` 的 `cancel`。

还应测试候选撤销：先选择一部分文字，在剩余拼音仍存在时按 Backspace。第一次应恢复
已选文字对应的原拼音，后续按键逐字删除，且在 `v` 本身被删除前不应收到 `cancel`。

## 自定义管道

PowerShell 启动脚本支持：

```powershell
.\scripts\start-agent-mock.ps1 -PipeName '\\.\pipe\My.Agent.Test.v1'
```

同时需要在 Agent IPC 插件配置页填入完全相同的管道名。模拟服务为管道设置仅允许当前 Windows 用户、SYSTEM 和管理员访问的 ACL，不接受远程管道。
