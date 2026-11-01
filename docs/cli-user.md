# OwO CLI 使用手册（用户版）

CLI 用于离线排查拼音、候选、分页、模型与插件问题。它不需要安装/注册输入法，
不会连接日常 Core、向应用上屏，或读取焦点窗口。当前支持 Windows 11 x64 开发构建。
它不是发行安装包自带的日常管理工具，需要项目源码和构建环境。

## 1. 准备与首次运行

需要 Visual Studio 2022 Build Tools（MSVC）、CMake 3.25+。运行：

```powershell
Set-Location D:\0program\OwO
.\owo-dev.cmd build
.\owo-dev.cmd version
.\owo-dev.cmd --dev --root D:\0program\OwO\build\cli-quality init
.\owo-dev.cmd --dev --root D:\0program\OwO\build\cli-quality doctor
```

`init` 只运行一次，目录必须全新或为空。后续复用同一目录，不要再次初始化。
源码修改后重新运行 `build`；普通命令不会偷偷重编译。
不要把日常数据目录、安装目录或它们的上级目录用作 `--root`。
同一目录不能并发运行；并行测试请使用多个独立目录。

单次查询不带 `--lexicon` 时，仅用“你好、世界、西安”等小型演示词典。
不能据此评价真实中文覆盖率。真实测试必须明确指定 `.owolx` 词典：

```powershell
.\owo-dev.cmd --dev --root D:\0program\OwO\build\cli-quality input query `
  --text yiyixiawenben `
  --lexicon D:\0program\OwO\build\windows-release\rime-ice-cn-2026.06.30-v2.owolx
```

## 2. 批量评测：推荐入口

```powershell
.\owo-dev.cmd --dev --root D:\0program\OwO\build\cli-quality input batch `
  --file .\examples\cli\pinyin-quality.jsonl `
  --lexicon D:\0program\OwO\build\windows-release\rime-ice-cn-2026.06.30-v2.owolx `
  --report baseline --top-k 10 --max-pages 4 --page-size 7 `
  --check-expanded --repeat 3 --hash-assets --allow-misses
```

完整运行后，在 `--root\reports\baseline` 中生成：

- `summary.md`：简短摘要。
- `summary.json`：完整统计、标签分组、耗时和退出码。
- `cases.jsonl`：运行参数、每条输入、解析路径、实际候选及末尾汇总。
- `failures.jsonl`、`failures.tsv`：失败项，可直接再次批量运行。
- `dataset.jsonl`：本轮已规范化、固定纠错开关的数据集快照。
- `effective.conf`：本轮实际使用的配置快照，不改写根目录 `owo.conf`。

报告名只能是字母、数字、`-`、`_`，不可作为路径；同名报告拒绝覆盖。
未指定名称时自动生成。中断或写入失败时，不把 `.partial` 文件视为完整报告。
只有最终 `summary.json`、`cases.jsonl` 完整且进程退出码符合预期，才算运行完成。

默认任一断言不满足就返回退出码 1，但仍完成整个数据集和报告。
`--allow-misses` 允许质量未命中，不会把未命中改成通过，也不掩盖运行错误、模型失败或显式门槛失败。
示例 `pinyin-quality.jsonl` 含已知失败项，目的是建立真实基线，不承诺全部通过，也不是有代表性的准确率基准。

## 3. 编写数据集

推荐 UTF-8 JSONL：每行一个对象，不是 JSON 数组，不支持注释。可带 UTF-8 BOM。

```jsonl
{"id":"hello","input":"nihao","expected":["你好"],"segmentation":"ni'hao","tags":["exact"]}
{"id":"partial","input":"nihaoshijie","expected":["你好"],"consumed":5,"tags":["partial"]}
{"id":"ambiguous","input":"chnag","expected":["长","常","场"],"tags":["correction"]}
{"id":"context","input":"wang","expected":["妄"],"context":"狂","tags":["context"]}
{"id":"invalid","input":"123","valid":false,"tags":["invalid"]}
```

| 字段 | 含义 |
| --- | --- |
| `input` | 必填，1–256 个可打印 ASCII 字节；支持大小写、撇号、混拼；非法输入也可作为负例 |
| `id` | 可选，唯一标识；省略时用 `line-行号`，建议固定填写方便 A/B 对照 |
| `expected` | 可接受候选数组，命中任一项即可；未填写则不进入候选准确率分母 |
| `consumed` | 目标候选应消耗的原始输入字节数；默认整个 `input` 长度；短词测试必须填写 |
| `segmentation` | 期望的拼音预览，例如 `yi'yi'xia'wen'ben`，与预览精确比较，也检查解析路径是否包含它 |
| `valid` | 期望输入可解析与否，默认 `true`；`false` 时不能设置候选/分割期望 |
| `correction` | 本条是否纠错；省略时使用命令行 `--correction on/off` |
| `context` | 显式提供的合成中文上下文，最多 256 UTF-8 字节；不自动采集真实文本 |
| `previous` | 传给 Core `input` 字段的上一段编码，最多 256 字节；不是上一段已上屏中文 |
| `tags` | 分类数组，用于分别统计全拼、简拼、纠错、句子等质量 |

