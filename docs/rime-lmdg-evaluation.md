# RIME‑LMDG 离线对照评测

目的：判断 `zhifujiefei → 制服劫匪` 等同音替换问题是否能由语言模型改善，并区分模型收益与词库差异。本实验运行独立的 librime 会话，没有安装 Rime 输入法，没有改动 OwO 的运行版本、设置、0 号候选或学习数据，也没有分发模型。

## 方法与边界

- 固定 [librime 1.17.0 Windows MSVC x64](https://github.com/rime/librime/releases/expanded_assets/1.17.0)（内含 octagram），使用简体 LTS `.gram`。该次 [LTS 模型资产](https://github.com/amzxyz/rime-wanxiang-data/releases/expanded_assets/LTS) 为 407,160,876 字节，SHA‑256 `d4f64356231764ff27b79c9b0db79cd9d665013464cf5544e2fbd9a71e0f1c5a`；LTS 标签可更新，复测时不能将文件名当作版本标识。
- 两组词库分别是 OwO 已缓存的雾凇源词典（目录 `rime-ice-2026.06.30`）和 [万象 Base v18.1.11](https://github.com/amzxyz/rime-wanxiang/releases/expanded_assets/v18.1.11) 的字典及全拼编码规则。用同一最小全拼方案，不加载 Lua、第三方输入法 UI 或用户词典。对于同一词库，只切换语法配置，其余参数相同（首屏 7 项，同音召回上限 8，句候选上限 2）。两套词库的编码规则不同，横向比较是「词库＋编码」对照，不是纯词频对照。
- 28 条样例：原句法 A/B 集的 24 条，外加 `制服劫匪`、`制服小偷`、`制服暴徒`、`支付费用`。每组先热身一轮，再跑 11 轮，每轮轮转顺序。P50/P95 计量 `set_input` 到取得首屏，包含一次完整拼音输入查询，不包含逐键 UI、OwO IPC、模型进程启动或安装耗时；不与 OwO 端到端耗时直接比较。排名指标以精确候选文字为准，召回范围截至第 32 项。
- 开启组必须同时满足部署配置、octagram 模块以及 `gram_db.cc` 的成功加载日志。脚本记录词库及模型校验和，并验证同一输入的目标排名在重复轮次中一致。

## 复现

先在 VS 2022 x64 开发者 PowerShell 中，使用指定版本 librime 包内的 `dist/include` 编译评测器。随后使用 PowerShell 7：

```powershell
cl /nologo /O2 /EHsc /std:c++20 /utf-8 /I<librime-dist-include> /Fo<build-obj> /Fe<build-exe> benchmarks/rime_lmdg_ab.cpp
pwsh -NoProfile -File scripts/run-rime-lmdg-ab.ps1 `
  -Harness <build-exe> -RimeDll <rime.dll> `
  -IceDirectory <雾凇词典目录> -WanxiangDirectory <万象Base解包目录> `
  -Model <wanxiang-lts-zh-hans.gram> -OutputDirectory <新评测目录>
```

用新目录追加 `-ReverseOrder` 可以反转四组执行顺序。输出 `summary.json`、`assets.json`、每组 `cases.json`、原始 `raw.jsonl` 和加载日志；均为本地实验产物。

## 首轮结果

| 词库 | 模型 | 制服劫匪 | Top 1 | 首屏 Top 7 | 前 32 | 热态 P50 / P95 |
|---|---|---:|---:|---:|---:|---:|
| 雾凇 | 关 | 未召回 | 23/28 | 23/28 | 23/28 | 0.072 / 0.206 ms |
| 雾凇 | 开 | 第 1 | 27/28 | 27/28 | 27/28 | 0.088 / 0.255 ms |
| 万象 | 关 | 未召回 | 23/28 | 24/28 | 24/28 | 0.215 / 0.646 ms |
| 万象 | 开 | 第 1 | 27/28 | 28/28 | 28/28 | 0.267 / 0.795 ms |

反序复测的各组排名和质量指标一致。两轮中，雾凇组热态 P95 为关闭 0.206–0.252 ms、开启 0.253–0.255 ms；万象组为关闭 0.646–0.664 ms、开启 0.750–0.795 ms。这是 librime 单次完整输入查询的数据，不代表模型接入 OwO 后的开销。

雾凇组还将「学生证在参加」「支付小偷」「支付宝突」分别改为「学生正在参加」「制服小偷」「制服暴徒」。万象＋模型组将「制服暴徒」召回到第 2 名，首选仍是「支付宝图」。「支付费用」在四组均为首选。样本小且包含有意选择的错例，不应把上述命中率当作通用输入准确率。

## OwO Core 原生实验接入

OwO 现在可在既有全拼成句搜索中显式启用该资产，不使用 librime 的词库或解码器。Windows Core 只读内存映射简体 LTS `.gram`；格式和查询参照 [librime-octagram](https://github.com/lotem/librime-octagram) 与 [Darts-clone](https://github.com/s-yata/darts-clone) 的公开实现。不安装 DLL、不访问网络，不需要载入第三方可执行组件。

- 在搜索淘汰和句子配额截断前，为既有词典的 `2–4 字 + 2–4 字`、总长不超过 6 字的精确搭配加分。不引入词库外的词，不因模型缺词扣分；不影响单字输入、纠错／简拼路径或 0 号候选。
- 每次生成最多 4,096 次搭配查询，累积查询约 3 ms 后不再开启新查询。单次冷缺页延迟不受这项软预算约束；第一轮不将低配机冷启动纳入性能结论。文件缺失、截断或格式不兼容时，Core 日志提示后继续用普通候选。
- 若首选是有强 LMDG 证据的完整精确候选，该轮不再由 libime 后置重排覆盖它；其他轮次仍按原逻辑调用 libime。调试与性能页增加「LMDG 搭配查询（实验）」；诊断只记录耗时、次数、命中和是否跳过后置重排。

仅开发启动显式启用，不改变设置默认值，不自动下载或分发模型：

```powershell
./scripts/start-dev.ps1 -BuildDirectory <本次构建的x64-Release> `
  -X86BuildDirectory <x86-Release> -LexiconPath <现有OwO词典> `
  -LmdgModelPath <本地简体LTS.gram绝对路径> -RestartCore
```

`Core` 启动日志中的 `lmdg_mapped` 表示已加载，`experimental_lmdg_unavailable` 表示已降级。不传 `-LmdgModelPath` 并重启可关闭。也可使用隔离开发 CLI 的 `input query` 或 `input batch`，额外传 `--lexicon` 与 `--lmdg-model`。配对评测合并 `tests/engine/syntax_collocation_ab.jsonl` 和 `tests/engine/rime_lmdg_ab.jsonl`；同一词库运行时不传模型参数作为对照，建议附加 `--check-expanded`。

28 条样例在 OwO Core 中对照：关闭 → 开启，首选 `21/28 → 25/28`，首屏 `23/28 → 26/28`，未发现分页与展开不一致。`制服劫匪`、`制服小偷`、`制服暴徒` 均从未召回升到首选，`支付费用` 仍为首选。另有一例「学生正在参加」由第二升为第一。基础请求 P95 在本机小样本上未观察到稳定增加，不能据此推定低配机或冷缺页成本。

另用本机 libime 模型和原有句法开关对四条同音专项进行共存复测：三条「制服…」的首选在候选更新后仍保留，未发出覆盖它们的 libime 请求；「支付费用」仍由 libime 后置处理，结果仍为首选。四条展开结果均与分页一致。

公开分发前对固定 `.gram` 资产单独确认许可，不能从仓库的 [许可范围](https://raw.githubusercontent.com/amzxyz/rime-wanxiang-data/wanxiang/LICENSE-SCOPE.md) 自动推断模型二进制的再分发授权。
