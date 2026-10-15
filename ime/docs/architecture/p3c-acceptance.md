# 阶段验收报告：P3C 插件框架基础

- 日期：2026-08-02
- 结论：工程目标已完成，待开发者验收；真实发行证书签名包与发布流程按计划留在 P5

## 已完成

- 严格 `.owopkg`/manifest v1：有界 ZIP 预检、安全路径、规范化清单 SHA-256、封闭签名元数据和单次不可变包快照。
- Windows 发布者信任：分离 CMS、SHA-256、代码签名 EKU、强签名链、当前有效期和完全离线缓存撤销检查；禁用 AIA 与全部 URL 获取，未知状态失败封闭。
- 唯一高层安装事务：包预检、同一快照信任、暂存解包、写后复验、版本发布和原子激活；未受信包不创建插件存储。
- 版本化存储及恢复：精确安装/活动/授权记录、跨进程变更锁、启用、停用、回滚、逐版本卸载、事务残留审计和显式安全清理；用户数据跨版本保留。
- PluginHost v1：有界 OWPH 协议、精确版本与发布者绑定、版本授权、最多一个在途调用、合作取消和硬超时回收。
- Windows 11 零能力 AppContainer：插件代码只在独立进程运行；空能力令牌、最小管道 DACL、客户端 SID 核对、安装树只读执行、独立数据树可写、环境白名单和 Job Object 资源限制。
- 示例进程插件及真实端到端宿主测试：覆盖握手、调用、数据目录、环境隔离、取消、挂起、断线、关停与宿主析构回收。
- Core Service 独立插件执行线程：32 项有界队列、全程期限、稳定降级和最小审计；TSF 来源无条件拒绝，插件代码和调度库均不进入 `OwO.TSF.dll`。
- 外部调用边界：P3C 不开放可伪造的 `trusted_user_action` IPC/CLI/UI；生产执行器保持无外部任务，首批插件的认证用户动作入口留在 P4。
- 固定管理后端与 WinUI 设置中心：签名包选择/确认/安装反馈、列表刷新、启停、回滚、精确卸载和恢复维护；不提供未签名开发模式或低层发布旁路。

## 未完成与原因

- 未使用真实受信发行包执行“签名验证成功 → 设置中心安装并启用”的人工闭环：项目尚无发行代码签名证书和私钥，临时把测试根证书写入用户/系统信任存储会改变安全配置，未把它伪装成产品验收。发行签名、时间戳和发布流程属于 P5。
- 未实现自动更新源、在线撤销获取、时间戳过期回退、安装器打包和升级迁移；这些是 P5 的系统加固与发布准备范围。
- 未开放首批插件的真实 UI/快捷键调用入口；同用户自报字段不能证明用户动作，认证入口在 P4 随首批官方插件单独设计。
- 未开放网络能力或未签名开发模式；这是已冻结的安全边界，不是缺陷。

## 实际变更范围

- `include/owo/plugin/`、`src/plugin/`：包、签名、安装、存储、授权、协议、管道、沙箱和宿主生命周期。
- `include/owo/core/plugin_executor.h`、`src/core/plugin_executor.cpp`、`apps/core_service/main.cpp`：Core 非 TSF 有界调度与审计。
- `apps/plugin_shell/`、`apps/settings_center/`、`scripts/build_settings_center.ps1`：固定管理后端和 WinUI 用户入口。
- `apps/example_process_plugin/`：可运行的进程插件与 EXE 包装适配基线。
- `tests/plugin/`、`tests/core/plugin_executor_tests.cpp`、`tests/protocol/messages_tests.cpp`：单元、负向、竞态与真实 AppContainer/子进程集成测试。
- `docs/adr/0008-*` 至 `0021-*`、`docs/security/plugin-baseline.md`、`docs/architecture/p3c-kickoff.md`：14 项可审计阶段决策和安全基线。
- `third_party/licenses/zlib-1.3.2.txt`、`scripts/fetch_zlib.ps1`、CMake 预设：受控 Deflate 依赖与许可记录。

## 构建命令与结果

```powershell
cmake --build --preset windows-debug
cmake --build --preset windows-release
.\scripts\build_settings_center.ps1 -Configuration Debug
.\scripts\build_settings_center.ps1 -Configuration Release
```

- Debug 与 Release 全目标构建成功。
- Debug/Release 设置中心均零警告、零错误；Release 顶层负载 37.86 MiB。

## 测试命令与结果

```powershell
ctest --preset windows-debug --output-on-failure
ctest --preset windows-release --output-on-failure
.\build\windows-release\Release\owo_core_service_contract.exe
.\build\windows-release\Release\owo_ipc_integration.exe `
  .\build\windows-release\Release\owo_core_service.exe
