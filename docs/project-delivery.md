# OwO 0.2.7 项目交付说明

本说明与 [HTML 架构图](project-architecture.html) 使用同一组件编号，以当前仓库为基线；实验性能力不视为稳定对外接口。

## 5. API 与数据设计

### 接口边界

| 边界 | 契约 / 核心字段 | 约束 |
| --- | --- | --- |
| ① TSF ⇄ ② Core | `include/owo/protocol/messages.h`：`candidate_request/response`、`candidate_update_request/response`、`candidate_committed`、`user_lexicon_request/response`；`request_id`、`context_generation`、`input`、`page`、`page_size`、`candidates`、`candidate_consumed`、`candidate_groups` | 本机命名管道；内部 JSON 契约，**尚非稳定公开 API**；旧请求与旧焦点结果不得覆盖新快照 |
| ② Core ⇄ ③ ModelHost | `include/owo/model/model_protocol.h`：`rank_request/response`、`request_id`、`timeout_ms`、`input`、`context`、`candidates` | 协议 v2；模型失败、超时不阻塞基础候选 |
| ② Core ⇄ ④ 插件 | `include/owo/plugin/plugin_protocol.h`：`hello/invoke/cancel/shutdown`、`plugin_id`、`service`、`request_id`、`payload` | 协议 v1；单条 payload 最多 256 KiB；服务需版本后缀；清单与逐版本授权双重校验 |
| ④ Agent 插件 ⇄ ⑧ Agent | `include/owo/agent/agent_protocol.h`：`session_id`、`action`、输入与应用上下文、结构化命令 ID、状态 | 协议 v3，兼容 v2；脱敏与限长；仅用户选择可执行候选，高风险操作另行确认 |
| 插件化输入方案 | `include/owo/engine/input_method_provider.h`；请求含 `request_id/session_id/scheme_id/input/page`，响应含候选 ID 与 `consumed_ranges` | 实验接口；原始 UTF-8 半开字节区间，宿主负责光标转换与上屏 |

这些不是 HTTP API；不可把内部 JSON 或 C++ 结构直接当成跨版本 SDK。插件对外开发以 [插件指南](plugins/development.md) 和 [输入方案说明](plugins/input-schemes.md) 为准。

### 数据存储（当前没有关系型数据库）

| 数据 | 位置 / 格式 | 所有者与规则 |
| --- | --- | --- |
| 通用配置 | `%LOCALAPPDATA%\OwO\InputMethod\config\owo.conf` | 配置模块读写；设置导入需校验、确认和备份；Core 监视变更 |
| 基础词典 | 安装包 / 启动参数指定的 `.owolx` 紧凑二进制词典 | Core 只读查询；停用候选来源不等于可删除启动必需文件 |
| 独立导入字典 | 词库管理器维护的独立字典项 | 可启停、重命名、导出、删除；缺失拼音编码需补码后才可正常参与拼音查询 |
| 用户学习库 | `%LOCALAPPDATA%\OwO\InputMethod\data\user-frequency.owuf` 及 `.bak` | Core 单一所有者串行修改；版本代数防并发覆盖；清空需确认，备份仍可能保留敏感用词 |
| 插件配置与授权 | 插件专属 `settings.json`、清单及当前版本授权记录 | 插件只能访问获准数据；配置与授权不能视作同一信息，安装包不复制原机授权 |
| 模型资源与日志 | 本地 libime 模型文件、`logs` | 模型可选，日志用于诊断；不应记录密码或完整敏感输入 |

用户词库导入/导出格式为 JSON 或 TSV（`input/text/count`），不包含上下文历史；详见 [用户词库管理](user-lexicon-editor.md)。独立字典格式与边界见 [词库管理器](dictionaries.md)。

## 6. 自定义项目验收清单

将“自定义项目”按基于 OwO 源码二次开发、加入输入方案或插件理解。以下清单供每个交付版本逐项勾选，不声称当前仓库已满足所有未来扩展需求。

