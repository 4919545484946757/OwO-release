# 配置 Schema v13

Schema v13 在 v12 全部字段基础上增加：

```text
input_scheme=full_pinyin
```

允许值：

- `full_pinyin`：默认全拼方案，支持纠错、补全、简拼和混拼。
- `exact_code`：实验性精确编码方案，只接受 ASCII 字母和 `'` 分段，只执行精确词典查询。

v1～v12 配置继续读取，并自动使用 `full_pinyin`。保存后统一写为 v13。Core 在请求边界热加载方案；配置代数隔离候选缓存，不需要重启 TSF。
