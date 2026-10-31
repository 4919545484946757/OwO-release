# Wubi86 Phase 2 final verification — 2026-10-31

最新结论：Notepad 兼容性修复及真实五笔上屏 PASS（见文末“Notepad 最终兼容性修复”）。Phase 2 全部历史验收项不因此自动变为 PASS；本轮没有重新执行全仓完整验收。下文早期 Notepad FAIL/BLOCKED 为排查历史，已由文末新证据更新。

## 实际生产实现与文件

- `plugins/wubi86/manifest.json`、`src/main.cpp`、`provider.h/.cpp`：独立 table 进程、正式生产词典、精确/前缀候选、分页、稳定 ID、选择校验和随机提交事务。
- `plugins/wubi86/src/learning.h/.cpp`：独立版本化学习快照、跨进程写锁、原子替换、损坏隔离、重启恢复、提交去重。新增写入端 2 MiB 上限，避免生成读取端不能接受的快照；失败回滚内存。
- `include/owo/engine/input_method_provider.h`、`src/engine/input_method_provider.cpp`：可选成功提交确认能力和严格请求/响应校验。
- `src/plugin/input_method_protocol.cpp`：可选 OWCC 描述能力和 OWIC 提交标识尾部，旧插件默认编码保持不变。
- `src/core/input_method_runtime.cpp`：进程代理保存已选择事务，绑定 Session、输入和页码，拒绝过期/伪造确认，重复确认幂等。
- `include/owo/tsf/scheme_commit.h`、`src/tsf/scheme_commit.cpp`：实际 ITfRange::SetText 公共提交路径。
- `src/tsf/text_service.h/.cpp`：SchemeEditSession 成功写入后才发送 committed；失败保留输入、不学习；确认期间串行化后续输入；退出上下文后后台排空已提交确认。线程持有真正的 DLL 模块引用，通过 FreeLibraryAndExitThread 结束，避免卸载竞态。
- `apps/settings_center/InputSchemeSettingsPage.cs`、`InputSchemeSettingsPolicy.cs`、`PluginSettingsPage.cs`：发现、选择、自动保存和禁用/撤权回退，刷新列表时抑制临时选择保存。
- `CMakeLists.txt`：插件、学习和真实 TSF 编辑会话测试注册，按 Debug/Release 隔离插件 staging。
- `tests/wubi86/`、`tests/tsf/scheme_commit_session_tests.cpp`、`wubi_commit_service_tests.cpp`、`test_text_store.h`、`tests/settings_center/SettingsModulesTests.cs`：对应验证。

保留 Phase 1 数据；没有重新生成生产词典，没有修改 FullPinyinSchema、CandidateGenerator 或拼音学习算法。工作区 `.gitignore` 等原有修改保留。

## 完整调用链

TextService 键盘/候选窗口 → InputMethodContext → InputMethodClient/Core named pipe → InputMethodRuntime ProcessProvider → 原 PluginHost 权限验证/AppContainer → Wubi Provider → Phase 1 Dictionary/Trie → 原候选窗口。选择先校验已发候选，得到随机 32 位十六进制 commit ID；实际 SchemeEditSession SetText 成功后经同一 IPC 发送 committed；插件核实 Session/输入/页码/token 后持久化学习。SetText 失败、取消或上下文失效不发成功确认。重试仅重发确认，不重复插入文字。

学习文件 `wubi86-learning.v1.tsv` 位于宿主提供的独立插件数据目录；生产 `assets/main.tsv` 不写入。格式头 `OWO-WUBI-LEARN\t1`，entry 和 commit 行；最多 4096 个学习条目和最近 4096 个提交 token，频率 1..1000000，快照最多 2 MiB。独占 sidecar 锁有界等待 200ms；临时文件 FlushFileBuffers 后原子替换。损坏隔离失败或无法读取时拒绝写入，保留原始文件。精确候选优先，再结合学习频率和原始稳定排序。

## Settings Center

PASS：现有 SDK 的 net10 x64/x86 构建完成；2946 项真实配置后端/策略检查通过。没有更改项目 SDK 固定版本或目标框架。

PASS：实际 x86 WinUI 设置窗口运行，使用 build/windows-debug/wubi86-settings-ui 下隔离配置和插件 store，连接真实 config/plugin shell；显示一个插件版本，下拉含“五笔86（插件）”，选择后显示 owo.plugin.wubi86 · zh-Hans，实际配置写入 `input_scheme=plugin:owo.plugin.wubi86/wubi86`，重新加载仍保持选择。

NOT TESTED：本次 UI 重启恢复、UI 禁用/启用全部交互以及实际前台输入切换未全部完成；后端持久化/回退测试不能替代这些 UI 验收。

## 真实 Windows 应用验证