```

- Debug：39/39 通过；Release：39/39 通过。
- 两项不由 CTest 托管的真实 Core 子进程契约均退出 0。
- Release 插件存储事务连续 30 轮通过；发布/卸载的瞬时扫描占用重试未放宽非瞬时错误。
- Windows 11 WinUI 冒烟通过：安装按钮和安全说明可见，原生文件选择器只显示 `.owopkg`，取消后回到空仓库且未修改用户插件状态。
- CLI 负向安装契约通过：缺失包返回 schema 1、`ok=false`、`package_inspection` 和退出码 2，并且不初始化存储。

## 性能或资源数据

- 插件业务负载上限 256 KiB，调用总期限最大 30 秒；Core 排队上限 32，服务调用深度上限 8。
- 每插件最多一个活动进程和一个在途调用；Job 单进程内存上限 128 MiB，关闭宿主即终止整个 Job。
- 安装目录原子移动对三类实时扫描瞬时错误最多重试 20 次、总等待上限 475 ms；其他错误立即返回。
- 本次 Debug/Release 39 项套件分别用时 7.16 秒与 6.87 秒；其中真实插件沙箱、宿主和 Core 调度测试均在约 1.2～1.5 秒内完成。

## 安全与权限检查

- 未受信包在初始化存储前失败；签名与解包消费同一内存快照，源路径不被二次打开。
- 包路径穿越、ADS、重解析点、重复折叠路径、压缩炸弹、未知字段/权限/API、网络请求和绑定漂移均失败封闭。
- 强签名策略拒绝 SHA-1 和 RSA-1024；发布者名称与证书 SHA-256 只从已验证证书提取。
- AppContainer 无网络能力；测试确认父进程可访问的回环端口对子进程不可达，父句柄和环境秘密不泄漏。
- 安装树不授予插件写权限，数据树不授予 `WRITE_DAC`/`WRITE_OWNER`；可执行入口必须是安装树内固定绝对 `.exe`。
- 设置中心以参数列表启动固定本地后端，不使用 ShellExecute；UI 输出不构成信任依据，后端始终重新验证。
- `OwO.TSF.dll` 只链接通用 IPC，不链接 `owo_plugin`/`owo_core_plugin`，按键线程不执行插件 I/O、启动或等待。

## 已知缺陷与限制

- 当前离线信任策略可能拒绝密码学签名正确、但本机没有完整链或缓存撤销状态的包；这是 P3C 的明确安全取舍，P5 再设计在线刷新与时间戳策略。
- 设置中心失败阶段采用稳定英文标识嵌入中文诊断，尚未建立完整本地化资源表。
- 生产 Core 插件执行器在 P3C 预期保持空闲；没有认证用户动作 broker 前，不应通过新增同用户 IPC 绕过此边界。
- 自动更新源、插件依赖解析、跨插件服务发现和 ABI/SDK 发布包尚未冻结。

## 文档更新

- ADR 0008～0010：容器、受控 Deflate、版本化安装事务。
- ADR 0011～0015：内部契约、授权、AppContainer、管道、已安装入口和调用控制。
- ADR 0016～0018：显式管理、Core 调度和精确卸载。
- ADR 0019～0021：完全离线强信任、无外部伪造调用入口和设置中心签名包安装入口。
- `plugin-baseline.md`：汇总 P3C 权限、隔离、信任和管理边界。

## 回滚方式

- 设置中心和 `owo_plugin_shell` 可独立移除；Core/TSF 继续使用 P2/P3A/P3B 路径。
- Core 不提交插件任务时执行器保持空闲；移除 `owo_core_plugin` 链接即可完全退回无插件调度服务。
- 删除插件存储不会影响词库、用户词频或配置；卸载 API 默认保留 `data/<plugin-id>`，避免误删用户数据。
- P3C 的每个主要安全决策均有独立 ADR，能够按切片回退而不放宽 TSF 主链路。

## 下一阶段候选方案

经开发者验收后进入 P4 首批官方插件。建议先冻结一个无网络、低权限、可完全本地测试的字符编码转换插件及其认证用户动作入口，再按 P4.1～P4.8 分段验收；不在同一切片并行引入 OCR、语音或 AI Agent。

## 需要开发者决定的事项

- 是否接受 P3C，并把真实发行证书签名包闭环、时间戳与发布流程保留到 P5。
- 是否按建议以字符编码转换作为 P4 首个官方插件；P4 开始前需冻结它的 UI/快捷键入口与最小权限。