文字不自动去空格、不忽略标点、不繁简转换，也不把短候选当整句命中。
拒绝未知字段、重复字段/ID、坏 Unicode、错误类型、超限内容；整份校验完成后才启动评测。
最多 10000 条、文件 16 MiB、单行 64 KiB、每条最多 64 个期望词与标签。

TSV 也可用，必须有 `input` 表头。字段名与 JSONL 相同，布尔值为 `true/false`。
`expected`、`tags` 可使用紧凑 JSON 数组（推荐），或用 `|` 分开多个值；空列表示省略。
TSV 不支持多行单元格、CSV 引号语法；以 `#` 开头的行可作注释。自动识别小写 `.tsv`，其他后缀用 `--format` 明确指定。

## 4. 如何阅读指标

| 指标 | 分母与范围 |
| --- | --- |
| `base_top1` | 有候选标签的用例中，基础第一项命中目标的比例 |
| `base_first_page` | 目标在基础首页出现的比例，按实际页长计算 |
| `base_topk` | 目标在扫描结果前 K 项出现的比例 |
| `base_mrr_at_k` | 前 K 项中首个匹配项倒数排名的平均值；未找到为 0 |
| `final_top1` / `final_first_page` | 模型细化后首页的命中率；无模型时等同基础首页 |
| `parse_validity_accuracy` | 所有用例的可解析/不可解析判断是否符合 `valid` |
| `preview_accuracy` | 有分割标签的用例中，实际预览完全匹配的比例 |
| `parse_path_coverage` | 有分割标签的用例中，任一解析路径包含目标分割的比例 |

比例为 0–1；`null` 表示没有适用标签，不是 0 或 100%。
`base_rank` 从 1 开始，同时匹配文字和消耗字节；未找到为 `null`。
`base_scan_truncated=true` 表示页数预算用完仍有候选，不能断言目标在整个词库中不存在。
Top-K 必须满足 `max-pages × page-size >= top-k`。扫描范围外不计命中。

默认单条用例要求：可解析判断正确、填写的分割预览/路径正确、目标出现在基础 Top-K。
启用模型时，候选断言改为目标出现在模型细化后的首页；分页/展开核对检查当前展示快照，含已应用的首页细化。
展开核对比较已扫描范围内（最多 256 项）的文字、消耗字节和分区标识，不证明鼠标、动画或实际窗口正确。

## 5. 常用参数与质量门槛

| 参数 | 默认 / 用途 |
| --- | --- |
| `--mode both` | `both/candidates/parse`；前两者检查完整候选链路，`parse` 不启动 Core，只检查拼音解析 |
| `--format auto` | `auto/jsonl/tsv` |
| `--correction on` | 全局默认纠错，可被每条用例覆盖 |
| `--paths 32` | 解析诊断路径上限，1–128；不改变 Core 内部生产解析预算 |
| `--page-size N` | 本次页长 4–9，省略则采用隔离配置 |
| `--partition on/off` | 本次分区开关，省略则采用隔离配置 |
| `--top-k 10` / `--max-pages 4` | 基础候选质量排名范围 / 最多查询页数，页数 1–101 |
| `--typing incremental` | 先逐字请求前缀，再评估最终输入；默认 `whole` |
| `--repeat 3` | 首次最终请求后再查两次，记录重复请求缓存耗时；范围 1–100 |
| `--check-expanded` | 核对基础分页与展开候选映射 |
| `--timeout-ms 2500` | 单次 IPC / 细化等待预算，1–30000 毫秒 |
| `--hash-assets` | 记录 CLI、数据集、词典和模型文件 SHA-256；大模型哈希有额外启动耗时 |
| `--use-learning` | 显式读取隔离根目录中的学习记录，默认不读；不会写学习反馈 |
| `--min-top1` / `--min-first-page` | 首项/首页最低比例 0–1；模型启用时针对最终首页 |
| `--min-topk` | 基础 Top-K 最低比例 0–1，始终不是模型深分页指标 |
| `--max-p95-ms` | 基础最终输入请求的 p95 耗时门槛，不包含模型细化、重复请求或 UI 渲染 |

需要容许部分未命中但限制整体质量时，搭配 `--allow-misses` 使用门槛。
没有候选标签时，候选质量门槛不能通过。不要把 `--allow-misses` 的退出码 0 当作每条用例正确。

耗时分别记录：独立解析、基础最终请求、模型完成后的请求、增量前缀、重复查询。
首个请求可能包含管道首次连接；不称为系统冷启动。测量不包含真实按键、TSF 上屏及渲染。
不要仅凭一次运行的 p95 判定微小性能回归。

