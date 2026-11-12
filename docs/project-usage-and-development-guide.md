# OwO 项目使用及源代码环境配置与运行说明书

适用版本：OwO `0.3.0`
更新日期：2026-11-12
目标平台：Windows 11 x64；发行包同时携带 x86 TSF，用于 x64 Windows 中的 32 位应用。不支持 ARM64 Windows。

本文是单入口手册：前半部分面向普通用户，后半部分面向源代码构建、调试和打包。专题细节见文末导航。

## 1. 普通用户使用

### 1.1 安装前检查

- Windows 11 x64。
- 已安装“中文（简体，中国）”语言和基本输入支持。OwO 不要求把中文设为 Windows 显示语言。
- 安装 Setup 需要管理员授权。安装助手自带 .NET 与 Windows App SDK 运行时，不需要用户另行安装。
- 从可信来源获取安装包，并在安装前核对发行目录中的 `SHA256SUMS.txt`。当前项目未进行商业代码签名。

### 1.2 安装流程

推荐运行 `OwO-Input-Method-0.3.0-windows-x64-Setup.exe`：

1. 原生 Setup 检查 x64 架构并请求 UAC 授权。
2. Setup 解包并启动 WinUI 3 安装助手。
3. 阅读软件介绍。
4. 选择安装位置、快捷方式和随包插件选项。
5. 勾选“我确认安装来源可信”后开始安装。
6. 等待进度完成。进度按复制、校验、注册、配置和启动等真实阶段展示。

便携 ZIP 可解压后运行 `Install-OwO.cmd`。该入口不自动导入插件授权和状态。

### 1.3 启用输入法

安装完成后按 `Win + Space`，选择 `OwO Input Method`。如果目标应用此前已打开，可能仍保留旧输入法，请新开窗口或重新切换一次。

常用入口：

- 设置中心：开始菜单“OwO 输入法设置”，或安装目录中的 `Open-OwO-Settings.cmd`
- 重启服务：`Start-OwO.cmd`
- 卸载：开始菜单卸载项，或 `Uninstall-OwO.cmd`

### 1.4 日常输入

默认输入方案为全拼。按 `Ctrl + Space` 可在启用的输入方案间切换；当绑定仍为默认 `Ctrl + Space` 时，按住 `Ctrl` 并点按空格会显示方案列表，松开 `Ctrl` 确认。

常用操作：

| 操作 | 默认按键 |
| --- | --- |
| 选择首候选 | `Space` |
| 选择编号候选 | 数字键 |
| 上一页 / 下一页 | `[` / `]` |
| 自动展开并切换选中页 | `↑` / `↓` |
| 移动拼音预编辑光标 | `←` / `→` |
| 移动应用文本光标 | `Shift + 方向键` |
| 切换全拼纠错 | `Ctrl + Q` |
| 原样输出当前编码 | `Enter` |

设置中心支持启用、关闭、改键和为同一动作添加多组快捷键。方案列表顺序也在设置中心的输入方案页面管理。

### 1.5 设置中心

设置中心按页面组织：

- 通用：自动保存、外观、配置导入导出、更新与资源下载、调试与性能。
- 输入方案：方案启用、排序、方案专属配置。
- 候选与显示：每页数量、分区、换行、预览位置等。
- 词频学习：学习开关、灵敏度、用户词拼音补全。
- 词库管理器：用户词库、基础中文词典、独立导入字典、发现词库。
- 模型管理器：用户模型 API 与选区大模型功能。
- 快捷键：全局及方案专属按键。
- 插件管理器：已安装、在线发现、权限、版本与恢复。

搜索框可查找设置项、快捷键和插件配置。配置默认自动保存；关闭自动保存后会显示本页保存按钮。

### 1.6 数据位置与隐私

默认数据目录：

```text
%LOCALAPPDATA%\OwO\InputMethod
```

主要内容：

- `config\owo.conf`：主配置
- `data\user-frequency.owuf`：用户词频
- `data\dictionaries`：独立字典快照
- `plugins`：插件包、授权和插件数据
- `diagnostics`：本地性能诊断
- `logs`：运行与安装日志

卸载默认保留用户数据。词库、配置和插件授权均可能包含个人信息，迁移或共享前应先检查内容。

基础输入、词典查询和上屏可离线运行。智能排序依赖本地 libime 模型；模型不可用时基础候选继续。联网更新、在线词库、插件联网和模型 API 均需显式操作与确认。

## 2. 源代码环境配置

