# 配置 schema v19

在 [v16](config-schema-v16.md) 的基础上，v17 增加了授权语料学习字段、v18 增加了下一词联想开关，v19 增加风格档案（L3）的两个字段：

| 字段 | 类型 | 默认 | 约束 |
| --- | --- | --- | --- |
| `style_scene_mask` | uint32 | `7` | 只允许最低 3 位：1 聊天、2 工作、4 其他；0 表示全部关闭 |
| `style_preferences` | 单行文本 | 空 | UTF-8，≤512 字节，不含换行；`~` 转义为 `~~` |

`style_scene_mask` 的位序与 `owo::engine::StyleScene` 一致，`owo.engine.style_profile` 用 `static_assert` 固定这两处；设置中心“我的风格”页的三个开关直接读写这三位。

`style_preferences` 是用户手写的一行偏好（例如“简短、少用感叹号”）。它不会被学习过程改写；`~` 转义保证 `key=value` 文件里不会出现歧义，悬空的 `~` 会被解析拒绝。设置中心保存前会把回车和换行替换为空格，避免用户在多行输入框里粘进内容后整份配置保存失败。

v17 之前（含 v16）的配置文件迁移后：语料学习默认关闭，`corpus_prediction_enabled`、`style_scene_mask` 取默认值，`style_preferences` 为空。迁移只补默认值，不改变已有字段。

相关实现：`include/owo/config/config_store.h`、`src/config/config_store.cpp`；风格统计见 `docs/style-profile.md`。