BLOCKED：只读检查现有 HKCU Software/Classes/CLSID/{6D31C9B1-8978-4F49-89B4-66EB1E741591}/InprocServer32，其默认值为 `D:\src\OwO-release\build\manual-deploy\OwO.TSF.e2755b44a07d4a9c.dll`，Test-Path 为 False。现有开发注册引用不存在的 DLL，无法加载当前生产 TSF。

没有注册服务或更改该注册项。解除条件：由有权限的开发环境维护者恢复可用且指向当前构建的 TSF 开发部署，再执行 Notepad 和交叉应用完整按键、候选、焦点、上屏和重启学习验证。所有真实应用输入项目均 NOT TESTED。

PASS：真实 Windows TSF 管理器、编辑锁、ITfContext/ITfRange 和生产 SchemeEditSession 路径测试，连接真实 Core IPC 与隔离插件并检查学习文件。应用 TextStore 为测试实现，明确不是 Notepad/Word 实测。覆盖失败 SetText、不重复写入、成功确认、失效上下文、写入后光标失败、上下文清理后排空确认。

## 构建、测试及静态分析

最新完整构建记录：build/wubi86/final-completion-debug-build.log、final-completion-release-build.log。最终修复后 Debug 重建记录 final-postfix-debug-build.log。原 1000ms 请求限制未提高，正式生产数据首次单键冷查询纳入真实进程测试。

完整 CTest 每配置 88 项，逐项状态见 [PHASE2_TEST_RESULTS.md](PHASE2_TEST_RESULTS.md)，原始 XML/日志：build/wubi86/final-completion-{debug,release}.xml/.log。未运行的依赖测试明确 NOT TESTED。

专项最后重跑记录：build/wubi86/final-postfix-{debug,release}.xml/.log。8 项为 wubi86 的基础测试、coverage、data_integrity、provider、process、learning、tsf.scheme_commit_session、tsf.wubi_commit_service（准确名称见 XML）。另单独 final-learning-debug.log 验证快照超限拒绝和回滚。

Settings：2946 checks PASS。MSVC /analyze 构建 PASS，记录 final-completion-analyze.log、final-learning-analyze.log；不是零警告。C6262（66360 字节栈）、两处 C6031 GetKeyboardState、Dll 导出 C28251 已在原始源码静态分析复现，证据 phase2-baseline-static.log。git diff --check PASS。

## 历史失败与新增失败

原始 HEAD 0c30b077b7fe5d883ac109e6157690ebf61d9f0c 的隔离源码/构建保留于 build/wubi86-baseline 和 baseline-build，未 reset/checkout 当前工作区。

- FAIL：owo.engine.candidates，原有 shorter reading 断言，原始基线已复现。
- FAIL：Debug core_service_contract、core_config_contract 超时，基线 phase2-baseline-ipc.xml 复现。
- FAIL：Release core_service_contract 的 long suffix/local bridge，phase2-baseline-release.xml 复现。
- FAIL：model.dataset_schema、ranker_evaluation、ranker_evaluation_failures，数据 SHA-256 不匹配；原始基线 final-baseline-extra.log 复现相同 digest。
- FAIL：lexicon.compiler_rime_yaml、compiler_rime_auto，source SHA-256 does not match manifest；基线编译真实 compiler 后 final-baseline-lexicon.log 同样失败。依赖 compiler_rime_auto_result NOT TESTED。

不修改无关模型/Rime 数据和拼音算法来掩盖这些失败。曾在并行构建时出现 core.input_method_process 打开 pipe 失败，随后空闲完整回归通过；没有将该瞬态擅自定为历史问题。没有已确认本阶段新增回归；全量仍有上述 FAIL，不能描述为全绿。

## 下载和环境

Phase 1 原始数据复用，未重新下载词库。Settings 构建所需 10 个 NuGet 包均保存在 env/wubi86/nuget；每个 URL、版本、文件大小、SHA-256、许可证和用途见 env/wubi86/nuget-downloads.json。未下载/安装 SDK 或运行安装器；缓存与 CLI_HOME 均定位 env/wubi86。

环境异常如实记录：此前 SDK 首次运行自动创建过 ASP.NET 开发 HTTPS 证书，后来显式禁止该 first-run 行为；没有删除用户证书。没有注册 TSF、修改系统环境变量或生成安装包。

运行时代码使用正式 data/ 和插件 assets/main.tsv，不依赖 env/ 原始下载；Phase 1 data_integrity 测试仍有原始数据归档校验依赖，未删除或弱化。

## 验收

独立插件、协议校验、生产查询、提交成功确认、独立持久化学习和生产编辑会话测试已落实。真实应用输入及完整 UI 验收未闭环，Phase 2 最终 BLOCKED，不进入 Phase 3。


