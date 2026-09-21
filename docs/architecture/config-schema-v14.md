# 配置 Schema v14

在 [v13](config-schema-v13.md) 的基础上增加用户词拼音补全设置：

```text
user_lexicon_completion_enabled=true
user_lexicon_completion_max_syllables=1
```

`user_lexicon_completion_enabled` 控制是否根据未输入完整的拼音前缀召回本机用户词；关闭后，完整编码的用户词匹配不受影响。
`user_lexicon_completion_max_syllables` 允许值为 0～8：0 只补完当前音节，1 允许再补一个音节，数值越大越可能提前显示长用户词。默认开启、上限为 1。

v1～v13 配置按上述默认值读取；下一次保存统一写为 v14。Core 在候选请求边界热加载，并按配置代数隔离候选缓存。