### 2.1 获取源代码

Git 方式：

```powershell
git clone <可信仓库地址> OwO
cd OwO
```

发行源码包方式：解压 `OwO-Input-Method-<版本>-source.zip`，并核对 `SHA256SUMS.txt`。

源码主许可为 `GPL-3.0-only`。发行物还包含 libime、Fcitx5Utils、词典、Windows App SDK 等第三方许可，必须保留 `LICENSES` 目录。

### 2.2 必需工具

| 工具 | 版本或组件 | 用途 |
| --- | --- | --- |
| Windows 11 | x64 | 开发与运行 |
| Visual Studio 2022 Build Tools | MSVC v143，含 x64 与 x86 工具链 | C++ 编译 |
| Windows SDK | Windows 11 SDK，当前开发环境使用 10.0.26100 | Win32、TSF、D2D/DWrite |
| CMake | 3.25 或更高 | 生成构建树 |
| .NET SDK | `global.json` 指定 `10.0.403`，允许同补丁号前滚 | 设置中心与托管测试 |
| PowerShell | Windows PowerShell 5.1 或 PowerShell 7 | 构建脚本 |
| Git | 任意近版本 | 获取源码 |
| tar / curl | Windows 内置即可 | 固定来源依赖下载 |

安装 Visual Studio Build Tools 时至少选择“使用 C++ 的桌面开发”、MSVC v143、Windows 11 SDK 和 CMake 组件。若使用完整 Visual Studio 2022，也选择相同工作负载。

首次构建设置中心需要 NuGet 还原，因此需要联网或预先准备本地 NuGet 缓存。构建脚本会优先使用 `build\dependencies\dotnet-<版本>` 中的固定 SDK，找不到时使用系统 `dotnet`。

### 2.3 可选工具

- Inno Setup 6 或 7：制作 Setup。未安装到默认位置时，用 `package_setup.ps1 -CompilerPath` 指定 `ISCC.exe`。
- MSYS2 CLANG64 工具链：编译和更新 libime / Fcitx5Utils 原生运行时。
- Python 3：部分数据完整性、词库和工具脚本。
- Node.js：部分质量审查和报告检查脚本。
- ONNX Runtime 1.28.0：实验性 ONNX 排序路径。
- zlib 1.3.2：启用插件压缩包 Deflate 提取时使用。

这些不是基础编译的必需项，但完整发行或特定实验功能可能需要。

### 2.4 源码目录速览

| 目录 | 内容 |
| --- | --- |
| `apps` | 设置中心、Core、ModelHost、CLI、插件管理等应用 |
| `src` | C++ 生产源码 |
| `include` | 公共头文件与协议 |
| `plugins` | 官方插件源码 |
| `data` | 固定来源清单和五笔数据 |
| `docs` | 项目、功能、插件、性能与发行文档 |
| `scripts` | 构建、启动、测试、打包脚本 |
| `tests` | 原生、托管、UI 与合同测试 |
| `tools` | 词典编译、质量工具与辅助程序 |
| `packaging` | Setup、Agent SDK 等打包定义 |