- [ ] 需求与范围：注明目标 Windows 版本、输入方案、插件权限、离线能力、可选模型和明确不做项。
- [ ] 构建：按 README 的 CMake/.NET 前置条件完成 x64 Debug/Release；无未解释的编译警告；版本号与包元数据一致。
- [ ] 输入链路：目标应用中可激活 TSF；输入、分割、候选、翻页、展开、鼠标选取、部分上屏、失焦与重连均可复测。
- [ ] 质量：固定测试集包含完整/不完整拼音、简拼、纠错、手动边界和上下文；记录首选、首屏覆盖、Top-K 与延迟基线，并比较回归。
- [ ] 性能：冷启动、连续按键、分页和低端机器候选延迟有测量记录；ModelHost 不可用时基础候选仍可用。
- [ ] 数据：配置、词典、用户学习的增删改、备份、导入/导出及重启持久化通过；错误文件不会导致静默丢失原数据。
- [ ] 插件：仅声明所需权限；安装、启用、升级分别确认；取消、超时、非法响应、越界数据与旧请求均不产生上屏或执行副作用。
- [ ] 安全：密码框不采集上下文；Agent 结果文本不作为控制命令；敏感日志受限；高风险操作由宿主再次确认。
- [ ] UI：深浅主题、缩放和多分辨率下设置中心、安装助手、候选框均可读且无溢出/闪烁。
- [ ] 发行：安装、覆盖升级、卸载、失败回滚在干净 Windows 11 环境测试；校验发行包 SHA-256 和许可清单。
- [ ] 文档：更新架构、接口变更、迁移说明、已知限制和操作手册；实验接口明确标为不稳定。

推荐证据：`ctest` 结果、设置中心专项测试、CLI 候选报告、安装向导只读导航检查、手测应用/机器记录及发行校验清单。自动化通过不能代替真实应用手测。

## 7. 项目结构与编码规范

### 已有结构

| 路径 | 用途 |
| --- | --- |
| `apps/` | Core Service、ModelHost、运行启动器、WinUI 设置/安装、各类 shell/CLI 入口 |
| `include/owo/` | 原生模块公开头文件与内部协议声明 |
| `src/engine/`、`src/tsf/` | 输入方案、解析、候选、学习；TSF 组合态与候选渲染 |
| `src/ipc/`、`src/protocol/` | 本机管道、封装与消息验证 |
| `src/model/`、`src/config/`、`src/plugin/`、`src/agent/`、`src/core/` | 模型、设置、插件、Agent 契约及 Core 适配 |
| `plugins/` | 官方资源插件和示例进程插件 |
| `tests/`、`benchmarks/` | 契约、集成、质量和性能验证 |
| `scripts/`、`packaging/`、`tools/` | 构建、安装、发行脚本与词典编译等工具 |
| `docs/`、`artifacts/`、`build/` | 设计文档；发行产物；本地构建产物（后两者不作为源码） |

### 后续开发统一遵循的项目规范

1. C++ 使用 C++20；原生库头文件放 `include/owo/<module>/`，实现放 `src/<module>/`；WinUI 页面与代码放对应 `apps/`；文件名优先延续现有 `snake_case`，类型名沿用 `PascalCase`。
2. 新功能先确定归属：TSF 只做交互与上屏；Core 负责候选和学习；模型、插件在独立进程。禁止把第三方 DLL 加载进 TSF/Core。
3. 跨进程变更必须定义版本、请求 ID、容量上限、超时、取消、错误状态与旧结果丢弃规则；不得用展示文本隐式触发命令。
4. 新配置给出默认值、校验、迁移/回退行为；写文件使用校验与原子替换，持有用户数据时保留可恢复副本。
5. 每次修复补最小可复现测试；覆盖成功、失败、空值、超长、恶意输入、焦点变更与重复请求。基准测试记录机器与配置，不只报告主观“更快”。
6. 插件权限按最小必要原则声明；新增权限需同步更新风险说明、清单验证、授权测试和开发文档。
7. 提交只包含本次范围内的源码/文档/测试；发行前核对版本、许可证、SHA-256 与实际安装包。不要把 `build/`、用户词库、个人配置或凭据提交到仓库。

现有构建选项、包格式和具体测试命令以仓库根目录 [README](../README.md) 与 `CMakeLists.txt` 为准；以上规范不追认历史代码完全符合全部条目。
