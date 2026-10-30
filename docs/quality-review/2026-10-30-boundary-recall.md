# 分词边界候选召回修复

## 问题与范围

`nipingchangyong` 解析为 `ni'ping'chang'yong`，但基础频率与分段代价偏向
“倪萍 + 常用”。此前模型池过滤掉了需要更多分段的另一种解释，模型看不到“你平常用”。
同一文字去重后还可能保留“你 + 平 + 常用”的分词，不能仅据此认定它没有“平常”词证据。

修复只为同一精确读音、不同词边界保留有字典交叉词证据的代表，最多 8 种结构。
基础模式最多补回一个可选解释，不扩大两条句子上限；模型模式允许这些解释参与原有排序。
不增加 Beam，不修改词典，不按输入串或汉字写死答案，不放开纯拆词/同结构同音排列。

## 针对性验证

- `tests/data/common-adverb-segmentation.jsonl`：6 例，包括显式分隔、不同主语、短串控制。
  整串与逐字输入、基础与 libime 模式均无缺失；基础首屏覆盖率 6/6，首选 3/6；
  libime 最终首选 6/6。分页与展开一致性均通过。
- `tests/data/suffix-ranking-regression.jsonl`：8 例，基础与 libime 首选均 8/8，展开核对通过。
- `owo.engine.candidates`、`owo.engine.phrase_chains` 通过。
- `owo.engine.dictionary_bridges` 有已有失败：断言要求“按有限度”的 coherence 为 0，
  当前实际为 6000；暂时还原生成器至 HEAD 后仍复现。未修改该断言或评分规则。

报告目录：`build/usual-segmentation/reports/`；未执行全量自检。
这些结果只说明上述定向用例，不代表所有中文语句的首选正确率。
