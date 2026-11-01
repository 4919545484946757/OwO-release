# Windows application compatibility / TSF hardening

日期：2026-10-31。总体状态：BLOCKED，未完成最终验收，不能宣称全面兼容。计算机操作工具在 Edge 本地页检查时强制终止本轮自动化，原因是无法可靠确认浏览器 URL 以执行策略检查；没有绕过或重试该限制。

## 已实施的生产修改

- `include/owo/compatibility/input_context.h`：共享 ALLOWED / DENIED / UNKNOWN、拒绝原因和原生控件证据策略。验证进程、焦点线程、父子关系、密码、只读、禁用及禁用祖先；使用系统 RealGetWindowClass 识别标准 Edit 子类，不使用应用或进程名称白名单。
- `include/owo/tsf/input_method_context.h`、`src/tsf/input_method_context.cpp`：结构化 `resolve_input_context`，输出视图、焦点、进程/线程、范围存在性、InputScope HRESULT。标准 TSF 属性优先，支持可用的 ITfContextOwner 属性；S_OK/VT_EMPTY、E_FAIL/VT_EMPTY、E_NOTIMPL/VT_EMPTY 才可使用独立原生证据，S_FALSE 混合属性及无效显式对象仍拒绝。显式原生密码、只读、禁用优先于默认 Scope。
- `src/ipc/input_method_server.cpp`：复用共享原生证据；UIA 增加启用/只读验证、TextPattern 只读属性备选及最终焦点复查。UIA 仅在独立 Core MTA 线程执行，不在应用 UI 线程查询自己的 UI。不读取正文。增加分阶段失败 HRESULT。该 UIA 扩展曾造成 Codex 回归，最后的查询拆分尚未实测确认，不能标为 PASS。
- `include/owo/tsf/candidate_ui_element.h`、`src/tsf/text_service.h/.cpp`：标准 ITfCandidateListUIElementBehavior、有界当前页快照、Begin/Update/EndUIElement、宿主控制 Show、选择/确认/取消回调、过期请求拒绝、COM 生命周期与 DLL 服务计数。继续复用原候选窗口，未创建第二套绘制实现。
- `src/tsf/text_service.cpp`：原生提交的 SetText 成功不再被后续光标定位失败覆盖；键入及写入应用统一上下文检查；文档管理器销毁时全部方案清空组合和旧候选；处理 UIElement 回调重入后已取消的显示状态。
- `scripts/tsf-dev-image.ps1`、`register-dev.ps1`、`unregister-dev.ps1`、`deploy-tsf-dev.ps1`：PE 架构校验、x64/x86 正确 regsvr32、注册备份及 SHA-256 记录、固定项目内部署路径。保留旧 Notepad 私有注册迁移；同路径部署后不需要为每个应用逐一改私有注册。映射文件不能更新时停止，不强杀应用或覆盖用户数据。正式发布仍需签名/正式部署方案，本轮未制作安装器。
- `tools/tsf_profile_check.cpp`：焦点诊断补充系统原生类身份。
- `tools/compatibility/native_host.cpp`、`managed_host.cs`、`scripts/show-compatibility-host.ps1`、`CMakeLists.txt`：可重复的真实 Win32 Edit/RichEdit、WPF、WinForms 测试工具，包含密码/只读/禁用控件；仅使用现有 C++/.NET Framework 工具，无下载或安装。

调用链保持：应用 → TSF TextService → 上下文/组合/候选 UI → 原生拼音或 Core IPC → InputMethodRuntime → PluginHost → 独立 Wubi86。没有修改拼音编码、词典、候选算法或学习算法；五笔正式数据未重新生成。

## 根因、回归与修复证据

1. 标准 Edit/RichEdit 返回 S_OK + VT_EMPTY InputScope，旧回退只识别 E_FAIL。新增明确缺省属性策略及独立原生证明，真实 x64 Edit/RichEdit 候选及上屏 PASS。
2. WinForms 的注册类是 WindowsForms10.EDIT…，但 RealGetWindowClass 返回标准 Edit。真实超类控件测试先 FAIL 后 PASS；WinForms 修复后真实 wqiy → 你 PASS。
3. 本轮引入的原生焦点共用校验错误地要求顶层视图与焦点线程一致，导致 Notepad 回归。日志显示 TSF RESOLVE_ALLOWED、Core known_edit=1 却拒绝。新增同进程跨线程真实窗口测试先 FAIL 后 PASS，改为分别验证视图进程与焦点线程；更新 Core 后 Notepad 已重新实际显示候选并 Space 上屏。
4. Codex 本轮出现新增回归，用户确认“输入框不行但是 Notepad 可以”。Core 日志为 UIA focused_element HRESULT 0x80131505，发生于插件调用前。最后修改将基本焦点缓存与启用/只读属性查询拆分，保留100ms UIA及1000ms请求限制，并复查焦点。最新 Core PID18004 已部署，但真实恢复尚未验证：BLOCKED，不能归为历史失败。

