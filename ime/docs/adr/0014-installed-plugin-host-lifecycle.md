# ADR 0014：已安装进程插件的受控生命周期

- 状态：P3C 已采用，invoke 调度已由 ADR 0015 接入
- 日期：2026-08-02

## 背景

包安装、版本绑定、AppContainer profile 和安全管道分别通过验收后，仍不能直接执行 manifest 入口。启动时必须把这些边界组合为一条失败封闭的生命周期，并保证插件只能读取其固定安装版本、只能写入自己的数据目录，且启动失败不会留下游离进程。

## 决策

1. `launch_active_plugin` 只接受本地绝对插件存储和严格插件 ID。每次启动重新读取活动记录，并交叉核对已安装 manifest、精确版本、规范化包清单摘要和已验证发布者证书摘要；不缓存或信任调用方传入的入口路径。
2. v1 只启动 `runtime=process`、`network=false` 的固定绝对 `.exe`。入口必须位于活动版本目录内、不是目录或重解析点；安装树和数据树在授权 AppContainer SID 前完整枚举，任何重解析点、枚举错误或超过 2048 项均失败。
3. 安装树使用受保护 DACL：管理员、SYSTEM 和当前用户保留完全控制，插件 AppContainer 仅获读取/执行。独立 `data/<plugin-id>` 树向同一 SID 授予可读写删除但不含 `WRITE_DAC/WRITE_OWNER` 的修改权限；新文件通过目录继承保持相同边界。
4. 子进程不继承句柄，以挂起状态创建，加入关闭即终止、活动进程数 1、单进程内存 128 MiB、未处理异常终止的 Job 后才恢复。任一步骤失败都终止进程并关闭 Job，不提供普通进程降级。
5. 环境块仅复制显式 Windows 启动白名单，另设 `OWO_PLUGIN_DATA`，并将 `TEMP/TMP` 固定到插件数据目录。父进程任意环境变量不会继承；命令行只包含宿主生成的管道名、已验证插件 ID 和数据目录。
6. 启动成功的门禁是：安全管道连接、客户端 AppContainer SID 核对、有效 `hello_request` 和精确插件 ID。ADR 0015 接入调用控制面后，握手才启用排序后的 `cancel.v1`、`invoke.v1` 能力。
7. 正常关停发送有限时 `shutdown_request`，要求匹配请求 ID 与插件 ID 的 acknowledgement，并等待进程以 0 退出。发送、确认、期限或退出码任一异常都终止整个 Job。会话析构和覆盖式移动赋值同样回收 Job。
8. 示例进程插件只依赖独立 `owo_plugin_contract`；集成测试从版本化存储启动真实复制入口，验证 AppContainer/Job、安装目录不可写、数据目录可写、父环境秘密不泄漏、正常关停和析构强制回收。

## 影响

P3C 首次执行真实已安装插件代码，但执行仅发生在零能力 AppContainer 内。当前 API 已完成启动、握手、授权调用、取消、超时、关停和异常回收；管理面与 Core Service 接入仍未完成，因此尚不能作为公共插件 SDK 或默认输入路径。

## 回滚

删除 `plugin_host` API/实现、示例插件、宿主集成测试和本 ADR，即可回到仅验证沙箱与管道的状态。已安装版本、授权记录和输入主链路不受影响；不得以放宽 ACL、继承父环境或非沙箱启动作为回滚方式。

## 参考

- [Microsoft：Implementing an AppContainer](https://learn.microsoft.com/windows/win32/secauthz/implementing-an-appcontainer)
- [Microsoft：CreateProcessW](https://learn.microsoft.com/windows/win32/api/processthreadsapi/nf-processthreadsapi-createprocessw)
- [Microsoft：Job Objects](https://learn.microsoft.com/windows/win32/procthread/job-objects)
- [Microsoft：File Security and Access Rights](https://learn.microsoft.com/windows/win32/fileio/file-security-and-access-rights)
