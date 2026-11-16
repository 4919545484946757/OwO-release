# 中文句法搭配数据

`zh-gsdsimp-r2.17.inc` 来自 [Universal Dependencies Chinese‑GSDSimp r2.17 的训练集](https://github.com/UniversalDependencies/UD_Chinese-GSDSimp/tree/r2.17)，感谢简体版贡献者 Peng Qi、Koichi Yasuoka 及原始 Chinese-GSD 标注团队。该派生索引按 [CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/) 分发；这一许可适用于数据文件，不改变 OwO 其他代码的许可。这里只保留标注产生的相邻词对及聚合频次，不包含原始句子；验证集和测试集未参与生成。上游注明其授权适用于标注，不为原始文本另行授权。

从上游下载 `zh_gsdsimp-ud-train.conllu` 后，运行 `scripts/build-syntax-collocations.ps1 -Conllu <文件>`。脚本验证固定的 SHA-256，按表面词序聚合主语、宾语、修饰、复合、补语和量词关系；过滤低频记录，仅保留最常见的关系类别与其频次。OwO 使用这些依存证据给解码中的相邻分词小幅加分，不把未见词对当作错误，也不运行完整的句法树解析器。

参考：[Chinese-collocation-complexity](https://github.com/iris2hu/Chinese-collocation-complexity) 用作搭配分类与方案调研；未发现明确的再分发许可，本索引未使用其数据。
