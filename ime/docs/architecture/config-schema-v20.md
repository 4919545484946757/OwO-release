# 配置 schema v20

在 [v19](config-schema-v19.md) 的基础上，v20 增加按语气回复与续写（L4）的三个开关：

| 字段 | 类型 | 默认 | 约束 |
| --- | --- | --- | --- |
| `tone_reply_enabled` | bool | `false` | 打开后可对选中的消息组装按语气的回复请求 |
| `tone_continuation_enabled` | bool | `false` | 打开后可对接用户写下的开头组装续写请求 |
| `tone_cloud_model_enabled` | bool | `false` | 是否允许把已组装的请求交给云端模型；关闭时只能本机使用 |

三个字段都是严格的：缺失、重复或不是 `true`/`false` 都会让整份配置解析失败，不会静默取默认值。字段顺序按写入顺序排在 `style_preferences` 与 `model_ranking_enabled` 之间，`tone_*` 三行连续。

默认全关是刻意的：L4 会读取授权语料并把当前消息写进请求，与语料学习本身一样属于需要明确同意的行为。从 v19 及更早版本迁移时只补默认值（`false`），不改变任何已有字段，也不因为语料学习已开启而自动打开。

`tone_cloud_model_enabled` 是语气模型的云端发送授权，不代表请求已经发出。输入法后台线程已接入 WinHTTP 纯文本生成，默认全关；只有学习、应用、场景和对应起草开关允许时才可发起。关闭云端开关时仅允许显式配置的字面回环模型；远端要求 HTTPS 和环境变量凭据。普通 ModelHost 仍只做候选排序。操作、传输与验收范围见 [tone-input-flow.md](../tone-input-flow.md)。

`set-all` 的完整形态因此扩展到 50 个参数，末尾追加 `<tone-reply-enabled> <tone-continuation-enabled> <tone-cloud-model-enabled>`；原有长度前缀保持兼容。

L5 不新增 schema 字段：新增反馈复用学习总开关、暂停开关、起草开关与场景掩码；关闭学习后仍允许查看、删除和清空反馈。反馈存储独立于语料，详见 [tone-feedback.md](../tone-feedback.md)。

相关实现：`include/owo/config/config_store.h`、`src/config/config_store.cpp`；起草逻辑见 `docs/tone-drafting.md`。
