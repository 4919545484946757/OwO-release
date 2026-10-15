# ADR 0007：WinUI 3 设置中心工具链与部署边界

- 状态：接受（P3B 首个 UI 切片）
- 日期：2026-08-01

## 背景

设置中心必须独立于 TSF DLL 和 Core Service，并使用 WinUI 3。当前仓库核心为 C++20/CMake；开发机只有 Visual Studio 2022 Build Tools，未安装 WinUI 工作负载。微软当前命令行路径支持使用 .NET 10 SDK 和 `dotnet new winui` 构建 WinUI 3，因此无需为首个 UI 切片安装完整 Visual Studio IDE。

## 决策

- 设置中心是独立 C#/.NET 10 WinUI 3 进程；输入核心、TSF、IPC、配置存储和模型进程继续使用 C++20。
- 固定 .NET SDK 10.0.302（允许同一功能带内的最新补丁）、模块化 `Microsoft.WindowsAppSDK.WinUI` 2.3.0、Windows SDK BuildTools 10.0.28000.2526 和 WinApp 0.5.0。设置中心不引用完整元包，避免携带未使用的 AI、ML、Widgets 等组件。
- 最低目标为 Windows 11 build 22000，x64 为首要交付架构；模板保留 x86/ARM64 项目声明供后续适配。
- 开发阶段使用 unpackaged、framework-dependent 桌面部署，避免 MSIX AppData 虚拟化，使设置中心与非打包 Core Service 共享同一 `%LOCALAPPDATA%\OwO\InputMethod`。正式安装方式随 P5 安装器统一冻结；本 ADR 不承诺 Store 分发或最终证书主体。
- 设置页不得复制 Schema 校验或直接写配置文件。首个切片通过随产品部署的 `owo_config_shell set-all` 调用 C++ 配置存储，以单次原子事务更新完整快照；后续可将该适配层替换为版本化本地 IPC，不改变页面模型。
- 移除模板自带但项目不需要的 `systemAIModels` 受限权限，仅保留桌面全信任能力。

## 后果与回滚

C# 只存在于独立 UI 边界，不进入输入按键路径。设置中心缺失或启动失败不会影响 TSF/Core；用户仍可使用默认配置或诊断 CLI。若 MSIX 与最终输入法安装器冲突，可保留页面和客户端模型，将部署切换为 unpackaged/self-contained 或 framework-dependent，而无需改变配置 Schema。

配置后端 EXE 的正式签名留给安装器切片；统一构建脚本将它聚合到设置中心目录。开发时仍可用 `OWO_CONFIG_SHELL_PATH` 和 `OWO_CONFIG_PATH` 指向测试产物与隔离配置。

## 依据

- Microsoft Learn：WinUI 命令行路径要求 .NET 10 SDK，并由 `dotnet new winui` 创建项目。
- Microsoft Learn：Windows App SDK 支持 packaged、unpackaged、framework-dependent 与 self-contained 等部署组合；选择应与最终分发方式共同决定。
