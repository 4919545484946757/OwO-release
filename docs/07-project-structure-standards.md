# 07 · 项目结构与编码规范

目录说明描述现有仓库；规范是后续改动应遵守的约定，不追认历史代码完全符合每一项。

## 已有目录

| 路径 | 职责 |
| --- | --- |
| `apps/` | Core、ModelHost、运行启动器、WinUI 设置/安装和开发 CLI 的进程入口 |
| `include/owo/` | C++20 模块头文件与内部协议声明 |
| `src/engine/`、`src/tsf/` | 输入方案、解析、候选、学习；TSF 组合态与候选渲染 |
| `src/ipc/`、`src/protocol/` | 本机管道、消息封装、严格解析与请求身份验证 |
| `src/model/`、`src/config/`、`src/plugin/`、`src/agent/`、`src/core/` | 模型、配置、插件、Agent 契约及 Core 适配 |
| `plugins/` | 官方资源插件和进程插件示例 |
| `tests/`、`benchmarks/` | 功能、契约、集成测试和性能基准 |
| `scripts/`、`packaging/`、`tools/` | 构建、注册、测试、发行脚本及词典编译工具 |
| `docs/`、`build/`、`artifacts/` | 文档、本地构建中间物、发行输出；后两者不作为源码目录 |

## 新功能放置原则

```text
应用输入事件 → src/tsf/ → 本机 IPC → Core / src/engine/
可选排序   → src/model/ + ModelHost
插件扩展   → src/plugin/ + 独立插件进程
用户设置   → apps/settings_center/ + src/config/
新增协议   → include/owo/<module>/ + src/<module>/ + tests/
```

## 编码规范

1. C++ 使用 C++20。沿用现有 `snake_case` 文件名、`PascalCase` 类型名；原生头文件放 `include/owo/<module>/`，实现放 `src/<module>/`。
2. TSF 负责交互与上屏，Core 负责候选和学习，模型与插件在独立进程。第三方 DLL 不加载到 TSF/Core。
3. 进程间协议应写明版本、请求 ID、容量上限、超时、取消、错误和旧结果丢弃规则；不得用展示文本隐式触发命令。
4. 新配置字段需有默认值、校验、迁移与回退；持久数据使用可恢复、原子化保存。
5. 修复时补最小复现测试，覆盖空值、超长、非法输入、焦点变化和连续请求。性能报告记录机器与配置。
6. 插件按最小权限声明；新增权限同步更新风险文案、授权验证、协议测试和开发文档。
7. 提交仅包含本次范围；发布时核对版本、许可证和 SHA-256。不要提交用户词库、凭据、个人配置或 `build/` 产物。

具体构建命令、平台前置条件和发行流程以[根 README](../README.md)与 `CMakeLists.txt` 为准。
