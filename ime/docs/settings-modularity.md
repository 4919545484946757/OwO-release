# 设置中心：模块与输入方案能力边界

日期：2026-10-26。本文说明设置中心的内部模块边界；模块化本身不改变插件权限。后续展开页快捷键涉及的跨层行为和版本迁移见[展开页选择](expanded-candidate-pages.md)。

## 当前结构

`MainPage` 负责页面外壳、导航、搜索和状态提示；不再逐项拼装完整配置，也不在功能页散布 `full_pinyin` / `plugin:` 判断。

- `SettingsModuleDefinitions.cs`：11 个配置模块及 52 个字段的唯一所有权、读写适配和能力需求。
- `SettingsModules.cs`：不依赖 WinUI 的加载、采集、按页重置和完整性检查；`ISettingsEditors` 是视图边界。
- `SettingsUiEditors.cs` / `SettingsPageBindings.cs`：字段与 WinUI 控件绑定、卡片描述、搜索入口和自动保存观察源。新增控件不再额外修改自动保存的总控件列表。
- `InputSchemeSettingsPolicy.cs`：宿主已接线的设置能力。`InputSchemeSettingsPage.cs` 处理方案选择与发现。
- `ShortcutSettingsPage.cs` / `ShortcutSettingsValidation.cs`：按键采集与独立配置校验。
- `PluginSettingsPage.cs` / `PluginSettingsSchema.cs`：插件管理视图和插件自己的声明式配置，不并入宿主设置快照。
- `ChinesePunctuationPage.cs`、`UserLexiconEditor.cs`、`SettingsAutoSave.cs`、`SettingsTransfer*.cs` 分别保留标点、词库、保存调度和迁移职责。

模块为输入方案选择、候选显示、候选策略、标点、Alt 符号预览、学习、补全、排序、语料与语气、风格、快捷键。宿主设置快照完整覆盖检查在启动时执行；重复字段、漏绑定和未知字段不能静默覆盖配置。

视图布局仍由现有 `MainPage.xaml` 承载，尚未全部转换为独立 `Page`/`UserControl`。这一轮先解耦数据流和能力策略，后续移动布局不必再次修改配置格式、导入导出或保存调度。

## 方案能力

| 功能 | 全拼 | 精确编码 | 当前插件方案 |
| --- | --- | --- | --- |
| 候选显示、编码光标、分页、原样输出、中英文切换 | 支持 | 支持 | 支持宿主已接线部分 |
| Alt 数字符号预览 | 支持 | 支持 | 宿主独立功能 |
| 候选分区、暂存上屏、失焦保护、宿主中文标点、应用文本光标移动 | 支持 | 支持 | 不接入，禁用 |
| 宿主用户学习与模型排序 | 支持 | 支持 | 不接入，禁用 |
| 拼音纠错、用户词拼音补全、完整单字拼音时单字区优先 | 支持 | 禁用 | 禁用 |

能力表示“宿主设置能否影响当前链路”，不是插件权限。插件即使属于音码类型，也不会自动得到宿主拼音纠错或学习能力；只有实际接好运行路径、完成安全和功能验证后，宿主策略才能增加能力。未知方案默认不启用未确认的功能，不悄悄退回全拼。

设置卡片依据字段能力和局部依赖更新：例如模型预算依赖智能排序、补全范围依赖补全总开关。禁用直接作用于内部 `Control.IsEnabled`，不再只是屏蔽鼠标；动态重建的快捷键按钮也重新应用能力约束。切换方案取消尚未完成的改键采集，避免已禁用项继续捕获按键。

## 值保留、重置和兼容

- 切换方案只改变功能可用性，不改写停用功能的存储值。切回受支持方案后恢复原偏好。
- 自动保存首次使用默认开启；缺少偏好字段时也使用开启默认值。启动不会写回默认值，用户已明确关闭时保持关闭；偏好读取失败则暂停自动保存，避免覆盖已有配置。
- 自动保存、手动保存、重新加载和导入共享同一字段适配管线；批量载入/重置期间抑制自动保存，避免保存中间状态。
- 页面重置仅处理归属该页、且当前方案支持的字段；不重置其他模块或不支持的偏好。例如候选页重置不会重置中文标点掩码、学习灵敏度或方案选择；中文标点页独立重置标点总开关和逐项掩码。
- 搜索保留未支持功能的入口并标注原因，不将其伪装为已经可用的功能。用户词库仍管理宿主内置词库，而不是插件私有词库。
- 界面不再常驻显示方案范围、全拼能力、词库加载成功和自动保存合并说明。错误、明确操作结果、导出数据范围说明与高风险操作确认仍保留；隐藏说明不改变配置能力和安全门禁。
- 通过 `SettingsSnapshot` 适配原生配置；当前 Schema 17 / JSON v2 为新增展开页快捷键提供显式迁移，兼容读取旧配置。数值范围与全局快捷键约束仍由原生后端最终校验，不放宽、不按方案隐式重写绑定。

当前仍是兼容的全局设置存储，不声称已实现“每个输入方案独立保存一套值”。插件自有设置继续通过插件 schema 管理。方案独立词库/学习命名空间、每方案快捷键与配置覆盖需要后续显式迁移和 Core/TSF 配套支持，不能仅增加 UI 开关。

## 扩展步骤

1. 新增宿主配置字段时先在原生配置层定义默认值、范围与迁移；在 `SettingsModuleDefinitions` 注册唯一所有权和能力需求。
2. 在 `SettingsPageBindings` 绑定类型明确的编辑器与卡片。卡片的搜索能力需求和自动保存监听从绑定取得，不复制额外判断。
3. 只有确认实际输入链路支持后，才在宿主方案能力描述中启用该功能；不要让插件配置或自然语言直接控制能力位。
4. 为新字段补充模块/方案矩阵测试、导入导出往返，以及真实控件禁用/切换测试。

这是内部适配接口，不是新的插件 SDK、权限清单或跨进程能力协商协议；对外方案协议仍见[插件输入方案](plugins/input-schemes.md)。

## 定向验证

```powershell
dotnet run --project tests/settings_center/settings_modules_tests.csproj -c Release -- build/windows-release/Release/owo_config_shell.exe
dotnet run --project tests/settings_center/configuration_transfer_tests.csproj -c Release -- build/windows-release/Release/owo_config_shell.exe
dotnet build apps/settings_center/OwO.Settings.csproj -c Release -r win-x64 --no-restore -p:Platform=x64 -p:NuGetAudit=false
.\scripts\test_settings_modules_ui.ps1 -PublishDirectory .\build\settings-publish\win-x64
.\scripts\test_settings_modules_ui.ps1 -PublishDirectory .\build\settings-publish\win-x64 -FreshAutoSaveDefaults
```

UI 检查只启动自己创建的测试进程，用随机 `build/settings-modules-ui-*` 配置及空插件目录；测试全拼、精确编码、未安装的插件方案引用（保守能力，不安装或授权插件）、同窗口方案切换、实际控件禁用、按页重置和自动保存。关闭/超时只结束自己启动的窗口，保留夹具供检查；不重启 Core、不操作日常词库或用户打开的设置窗口。

`tests/settings_center/Directory.Build.props` 为同目录的不同 C# 测试工程隔离 `obj/<工程名>` 与 `bin/<工程名>`。不能恢复共用的中间目录：SDK 的共享 apphost 缓存可能造成交替执行两个项目时误启动另一个项目的入口，产生不可信的“通过”结果。

回滚可撤销本次设置中心提交；没有用户数据迁移或安装包升级步骤。