## 2026-10-31 开发部署授权后实测补充

PASS：已备份 HKCU CLSID 与 TIP 到 build/wubi86-dev-final/before-clsid.reg、before-tip.reg。使用原 scripts/register-dev.ps1 x64 用户级注册，未修改 HKLM，也未注册 x86。

PASS：Release 实际部署 DLL 为 D:\src\OwO-release\build\wubi86-dev-final\OwO.TSF.1bd7bb182f08e538.dll；SHA-256 为 1BD7BB182F08E538FE1068346F1802753AABE293A5899D4A542C89CB66822870，与 build/windows-debug/Release/OwO.TSF.dll 一致。未额外复制依赖或生成安装器。当前注册已指向该实际存在路径，旧缺失路径阻塞已解除。

PASS：profile_check --activate-session 返回 active=true，ctfmon 正常运行。启动本次 Release Core 进程，PID 记录于 core.pid，使用隔离的 settings-ui 配置/store，不改用户学习数据。

FAIL：真实 Notepad 独立无标题测试页按键 wqiy 直接显示英文；没有观察到 OwO DLL 加载。已尝试 profile 激活、启动 ctfmon 和 Ctrl+Shift/Alt+Shift 输入语言切换，仍未获得候选或汉字提交。没有用 TextStore 测试结果替代真实应用成功。真实应用全部候选/上屏/学习验收尚未通过；交叉应用 NOT TESTED。当前问题是前台 TSF 加载/激活未闭环，不能继续沿用“DLL 不存在”的旧原因。

最新重新完整构建和全量回归记录位于 build/wubi86-dev-final/debug-build.log、release-build.log、debug-tests.log、release-tests.log；/analyze 为 analyze.log。最终以这些日志为准，历史哈希/Engine/IPC 失败仍保留。

### 可复制注销与回滚

当前没有执行卸载，部署保留。正常清理运行：

```powershell
Set-Location D:\src\OwO-release
& .\docs\wubi86\UNREGISTER_DEV.ps1
```

脚本先核实当前注册仍属于本次部署，再调用项目原注销脚本，停止 PID 文件绑定且映像路径匹配的测试 Core，检查 DLL 不再被任何进程加载后仅删除本次 DLL。不会删除学习数据、生产词典或源码，保留日志和备份。

注册失败需要恢复原项时，先确认未被其他版本接管，再运行：

```powershell
reg.exe import D:\src\OwO-release\build\wubi86-dev-final\before-clsid.reg
reg.exe import D:\src\OwO-release\build\wubi86-dev-final\before-tip.reg
```

注意备份原先就是指向不存在 DLL 的失效注册；这只能还原修改前状态，不代表可用输入环境。未发现修改前其他有效 x64 注册，未覆盖或改动 x86 注册。不要在本次部署仍被加载时删除 DLL。

检查注销结果：

```powershell
& D:\src\OwO-release\build\windows-debug\Release\owo_tsf_profile_check.exe
reg.exe query 'HKCU\Software\Classes\CLSID\{6D31C9B1-8978-4F49-89B4-66EB1E741591}' /s
reg.exe query 'HKCU\Software\Microsoft\CTF\TIP\{6D31C9B1-8978-4F49-89B4-66EB1E741591}' /s
Get-Process | ForEach-Object { try { $_.Modules | Where-Object FileName -eq 'D:\src\OwO-release\build\wubi86-dev-final\OwO.TSF.1bd7bb182f08e538.dll' | Select-Object FileName } catch {} }
```

注册注销时 profile_check 应找不到该 profile；registry query 应找不到相应本次项；模块列表应为空。受保护进程不能枚举时不能仅凭空输出证明完全释放，必要时退出测试应用或注销 Windows 会话后再检查。没有自动结束用户 Notepad 或删除其中内容。

Phase 2 总体仍 BLOCKED，不能宣布 PASS。

### Activation diagnostics investigation

COM probe PASS: user-level CLSID successfully loads the deployed DLL; DllGetClassObject and ClassFactory CreateInstance return S_OK. This is an external COM probe, not Notepad activation. Added bounded (4 MiB) LOCALAPPDATA/OwO-tsf-activation.log records outside DllMain: module path, PID/TID, HRESULT, construction/destruction, ActivateEx/Deactivate and key sink/key callbacks. No text contents logged. Changes: src/tsf/activation_trace.h, dll_main.cpp, text_service.cpp.

Release regression dll_smoke/scheme_commit_session/wubi_commit_service 3/3 PASS (activation-tests.log). No root-cause fix is yet proven. Notepad loading and actual input remain FAIL/NOT TESTED; test-process Activate logs must not be counted as Notepad evidence. Assembly cache refresh and session activation both return success. Existing HKLM profile/category registration also exists, inspected but not removed. Process Monitor trace NOT TESTED; no available installation was found in inspected locations. Current deployed path is recorded in build/wubi86-dev-final/deployed-path.txt, superseding the older fixed hash above; cleanup script reads this path dynamically.