## 测试证据

- 新增 `tests/tsf/compatibility_policy_tests.cpp`：未知元数据、密码、只读、禁用祖先、错误 PID/线程/视图、真实 Edit 超类、同进程跨线程视图。最后跨线程修复后 Release PASS。
- 新增 `tests/tsf/candidate_ui_element_tests.cpp`：COM 接口、边界、BSTR、选择、脱离后拒绝、真实 Windows ITfUIElementMgr 与宿主 UILess sink 的 Begin/Update/End。PASS。
- 扩展 `input_method_context_tests.cpp`：缺省 InputScope 与混合/无效对象区分。
- 扩展 `test_text_store.h`、`scheme_commit_session_tests.cpp`：真实 Windows TSF 上下文与测试 TextStore 的 composition owner 通知、宿主拒绝时不写入、不确认。已有 SetText 路径经 Windows 触发组合通知；未添加第二套 composition 引擎。
- 扩展 `wubi_commit_service_tests.cpp`：原生提交在光标定位失败后仍正确报告 SetText 成功，先 FAIL 后 PASS。
- 扩展 `expanded_candidate_service_tests.cpp`：文档销毁必须清理原生组合，先 FAIL 后 PASS。

| 构建/测试 | 状态 | 实际记录 |
|---|---|---|
| x64 Debug/Release TSF及相关目标 | PASS | compatibility-final-debug-build.log、compatibility-final-tsf-build.log |
| x86 Release/Debug TSF | PASS | compatibility-final-x86-build.log、compatibility-x86-debug-build.log |
| x64 Debug 专项35项 | PASS | compatibility-verified-debug-tests.log，35/35，43.75s |
| x64 Release 专项35项 | PASS | compatibility-verified-release-tests.log，35/35，9.15s |
| x86 Release 上下文/UIElement/提交4项 | PASS | compatibility-x86-tests.log，4/4 |
| Engine/Plugin/IPC/TSF/Wubi 合并61项 Debug | FAIL | compatibility-debug-final-tests.log，58/61；candidates、core_service、core_config |
| 同组 Release | FAIL | compatibility-release-final-tests.log，59/61；candidates、core_service |
| MSVC /analyze | PASS | compatibility-final-analyze.log，构建完成；保留已有 C6262/C6031，非零警告 |
| git diff --check | PASS | 本轮执行退出0；仅有行尾规范化提示 |
| 最后跨线程/UIA拆分之后全部重建回归 | NOT TESTED | 最后已重建 Core 和 Release policy；上述35/61项不能冒充这之后的最终全量结果 |

日志位于仓库 `build/`。历史 candidates / core_service / Debug core_config 的原始基线证据见 `build/wubi86/phase2-baseline-ipc.xml`、`phase2-baseline-release.xml`。新增 Codex 实测回归单独列出，不用历史失败掩盖。

## 真实应用矩阵

PASS 只表示下列已执行操作，不表示所有列出的边界功能、全部版本均兼容。

| 应用 | 状态 | 实测范围 |
|---|---|---|
| x64 原生 Edit/RichEdit | PASS | 正式五笔候选、数字1/Space及实际正文“你”；密码/只读无候选，学习仍为13 |
| x86 原生 Edit | PASS | 实际加载x86 DLL，经共享64位Core IPC查询并Space上屏“你” |
| WPF TextBox | PASS | 标准TSF Scope，正式五笔候选及Space上屏；早期一次Core拒绝后重试通过，保留该观察，不宣称冷启动全覆盖 |
| WinForms TextBox | PASS | RealGetWindowClass 修复后候选及Space上屏 |
| 新版 Notepad MSIX x64 | PASS | 本轮新增跨线程Core回归修复后候选和Space上屏；最后完整源代码DLL尚需再次构建部署复验 |
| Codex | FAIL | 用户本轮确认回归；最后UIA查询拆分尚待复测 |
| Edge/Chrome | BLOCKED | 本轮浏览器工具URL识别策略终止；未完成实际输入，不以之前结果代替 |
| VS Code | NOT TESTED | 启动至发行说明，尚未验证编辑器输入 |
| Word | NOT TESTED | 已启动空白文档，尚未验证输入 |
| Excel | NOT TESTED | 检测到已安装，未启动/输入验证 |
| 其他 WinUI、全屏、多显示器、不同DPI | NOT TESTED | 不推断兼容 |

