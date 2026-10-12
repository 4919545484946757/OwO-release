# 独立 WinUI 3 安装器

`apps/setup_center/OwO.Setup.csproj` 生成独立的 `OwO.Setup.exe`。它共享设置中心的向导页面，但不编译普通设置页面。`scripts/package_setup.ps1` 不再调用 Inno Setup；旧 `.iss` 仅保留作历史参考。

## 执行顺序

1. .NET 单文件宿主展开自身运行依赖并显示 WinUI 开始页。此时不解压或安装 OwO 载荷，不请求管理员权限。
2. 安装选项页检查系统架构、语言支持、用户路径和可信确认。
3. 用户点击安装后，在后台验证内置 ZIP 的 SHA-256，检查路径、重复项、链接属性和展开大小限制。
4. 在独立 GUID 临时目录展开 ZIP，然后按包内 `SHA256SUMS.txt` 校验所有文件。校验失败不启动安装脚本。
5. 调用现有 PowerShell 安装后端；TSF 注册需要的 UAC 保持为 Windows 系统界面。安装期间拒绝关闭窗口；结束或失败后可以关闭并清理本次安装临时目录。

独立安装器只接受 x64 Windows。ZIP 脚本入口仍可用。单文件不等于零解压：运行依赖由 .NET 单文件宿主管理，其缓存不由 OwO 删除；完整安装载荷只在确认后展开。因为安装器与被安装的设置中心分别携带依赖，Setup 体积比旧 Inno 包更大。

## 安全边界

- 提权前验证载荷。哈希只用于完整性检查，不替代来源信任或代码签名；当前仍未签名。
- 拒绝绝对路径、路径越界、重复文件名、符号链接和未列入校验清单的文件。
- 只清理本次随机工作目录，遇到重解析点不执行递归删除。
- 真实安装阶段继续使用既有注册、升级、卸载后端，不把 WinUI 安装器安装到目标目录。
- 提权取消或后端失败会显示错误，允许返回选项重试。

## 构建与验证

先发布最新设置中心并生成发行 ZIP，再运行：

```powershell
.\scripts\package_setup.ps1 -Version 0.2.3 -OutputDirectory <新的发行目录>
dotnet run --project tests/settings_center/preferences_tests.csproj -c Release -- <发行ZIP绝对路径>
```

Setup 发布使用专用锁文件和自包含单文件模式。构建脚本检查未留下运行必需的独立 DLL/XBF/PRI，并避免覆盖已有 Setup。

独立宿主使用最小 `App.xaml` 初始化 WinUI，窗口以代码创建。启动时把运行库路径定位到 .NET 自解压目录，并通过 `ResourceManagerRequested` 显式加载该目录的 `OwO.Setup.pri`，避免运行目录或 EXE 改名影响资源加载。启动阶段和失败日志分别写入 `%LOCALAPPDATA%\OwO\InputMethod\logs\setup-startup.log` 与 `setup-crash.log`。

测试覆盖延迟解压、真实发行载荷的逐文件校验、越界/重复/链接/损坏载荷拒绝和临时目录清理。启动日志已验证 XAML 初始化和向导窗口激活；本机界面自动化工具将窗口误识别为 Visual Studio Installer，无法可靠绑定，因此步骤切换、布局和阶段预览仍需手动验收。未自动确认可信来源、批准 UAC 或执行实际安装。

单文件部署与自定义资源管理器采用微软提供的机制：[单文件部署要求](https://learn.microsoft.com/zh-cn/windows/apps/package-and-deploy/unpackage-winui-app)、[ResourceManagerRequested](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.application.resourcemanagerrequested)。

在未安装 .NET/Windows App SDK 的干净 Windows 11 x64 虚拟机上仍需完成最终安装、升级、卸载验收。
