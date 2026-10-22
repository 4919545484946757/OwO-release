# TSF 开发注册与缓存刷新

候选栏外观、字体尺寸和动画已恢复到 `779742a` 时期；新增的动效辅助模块与实验预览目标已移除。候选生成、用户词补全污染修复和注册缓存刷新功能保留。没有更改系统动画设置、皮肤插件或用户词库。

## 注册成功但界面未更新

本机复核发现，新记事本也曾加载安装目录的旧 DLL，而用户级 COM 注册已指向新版。只检查注册路径或重启 Core 不能证明新界面实际生效。

Windows 的 [TF_InvalidAssemblyListCacheIfExist](https://learn.microsoft.com/en-us/windows/win32/tsf/tf-invalidassemblylistcacheifexist) 可刷新当前会话的 TSF 描述缓存。注册入口现从系统目录加载 `msctf.dll` 并调用该 API；开发注册脚本也执行显式检查。缓存刷新失败只警告，不撤销已有可用输入法配置；可用以下命令单独诊断：

```powershell
.\build\windows-release\Release\owo_tsf_profile_check.exe --refresh-cache
```

`scripts/open-candidate-testpad.ps1` 提供全新进程的普通文本控件，规避记事本复用旧进程的影响。它不重启现有应用，不保存文本，不更改系统动画设置。使用 Windows PowerShell 的 `-STA -File` 参数运行即可。验证部署时应读取该进程实际装载的 `OwO.TSF.*.dll` 路径，而不只查看注册表。

此前诊断中，缓存刷新后独立测试进程能够加载新注册的 DLL，证明只核对注册表不足以验证更新。候选栏视觉及动效实验现已回退，今后的 UI 更新或回退仍应检查实际加载路径。已打开的应用可能保留之前的模块，需保存并完全退出后再开启。

## 回退验证

```powershell
cmake --build --preset windows-release --target owo_tsf owo_tsf_profile_check owo_tsf_smoke owo_candidate_window_order_tests owo_pinyin_cursor_tests
ctest --test-dir build/windows-release -C Release -R '^owo.tsf.(dll_smoke|candidate_window_order|pinyin_cursor)$' --output-on-failure
cmake --build --preset windows-x86-release --target owo_tsf
```

部署仍使用独立的哈希命名 DLL，不覆盖被应用占用的旧文件。Core 未发生源码改动，无需因此重启；已经加载实验 UI 的应用不会被强制终止。
