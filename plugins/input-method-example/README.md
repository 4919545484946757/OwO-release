# 实验表码输入方案示例

这是独立 AppContainer 进程示例，不是完整仓颉或五笔词库。仅演示字母编码查询、分页、候选 ID 及原始输入消耗范围：`a → 日 / 昌 / 明`，`ab → 明`，`b → 月`。

构建：

```powershell
cmake --build --preset windows-release --target owo_example_input_method_plugin
```

产物：`build/windows-release/Release/owo_example_input_method_plugin.exe`。开发打包时放入本目录的 `bin/`，与清单及配置文件组成普通插件文件夹。不要把测试探针 `owo_input_method_process_probe.exe` 打包，它带有仅用于测试的故障注入。

本阶段已开放实验性设置中心选择及 TSF 输入接线。安装并明确授权、启用后，在「输入方案」页点击「发现已授权插件输入方案」，选择本方案并保存。默认仍为全拼，不自动切换方案。真实的进程链路通过 `owo.core.input_method_process` 专用测试验证，在独立临时仓库中授权与运行，不启用用户已安装的插件。当前限支持通过安全检查的原生 Edit/RichEdit 控件；自定义宿主兼容性仍待完善。

宿主必须先验证插件安装绑定与明确的 `input.scheme` 授权，再调用 `discover_plugin_input_methods()` 获取描述，通过 `InputMethodSessionRouter.open()` 绑定实际声明的方案。旧 `input.context` 授权不等于新方案授权，不会自动迁移。测试用例自动授权只针对隔离测试夹具，不能作为发行安装授权流程。

隐私：接收本次编码、光标字节位置、候选分页、会话/请求/方案 ID 和用户选择的候选 ID。不会采集应用文本、剪贴板、跨应用上下文；不联网、不执行程序、不直接上屏、不修改系统。密码与安全输入场景由宿主检测并阻断，不能由插件自行声明安全。

协议及阶段边界见[输入方案接入文档](../../docs/plugins/input-schemes.md)。方案能力与线协议仍为实验设计；不要把实验服务或 C++ 虚接口视为稳定 ABI。

资源：沙箱插件可读取自身包内词典资源，并在宿主传入的私有数据目录读写学习数据，不能修改包内资源。已明确授权 `input.scheme` 的沙箱进程内存上限为 512 MiB（不是预分配），无需为普通本地词典申请全盘访问或 full-trust。网络、应用上下文及系统操作仍需独立权限和相应宿主接口，不随方案授权自动开放。