## 当前实际 Release Store 部署修复

根本原因：实际运行 Core 为 build/windows-release/Release/owo_core_service.exe，没有 --plugin-store 参数，按源码使用 C:\Users\12282\AppData\Local\OwO\InputMethod\plugins。该 Store 原先 list.plugins=[]，input-schemes.schemes=[]。此前隔离 Store 不会被当前 Settings/Core 使用。并非缺少硬编码五笔 UI 选项。

PASS：构建 Release owo_wubi86_plugin/owo_plugin_shell（discovery-build.log）。正式 staging 为 build/windows-release/wubi86-plugin/Release，包含 manifest、schema、bin/owo_wubi86_plugin.exe、assets/main.tsv 和 LICENSE.pywubi/LICENSE.upstream。词典 1868289 bytes，部署后 SHA256 A7EBE25CE75257453C4864BC00C24E0E3D5F5B3706168152B8A005BAFFE82263。

PASS：通过真实 plugin_shell inspect-install/install-risk/activate 安装至实际 Store 的 versions/owo.plugin.wubi86/1.0.0。inventory SHA256 fae662f9802f267417040bcb7a86ad3da7806c72cf14dbcd0c9004571745e6c3。开发包无 signature.json，现有机制记录 unverified_package/critical；明确走开发风险确认，不伪造发布者签名，不改信任校验。permissions_authorized=true，仅 input.scheme，无 system.full_trust/network。active=true。发现调用实际启动隔离插件并取得 descriptor，不是静态增加 UI 候选。

PASS：实际 input-schemes 响应为 {"schemes":[{"id":"plugin:owo.plugin.wubi86/wubi86","name":"五笔86","plugin_id":"owo.plugin.wubi86","language":"zh-Hans"}]}。证据 discovery-schemes.json / discovery-after-restart.json。

PASS：实际 published x64 Settings 窗口下拉出现“五笔86（插件）”，通过界面 End/Enter 选择。真实 LocalAppData 配置保存 input_scheme=plugin:owo.plugin.wubi86/wubi86。随后新启动 Settings 窗口（handle 461510）显示 owo.plugin.wubi86 · zh-Hans，插件状态 1 个版本；不是隔离配置验证。Core 使用原命令参数正常 shutdown/restart 两次后仍发现五笔；没有修改 TSF 注册。

新增 scripts/deploy-wubi86-dev.ps1，提供可重复的构建、检查、安装与激活流程，未签名包必须显式传 -AcceptDevelopmentPackageRisk；不修改 PluginHost/IPC 权限逻辑。脚本语法解析 PASS。本次实际部署执行了脚本对应的原生 shell 命令，完整重跑脚本 NOT TESTED（避免覆盖已安装版本）。

PASS：Release 回归 5/5（engine.full_pinyin、engine.input_method_provider、wubi86.provider、process、learning），discovery-tests.log。专项进程测试为测试 Store；实际 Store 成功证据独立来自上述真实 shell/UI 操作，二者不混同。

FAIL：Notepad 本次实际按键未产生 OwO 五笔候选及汉字上屏。激活既有 profile 后确认 Notepad 加载 build/manual-deploy/OwO.TSF.0176b8f3a2e0e750.dll（并非本轮修改注册）。此前观察到的拼音候选属于其他输入法，不能计为 OwO 拼音 PASS。实际 OwO 拼音输入 NOT TESTED；当前 Core 命令使用 build/windows-debug/lexicon_fixture.owolx，不能宣称完整生产拼音词库验证成功。

新脚本 diff 检查 PASS；整个工作区 git diff --check 另报用户已有 global.json EOF 空行，未擅自修改该文件。无下载、无安装软件、无注销 TSF、无系统环境变量修改。用户数据保留。

本轮插件部署/发现/设置恢复 PASS，真实输入验收 FAIL，整体仍 BLOCKED，未报告任务完成。

## 首次发现与真实输入专项（2026-10-31，最新）

### 已确认并修复的启动问题

原 start-dev.ps1 在 Core/Settings 前未部署五笔；deploy-wubi86-dev.ps1 每次重复安装和激活。现启动脚本在 Core/Settings 前调用部署、校验 descriptor，Core listener Connect(1000) 成功后才打开 Settings。没有延长输入请求期限。部署脚本新增 StorePath/BuildDirectory，校验正式包 inventory；同版本同内容跳过安装，已授权且已激活跳过授权/激活；已撤权且无明确确认时拒绝；同版本内容不同拒绝覆盖并要求更新版本号。MainPage 的“重新加载”现在串行加载配置和插件列表，避免只刷新配置保留旧发现结果。