不要把个人配置、词典快照或密钥放入仓库目录。`build\`、`artifacts\` 和 `bin\obj` 均为生成物。

## 3. 构建与运行

以下命令均在仓库根目录的 PowerShell 中执行。

### 3.1 基础 x64 构建

```powershell
cmake --preset windows-release
cmake --build --preset windows-release
```

生成目录：

```text
build\windows-release\Release
```

关键产物包括：

- `OwO.TSF.dll`
- `owo_core_service.exe`
- `owo_model_host.exe`
- `owo_model_shell.exe`
- `owo_ipc_shell.exe`
- `owo_config_shell.exe`
- `owo_plugin_shell.exe`
- `owo_tsf_profile_check.exe`
- `owo_runtime_launcher.exe`

### 3.2 x86 TSF

x64 Windows 中的 32 位应用需要 x86 TSF：

```powershell
cmake --preset windows-x86-release
cmake --build --preset windows-x86-release --target owo_tsf
```

生成目录：

```text
build\windows-x86-release\Release
```

### 3.3 设置中心

```powershell
.\scripts\build_settings_center.ps1 -Configuration Release -RuntimeIdentifier win-x64
```

发布目录：

```text
build\settings-publish\win-x64
```

该脚本会构建 `owo_config_shell.exe` 与 `owo_plugin_shell.exe`，发布自包含 WinUI 3 设置中心，并检查必需文件。

### 3.4 一键开发运行

完成至少一次 x64 Release 配置后，可运行：

```powershell
.\start-owo-dev.cmd
```

该脚本会：

1. 重编当前工作区的 Release Core、TSF 和相关工具。
2. 构建 x86 TSF。
3. 重启本工作区对应的 Core / ModelHost。
4. 用哈希命名 DLL 注册开发版 x64 / x86 TSF。
5. 刷新 TSF 描述缓存。
6. 激活当前会话的 OwO 输入配置。
7. 打开记事本供手动测试。

常用参数：

```powershell
.\start-owo-dev.cmd -OpenSettings
.\start-owo-dev.cmd -DisableModelHost
.\start-owo-dev.cmd -OpenSettings -OpenNotepad
.\start-owo-dev.cmd -DeployWubi86 -AcceptDevelopmentPackageRisk
```

开发注册只影响当前 Windows 用户，不会卸载正式安装版。已打开应用可能继续加载旧 DLL；请使用新窗口或 `scripts\open-candidate-testpad.ps1`。

### 3.5 仅打开开发设置中心

```powershell
.\open-owo-settings.cmd
```

该入口会停止锁定开发输出的设置中心进程、重编设置中心、发布并启动，再重启开发 Core。若只想运行已有设置中心，可运行：

```powershell
.\scripts\open-settings.ps1
```

### 3.6 开发隔离环境变量

自动化和联调应使用独立目录，不要指向日常数据：

```powershell
$env:OWO_CONFIG_PATH = "D:\owo-test\config\owo.conf"
$env:OWO_PLUGIN_STORE_PATH = "D:\owo-test\plugins"
$env:OWO_DICTIONARY_STORE_PATH = "D:\owo-test\dictionaries"
```

设置中心还支持：

- `OWO_CONFIG_SHELL_PATH`
- `OWO_PLUGIN_SHELL_PATH`

测试结束后在同一 PowerShell 会话中移除变量，避免影响后续命令。

## 4. 词典与智能排序依赖

### 4.1 从固定来源重建基础词典

`data\*.manifest` 记录 rime-ice 固定提交、SHA-256、来源和许可。完整流程：

```powershell
cmake --build --preset windows-release --target owo_lexicon_compiler owo_lexicon_merge
```

按清单下载并校验以下来源：

- `base.dict.yaml`
- `8105.dict.yaml`
- `ext.dict.yaml`
- `others.dict.yaml`
- `tencent.dict.yaml`

使用 `owo_lexicon_compiler` 生成各来源 `.owolx`；`others` 和 `tencent` 需要按清单传入 `8105.dict.yaml` 作为注音来源。随后用 `owo_lexicon_merge` 合并为：

```text
build\windows-release\rime-ice-cn-2026.06.30-v2.owolx
```

再生成运行时 v3 词典：

```powershell
.\scripts\prepare-runtime-lexicon.ps1
```

输出：

```text
build\windows-release\rime-ice-cn-2026.06.30-v3.owolx
```

如果已有可信 v2 词典，可直接放到上述路径并运行 `prepare-runtime-lexicon.ps1`。不要只修改版本号冒充 v3；脚本会重建排序和索引。

### 4.2 libime 模型

下载固定模型：

```powershell
.\scripts\fetch_libime_model.ps1
```

输出：

```text
build\dependencies\libime-1.1.15\model-20260804\lib\libime\zh_CN.lm
```

基础开发可在模型运行时缺失时使用：

```powershell
.\start-owo-dev.cmd -DisableModelHost
```

完整智能排序和发行包需要 `build\dependencies\libime-1.1.15\runtime-x86_64` 中的桥接 DLL 及其依赖。更新桥接时使用 MSYS2 CLANG64 构建 `owo_libime_bridge`，停止占用 DLL 的 ModelHost，再运行：

```powershell
.\scripts\stage_libime_runtime.ps1
```

当前仓库没有从零安装 MSYS2 并编译全部 libime 依赖的一键脚本；新环境应使用可信预置运行时，或按 libime、Fcitx5Utils 和 MSYS2 文档自行构建后再暂存。

### 4.3 其他可选依赖

ONNX Runtime：

```powershell
.\scripts\fetch-onnxruntime.ps1
cmake --preset windows-release-ort
cmake --build --preset windows-release-ort
```

zlib：

```powershell
.\scripts\fetch_zlib.ps1
cmake --preset windows-release-zlib
cmake --build --preset windows-release-zlib
```

这两个是可选功能路径，不用于基础输入法运行。

## 5. 测试与调试

### 5.1 原生测试

```powershell
ctest --preset windows-release
```

或指定目录：

```powershell
ctest --test-dir build\windows-release -C Release --output-on-failure
```

定向示例：

```powershell
ctest --test-dir build\windows-release -C Release `
  -R "^owo\.(tsf\.input_method_context|engine\.input_method_composition|config\.store)$" `
  --output-on-failure
