# Setup 引导与 WinUI 3 安装助手

当前发行恢复使用 `packaging/windows/OwO-Setup.iss`：Inno Setup 负责管理员授权、完整解包和临时目录生命周期，随后运行 `settings/OwO.Settings.exe --install <安装包目录>`。保留最新 WinUI 页面，不回退整个代码版本。`apps/setup_center` 的单文件实验宿主仍保留，但不再作为默认发行入口。

## 执行顺序

1. 原生 Setup 先检查 x64 架构并请求 Windows UAC 授权，再将随包文件解压到自己的临时目录。这个解包进度不是输入法的实际安装进度。
2. 启动自带 .NET / Windows App SDK 的 WinUI 助手；Setup 等待其结束，不提前清理临时目录。
3. 助手保留“软件介绍→安装选项→安装进度”三个阶段，检查当前用户语言支持、用户路径和可信确认。未确认前不复制到安装位置，不注册 TSF。
4. 用户点击安装后调用现有 PowerShell 后端。助手已由 Setup 提权，通常无需再次创建提权进程，标准输出/错误可直接回传到详细信息区。
5. 安装期间拒绝关闭窗口；失败可返回选项重试，结束并关闭助手后由 Setup 清理自己的临时文件。

当前发行只接受原生 x64 Windows；x64 Windows 中的 32 位应用仍使用 x86 TSF。ZIP 脚本入口继续可用。UAC 授权不能代替 WinUI 页内的可信确认；使用另一个管理员账户授权时，语言配置和用户数据归属于实际运行助手的账户，最终仍需在目标用户账户验收。

## 安全边界

- 打包时核验发行 ZIP 的 SHA-256、暂存目录与 ZIP 内清单一致性及暂存目录逐文件哈希；拒绝越界、重复、链接和未列入清单的文件。
- Inno 使用自身的打包文件完整性检查和临时目录管理，不依赖单文件 WinUI 的运行时资源重定位。现有源码中的安全 ZIP 解压实现及测试保留，但不再属于默认 Setup 运行路径。
- 哈希不替代来源信任和代码签名；当前仍未签名。请从可信来源获取并核对发行校验清单。
- 实际安装沿用目录重叠保护、并排升级避免覆盖已加载 TSF、原生 x64 / x86 注册、语言检测和卸载后端。
- 本次切换不修改设置页、候选算法、插件或用户配置。

## 构建与验证

需要 Inno Setup 6/7。依次构建最新版设置中心、生成发行目录、编译 Setup（新目录避免覆盖旧包）：

```powershell
.\scripts\build_settings_center.ps1 -Configuration Release -NoRestore
.\scripts\package_release.ps1 -Version 0.2.5 -OutputDirectory <新的发行目录>
.\scripts\package_setup.ps1 -Version 0.2.5 -OutputDirectory <新的发行目录>
.\scripts\package_setup.ps1 -Version 0.2.5 -OutputDirectory <新的发行目录> -ValidateOnly
dotnet run --project tests/settings_center/preferences_tests.csproj -c Release -- <发行ZIP绝对路径>
powershell.exe -NoProfile -ExecutionPolicy Bypass -File tests/settings_center/SetupPackagingTests.ps1
```

`-CompilerPath` 可指定 `ISCC.exe`。设置中心仍使用锁文件发布自包含依赖，必须带齐 XBF、PRI、图标和横幅。打包不会运行真实安装。

### 保留的界面与关键修复

- 三阶段界面、顶部横幅、步骤指示、百分比排版、可折叠详情及保持窗口位置的响应式尺寸。
- 选项/进度页面缓存，避免重复挂接控件；可信按钮、语言检测超时/异常区分、当前用户语言回退及路径检查。
- 完整 WinUI 资源发布，避免缺少 XBF/PRI；旧实验宿主的固定 `resources.pri` 改名修复同样保留，没有使用 Git 整体回退。

### 导航回归检查

```powershell
.\scripts\test_setup_navigation.ps1 -PackageRoot <发行暂存目录>
```

该检查直接运行包内助手，不运行外层 Setup、不触发 UAC。重复四轮“下一步→选项页→上一步”，检查真实语言探测结束及未确认时安装按钮保持禁用，再检查进度页和详情展开/收起。只读验证模式 `--verify-installer-navigation` 不授权、不安装、不注册输入法，完成后自动关闭窗口。旧单文件实验宿主的三种文件名检查须显式使用 `-Standalone -SetupPath <旧宿主EXE>`，不可将新 Inno Setup 传入该模式。

现有安全解压、前置条件、导航及设置偏好测试继续保留。外层 Setup 的 UAC、临时目录等待/清理、实际安装、升级与卸载须在 Windows 11 x64（含无 .NET / Windows App SDK 的干净虚拟机）进行最终手动验收。
