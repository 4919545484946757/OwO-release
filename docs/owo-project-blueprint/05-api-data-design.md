# 05 · API 与数据设计

OwO 当前没有 HTTP 服务或关系型数据库。这里的 API 是本机 IPC、插件和实验性输入方案契约；“数据库设计”指实际本地文件与所有权。以下将当前契约和未来版本化方向分开。

## 当前接口

| 边界 | 消息 / 关键字段 | 约束 | 源码 |
| --- | --- | --- | --- |
| ① ⇄ ② | `candidate_request/response`、`candidate_update_request/response`、`candidate_committed`；`request_id`、`context_generation`、`input`、`page`、`candidates`、`candidate_consumed`、`candidate_groups` | 本机命名管道，严格解码，旧请求和旧焦点结果失效；内部 JSON 非稳定公共 API | `include/owo/protocol/messages.h` |
| ② ⇄ ③ | `rank_request/response`、请求 ID、超时、输入、上下文、候选 | 模型协议 v2；失败退回基础排序 | `include/owo/model/model_protocol.h` |
| ② ⇄ ④ | `hello/invoke/cancel/shutdown`、插件 ID、服务名、请求 ID、payload | 插件协议 v1；单消息 payload 上限 256 KiB；清单及授权双重检查 | `include/owo/plugin/plugin_protocol.h` |
| ④ ⇄ ⑧ | `session_id`、`action`、应用上下文、结构化命令 ID | Agent 协议 v3，兼容 v2；脱敏、限长、会话身份校验；文本仅展示 | `include/owo/agent/agent_protocol.h` |
| 插件输入方案（实验） | 方案 ID、输入、页码、候选 ID、`consumed_ranges` | UTF-8 原始输入半开字节区间；宿主决定消费、上屏和学习 | `include/owo/engine/input_method_provider.h` |

跨边界实现都应记录协议版本、请求身份、最大长度、超时、取消、错误类型、重放防护和旧结果丢弃规则。对外开发依[插件指南](../plugins/development.md)和[输入方案说明](../plugins/input-schemes.md)；不要把内部 C++ 结构当稳定 SDK。

## 当前文件数据模型

| 数据 | 格式 / 默认位置 | 所有权及一致性 |
| --- | --- | --- |
| 通用设置 | `%LOCALAPPDATA%\OwO\InputMethod\config\owo.conf` | 配置模块验证、保存与监视；导入前确认和备份 |
| 基础词典 | 安装包或启动参数指定的 `.owolx` | Core 只读查询；停用候选来源不等于可删除启动文件 |
| 独立字典 | 词库管理器维护的字典项 | 可启停、重命名、导出、删除；未补编码需补码 |
| 用户学习 | `data\user-frequency.owuf` 及 `.bak` | Core 串行修改；版本代数防并发覆盖；备份可能保留个人用词 |
| 插件 | 各插件 `manifest.json`、专属 `settings.json` 和版本授权记录 | 配置不等于授权；安装包不继承其他机器授权 |
| 模型与日志 | 本地 libime 模型文件和 `logs` | 模型可选；诊断不得写入密码或完整敏感输入 |

用户词库 JSON/TSV 导出字段为 `input/text/count`，不包含上下文历史；详见[用户词库](../user-lexicon-editor.md)。

## 未来 API 演进（规划，不代表已实现）

1. 稳定插件 SDK：明确最低宿主版本、能力协商、服务名版本和向后兼容窗口；旧插件遇到未知能力应安全拒绝。
2. 方案级命名空间：为拼音、五笔、仓颉等分别命名学习键和字典来源；迁移旧拼音数据时不得改变已有排序。
3. 模型/资源清单：新增后端类型、架构、最低版本、许可证和 SHA-256 等结构化字段；先校验、再激活，并保留上一可用版本。
4. 可观测性契约：分阶段耗时、失败码和请求链路 ID 可用于调试，但默认不记录明文输入；定义限额与删除策略。
5. 跨平台适配：将焦点、安全输入和提交接口抽象为平台契约；不要把 Windows `HWND`、TSF 或注册表字段加入通用协议。

## 未来数据迁移门槛

- 每次格式升级须写 `schema_version`、读旧版、备份、校验、原子切换及回退步骤。
- 词典和模型下载需要来源许可证和内容哈希；用户学习与插件授权不得随资源安装静默迁移。
- 禁止把用户词频文件转换为单一不可回滚的大数据库，除非已通过冷启动、低内存、故障恢复和隐私对比验收。