拼音算法及full_pinyin自动化回归保留通过记录；本轮最终部署后的真实拼音应用输入 NOT TESTED。当前 Core 仍使用原开发环境 lexicon_fixture.owolx，不宣称正式拼音词库覆盖。

## 部署与回滚

当前 HKCU Apartment 固定路径：

- x64 `D:\src\OwO-release\build\tsf-dev\x64\OwO.TSF.dll`，SHA-256 D9217B621AAB3009F6273886B41EA536F3A86A6F24DF1AE902DB59932A3BD34F。
- x86 `D:\src\OwO-release\build\tsf-dev\x86\OwO.TSF.dll`，SHA-256 48F7EE67A7A61397ADF6E522F6052A7EC41C8168170E74FD8D041F79E6BE9625。

这两个DLL早于最后共用跨线程策略修改；不能宣称注册DLL包含最后所有源码。最新 Core 已包含 UIA 拆分。未自动卸载、未改其他输入法、未下载软件、未使用子智能体、未执行Git破坏性操作。

本轮首次稳定路径部署前备份：x64 `build/tsf-registration/d1868eb9394f46a1a743157d7ff138c2`，x86 `422f585e6e3549fca7627a75124b0354`；最后部署备份：x64 `8d15f165a1624445b6e5b238503ee7e4`，x86 `ae237b6a496f45d5bbe1ad62a3c3085a`。包含before-com.reg；最后备份额外包含可用的Own TIP注册导出。Notepad迁移备份在对应notepad-legacy-view/before.json。

保存并退出测试应用后，从独立x64 PowerShell手动注销共享OwO TIP（会影响拼音与五笔）：

```powershell
Set-Location D:\src\OwO-release
& .\build\windows-release\Release\owo_tsf_profile_check.exe --disable
& .\scripts\unregister-dev.ps1 -Configuration Release -Architecture x86 -DllPath .\build\tsf-dev\x86\OwO.TSF.dll
& .\scripts\unregister-dev.ps1 -Configuration Release -Architecture x64 -DllPath .\build\tsf-dev\x64\OwO.TSF.dll
# 不强杀应用；仍映射时保留文件，直到相关应用退出。
$paths=@((Join-Path $PWD 'build\tsf-dev\x64\OwO.TSF.dll'),(Join-Path $PWD 'build\tsf-dev\x86\OwO.TSF.dll'))
$loaded=@(Get-Process | ForEach-Object {try{$_.Modules | Where-Object FileName -in $paths}catch{}})
if($loaded.Count){throw 'DLL仍加载，先保存并关闭相关测试应用'}
$paths | ForEach-Object {Remove-Item -LiteralPath $_}
reg.exe query 'HKCU\Software\Classes\CLSID\{6D31C9B1-8978-4F49-89B4-66EB1E741591}' /reg:64
reg.exe query 'HKCU\Software\Classes\CLSID\{6D31C9B1-8978-4F49-89B4-66EB1E741591}' /reg:32
```

需要恢复本轮之前的有效路径时，确认备份中DLL仍存在、没有其他版本接管后：

```powershell
Set-Location D:\src\OwO-release
reg.exe import .\build\tsf-registration\d1868eb9394f46a1a743157d7ff138c2\before-com.reg /reg:64
reg.exe import .\build\tsf-registration\422f585e6e3549fca7627a75124b0354\before-com.reg /reg:32
& .\build\windows-release\Release\owo_tsf_profile_check.exe --refresh-cache
```

注意：Notepad私有覆盖必须在同一包上下文单独恢复/删除Own CLSID；旧 `docs/wubi86/NOTEPAD_DEV_CLEANUP.md` 是旧部署路径专用命令，不能直接用于此次stable路径。此部分稳定部署通用清理脚本尚未闭环，状态 NOT TESTED，不能声称已完整注销验证。禁止删除整个包注册Hive、用户学习目录、词典或其他版本DLL。

## 尚需继续

首先复测并修复 Codex 最后 UIA 路径；随后将最后共用策略重新构建部署到全部架构，补齐最终回归、标准持续preedit生命周期/完整UILess分页能力与应用矩阵操作覆盖，以及stable路径私有注册清理/回滚的运行验证。当前只具备受控提交事务及候选UIElement，不能宣称完整持续文档内preedit已实现。总体 BLOCKED，未宣布任务完成。
