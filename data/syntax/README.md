# 中文句法搭配数据

`zh-gsdsimp-r2.17.inc` 和 `zh-gsdsimp-r2.17-roles.inc` 来自 [Universal Dependencies Chinese‑GSDSimp r2.17 的训练集](https://github.com/UniversalDependencies/UD_Chinese-GSDSimp/tree/r2.17)，感谢简体版贡献者 Peng Qi、Koichi Yasuoka 及原始 Chinese-GSD 标注团队。上游 [LICENSE.txt](https://github.com/UniversalDependencies/UD_Chinese-GSDSimp/blob/r2.17/LICENSE.txt) 明确给依存标注采用 [CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/legalcode)，允许按许可条件复制、改编和再分发。本派生搭配库两份文件同样使用 CC BY-SA 4.0，保留署名和修改说明，不改变 OwO 其他代码的许可。库中不分发原始语句，也未使用验证集和测试集；上游没有对原始文本另行授权。

从上游下载 `zh_gsdsimp-ud-train.conllu` 后，运行 `scripts/build-syntax-collocations.ps1 -Conllu <文件>`。脚本验证固定的 SHA-256，将真实依存边按词对、关系和中心词方向聚合，分别统计相邻与非相邻频次；同时从 UPOS/XPOS 与功能关系建立词语成分表。非相邻边只采集同一无句界片段内的主谓、动宾和定中，跨越不超过三个语料词。省略低频模式，不导出句子、语料位置或词序列。

参考：[Chinese-collocation-complexity](https://github.com/iris2hu/Chinese-collocation-complexity) 用作搭配分类与方案调研；未发现明确的再分发许可，本索引未使用其数据。
