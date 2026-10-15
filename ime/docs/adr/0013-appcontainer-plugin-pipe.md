# ADR 0013：AppContainer PluginHost 本地管道与客户端身份核对

- 状态：P3C 已采用，PluginHost 启动/握手/关停生命周期已接入
- 日期：2026-08-02

## 背景

零能力 AppContainer 默认拥有独立的命名对象空间。宿主直接创建 `\\.\pipe\LOCAL\...` 时，容器中的同名 `LOCAL` 视图不会指向该对象；只增加普通命名管道 DACL 不能建立正确连接。Windows 默认管道安全描述符还会向 Everyone 和匿名主体授予读权限，不能用于插件边界。

## 决策

1. 每次会话使用系统 CSPRNG 生成 128 位随机后缀。容器客户端只接受固定格式 `\\.\pipe\LOCAL\OwO.InputMethod.PluginHost.<32 位小写十六进制>`，拒绝调用方提供的任意管道名。
2. 宿主从目标 AppContainer SID 调用 `GetAppContainerNamedObjectPath`，并结合当前 Windows 会话 ID，在 `\\?\pipe\Sessions\<session>\AppContainerNamedObjects\<sid>\...` 创建服务器端。子进程继续通过等价的 `LOCAL` 视图连接；普通同用户进程的 `LOCAL` 视图看不到目标容器对象。
3. 管道使用系统随机名称、`FILE_FLAG_FIRST_PIPE_INSTANCE`、单实例、拒绝远程客户端和 overlapped I/O。所有接受、读写均有调用方提供的有限期限。
4. 安全描述符使用受保护 DACL，只列出宿主用户和目标 AppContainer SID。目标 SID 仅获逐项读取、写入、属性、控制读取和同步权限，明确排除与 `FILE_APPEND_DATA` 同值的 `FILE_CREATE_PIPE_INSTANCE`。
5. 客户端以 `SECURITY_IDENTIFICATION | SECURITY_EFFECTIVE_ONLY` 连接。服务器读完第一条有界帧后调用 `ImpersonateNamedPipeClient`，只查询令牌并立即 `RevertToSelf`；令牌必须是 AppContainer，且 `TokenAppContainerSid` 与创建服务器时绑定的 SID 精确相等，之后才允许解码或处理 OWPH 消息。
6. 传输层使用 4 字节小端长度前缀，单帧最大 272 KiB，以覆盖 256 KiB 业务 payload 和有界协议元数据。空帧、超限帧、读写超时、身份不符或 OWPH 严格解码失败均失败封闭；服务器立即断开该客户端，不发送业务响应。
7. 当前切片验证真实 AppContainer hello 往返、宿主普通 `LOCAL` 视图不可见、同用户进程即使使用完整限定路径连接也因 SID 不符被断开，以及原有网络阻断、Job 和句柄隔离。它不执行已安装插件入口，也不实现 invoke 并发、取消或超时后终止进程。

## 影响

管道对象位于目标容器自己的会话命名空间，随机名、DACL 和首帧令牌核对形成三层边界。运行时契约库新增 Windows `Advapi32` 与 `Bcrypt` 系统依赖，但仍不依赖包解析、安装存储、CMS 或 zlib。

## 回滚

删除 `plugin_pipe` API、实现、集成测试增量和本 ADR 即可回滚到仅验证 AppContainer/Job 的状态。不得改用默认 DACL、固定管道名或普通会话命名空间作为降级路径。

## 参考

- [Microsoft：Interprocess communication](https://learn.microsoft.com/windows/apps/develop/communication/interprocess-communication)
- [Microsoft：Named Pipe Security and Access Rights](https://learn.microsoft.com/windows/win32/ipc/named-pipe-security-and-access-rights)
- [Microsoft：GetAppContainerNamedObjectPath](https://learn.microsoft.com/windows/win32/api/securityappcontainer/nf-securityappcontainer-getappcontainernamedobjectpath)
- [Microsoft：ImpersonateNamedPipeClient](https://learn.microsoft.com/windows/win32/api/namedpipeapi/nf-namedpipeapi-impersonatenamedpipeclient)