PASS：空 Store first-store-20261031200807 初次风险授权安装/激活/discovery、重复执行均通过，日志 first-store-deploy.log、first-store-repeat-final.log。未明确接受开发包风险时首次安装拒绝，first-store-denied.log。实际用户 Store 重复执行通过，idempotent-final.log。

PASS：编译的 x64 Release Settings 在该新 Store 的第一次窗口 handle 3083242 首次加载显示 1 个版本，下拉实际含“五笔86（插件）”；界面选择后 first-settings-config/owo.conf 实际保存插件 scheme。NOT TESTED：含独立 Core 启动和此窗口在内的整条 start-dev.ps1 空 Store 自动化验收尚未执行；不能把分项结果写成整条启动脚本全部 PASS。

PASS：.NET10.0.403 Settings build 零警告零错误（first-settings-build.log），2946 配置/策略检查 PASS（first-settings-tests.log）。用户 global.json 修改保持不动。

### 激活调查与当前第一个未证实节点

现 profile checker 在 --activate-session 中通过系统 input.dll 的 InstallLayoutOrTip 将 OwO 加入本用户启用列表，再 ChangeCurrentLanguage(0x0804)，最后用 FORSESSION 严格激活；去掉 DONTCARECURRENTINPUTLANGUAGE，防止延迟激活被误报为前台已激活。--disable 对应调用 InstallLayoutOrTip 的 uninstall flag，只影响此 OwO profile。微软资料：https://learn.microsoft.com/en-us/windows/win32/api/msctf/nf-msctf-itfinputprocessorprofilemgr-activateprofile ，https://learn.microsoft.com/en-us/windows/win32/tsf/installlayoutortip 。这些修正不是 Notepad 根因已解决的证明。

新增诊断阶段 AdviseThreadSinks.result、ThreadManagerSink.result、InitializeWindows.result、CONTEXT_NOT_CURRENT、CONTEXT_EDIT_REQUEST/RESULT、CONTEXT_ALLOWED/DENIED、ENGLISH_MODE。日志仍不在 DllMain 执行。不记录密码或提交 token。完整 correlation 跨 TSF/Core/Provider 追踪要求 NOT TESTED/未完成。

PASS：新 Settings 进程 PID32288 在实际 DLL 5f341266b1486a61 中 DllGetClassObject、CreateInstance、ActivateEx、AdviseKeyEventSink、ThreadManager sinks 和 InitializeWindows 均 S_OK。证据 activation-evidence.log。该证据不是 Notepad 证据。

FAIL：新 Notepad 输入仍显示 wqiy，没有五笔候选或实际汉字上屏。当前最前端缺失的是该 Notepad 的 KEY_TEST/KEY_DOWN 回调证据，尚不能归因于 Dictionary/Trie、IPC、CandidateWindow。不能因为无日志就断言函数一定未执行，尚需排除宿主加载/日志访问问题。原 PID8004 仍有旧 DLL；新 PID9708 尚未观察到 OwO 模块。已请用户手动在新窗口选择 OwO 并输入来区分前台 TIP 选择与自动化按键路径；不关闭未保存文档。

当前注册：HKCU x64 Apartment，build/wubi86-dev-final/OwO.TSF.aaeb2b31ce212a83.dll，未改 x86/HKLM。实际插件仍为 LocalAppData/OwO/InputMethod/plugins/versions/owo.plugin.wubi86/1.0.0，input.scheme 已授权。实际运行时 TSF scheme 未取得真实按键关联证据；持久配置为 plugin:owo.plugin.wubi86/wubi86。

### 本轮回归

Debug 59项：55 PASS / 4 FAIL（first-input-debug-tests.log）。其中 candidates、core_service、core_config 是已复现历史问题；wubi_commit_service 此次报“submitted real confirmation survives immediate UI reset and learns once”，单独重跑 PASS（first-input-debug-tsf-recheck.log），保留首次 FAIL，不能归为历史问题，稳定性原因尚未确定。

Release 59项：57 PASS / 2 FAIL，仍为历史 candidates/core_service（first-input-release-tests.log）。本轮 provider/process/learning 和 full_pinyin 均通过。不是全量仓库88项 CTest，不冒充完整全仓回归。

/analyze 编译完成（first-input-analyze.log），保留原有 C6262/C6031，未弱化检查。Debug/Release TSF和profile checker 构建完成。新增脚本解析和已修改源码 diff 检查通过；工作区 global.json 原有 EOF 空行保持不动。

### 手动清理（未执行）

OwO TSF 是拼音与五笔共享宿主，注销会同时影响两者，不存在仅“五笔 DLL 注册”。确认要退出开发部署后：

