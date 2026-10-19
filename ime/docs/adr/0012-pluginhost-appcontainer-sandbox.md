# ADR 0012：PluginHost 零能力 AppContainer 沙箱基线

- 状态：P3C 已采用，命名管道与已安装插件入口已接入
- 日期：2026-08-02

## 背景

`runtime=process` 只解决地址空间隔离，不能自动阻止插件访问网络、继承宿主句柄或创建无限子进程。Windows Job Object 可限制进程树和内存，但不能表达“禁止网络”；因此在实际插件入口启动前，必须先建立操作系统可验证的权限边界。

## 决策

1. Windows 11 的进程插件必须在 AppContainer 中启动。每个插件 ID 使用 `OwO.Plugin.` 加插件 ID SHA-256 前 128 位小写十六进制生成确定性 profile name；插件 ID 仍使用 manifest 的严格格式校验。
2. v1 AppContainer 的 `SECURITY_CAPABILITIES` 能力数组固定为空，不授予 `internetClient`、`privateNetworkClientServer` 或其他网络能力。创建或打开 profile 后以其 SID 查询隔离目录；中途失败时回滚本次新建 profile。
3. 删除 API 只接受 OwO 的固定前缀和 32 位小写十六进制后缀，兼容 Windows 对 moniker 前缀的大小写规范化，不提供任意 AppContainer 删除能力。
4. 子进程以 `bInheritHandles=FALSE`、挂起状态和扩展启动信息创建；在恢复执行前加入 Job Object。Job 固定启用关闭即终止、活动进程数 1 和单进程内存 128 MiB 限制。
5. 集成测试创建真实零能力 AppContainer，并验证：Token 标记为 AppContainer、位于 Job 中、父进程可连通的回环监听端口对子进程不可达，以及显式标记为可继承的父事件并未指向子进程中的同一内核对象。测试结束删除精确 profile。
6. 本切片只提供并验证沙箱 profile 与启动约束，不执行已安装插件入口。AppContainer 本地命名管道、最小 ACL 和客户端 SID 核对随后由 ADR 0013 固化；在完整 PluginHost 生命周期完成前不得退化为普通进程启动。

## 影响

网络默认关闭由 Windows 安全令牌强制，而不是依赖 manifest、自报状态或 Job Object。插件进程树和内存有确定上限，宿主句柄默认不泄漏。每插件 profile 会产生少量 Windows 用户级状态，需要安装卸载与恢复流程按精确名称管理。

## 回滚

当前沙箱尚未连接 PluginHost 或插件入口。删除 `plugin_sandbox` API、测试和本 ADR 即可回滚，不影响包安装、授权记录或现有输入链路；不得以非沙箱启动作为回滚替代。

## 参考

- [Microsoft：AppContainer isolation](https://learn.microsoft.com/windows/win32/secauthz/appcontainer-isolation)
- [Microsoft：Implementing an AppContainer](https://learn.microsoft.com/windows/win32/secauthz/implementing-an-appcontainer)
- [Microsoft：GetAppContainerFolderPath](https://learn.microsoft.com/windows/win32/api/userenv/nf-userenv-getappcontainerfolderpath)