## 6. 测试智能排序

```powershell
.\owo-dev.cmd --dev --root D:\0program\OwO\build\cli-quality input batch `
  --file .\examples\cli\pinyin-quality.jsonl --report libime `
  --lexicon D:\0program\OwO\build\windows-release\rime-ice-cn-2026.06.30-v2.owolx `
  --model-backend libime --run-model `
  --bridge D:\TrustedRuntime\owo_libime_bridge.dll --model D:\TrustedModel\zh_CN.lm `
  --model-timeout-ms 100 --allow-misses
```

替换为实际存在且可信的桥接库和模型。`--run-model` 是明确加载原生代码的许可，
不是下载/安装或授权第三方插件的许可。每轮只创建自身私有 Core/ModelHost 管道，
使用生产异步细化与分区约束，不接入日常 ModelHost。
`--model-backend mock` 只验证链路，不代表中文质量。

模型质量只统计最终首页，基础 Top-K 另行统计。后续页不是整段模型全局重排的延续。
Core 会复用当前输入/上下文已细化的展示快照，因此“基础”字段指本次首次协议响应，
命中快照时可能已经含此前模型顺序。严格的无模型基线请另跑 `--model-backend base`。
展开核对在首页已细化时，以实际展示的最终首页及其后续页为准。
没有请求模型、请求后成功应用、失败/超时分别计数；失败不会算作成功的基础降级。

## 7. 重试失败项与比较两个版本

```powershell
.\owo-dev.cmd --dev --root D:\0program\OwO\build\cli-quality input batch `
  --file D:\0program\OwO\build\cli-quality\reports\baseline\failures.jsonl `
  --lexicon D:\0program\OwO\build\windows-release\rime-ice-cn-2026.06.30-v2.owolx `
  --report retry --allow-misses

.\owo-dev.cmd compare `
  -Baseline D:\0program\OwO\build\cli-quality\reports\baseline `
  -Candidate D:\0program\OwO\build\cli-quality\reports\after `
  -FailOnRegression
```

对照必须用同一完整数据集、相同评测预算、相同后端。仅拿失败子集与原完整集比较会被拒绝。
对照会输出命中/排名改善、退步及汇总，配置、CLI 和词典身份一并保留供检查。
退步包括从命中变未命中或排名变差。速度不在这个自动退步判定中。
它不证明模型、词典或学习快照完全一致；严谨实验应启用哈希，并人工核对这些条件。

## 8. 其他 CLI 功能

- `input parse/query/bench`：解析路径、单条真实候选、重复请求基准。
- `input replay/repl`：回放或交互输入，支持 `query/type/backspace/page/expand/select/clear/correct`，及 `expect/expect-input/expect-commit` 断言。
- `config show/import`：读取/导入隔离配置；不会修改日常设置。
- `plugin inspect/install/list/audit/activate/deactivate/revoke/uninstall/schema/settings-read/invoke`：插件生命周期与服务调试，不绕过权限或安装确认。
- `scheme list/query`：已明确授权的插件输入方案发现、分页和模拟选择。
- `resource validate`：主题、Unicode 符号资源校验，不验证渲染效果。
- `protocol validate`：Core、Plugin、Agent 请求/响应生产解码校验。
- `agent sample-request/filter`：合成 Agent 请求与敏感信息过滤，不连接真实 Agent。
- `model rank`：独立模型诊断，与批量的生产 Core 细化指标不同。
- `test --suite cli/engine/plugin/scheme/agent`：仅在明确执行此命令时运行对应专项测试。

完整单条/插件/Agent 参数和示例见[开发 CLI 参考](developer-cli.md)，
自动化输出规范和无歧义调用规则见[AI 版手册](cli-ai.md)。

## 9. 隔离、隐私与错误处理

批量评测不提交词语，不更改学习记录。已有用户字典只会从显式隔离根目录
`dictionaries` 中读取，不继承外部 `OWO_DICTIONARY_STORE_PATH` 指向的日常字典。
它仍会受到隔离根目录内已有独立字典的影响；干净基线请使用新根目录。
需要测试学习时，先通过隔离 `input replay --learn` 制造记录，再用 `--use-learning` 读取。

报告含输入、期望词、显式上下文、绝对路径和可用资产信息，不自动脱敏。
请只使用合成/已获授权的数据；备份、上传 CI 日志前检查隐私。
CLI 不下载或执行数据集内容，不安装插件，也不将候选作为系统命令。

退出码：0 成功；1 质量门槛/断言失败；2 参数或数据文件错误；3 隔离路径/会话锁/
授权门禁拒绝；4 Core、词典或 IPC 运行故障；5 模型/插件初始化或服务失败；6 工具内部异常。
批量中已收到但无法应用的模型结果单独计 `model_failed`，返回 1；不要忽略该计数。
退出非零先查看结构化错误和报告，不要自动放宽期望词、候选上限或权限来“刷通过”。