```powershell
Set-Location D:\src\OwO-release
# 退出本次用户启用列表，再按现有动态部署路径注销并检查模块释放
& .\build\windows-release\Release\owo_tsf_profile_check.exe --disable
& .\docs\wubi86\UNREGISTER_DEV.ps1
# 使用现有插件卸载接口，保留插件用户 data 目录；不要手动递归删 Store
$store = Join-Path $env:LOCALAPPDATA 'OwO\InputMethod\plugins'
& .\build\windows-release\Release\owo_config_shell.exe (Join-Path $env:LOCALAPPDATA 'OwO\InputMethod\config\owo.conf') set input_scheme full_pinyin
& .\build\windows-release\Release\owo_plugin_shell.exe $store deactivate owo.plugin.wubi86 1.0.0
& .\build\windows-release\Release\owo_plugin_shell.exe $store uninstall owo.plugin.wubi86 1.0.0
& .\build\windows-release\Release\owo_plugin_shell.exe $store input-schemes
```

此前 before-clsid.reg/before-tip.reg 备份是失效旧注册；仅在确认需要回到该旧状态且无其他部署接管时 reg import，不能当作恢复有效注册的保证。保留所有 DLL、备份、日志、原始词典和学习数据，不自动清理。

总体仍 BLOCKED/FAIL；首次窗口分项已通过，真实 Notepad 上屏未通过，未宣布完成。

## 最新实测：Chromium 输入框失败节点已确认并修复

用户反馈在 Codex 回复框显示“输入方案不可用或授权已取消”。实际 Core 日志 ipc-rejection-live-final.log 捕获：client_pid=23548、known_edit=0、class=Chrome_WidgetWin_1，request_id=25/26/27 均为 Control busy or unsafe focus。首个已确认失败点是 Core native_focus 的传统窗口类识别，发生在插件调用前，不是撤销授权或词典为空。

修复生产代码：
- src/ipc/input_method_server.cpp：传统 Edit/RichEdit 检查保留；现代宿主额外使用系统 IUIAutomation2 只读校验缓存焦点元素的 PID、Edit 控件类型、HasKeyboardFocus、IsPassword。任何属性未知/失败/密码输入均拒绝。获取并校验有界 VT_I4 RuntimeId（长度1..64）形成独立焦点身份，再复查原生前台进程/焦点 HWND，避免把整个 Chromium 宿主视作同一个输入会话。连接和事务超时设100ms，不读取正文，不改变 AppContainer/IPC 授权。
- src/ipc/input_method_client.cpp：不再丢弃 Core 已验证的拒绝诊断，失败文本限制512字节。
- src/tsf/text_service.cpp：用真实 IPC 错误替代一概“授权已撤销”的误导提示，保留编码及 Enter 原样提交。
- CMakeLists.txt：仅增加 Windows 自带 oleaut32 链接，无第三方依赖。
- tests/wubi86/process_tests.cpp：真实 named-pipe 拒绝焦点测试，确认准确返回 Control busy or unsafe focus；没有弱化拒绝或超时断言。

PASS（用户真实应用验证）：用户在部署后于同一 Codex 回复框输入并上屏“你”，明确回复“你，已经成功了”。这属于真实 Chromium 应用输入，不是 TextStore 模拟，也不是 Notepad。正式用户 Plugin Store 的独立学习文件 wubi86-learning.v1.tsv 已有 entry/wqiy/你/2（仅检查 entry，不输出提交 token）。没有修改生产词典。

PASS：Release IPC控制/WubiProcess/生产TSF提交回归3/3，ipc-diagnostic-tests.log；新拒绝原因测试包含在 uia-final-tests.log，进程测试PASS。Debug对应3/3 ipc-diagnostic-debug-tests.log，最终UIA进程回归 uia-debug-tests.log PASS。/analyze 构建通过 uia-final-analyze.log。尚未完成真实 UIA 密码框/导航切换专项安全验收，不将这些项目视作PASS。

实际 Core 已重启运行最新Release文件，最终构建 uia-core-final-build.log，实时日志 uia-live-final.log。UIA 最终数组类型/范围检查已部署。未下载软件或关闭安全机制。

NOT TESTED/仍未闭环：修复后 Notepad 的真实候选及上屏仍待用户复测；不能将 Chromium 用户确认替代 Notepad 验收。旧“没有任何真实输入成功”的记录已过时，仅保留为前期排查证据。Phase2总体尚未PASS。

UIA超时接口依据：https://learn.microsoft.com/en-us/windows/win32/api/uiautomationclient/nf-uiautomationclient-iuiautomation2-put_connectiontimeout ，https://learn.microsoft.com/en-us/windows/win32/api/uiautomationclient/nf-uiautomationclient-iuiautomation2-put_transactiontimeout 。

