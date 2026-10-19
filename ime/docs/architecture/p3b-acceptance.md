# 阶段验收报告：P3B 设置中心与配置服务

- 日期：2026-08-01
- 结论：通过；最终安装器签名与视觉扩展留在后续发布阶段

## 已完成

- 内部配置 Schema v1，包含候选页大小、用户学习、模型排序和模型超时，具备严格 UTF-8、类型、范围、字段集合与版本校验。
- Windows 同卷临时文件、`FlushFileBuffers`、写穿透原子替换和上一有效主文件备份；损坏恢复与失败诊断明确。
- `ConfigMonitor` 以不可变原子快照和单调 generation 发布有效变化；无效热加载保留上一有效值，停止可立即唤醒。
- Core Service 默认监听 `%LOCALAPPDATA%\OwO\InputMethod\config\owo.conf`，支持 `--config` 覆盖和 `--no-config` 测试隔离。
- 四项配置均在 IPC 请求安全边界生效；TSF 按键线程不执行配置 I/O。
- 独立 WinUI 3 设置中心，使用 .NET 10、模块化 Windows App SDK WinUI 2.3 稳定组件和锁文件；最低 Windows 11 build 22000。
- 设置页不复制 C++ Schema 解析，通过同目录 `owo_config_shell set-all` 单次原子保存完整快照。
- unpackaged、framework-dependent 部署避免 MSIX AppData 虚拟化，不申请 `unvirtualizedResources` 受限权限。
- 统一脚本构建 C++ 配置后端与 WinUI 页面、聚合产物，并拒绝 AI/ML/ONNX/Widgets 依赖意外进入设置中心。

## 验收证据

- 配置存储覆盖缺失文件、有效文件、BOM、非法 UTF-8、未知/重复/缺失字段、类型错误、范围错误、损坏主文件、有效/损坏备份与原子写入。
- 监控竞态连续 20 次通过；配置 CLI 跨进程 watch/set/repair 连续 10 次通过。
- Core 命名管道契约覆盖学习开关、模型开关、5 ms 模型预算、页大小 3→2 热切换、无效配置保持旧快照和关停落盘。
- Windows 11 真实 WinUI 窗口加载和保存通过；外部 C++ 进程读取同一隔离配置文件，Schema 与四项值完全一致。
- WinUI Release x64 零警告，顶层负载 37.51 MiB；未发现 AI、ML、ONNX 或 Widgets 文件。
- 默认 Release 运行时套件 23/23 通过；ONNX Runtime Release 运行时套件 25/25 通过。
- 六个未改动的 Python 离线模型数据/评估测试因当前沙箱临时目录权限未纳入本次计数，不属于 P3B 运行时范围，未据此放宽门禁。

## 安全与隐私

- UI、配置监控和磁盘写入均不进入 TSF DLL 或按键线程。
- 配置格式不接受路径、网络地址、插件权限、任意命令或自由扩展字段。
- 设置中心只启动随产品部署的固定名称本地后端，不使用 ShellExecute，不显示控制台，并通过参数列表传值。
- MSIX 试验发现虚拟化不兼容后已回滚；开发人员模式注册表项恢复到操作前的未配置状态。

## 已知限制

- 当前页面只覆盖 Schema v1，不包含主题、快捷键、账号、同步、模型路径或插件管理。
- 配置后端适配当前为同目录子进程；后续若改为版本化本地 IPC，页面模型和磁盘 Schema 不需改变。
- 正式安装器、代码签名、Windows App Runtime 部署和最终图标/视觉资产在 P5 冻结。
- 真实模型仍受 P3A 许可与质量门禁限制；设置中的模型开关默认关闭。

## 回滚方式

- 删除设置中心不影响 Core/TSF；Core 在配置缺失时使用编译期默认值。
- `--no-config` 可恢复为完全忽略配置的诊断路径。
- 移除 Core 的默认路径接入即可退回显式 `--config`，配置文件本身仍可由 CLI 独立管理。

## 下一阶段

进入 P3C 插件框架基础。首个切片只冻结包容器、manifest、签名/信任与安装目录威胁模型，并实现不加载代码的严格 manifest 校验器；在权限和进程边界验收前不执行第三方插件。
