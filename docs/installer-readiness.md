# 安装引导检查与页面切换

安装选项、安装进度页面各构建一次并缓存，返回页面时只切换根节点。控件不会被重复添加到不同父节点；安装失败后重试会清空上次进度与日志。

实际安装须同时满足：安装包结构完整、当前用户简体中文语言检查通过、安装路径有效、用户勾选可信确认。按钮下方显示阻塞原因。`--install-preview-stage` 专用于 UI 预览，始终禁止安装。

语言检查使用系统目录中的 Windows PowerShell，优先检查当前用户语言配置，必要时查询 `Get-WinUserLanguageList`。明确区分成功、未配置简体中文、检测异常；检测最长等待 20 秒，同一时间只执行一次。检测不更改系统语言配置，也不将机器显示语言视为输入支持。语言配置存在不代表所有可选语言组件已下载完毕，如 Windows 尚在下载基本输入组件，需等待完成。

脚本安装后端采用相同的当前用户语言配置回退规则。检测异常会显示诊断，不伪装成“缺少语言包”。

当前发行包仅支持原生 x64 Windows。Inno 安装入口使用 `x64os`，WinUI 使用 `RuntimeInformation.OSArchitecture` 检查操作系统架构，ZIP 安装脚本也在提权或复制文件前拒绝 ARM64、32 位 Windows。x64 Windows 下的 32 位应用仍使用包内 x86 TSF，并非取消这部分支持。

脚本始终尝试读取可修改的用户语言列表。只有取得中文语言对象才修改 `InputMethodTips`；注册表回退仅能证明配置存在，不能用于生成可修改对象。回退时保留语言配置并提示用户，不对空对象调用 `Add()`。

从 32 位进程启动时，提权使用 `Sysnative` 下的原生 Windows PowerShell，x64 TSF 注册/注销使用原生 `regsvr32`；x86 TSF 继续使用 `SysWOW64`。

安装复制前写入 `.owo-install-state.json`，成功后标记完成。重试遇到完整 OwO 安装、有效状态标记或与当前包 TSF/Core 二进制哈希相符的旧版失败残留时，使用全新的同级目录，不覆盖或删除残留目录。缺少卸载脚本的残留不会作为旧安装清理目标；无关的非空目录、无法核验的残留、文件路径或链接目录仍拒绝安装。日志记录实际安装位置。

安装助手以 `-ReportProgress` 启动后端，按复制组件/词典/模型/设置中心、校验、注册、配置、启动等真实阶段输出结构化进度；这些百分比表示阶段完成度，不是逐字节进度或剩余时间预测。仅解析 stdout 中合法的阶段标记，stderr、未知标记和倒退进度不改变显示。100% 只在后端退出码为 0 后显示。详细信息实时更新，阶段及输出记录在 `setup-startup.log`，失败保留最后已知进度而不是重置为 0%。

启动器使用 `PROC_THREAD_ATTRIBUTE_HANDLE_LIST`，仅将日志句柄传给 Core / ModelHost，避免继承安装器的 stdout/stderr 管道。助手等待后端进程结束后最多再收尾读取 2 秒，之后关闭自己的读取端并保留退出码及诊断，不等待或终止持续运行的后台服务。因此不会因安装已经结束、子进程仍持有管道而卡在进度页。

定向验证：

```powershell
dotnet build apps/settings_center/OwO.Settings.csproj -c Release -p:Platform=x64 --no-restore
dotnet run --project tests/settings_center/preferences_tests.csproj -c Release
powershell.exe -NoProfile -ExecutionPolicy Bypass -File tests/settings_center/InstallerPrerequisitesTests.ps1
powershell.exe -NoProfile -ExecutionPolicy Bypass -File tests/settings_center/InstallerDirectoryTests.ps1
cmake --build --preset windows-release --target owo_runtime_launcher owo_runtime_launcher_handles_tests
ctest --test-dir build/windows-release -C Release -R '^owo.runtime.launcher_handles$' --output-on-failure
dotnet run --project tests/settings_center/preferences_tests.csproj -c Release -- <发行ZIP> --process-fixture <owo_runtime_launcher_handles_tests.exe>
```

手动验证：运行 `open-owo-installer.cmd`（不使用阶段预览参数），反复切换上一步/下一步，检查设置与可信确认是否保留；检查完成后，勾选可信应启用安装按钮。安装失败返回选项后可重试。阶段预览脚本只用于观察布局，不能用于验证实际安装按钮。