UIA修复后的相关回归执行结果：Debug 62项，59 PASS、历史 candidates/core_service/core_config 三项 FAIL；Release 62项，60 PASS、历史 candidates/core_service 两项 FAIL。日志 uia-regression-debug.log / uia-regression-release.log。改变的IPC库与Wubi进程测试已单独重新构建并执行；该轮CTest不是完整仓库88项测试。最新实际Notepad复测请求仍待回复；不改为PASS。

## Notepad 最终兼容性修复 — 2026-10-31（最新状态）

### 根因及生产修改

两个相继阻断点均有真实进程证据：

1. Notepad 的 RichEditD2DPT 对 GUID_PROP_INPUTSCOPE 的 GetValue 返回 E_FAIL (80004005)。旧代码将其判为未知敏感上下文，OnKeyDown 已执行但在 IPC 前拒绝。保留已有最小安全回退：src/tsf/input_method_context.cpp / include/owo/tsf/input_method_context.h 独立验证上下文视图与焦点的父子关系、当前进程及线程身份、Edit/RichEdit 类、ES_PASSWORD、ES_READONLY、IsWindowEnabled。仅在没有明确 InputScope 对象且 HRESULT 为 E_FAIL/E_NOTIMPL 时使用；未知控件、显式敏感 Scope 和其他元数据缺失仍拒绝。
2. 最新 DLL 注册后仍加载旧 DLL 的来源不是已证实的 ctfmon 缓存，而是 Notepad **MSIX 私有注册覆盖**。普通 HKCU/HKCR x64 返回 faad，使用 Windows 自带 Invoke-CommandInDesktopPackage 在 Notepad 包上下文读取同一 HKCU CLSID，返回 manual-deploy/08fb。更换会话后仍复现。只读探针 notepad-package-registry.log 是直接证据。微软文档：https://learn.microsoft.com/en-us/windows/msix/desktop/flexible-virtualization 、https://learn.microsoft.com/en-us/powershell/module/appx/invoke-commandindesktoppackage 。

本轮新增 scripts/sync-notepad-tsf-dev.ps1：经文档化包上下文接口，只同步 OwO 自己的 InprocServer32 默认值及 ThreadingModel；同步前保留这两个值，验证结果有界等待15秒，仅用于开发注册，不改变输入请求1000ms期限。拒绝覆盖已有证据目录；同路径/Apartment 时无写入。scripts/register-dev.ps1 的 x64 分支调用此同步。没有重置 Notepad 包，没有操作其他 CLSID 或关闭隔离。

本轮修改 tests/tsf/input_method_context_tests.cpp：保留原有密码/只读/未知/关系检查，补充实际禁用原生 Edit 的拒绝测试。原生回退、activation_trace.h 及相关日志代码沿用当前工作区已有实现，没有重写五笔模块。

新增 docs/wubi86/NOTEPAD_DEV_CLEANUP.md：可复制注销、包内检查和回滚命令，三个 PowerShell 代码块均通过语法解析。注销/回滚实际执行 NOT TESTED，按要求保留最终运行部署。部署同步本身已实测 PASS。

### 注册、DLL、运行数据

- PASS：当前 x64 HKCU Apartment：D:\src\OwO-release\build\wubi86-dev-final\OwO.TSF.faad3a4b196ce326.dll。
- SHA-256：FAAD3A4B196CE326803B0D8CDCAC841500393159000C5199256FDE397F851BA0；与最新 Release 构建 OwO.TSF.dll 一致。
- PASS：Notepad PID29536 实际模块加载 faad，关闭重启后的 PID27424 再次加载同一路径。x64 WindowsNotepad 11.2607.14.0。本轮未改 x86 注册，仍指向其独立 x86 DLL，未混用架构。
- 备份：build/wubi86-dev-final/notepad-package-sync-20261031/before.json（原包内08fb）、notepad-current-desktop-x64.reg（原桌面faad）。重复执行证据 notepad-package-idempotent-20261031、notepad-package-final-20261031。原始备份不覆盖。
- 正式用户 Store：%LOCALAPPDATA%\OwO\InputMethod\plugins\versions\owo.plugin.wubi86\1.0.0；input.scheme 保持授权。Scheme plugin:owo.plugin.wubi86/wubi86。
- 正式五笔 main.tsv 源文件在 data/wubi86，构建复制到 staging/assets；用户 Store assets/main.tsv SHA-256 A7EBE25CE75257453C4864BC00C24E0E3D5F5B3706168152B8A005BAFFE82263。本轮未修改/生成词典。
- 当前测试 Core PID6236，使用现有拼音 lexicon_fixture.owolx：拼音实测只证明该样本端到端工作，不宣称完整生产拼音词典覆盖。未更改拼音源码/词典/排序算法。

