# OwO Agent IPC SDK 与可移植模拟 Agent

SDK 与连接器版本：`1.3.0`（Agent 协议 v3，兼容读取 v2）

协议 v3 还提供结构化任务草稿：候选可携带槽位补丁、任务版本以及槽位到原始拼音区间的映射。选择候选只更新对应槽位，不要求重新提交整条自然语言，也不会直接把候选文字上屏。

该包用于开发、联调第三方 Agent。Windows x64 用户无需安装编译工具即可运行模拟服务。

## 快速测试

1. 双击 `Start-Mock-Agent.cmd`，保持控制台打开。
2. 如尚未安装连接器，在 OwO 设置中心安装 `connector/org.owo.agent-ipc-1.3.0.owopkg`。
3. 阅读权限和隐私提示，明确同意后启用连接器。
4. 确认插件管道为默认值 `\\.\pipe\OwO.Agent.External.v1`。
5. 输入 `vbangwozhaowenjian`；完整选择拼音候选会自动发送，也可按 Enter 或 Space 显式发送。
6. 在模拟候选中测试回显、应用上下文、执行状态、高风险拦截、翻页和取消。

模拟窗口会打印收到的输入、当前应用信息和带应用标识的上下文分段。请勿输入真实密码、令牌、支付信息或其他秘密。按 Ctrl+C 或关闭窗口停止服务。

## 目录

```text
bin/                         可移植 Windows x64 模拟 Agent
connector/                   OwO Agent IPC 连接器测试包
docs/                        协议、JSON Schema、隐私和联调文档
examples/mock-agent/         模拟 Agent C++ 源码
include/owo/agent/           协议与管道头文件
src/                         协议参考实现
CMakeLists.txt               独立示例工程
SHA256SUMS.txt               包内文件校验值
```

## 从源码构建

需要 Visual Studio 2022 C++ Build Tools、Windows SDK 和 CMake 3.24 或更高版本：

```powershell
cmake -S . -B build -A x64
cmake --build build --config Release
```

生成文件位于 `build/Release/owo_agent_mock.exe`。示例使用静态 MSVC 运行库，发行目录不需要额外携带项目 DLL。

## 安全边界

模拟 Agent 仅监听本机命名管道，并把 ACL 限制为当前 Windows 用户、SYSTEM 和管理员。它不会执行返回的自然语言，也不会真的删除文件或修改系统。标记为高风险的删除命令仅用于验证 OwO 的二次确认拦截。

连接器和模拟程序均未进行商业代码签名。第三方实现仍必须阅读
`docs/agent-ipc-integration.md` 和 `docs/agent-ipc-privacy.md`，不能把该模拟程序当作生产安全实现。
