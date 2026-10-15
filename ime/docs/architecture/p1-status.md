# P1 实施状态

## 已完成

- C++20/MSVC/CMake 多目标工程。
- 1 MiB 有界、长度前缀 UTF-8 JSON 消息。
- 请求 ID、上下文代际、协议版本和显式错误响应。
- 命名管道 Core Service 与命令行测试壳。
- 固定候选请求、响应和协议化有序关停。
- 最小 `ITfTextInputProcessorEx` COM DLL、类工厂和生命周期。
- 带失败回滚的开发注册/注销脚本；TSF 配置注册需要提升权限。
- 注册后配置默认禁用，不改变用户当前输入法；测试宿主按进程显式启用。
- 注册 `GUID_TFCAT_TIP_KEYBOARD` 类别，并在注销和失败回滚时对称移除。
- DLL 加载和 COM 实例化烟雾测试。
- `ITfKeyEventSink` 按键接入；ASCII 字母形成 P1 预编辑缓冲。
- 最新请求覆盖队列和上下文代际校验，旧候选不会覆盖新输入。
- TSF 后台工作线程调用 Core Service，宿主按键线程不等待 IPC。
- 轻量 Win32 非激活候选窗，`1`/空格通过同步 TSF 编辑会话提交。
- 候选窗优先通过 `ITfContextView::GetTextExt` 定位文本光标，无法取得范围时回退鼠标位置。
- Escape 取消、Backspace 更新，以及停用时工作线程有序退出。
- IPC 写入和读取共享截止时间；无响应服务可取消挂起 I/O。
- Core Service 连续请求、服务缺失、无响应超时和协议化关停集成测试。

## 阶段验收报告：P1

### 已完成

- P1 要求的 TSF → Core Service → 固定候选 → 候选窗 → 选择上屏纵向闭环。
- CMake/vcpkg 工程、IPC 协议、结构化日志、有序启停、开发注册与注销脚本。
- 单元、契约、DLL 烟雾、超时、断连、崩溃与重启隔离测试。
- 2026-07-31 在 Windows 11 真实应用中手工验证：用户通过 `Win + Space` 选择 OwO，输入 `ni` 后候选窗显示，按空格成功上屏。

### 未完成与原因

- 无 P1 范围内未完成项。真实中文输入引擎、翻页、用户词频与延迟基准属于 P2。

### 构建与测试结果

- Windows Release：`ctest --preset windows-release --output-on-failure`，4/4 通过。
- 真实 Windows 11 TSF 端到端手工验收通过。
- 验收后 Core Service 、TSF 配置和 COM 开发注册已清理。

### 结论

P1 验收完成。根据阶段门禁，在开发者确认前不进入 P2。
