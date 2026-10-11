# 安装引导检查与页面切换

安装选项、安装进度页面各构建一次并缓存，返回页面时只切换根节点。控件不会被重复添加到不同父节点；安装失败后重试会清空上次进度与日志。

实际安装须同时满足：安装包结构完整、当前用户简体中文语言检查通过、安装路径有效、用户勾选可信确认。按钮下方显示阻塞原因。`--install-preview-stage` 专用于 UI 预览，始终禁止安装。

语言检查使用系统目录中的 Windows PowerShell，优先检查当前用户语言配置，必要时查询 `Get-WinUserLanguageList`。明确区分成功、未配置简体中文、检测异常；检测最长等待 20 秒，同一时间只执行一次。检测不更改系统语言配置，也不将机器显示语言视为输入支持。语言配置存在不代表所有可选语言组件已下载完毕，如 Windows 尚在下载基本输入组件，需等待完成。

脚本安装后端采用相同的当前用户语言配置回退规则。检测异常会显示诊断，不伪装成“缺少语言包”。

定向验证：

```powershell
dotnet build apps/settings_center/OwO.Settings.csproj -c Release -p:Platform=x64 --no-restore
dotnet run --project tests/settings_center/preferences_tests.csproj -c Release
```

手动验证：运行 `open-owo-installer.cmd`（不使用阶段预览参数），反复切换上一步/下一步，检查设置与可信确认是否保留；检查完成后，勾选可信应启用安装按钮。安装失败返回选项后可重试。阶段预览脚本只用于观察布局，不能用于验证实际安装按钮。