### 真实输入证据

| 项目 | 状态 | 证据 |
|---|---|---|
| DLL/Class Factory/ActivateEx/KeyEventSink | PASS | faad...dll.29536.trace.log，各激活阶段 S_OK；实际已加载模块路径 |
| OnKeyDown 与输入上下文 | PASS | InputScope 80004005，随后 CONTEXT_NATIVE_SCOPE_ALLOWED、CONTEXT_ALLOWED，未泛化允许未知范围 |
| 正式 wqiy 候选 | PASS | Notepad 实际窗口显示“1 你 / 2 仰泳”，不是测试 TextStore；用户另确认“出现了” |
| Space、数字1、鼠标选择 | PASS | 实际正文依次“你”“你你”“你你你”；候选收起，全部经生产 commit_scheme_selection 的 ITfRange::SetText 路径 |
| 成功后独立学习 | PASS | 受控观察 wqiy/你 由3到4到5到6；仅统计entry，不输出提交token |
| Backspace/Escape | PASS | wqiy删除成wqi，Escape关闭候选，正文保持“你你你”；学习仍为6 |
| Core和Notepad重启 | PASS | Core重启至PID6236，Notepad重启至PID27424；恢复后再候选及Space上屏，学习6到7，持久数据未丢失 |
| 当前输入上下文/方案切换 | PASS | 新建独立页，配置切全拼，nihao出现“你好/你号”，Space实际正文“你好”；再恢复五笔配置 |
| Codex实际输入回归 | PASS | 用户本轮在回复框用五笔回复“你”；前期Chromium焦点修复证据保留 |
| 独立Chrome本轮实测 | NOT TESTED | 创建本地无网络测试页，但窗口在输入前已关闭；不冒充浏览器实测 |
| 真实失败SetText/密码框应用测试 | NOT TESTED | 未在真实Notepad故意制造失败写入；有单元/真实TSF+测试TextStore拒绝与失败不学习测试，二者明确区分 |

日志目录均为 build/wubi86-dev-final：OwO.TSF.faad3a4b196ce326.dll.29536.trace.log、同名.27424.trace.log、notepad-session2-core.err、notepad-session2-restart-core.err、notepad-learning-evidence.txt。最终entry次数11含用户额外手动操作，不将额外次数算作受控自动化测试。日志没有专门逐条 SetText HRESULT/跨所有进程 correlation 记录，不能声称完整结构化全链路追踪已实现；本次上屏证据来自真实窗口、实际生产调用路径与持久学习变化。

### 最新构建与回归

| 验证 | Debug | Release |
|---|---|---|
| TSF/context/真实提交测试目标构建 | PASS | PASS |
| TSF、Wubi、Plugin、控制IPC、重连、full_pinyin 共33项 | PASS 33/33，36.92s | PASS 33/33，10.47s |
| Engine/IPC 共29项（与上行有交集，不重复累计） | FAIL 26/29，60.98s | FAIL 27/29，8.76s |

Engine/IPC：Debug candidates 原断言失败、core_service/core_config timeout；Release candidates 原断言失败、core_service long suffix/local bridge失败。未新增失败名称，基线证据仍为 build/wubi86/phase2-baseline-ipc.xml、phase2-baseline-release.xml（原HEAD0c30b077...）。不删除/跳过测试，不改变断言或拼音算法。Debug/Release日志分别 notepad-final-*-build.log、notepad-final-*-tests.log、notepad-final-*-engine-ipc.log。本轮不是全仓88项完整CTest。

PASS：MSVC /analyze TSF目标构建，notepad-final-analyze.log。原生回退首次分析 notepad-native-analyze.log 记录原有 C28251/C6262/C6031，原始源码 phase2-baseline-static.log 有对应警告；不是零警告。

PASS：本轮涉及已跟踪文件 git diff --check。全工作区 FAIL：用户已有 global.json 第8行 EOF空行，保留不修改（notepad-final-diff-check.log）。脚本和清理代码块语法 PASS。本轮无下载、安装或系统环境变量变更，无子智能体、无Git破坏性操作。未自动注销/卸载。

### 最终结论

Notepad 的原始兼容性故障已在真实应用修复：PASS。实际最新DLL、按键、候选、三种选择、上屏、独立学习、重启恢复及拼音样本回归均有实测；Codex输入由用户复核。独立浏览器本轮单独复测及真实敏感控件测试仍如上标记 NOT TESTED，不能据此宣布所有 Phase2 历史项目全绿。Phase2 全量验收本轮 NOT TESTED，不提前宣称总体PASS。清理/回滚命令见 NOTEPAD_DEV_CLEANUP.md；当前开发部署保持可用。
