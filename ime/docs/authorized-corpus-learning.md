# 授权语料学习（L1）

设置中心“学习与排序”页新增“授权语料学习”。它学到的句子还有两个独立用途：下一词联想（见 [next-phrase-prediction.md](next-phrase-prediction.md)）和风格档案（见 [style-profile.md](style-profile.md)）。默认关闭；只有用户打开后，输入法才会把经它上屏的文字整理成句子存到本机，供后续的下一词联想和按语气起草使用。

## 学什么

| 来源 | 处理 |
| --- | --- |
| 用户在允许的应用里、经本输入法上屏的文字 | 记录 |
| 在敏感输入框（密码、PIN 等）里的文字 | 不记录 |
| 未列入允许列表、或列入排除列表的应用 | 不记录 |
| Agent 回写、粘贴、其他输入法打的字 | 不记录 |

输入法只能看到经它上屏的内容，因此“上屏即本人打的”是可归因的。TSF 端把每次上屏作为 `agent_context_recorded` 消息发给 Core，并带上 `corpus_eligible`。只有用户在 TSF 编辑会话里输入的文字才是 `true`；`AgentWriteBack` 成功回写后记为 `false`。协议在编解码两侧都会拒绝敏感输入携带该标记（协议版本 14）。

Core 收到后依次检查：学习总开关、暂停开关、敏感标记、应用允许/排除、PII 过滤（`filter_sensitive_text`）。任何一步不通过都不进入语料。

## 句子怎么形成

`UserCorpusCollector` 按输入上下文缓冲片段：

- 遇到 `。！？…!?` 或换行即结束一句；
- 同一句超过 10 秒没有新输入即结束；
- 单个上下文最多缓冲 4 个，超出时先结束最旧的；
- 同一上下文切换到另一个应用时，各自成句；
- 单句上限 512 字节（UTF-8），超出即切分；
- 少于 2 个字符、或含 6 位以上连续数字（验证码、号码）的句子丢弃。

完成的句子先在内存里攒着，满 8 句或最旧的超过 30 秒再写盘；退出时强制写入。

## 存在哪、怎么保护

- 位置：`%LOCALAPPDATA%\OwO\InputMethod\data\user-corpus.owuc`（由 `runtime_launcher` 以 `--user-corpus` 传给 Core）。
- 格式：`OWUC` 魔数 + 版本 + 校验和；Windows 上用 DPAPI（当前用户）加密，换用户或换机器无法解开。
- 上限：20000 条，超出时删最旧的；另有保留天数（`corpus_retention_days`，0 表示不自动删除）。
- 写入是 临时文件 → 备份 → 原子替换，损坏时从 `.bak` 恢复。
- Core 与语料管理工具通过命名互斥量串行化读改写，避免互相覆盖。

## 用户可以做什么

设置中心可以开关、暂停、填写允许/排除应用和保留天数。命令行工具用于查看和删除：

```bash
owo_corpus_shell list
```

`list` 每行输出 `id`、记录时间（毫秒）、应用和句子；`delete <id>`、`clear-app <exe>`、`clear-all` 分别删除单条、按应用清空和全部清空。用 `--corpus <路径>` 可指向其他语料文件。

删除会被记住：清空时刻之前缓冲的句子即使之后才写盘，也会被拒绝，不会“删了又回来”。`clear-all` 同时删除备份文件。

## 配置字段（schema 20）

| 字段 | 默认 | 说明 |
| --- | --- | --- |
| `corpus_learning_enabled` | `false` | 总开关 |
| `corpus_learning_paused` | `false` | 暂停记录，保留已有语料 |
| `corpus_learning_allowed_apps` | 空 | 可执行文件名，`;` 分隔；空表示除排除项外全部允许 |
| `corpus_learning_excluded_apps` | `KeePass.exe;KeePassXC.exe;1Password.exe;Bitwarden.exe` | 永不记录 |
| `corpus_retention_days` | `180` | `0`–`3650`，`0` 表示一直保留到手动删除 |
| `corpus_prediction_enabled` | `true` | 下一词联想开关；仍需总开关打开且未暂停 |
| `style_scene_mask` | `7` | 风格档案统计哪些场景：1 聊天、2 工作、4 其他 |
| `style_preferences` | 空 | 用户手写的风格偏好，一行，`~` 转义为 `~~` |
| `tone_reply_enabled` | `false` | 按语气回复开关（L4），默认关闭 |
| `tone_continuation_enabled` | `false` | 整句续写开关（L4），默认关闭 |
| `tone_cloud_model_enabled` | `false` | 是否允许把组装好的请求交给云端模型，默认关闭 |

三个 `tone_*` 字段属于 schema 20，详见 [tone-drafting.md](tone-drafting.md)：它们控制起草和云端发送授权，不改变语料记录本身；输入法生成、预览与回写入口见 [tone-input-flow.md](tone-input-flow.md)。

L5 的反馈记录（`data/tone-feedback.owtf`）**不在语料里**，也没有配置开关：它是用户对草稿的采纳 / 修改 / 拒绝 / 撤销，采用与语料相同的加密与原子写入保护，可单独删除或清空，见 [tone-feedback.md](tone-feedback.md)。

应用名按不区分大小写比较，只匹配文件名（如 `WeChat.exe`），不接收路径。从旧 schema 升级时全部为默认值，学习保持关闭。

## 验证

- `owo.engine.user_corpus`：加密落盘、备份恢复、拒绝规则、按应用清空与清空全部、保留期、跨进程修改、句子切分与上限。
- `owo.config.store`：schema 19 往返、旧版本迁移默认关闭、允许/排除匹配与大小写。
- `owo.protocol.messages`：`corpus_eligible` 往返；敏感输入或非上下文消息伪造该标记会被拒绝。
- `owo.ipc.core_config_contract`：默认关闭时不记录；开启后只记录允许应用里的合格句子，未授权应用、敏感输入和 Agent 回写都不记录；另验证 L2 的联想出现、关闭、恢复与删除后消失。

以上为 Debug 构建下的自动化测试结果；尚未在真实 TSF 宿主中做人工上屏验收。
