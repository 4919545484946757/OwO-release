# ADR-0006：ONNX Runtime CPU 版本与 Windows 部署方式

- 状态：P3A 接受，真实模型许可门禁仍未解除
- 日期：2026-08-01

## 背景

P3A 已建立 ModelHost 进程、模型资产校验、WordPiece/BERT pair 张量、可替换推理 session 和 ONNX 元数据契约。下一步需要选择 Windows 11 x64 的首个真实推理运行时，但运行时不得进入 TSF DLL 或 Core Service，也不得让默认构建隐式联网。

## 调研结果

### 官方稳定版

- ONNX Runtime `v1.28.0` 于 2026-07-25 正式发布，标签提交为 `da9b5e3...` 且 GitHub 显示签名已验证。
- 1.28.0 包含 FlatBuffer/ONNX 模型加载、指针边界、整数溢出、外部数据和 protobuf CVE 等多项安全加固。模型文件属于不可信输入面，因此不采用缺少这些修复的旧版本作为新基线。
- 项目本体许可证为 MIT；官方 NuGet 包内同时包含 `LICENSE`、`ThirdPartyNotices.txt` 和 `.signature.p7s`。

### 包管理方案对比

| 方案 | 结果 |
| --- | --- |
| vcpkg `onnxruntime` | 2026-08-01 的端口仍为 `1.23.2#1`，从源码构建并引入 abseil、boost、protobuf、onnx、re2、eigen 等较大依赖图；版本落后且构建成本高，不采用。 |
| GitHub release ZIP | 官方原生 C/C++ 包，但 release 页面未提供独立校验和；可以固定自测哈希，但供应链元数据弱于 NuGet catalog。 |
| `Microsoft.ML.OnnxRuntime` NuGet | 官方 Microsoft owner、保留前缀、包含 native C/C++ headers/lib/DLL、NuGet 签名，官方 catalog 发布 SHA-512；采用。 |

## 固定资产

- 包 ID：`Microsoft.ML.OnnxRuntime`
- 版本：`1.28.0`
- 下载 URL：`https://api.nuget.org/v3-flatcontainer/microsoft.ml.onnxruntime/1.28.0/microsoft.ml.onnxruntime.1.28.0.nupkg`
- NuGet catalog 发布时间：`2026-07-25T05:32:31.663Z`
- 包大小：`139,145,017` 字节
- SHA-512（hex）：`d97d83d031fa744cd67dab61e88deecc8b6c3c11b1a98951b0fcf73852bb4bd4df935d0c71674a19e80d8d3f0a98e02d9129e8df24c5f5ea38de60ef9fdb1a97`
- SHA-512（NuGet catalog Base64）：`2X2D0DH6dEzWfath6I3uzItsPBGxqYlRsPz3OFK7S9Tfk10McWdKGegNjT8KmOAtkSno3yTF9eo43mDvn9salw==`
- 本地比对结果：完全一致

Windows x64 选择性部署文件：

- `onnxruntime.dll`：`15,809,848` 字节
- `onnxruntime.lib`：`2,124` 字节
- `onnxruntime_providers_shared.dll`：`21,856` 字节
- `onnxruntime_providers_shared.lib`：`2,314` 字节
- `build/native/include/` 下 C/C++ API headers
- `LICENSE` 与 `ThirdPartyNotices.txt`

139 MB 的 NuGet 包只进入本地构建缓存；发布包只复制 Windows x64 运行所需 DLL 和许可文件。实际发布增量约 15.9 MB，不把其他平台资产打包。

## 决策

1. 固定 ONNX Runtime CPU `1.28.0`，仅动态链接到 `owo_model_host.exe`。
2. 不写入 `vcpkg.json`，避免退回 1.23.2 和引入无关源码依赖图；这是有证据的局部例外，不改变其他依赖默认使用 vcpkg 的策略。
3. 默认 `OWO_ENABLE_ONNXRUNTIME=OFF`，普通构建、CTest 和基础输入均不下载或依赖 ORT。
4. 通过显式 PowerShell 脚本下载官方 NuGet 包、比对固定 SHA-512、选择性解包。哈希不符立即失败并不覆盖已验证 SDK。
5. CMake 仅接受显式 `OWO_ONNXRUNTIME_ROOT`，不得搜索用户全局 PATH 或隐式 NuGet 缓存。
6. 首版只启用 CPU Execution Provider；DirectML、CUDA、WinML 和自动 EP 下载均关闭。
7. `onnxruntime.dll` 仅与 ModelHost 同目录部署；TSF DLL 和 Core Service 不链接、不加载该 DLL。
8. 发布时必须携带 MIT LICENSE、ThirdPartyNotices，并记录 DLL SHA-256、包 SHA-512 与版本。

## 风险与缓解

- **新版本回归**：通过合成 ONNX、固定候选夹具、错误模型和进程退出测试后才允许默认启用。
- **DLL 劫持**：ModelHost 使用应用目录的固定 DLL；发布阶段配置安全 DLL 搜索策略，不依赖 PATH。
- **模型解析攻击面**：仅加载 manifest 同目录、哈希匹配且元数据契约通过的模型；模型加载失败不影响基础输入。
- **包体增加**：运行时约 15.9 MB，仅随启用真实模型的构建部署。
- **升级漂移**：版本与 SHA-512 同时固定；升级需新 ADR 记录安全变更、包体和回归数据。

## 回滚

关闭 `OWO_ENABLE_ONNXRUNTIME` 并删除本地构建缓存即可恢复纯 Mock ModelHost；基础输入、IPC 和模型抽象不依赖 ORT。

## 实施验证

- 新增 `windows-release-ort` 显式预设；SDK 缺失时配置立即失败，默认预设不读取 ORT 缓存。
- 使用项目生成的 589 字节 ONNX（opset 17）验证真实 CPU session 创建、运行时输入输出元数据、候选打分、取消、零预算超时及非法批次拒绝。
- ONNX Runtime 配置 CTest 19/19 通过；默认关闭 ORT 的 Release CTest 18/18 通过。
- ModelHost 与 shell 的命名管道实进程联调返回“泥号 / 你好”，排序、关闭请求和宿主退出码均成功。
- `dumpbin /dependents` 确认 ORT ModelHost 引用 `onnxruntime.dll`，同一构建中的 `OwO.TSF.dll` 不引用该 DLL。
