# 配置 Schema v16

在 [v14](config-schema-v14.md)（v15 仅新增 `chinese_punctuation_mask`，见 `docs/chinese-punctuation-settings.md`）的基础上增加智能/多模态候选开关：

```text
smart_candidates_enabled=true
expressive_candidates_enabled=true
concept_candidates_enabled=true
```

| 键 | 控制 | 默认 |
| --- | --- | --- |
| `smart_candidates_enabled` | 智能候选：算式/公式、数字中文读法、时长读法、日期结构与星期、符号归一化、日期/标点/括号修复、中英文空格、标点全角化、全角 ASCII 归一化、拆分/合并/重排/占位 | true |
| `expressive_candidates_enabled` | 表情与颜文字候选，以及「文字+表情」组合候选 | true |
| `concept_candidates_enabled` | 概念候选（描述读音 → 术语，含 ≥8 字母前缀提示） | true |

说明：

- 三个开关只影响「附加候选族」。无模式统一编码本身（混合预编辑、声调数字并入、字面候选、拼音解析与纠正）不受开关影响。
- v1～v15 配置按上表默认值读取；新配置与恢复默认均写入 true。下一次保存统一写为 v16。
- v16 配置缺键或取值非 `true`/`false` 会被拒绝（与 v14 新增键的处理一致）。
- 当前设置中心尚未为这三个键提供控件；可以先通过配置文件或配置 shell 修改。Core 在候选请求边界热加载并按配置代数隔离缓存。

已有自动化覆盖：

- `tests/config/config_store_tests.cpp`：v14/v15 → v16 迁移默认值、v16 往返、非法取值与缺键拒绝。
- `tests/ipc/candidate_toggle_tests.cpp`（CTest `owo.ipc.candidate_toggles`）：真实核心服务按配置开启/关闭各候选族，验证 `1234`、`leisi`、`chongfuzhixing` 的候选列表变化，并保持其他族不变。