```

### 5.2 设置中心与托管测试

```powershell
dotnet run --project tests\settings_center\preferences_tests.csproj -c Release
dotnet run --project tests\settings_center\dictionary_encoding_tests.csproj -c Release
```

UI 自动化脚本在 `scripts\test_*_ui.ps1`，例如：

```powershell
.\scripts\test_input_scheme_ui.ps1
```

带英文插件 fixture：

```powershell
.\scripts\test_input_scheme_ui.ps1 `
  -EnglishPackage artifacts\plugins\owo.plugin.english-1.0.1.owopkg `
  -RequirePackageIcon
```

### 5.3 开发 CLI

```powershell
.\owo-dev.cmd build
.\owo-dev.cmd version
.\owo-dev.cmd --dev --root D:\owo-cli\quality init
.\owo-dev.cmd --dev --root D:\owo-cli\quality doctor
```

批量候选质量测试：

```powershell
.\owo-dev.cmd --dev --root D:\owo-cli\quality input batch `
  --file .\examples\cli\pinyin-quality.jsonl `
  --lexicon .\build\windows-release\rime-ice-cn-2026.06.30-v2.owolx `
  --report baseline --top-k 10 --max-pages 4 --page-size 7 `
  --check-expanded --repeat 3 --hash-assets --allow-misses
```

CLI 不连接日常 Core，不上屏，不读取焦点窗口。用户手册见 `docs\cli-user.md`，AI 与 CI 接口见 `docs\cli-ai.md`。

### 5.4 TSF 问题排查

注册成功但界面未更新时：

```powershell
.\build\windows-release\Release\owo_tsf_profile_check.exe --refresh-cache
```

使用独立测试窗口：

```powershell
powershell.exe -NoProfile -STA -File .\scripts\open-candidate-testpad.ps1
```

开发版 TSF 使用哈希命名 DLL，不覆盖仍被旧应用占用的文件。判断是否加载新版时，应查看测试进程实际加载的 DLL 路径，不能只看注册表。

### 5.5 性能诊断

安装版或开发设置中心的“调试与性能”页面可测量排队、IPC、解析、纠错、词典查询、候选布局、模型和绘制提交耗时。采样默认关闭，不记录输入原文、候选文字、上下文或密钥。详细解释见 `docs\candidate-performance-debugging.md`。

## 6. 插件开发与输入方案

官方插件源码位于 `plugins\`：

- `candidate-glass`：候选框亚克力主题
- `unicode-palette`：Unicode 符号列表
- `agent-ipc`：第三方 Agent IPC 连接器
- `english`：wordfreq 英文输入方案
- `wubi86`：五笔 86 输入方案
- `input-method-example`：输入方案示例

插件不加载进 TSF 或 Core 进程。进程插件运行在受限宿主中，资源插件只读取声明式资源。所有权限均需逐版本确认。

入门文档：

- 插件 API：`docs\plugins\development.md`
- 输入方案：`docs\plugins\input-schemes.md`
- 方案设置与图标：`docs\plugins\scheme-settings-package.md`
- Agent 接入：`docs\plugins\agent-ipc-integration.md`

打包官方插件示例：

```powershell
cmake --build --preset windows-release --target owo_english_package owo_wubi86_package

.\scripts\package_plugins.ps1 -PluginIds owo.plugin.english
.\scripts\package_plugins.ps1 `
  -PluginRoot .\build\windows-release\wubi86-plugin `
  -PluginIds owo.plugin.wubi86
```

未签名开发包安装必须显式确认风险，不会自动复制授权记录。

## 7. 发行打包

### 7.1 打包前置

确认以下产物已准备好：

- x64 Core、ModelHost、TSF、设置中心
- x86 TSF
- v3 基础词典
- libime 运行时与 `zh_CN.lm`
- 需要随包的插件目录
- 第三方许可文件

不要在源码或用户数据目录中直接打包；脚本会生成独立 staging 目录。

### 7.2 生成发行目录

```powershell
.\scripts\package_release.ps1 `
  -Version 0.3.0 `
  -BundledPluginStore "$env:LOCALAPPDATA\OwO\InputMethod\plugins"
