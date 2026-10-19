# 阶段启动单：P3B 设置中心与配置服务

- 状态：已验收
- 日期：2026-08-01

## 阶段目标

建立可独立测试的版本化配置服务，使输入法配置能够严格校验、原子保存、损坏恢复和安全热加载；配置后端稳定后再接入独立 WinUI 3 设置中心。

## 本阶段范围

- 内部配置 Schema v1 与明确默认值。
- 严格配置解析，拒绝未知版本、未知字段、类型错误、越界值和无效枚举。
- 同目录临时文件写入、落盘、原子替换和上一有效版本备份。
- 启动时主文件损坏/缺失恢复，热加载失败时继续使用上一份有效内存配置。
- 配置 generation 与变更通知，使 Core Service 在安全边界应用新值。
- CLI/CTest 验证后建立独立设置中心进程；UI 不进入 TSF DLL。

## 明确不做

- 不在首个切片引入 WinUI 3、Windows App SDK 或安装器依赖。
- 不冻结最终视觉设计、主题编辑器、账号同步或云配置。
- 不允许设置中心直接改写 TSF 内存或绕过配置服务写文件。
- 不把模型路径、插件权限或任意命令行作为无约束字符串配置。

## 计划新增或修改的文件

- `include/owo/config/`、`src/config/`：配置值、验证、序列化、原子存储和热加载。
- `tests/config/`、`tests/data/`：有效、未知字段、越界、损坏、恢复与热加载夹具。
- `apps/config_shell/`：后续诊断与设置 UI 接入前的最小客户端。
- `docs/architecture/`、`docs/adr/`：Schema、所有权、迁移和 UI 部署决策。

## 核心接口与数据流

```text
设置 UI / config_shell
        ↓ 严格配置候选值
OwO Configuration Store
        ↓ 校验 → 临时文件 → flush → 原子替换 → 备份
版本化配置文件
        ↓ generation 变化通知
Core Service 安全应用
        ↓
TSF 只接收必要的已验证运行态状态
```

首个内部接口保持可替换，不冻结公共 ABI：`ConfigSnapshot`、`ConfigValidationResult`、`IConfigStore` 和 `ConfigReloadResult`。

## 验收标准

- 缺失配置使用文档化默认值；不因首次启动失败。
- 写入中断不能产生部分主文件；成功写入保留可验证上一版本。
- 主文件损坏时恢复备份；主文件与备份均损坏时回到默认值并返回明确诊断。
- 未知版本、未知字段、错误类型、越界和非法 UTF-8 失败封闭。
- 热加载无效文件不覆盖内存中的上一份有效配置。
- 配置 I/O 不发生在 TSF 按键线程。
- Windows Release 构建、功能测试和故障注入测试通过。

## 主要风险

- 配置格式过早膨胀：首切片只加入已存在行为所需字段，其他字段以后按 Schema 版本扩展。
- Windows 原子替换语义：复用 P2 用户频率已验证的同卷临时文件与替换思路，并单独测试备份恢复。
- 热加载竞态：通过不可变快照与单调 generation 发布，不向读取者暴露半更新对象。
- WinUI 3 部署体积：后置到配置后端验收后再做 ADR 和最小安装实验。

## 可回滚方案

配置模块先作为独立静态库和测试目标，不接入 TSF/Core 默认路径；删除新目标即可恢复 P3A 状态。接入后仍保留编译期默认配置作为最终降级。

## 需要开发者确认的问题

根据此前授权，首切片采用最小、可逆默认值并自行推进。涉及 WinUI 3 视觉语言、Windows App SDK 部署和面向用户的快捷键默认值时，再以独立 ADR 暂停冻结。

## 实施记录

- 2026-08-01：新增独立 `owo_config` 静态库和内部 Schema v1，字段只覆盖候选页大小、用户学习开关、模型排序开关和模型预算。严格解析拒绝 BOM、空行、重复/未知/缺失字段、错误类型、未知版本和越界值。
- 2026-08-01：实现同目录临时文件、Windows `FlushFileBuffers`、只备份有效主文件和 `MoveFileExW` 写穿透原子替换。启动加载支持有效备份恢复或带诊断默认值；热加载失败保持上一有效快照，只有内容变化才增加 generation。模块尚未接入 Core/TSF。
- 2026-08-01：新增后台 `ConfigMonitor`。`start()` 通过握手保证文件基线已建立；有效变化以原子 `shared_ptr<const AppConfig>` 和单调 generation 发布，等待接口支持超时/取消。无效热加载只留下诊断，停止令牌立即唤醒后台等待。20 次连续监控竞态测试通过，仍未接入 Core/TSF。
- 2026-08-01：新增 Windows `owo_config_shell`，支持 show、严格单字段 set、等待一次热加载和 repair。真实子进程测试验证 watch 握手后由独立 set 进程触发 generation、无效值不落盘、损坏主文件从最后有效备份修复；跨进程集成连续 10 次通过。
- 2026-08-01：Core Service 新增 `--config <path>`，在 IPC 请求边界读取 `ConfigMonitor` 原子快照；学习开关、模型排序开关与模型超时可热更新，未指定配置时保持既有行为。真实命名管道契约验证关闭学习不落盘、启用后只记录后续选择，以及模型排序关闭/启用/再次关闭的切换，连续 10 次通过。
- 2026-08-01：`candidate_page_size` 接入 Core 分页边界，限制 1～9 以保持 TSF 数字选词可达；配置变化只影响后续请求，不跨线程改写 TSF 当前页。契约测试覆盖页大小从 3 热切换为 2 后的页码、候选切片和 `has_more`。
- 2026-08-01：接受 ADR 0007，新增独立 .NET 10 / 模块化 Windows App SDK WinUI 2.3.0 设置中心骨架，最低 Windows 11 build 22000。页面覆盖 Schema v1 四个字段，通过 C++ `owo_config_shell set-all` 单次原子保存，不复制配置解析器；移除模板无关的系统 AI 受限权限与 AI/ML/Widgets 元包依赖。Debug 构建零警告，CLI 集成测试覆盖完整快照保存与无效快照不落盘。
- 2026-08-01：设置中心改为明确的 framework-dependent 组件引用，并由锁文件固定稳定依赖，Release 顶层负载从约 90 MiB 降至 37.42 MiB。统一构建脚本先生成 C++ 配置后端，再将其聚合到 WinUI 输出，并拒绝 AI/ML/ONNX/Widgets 文件意外回归。
- 2026-08-01：真实 MSIX 调试验收发现 AppData 写入被重定向到包私有 LocalCache，无法与非打包 Core 共享。拒绝申请 `unvirtualizedResources` 受限权限，改用 unpackaged、framework-dependent 启动；移除 WinApp 调试身份依赖和开发人员模式运行要求，保留同目录配置后端聚合。
- 2026-08-01：真实 unpackaged WinUI 窗口保存通过；外部进程使用 C++ 配置后端读取同一隔离文件，Schema v1 与四项值完全一致。构建脚本验证后端同目录聚合，设置中心进程响应正常。
- 2026-08-01：新增 Windows 已知文件夹路径模块，Core 默认监听与设置中心一致的 `%LOCALAPPDATA%\OwO\InputMethod\config\owo.conf`；`--config` 保留显式覆盖，`--no-config` 隔离进程集成测试，避免用户真实设置影响确定性结果。
