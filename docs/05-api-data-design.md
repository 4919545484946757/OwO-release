# 05 · API 与数据设计

OwO 当前没有 HTTP 服务或关系型数据库。这里的 API 指内部进程间契约和插件接口；“数据库设计”对应实际使用的本地文件数据模型。

## 接口边界

| 边界 | 消息与关键字段 | 约束和失败行为 | 契约源码 |
| --- | --- | --- | --- |
| ① TSF ⇄ ② Core | `candidate_request/response`、`candidate_update_request/response`、`candidate_committed`；`request_id`、`context_generation`、`input`、`page`、`candidates`、`candidate_consumed`、`candidate_groups` | 本机命名管道；严格解码；旧请求和旧焦点结果不得覆盖新快照；内部 JSON 尚非稳定公共 API | `include/owo/protocol/messages.h` |
| ② Core ⇄ ③ ModelHost | `rank_request/response`；请求 ID、超时、输入、上下文、候选 | 模型协议 v2；失败或超时退回基础排序 | `include/owo/model/model_protocol.h` |
| ② Core ⇄ ④ 插件 | `hello/invoke/cancel/shutdown`；插件 ID、服务名、请求 ID、payload | 插件协议 v1；单条 payload 最多 256 KiB；验证清单和实际授权 | `include/owo/plugin/plugin_protocol.h` |
| ④ Agent 插件 ⇄ ⑧ Agent | `session_id`、`action`、应用上下文、结构化命令 ID | Agent 协议 v3，兼容 v2；脱敏、限长、会话匹配；不得将回复文本当作控制命令 | `include/owo/agent/agent_protocol.h` |
| 插件化输入方案 | 方案 ID、输入、页码、候选 ID、`consumed_ranges` | 实验接口；范围按原始 UTF-8 半开字节区间；宿主决定上屏 | `include/owo/engine/input_method_provider.h` |

进程间变更应同时定义版本、请求身份、容量、超时、取消、错误与旧结果丢弃规则。对外开发以[插件指南](plugins/development.md)和[输入方案说明](plugins/input-schemes.md)为准，不应将内部 JSON 或 C++ 结构视为跨版本 SDK。

## 本地数据模型

| 数据 | 格式与位置 | 所有者和约束 |
| --- | --- | --- |
| 通用配置 | `%LOCALAPPDATA%\OwO\InputMethod\config\owo.conf` | 配置模块校验、保存和监视；导入前确认并备份 |
| 基础中文词典 | 安装包/启动参数指定的 `.owolx` 紧凑二进制文件 | Core 只读查询；停用候选来源不等于删除启动文件 |
| 独立导入字典 | 词库管理器维护的字典项 | 可启停、重命名、导出、删除；拼音编码缺失时需补码 |
| 用户学习 | `data\user-frequency.owuf` 及 `.bak` | Core 串行修改；版本代数防并发覆盖；清空需确认，备份可能含敏感用词 |
| 插件配置和授权 | 插件专属 `settings.json`、清单及当前版本授权记录 | 配置与授权分别管理；安装包不复制原机授权 |
| 模型与日志 | 本地 libime 模型及 `logs` | 模型可选；日志不得暴露密码或完整敏感输入 |

用户词库的 JSON/TSV 导出包含 `input/text/count`，不含上下文历史；详见[用户词库管理](user-lexicon-editor.md)。独立字典边界见[词库管理器](dictionaries.md)。