```

输出：

```text
artifacts\release\0.3.0
```

包含二进制 ZIP、源码 ZIP、完整 staging 目录、`SHA256SUMS.txt` 和 `release-build.json`。

### 7.3 生成 Setup

```powershell
.\scripts\package_setup.ps1 -Version 0.3.0
```

如 Inno Setup 不在默认位置：

```powershell
.\scripts\package_setup.ps1 -Version 0.3.0 -CompilerPath "C:\Path\To\ISCC.exe"
```

仅校验已有 Setup 输入：

```powershell
.\scripts\package_setup.ps1 -Version 0.3.0 -ValidateOnly
```

### 7.4 预览安装界面

不执行真实安装：

```powershell
.\open-owo-installer.cmd
```

分阶段预览：

```powershell
.\open-owo-installer-start.cmd
.\open-owo-installer-options.cmd
.\open-owo-installer-progress.cmd
```

状态页预览：

```powershell
.\open-owo-installer-checking.cmd
.\open-owo-installer-ready.cmd
.\open-owo-installer-installing.cmd
.\open-owo-installer-success.cmd
.\open-owo-installer-error.cmd
```

### 7.5 发布 GitHub Release

`scripts\publish_github_release.ps1` 面向官方仓库 `4919545484946757/OwO-release`。默认生成草稿；只有显式 `-Publish` 才会发布。发布前必须核对版本、文件清单和 Release Notes。

## 8. 常见问题

### 8.1 `cmake` 找不到 Visual Studio

确认 Visual Studio 2022 Build Tools、MSVC v143 和 Windows 11 SDK 已安装，并从普通 PowerShell 重新打开终端。不要在仓库外运行依赖仓库 `CMakePresets.json` 的命令。

### 8.2 Core 或 ModelHost 输出被锁定

先关闭设置中心和相关测试窗口，再运行：

```powershell
.\build\windows-release\Release\owo_ipc_shell.exe --shutdown
.\build\windows-release\Release\owo_model_shell.exe --shutdown
```

仍被锁定时，用任务管理器确认进程路径，只停止指向当前 `build\windows-release\Release` 的开发进程。

### 8.3 设置中心显示找不到后端

确认 `build\settings-publish\win-x64` 中存在：

- `OwO.Settings.exe`
- `owo_config_shell.exe`
- `owo_plugin_shell.exe`
- `OwO.Settings.pri`
- `App.xbf`、`MainPage.xbf`、`MainWindow.xbf`

缺失时重新运行 `open-owo-settings.cmd`。

### 8.4 输入法列表里没有 OwO

1. 确认系统已安装简体中文基本输入支持。
2. 运行 `owo_tsf_profile_check.exe --activate-session`。
3. 按 `Win + Space` 选择 OwO。
4. 新开应用窗口再测试。

### 8.5 开发版界面仍是旧效果

TSF 描述缓存或已打开应用可能保留旧 DLL。运行 `owo_tsf_profile_check.exe --refresh-cache`，并使用新窗口或 `open-candidate-testpad.ps1`。以进程实际加载的 DLL 为准。

### 8.6 打包提示词典或 libime 缺失

发行打包不自动下载所有依赖。先完成第 4 节的 v3 词典、libime 模型和运行时准备。只修改源码但未更新原生依赖时，不要跳过哈希校验。

### 8.7 插件安装后没有生效

检查插件是否已安装、已启用、权限已授权，且输入方案开关已开启。高风险或未签名插件安装后默认停用，需要用户再次启用。更新包即使版本相同也不可替换不同内容，必须提升版本。

## 9. 文档导航

- 项目蓝图：`docs\owo-project-blueprint\`
- 架构导航：`docs\project-architecture.html`
- 输入方案：`docs\plugins\input-schemes.md`
- 插件开发：`docs\plugins\development.md`
- 词库管理：`docs\dictionaries.md`
- 在线词库：`docs\dictionary-discovery.md`
- 模型管理：`docs\model-manager.md`
- Agent 隐私：`docs\plugins\agent-ipc-privacy.md`
- CLI 用户手册：`docs\cli-user.md`
- CLI AI 手册：`docs\cli-ai.md`
- 性能诊断：`docs\candidate-performance-debugging.md`
- 安装向导：`docs\standalone-winui-setup.md`
- 发行说明：`docs\releases\`
