# 无模式统一编码、智能候选与多模态输出（引擎切片 v1）

状态：已实现引擎层并提供实时 IPC 契约验证；TSF 按键路由与多模态素材插入为后续工作。
日期：2026-10-19
相关代码：`src/engine/unified_input.cpp`、`src/engine/smart_candidates.cpp`、`src/engine/expressive_candidates.cpp`、`src/ipc/named_pipe.cpp`

## 1. 三个方向与本次落点

| 方向 | 本次实现 | 尚未实现 |
| --- | --- | --- |
| 无模式统一编码 | 混合预编辑（拼音+英文+数字+符号）分型；声调数字折叠；混合内容仍在同一候选栏给出「照原样保留」与「主拼音段解码」候选 | TSF 把数字/符号/OEM 键纳入预编辑；插件输入方案的统一字母表协商 |
| 智能候选 | 算式结果（结构候选）、日期/标点/括号纠错、日期区间逻辑顺序、中英文/数字间自动空格、拆分/合并/重排/缺口占位、概念候选（描述拼音 → 术语） | 语义概念候选的本地向量/模型索引、数值单位一致性 |
| 多模态输出 | 表情与颜文字候选（本地内置表，整段预编辑匹配）；`ExpressiveAsset` 描述图片/动图/语音素材，供宿主渲染或插入 | 素材生成、图片/动图/语音的实际插入通道（剪贴板或富文本嵌入） |

设计原则：**不取代传统候选**。三组候选都以增量方式并入现有排序，分数低于词典词；只有拼音引擎完全无法覆盖的预编辑内容才会出现「照原样保留」候选。

## 2. 预编辑归属规则（无模式统一编码）

`owo::engine::plan_unified_input` 把预编辑切分为带区间与类型的片段：

| 片段 | 判定 | 示例 |
| --- | --- | --- |
| `pinyin` | 字母段（可含 `'` 与词内声调数字）能被拼音解析器接受（含纠错/缩写恢复路径） | `nihao`、`ni3hao3`、`wsm`、`ni'hao` |
| `latin` | 含大写字母的字母段，或解析器完全无法接受的字母段 | `API`、`x` |
| `number` | 数字、内部小数点与百分号 | `3.14`、`100%` |
| `symbol` | 其余非空白字符 | `=`、`.`、`+`、`_` |
| `separator` | 空白 | 空格 |

归属决策：

- 预编辑全部落在小写拼音字母表（`a-z` 与 `'`）内时，**拼音引擎独占该输入**：纠错（`niaho`）、缩写（`wsm`）与普通拼音一样只走原有路径，不注入字面或结构候选。这保持了既有候选契约（例如 `quan'loi` 仍返回空候选）。
- 出现拼音字母表之外的字节（数字、符号、空白、大写、CJK）或折叠了声调数字时，才进入统一候选：解码主拼音段、保留其余片段、并追加字面候选。
- 大写字母段是明确的英文/缩写意图：`API` 不会先被当成 `a+pi`。含大写字母的预编辑同时把字面候选提到 `kUnifiedLatinIntentScore = 5000`（高于普通字面候选的 450），因此在弱词典候选之上仍以用户所写为准；小写混合输入仍把字面候选排在词典词之后。

声调数字规则：`1-5` 紧跟在字母段之后，且后面还是字母或已到结尾时，按声调处理并从读音中去掉；否则按数字字面处理（`woaini123` 保留 `123`）。

## 3. 统一候选生成

`owo::engine::generate_unified_candidates(schema, lexicon, raw, ...)` 产出整段预编辑的候选：

1. 若主拼音段可严格解析（`exact` 或未完成为尾音节），用现有 `CandidateGenerator` 解码，并把前缀/后缀字面文本拼回候选（`baidu.com` → `百度.com`）。
2. 追加「照原样保留」候选（默认分数 `kUnifiedLiteralScore = 450`，低于词典词，保证中文候选优先；含大写字母的显式英文输入改用 `kUnifiedLatinIntentScore = 5000`）。
3. 追加智能候选与表情候选，按分数插入而非覆盖原顺序。

`merge_unified_candidates` 把增量并入已有的生成器结果：重复文本去重，其余按分数插入到第一个更低分候选之前，从而不改变既有候选的相对顺序。

**开销约束**（由 `benchmarks/unified_candidate_benchmark.cpp` 守护）：

- 分型不调用完整解析器，只在片段为拼音字母表内容时调用 `FullPinyinSchema::is_plausible_reading`（有界线性可达性检查，忽略缩写与纠错），因此 `hello`、`world` 这类英文词不会触发缩写搜索。
- 解码只在混合/声调输入且主拼音段读音 ≤ `kUnifiedDecodeMaximumReadingBytes`（32 字节）时进行，且始终以关闭纠错的解析进行；超过长度时只提供字面候选，避免与运行时的整句解析重复并放大长输入延迟。
- 统一候选只在核心服务缓存未命中时计算（`named_pipe.cpp`），因此单个预编辑字符串最多多付一次。

## 4. 智能候选规则

| 提供者 | 触发 | 输出 | 分数 |
| --- | --- | --- | --- |
| `generate_math_result_candidates` | 整段是 ASCII 算式（`+ - * / % ^ ( ) .`，可带尾随 `=` 或表格风格前缀 `=`，至少一个运算符，除数非零，长度 ≤ 64 字节；整段日期形状除外） | 结果值；带 `=` 时输出 `表达式=结果`（前缀风格为 `=表达式=结果`，如 `=1+2` → `=1+2=3`） | 5200 |
| `generate_date_structure_candidates` | 整段是一个合法日期（`2026-10-19`、`2026/1/5`、`2026年1月5日`、8 位紧凑 `20261019`）、整段日期区间（连接符 `到`/`至`/`~`/`～`/`—`/`-`）或 `H:MM` 时间 | 中文日期、日期+星期、ISO 日期、紧凑日期展开（`20261019` → `2026-10-19` / `2026年10月19日 星期一`）、区间本地化（`2026-10-19~2026-10-25` → `2026年10月19日 至 2026年10月25日`）、补零时间（`9:5` → `09:05`） | 2700 / 2600 |
| `generate_text_lint_candidates` | 不可能的日期（越界月进位、越界日收敛到当月最后一天，保持原分隔符族） | 更正后的整段文本 | 3000 |
| 同上 | 日期区间顺序矛盾：两个合法日期之间只有连接符（`到`/`至`/`~`/`～`/`—`/`-` 与空白）且后者早于前者 | 交换两端的整段文本 | 2850 |
| 同上 | 重复中文标点、未闭合引号/括号、紧邻中文的 ASCII 标点（`, . ? ! : ; ( ) "`） | 更正后的整段文本（标点转全角、引号成对） | 3000 |
| 同上 | 中文与英文/数字之间缺少空格（CJK 标点不参与；`年月日号时分秒` 等日期/时钟单位不插入空格） | 补空格后的整段文本 | 2200 |
| 同上 | 全角 ASCII（字母/数字/代码符号）与全角空格 | 半角等价文本（全角中文标点保持原样） | 2300 |
| `generate_text_shape_candidates` | camelCase/snake_case/kebab-case 拆分、空格分词合并为紧凑或下划线形式、逗号列表重排、结尾 `__` 缺口占位 | 变体文本或 `【待补】` 占位 | 2400–2700 |
| `generate_number_reading_candidates` | 整段是 ASCII 十进制数（可带小数或尾随 `%`，整数 ≤ 16 位，单数字除外） | 中文小写读法与（整数时）大写金额读法；`12.5` → 十二点五、`12%` → 百分之十二 | 2650 / 2550 |
| `generate_arabic_number_candidates` | 整段是中文数字（含大写/两/〇、`点` 小数、可选 `元/圆/整` 后缀） | 阿拉伯数字；`一千二百三十四` → 1234、`壹仟贰佰叁拾肆元整` → 1234、`十二点五` → 12.5；连续数字/重复单位/超过 1e16 拒绝 | 2600 |
| `generate_symbol_candidates` | 整段含 ASCII 箭头/比较符/省略号（`->`、`<=`、`>=`、`!=`、`=>`、`<-`、`-->`、`...`） | Unicode 等价符号（`x!=y` → `x≠y`、`a->b` → `a→b`）；`--help` 这类命令行参数不变 | 2500 |
| `generate_duration_candidates` | 整段是数字+时长单位 `h`/`min`/`s`（大小写敏感、单位后不能再有字符） | 中文时长（`90min` → 1小时30分、`1.5h` → 1小时30分、`75s` → 1分15秒） | 2450 |
| `generate_concept_candidates` | 整段预编辑的拼音等于术语表里的某条描述读音；无精确匹配且长度 ≥ 8 个字母时按描述前缀提示 | 术语本身（如 `chongfuzhixing` → 幂等，`chongfuzh` 边输入边提示） | 3400 / 3300 |
| `generate_combination_candidates` | 整段预编辑既是可解码的单一拼音段，又命中表情键 | 解码文本 + 表情/颜文字（`leisi` → `累死😩`） | 2900 |

所有规则确定性、离线、只看当前预编辑，长度上限 `kSmartMaximumInputBytes = 64`。

### 4.1 概念候选（从描述找术语）

- 数据模型：`ConceptEntry{term, term_reading, cue_readings, cue_text}`；匹配键是整段预编辑去掉非字母字符后的英文小写（`concept_lookup_key`），**精确匹配**优先。
- 索引：构造时建立 `cue → 条目` 哈希表与按 cue 排序的 `(cue, 条目)` 数组；精确匹配 O(1)、前缀匹配 O(log n + k)，项目术语表即使有上万条也不会让每次候选请求线性扫描。10,000 条表的实测（含测试循环里的字符串构造）精确约 2.0 µs、前缀约 5.1 µs/次。
- 渐进提示：没有精确匹配、且键长 ≥ `kConceptPrefixMinimumLetters`（8 个字母）时，按「描述前缀」给出术语（分数 3300，低于精确匹配 3400）；更短的前缀不提示，避免普通词被装饰。命中精确描述后不再追加前缀候选。
- 键的边界：匹配键只由字母、空格和单引号组成；预编辑里出现数字或其他符号（如 `chongfuzhixing123`、`chongfuzhixing!`）时视为不同 token，不返回概念候选，避免把带版本号/标点的输入改写成术语。
- 内置表（`builtin_concept_table`）只是 12 条常见工程术语的起步示例；术语表解析顺序为「`--concept-glossary` 参数 → 环境变量 `OWO_CONCEPT_GLOSSARY` → 数据目录默认文件 `<LOCALAPPDATA>/OwO/InputMethod/data/concept-glossary.tsv`」，找到的那一张**替换**起步表。
- 术语表格式：`term<TAB>reading<TAB>cue1,cue2[,cue3][<TAB>说明]`；示例见 `docs/architecture/concept-glossary.example.tsv`。
- 容错：加载失败返回错误而不是半张表；核心服务记录 `concept_glossary_load_failed` / `concept_glossary_rejected` 并继续使用内置表启动（进程不会被术语表问题拖垮）。
- 交付与隐私：术语表是本地文件；不联网、不做语义推断。真正的“含义描述 → 术语”语义检索仍待本地向量/模型索引（见第 9 章）。
- 已实测（真实进程 + 命名管道）：`--concept-glossary` 载入自定义表后 `maoyancue` → `冒烟用例`；环境变量 `OWO_CONCEPT_GLOSSARY` 与数据目录默认文件同样生效；坏文件时核心服务照常启动且内置表仍生效（`chongfuzhixing` → 幂等）。
- 健壮性：统一入口对截断 UTF-8、孤立续字节、超长输入（>128 字节）与标点串逐一验证「不越界、候选不超上限、`consumed_input_bytes` 等于预编辑长度」。

## 5. 多模态输出

- `ExpressiveAssetSource::lookup` 以「规范化整段预编辑」（小写、去空白）为键，返回素材描述；内置表当前 53 条素材、覆盖 25 组常见意图（情绪：`leisi`/`kaixin`/`shengqi`/`wuyu`/`nanguo`；社交：`xiexie`/`buhaoyisi`/`ganbei`/`shengrikuaile`/`wanan`；状态：`moyu`/`bailan`/`pofang`/`liekai`/`hungry`），并同时支持英文键（`tired`、`lol`、`thanks`）与中文键（`无语`）。
- 表情与颜文字直接就是可上屏文本，因此按普通候选返回（分数 3000 / 2600），选择后替换整段预编辑。
- 多模态组合：当整段预编辑既是一个可解码的拼音段、又命中表情键时，额外给出「解码文本 + 表情」候选（`leisi` → `累死😩`，分数 2900）。组合只在文字与素材描述同一意图时出现；没有词典解码或拼音段带前后缀时不生成。
- `ExpressiveAssetKind` 中的 `image` / `animation` / `audio` 只描述素材：`Candidate` 目前仍是纯文本通道，媒体素材由宿主（工作台、桌宠或后续协议版本）渲染与插入；引擎不内联 URL。
- 隐私：内置表编译进引擎，无网络、无文件、无剪贴板访问；`local_only` 固定为 `true`。远程生成属于用户显式开启的独立能力，不在本切片内。

## 6. 核心服务接线

`run_core_server` 在词典候选生成之后、分区布局之前调用统一候选：

- 仅在缓存未命中、拼音方案、输入 ≤ `kUnifiedInputMaximumBytes`（128 字节）且未取消时计算。
- 合并后 `full_input_bytes` 取实际预编辑长度，保证候选分组（句子/词/字）使用的消费长度正确。
- 候选预览用的音节分段跳过没有 `source_segments` 的统一候选，仍取词典候选的分段。
- 配置开关：`smart_candidates_enabled` / `expressive_candidates_enabled` / `concept_candidates_enabled`（配置 schema v16，默认全开）分别控制智能候选、表情+组合、概念候选；无模式统一编码本身（混合预编辑、声调数字、字面候选）不受开关影响。设置中心暂无控件，可先用配置文件或配置 shell。详见 `config-schema-v16.md`。
- 可观测性：`generate_unified_candidates` 通过 `UnifiedCandidateBreakdown` 返回各来源实际加入的数量（decoded/literal/rules/expressive/combinations/concepts），核心服务把它写进 `candidate_generated` 日志的 `unified` 字段，并附带 `concept_glossary_entries`。真实进程实测：`leisi` → expressive 2、`1234` → literal 1 + rules 2、`chongfuzhixing` → concepts 1、`=1+2` → literal 1 + rules 1、纯拼音 `nihao` → 全 0（仍由词典路径独占）。
- 协议边界：ADR-0025 规定 `candidate_consumed` 是候选消费的 ASCII 输入字节，而 TSF 从 UTF-16 预编辑里删除同样数量的代码单元；两者对 ASCII 一致。对非 ASCII 预编辑（粘贴文本或插件方案），运行时把 UTF-8 前缀折算为 UTF-16 代码单元（4 字节字符计 2），否则 `commit_candidate` 会因 `consumed > input_buffer_.size()` 拒绝候选。真实进程实测：`ＡＢＣ１２３` 的 consumed 由 18 修正为 6，`x!=y`（4）与 `ni3hao3`（7）保持不变。

## 7. 验证

新增测试（均在 `windows-debug`/Ninja 下通过）：

- `tests/engine/unified_input_tests.cpp`：分型、声调折叠、归属规则、混合解码、字面候选、按分数合并、`UnifiedCandidateBreakdown` 与实际候选数一致，以及恶意输入（截断 UTF-8、孤立续字节、超长输入、标点串）不越界且候选不超上限；另有 500 组固定种子伪随机 ASCII 预编辑，验证候选数 ≤ 上限、breakdown 总和一致、消费长度正确、结果可重复且无重复文本。
- `tests/engine/smart_candidates_tests.cpp`：算式（含“日期形状不当作减法”回归）、数字中文读法（小写/大写金额/小数/百分比/越界拒绝；8 位合法紧凑日期不产生读法）、中文数字转阿拉伯数字（十二/二十/一百零五/一千零十二/一万零一/一亿二千万/十二点五/壹仟贰佰叁拾肆元整/两百/十，以及连续数字与重复单位拒绝）、日期本地化/星期/ISO/时间补零、紧凑日期展开（`20261019`，非法月/7 位拒绝）、日期区间本地化（整段区间、中文连接符、部分区间/非法日期拒绝）、中文日期的 ISO + 星期、日期/时钟单位不触发中英文空格、符号归一化（`!=`/`<=`/`->`/`...`，`--help` 不变）、时长读法（h/min/s、小数、跨分/跨时、长单位名与大写单位拒绝）、日期/标点/括号纠错、日期区间顺序、中英文空格、标点全角化（含反例：`1,000`、`3.14`、`www.qq.com` 不变）、全角 ASCII 归一化、拆分/合并/重排/占位、合并去重与上限。
- `tests/engine/expressive_candidates_tests.cpp`：键规范化、素材描述、整段匹配与上限。
- `tests/engine/concept_candidates_tests.cpp`：概念键规范化、内置表、TSV 加载（合法/缺列/缺文件）、精确匹配与 ≥8 字母前缀提示（短前缀不提示、精确优先）、带数字/符号的预编辑不匹配、空格/单引号分隔仍可匹配、候选字段与上限，以及 2,000 条术语表上的索引正确性（精确/缺失/前缀范围/同一 cue 多术语/精确不进入前缀范围）。
- `tests/ipc/core_service_contract.cpp`：端到端覆盖 `ni3`、`ni3hao`、`ni3hao3`（TSF 并入声调数字后逐键可达的状态）、`nihao6`（数字后缀混合输入）、`90min`（时长读法）、`API`（大写=显式英文，字面候选领先）、`baidu.com`（混合字面）、`leisi`（表情）、`chongfuzhixing`（幂等概念）、`chongfuzhixing123`（数字后缀不匹配概念）、`1234`（中文读法）、`一千二百三十四`（中文数字转阿拉伯）、`20261019`（紧凑日期展开）、`xiangmucue`（项目术语表经 `run_core_server` 传入后仍可用）、`ＡＢＣ１２３`（全角转半角）、`2026-10-19`（日期 + 星期，且不再出现减法结果）、`2026-10-19~2026-10-25`（区间本地化）、`2026年1月5日`（中文日期 → ISO + 星期，consumed 为 9 个 UTF-16 单元）、`chongfuzh`（概念前缀提示）、`x!=y`（符号归一化）、`=1+2`（表格公式）、`ganbei`（社交表情），并保持全部既有断言。
- `owo.engine.unified_candidate_benchmark` 同时注册为 CTest：除延迟分位外，它会校验每个样本的期望候选是否出现，等于给统一入口加了一层跨规则族的输出回归。
- `tests/tsf/key_routing_tests.cpp`（CTest `owo.tsf.key_routing`）：数字键路由纯函数——候选序号优先、请求挂起时保留给选择、数字紧跟字母时并入预编辑（`1-5` 声调、`0/6-9` 混合输入）、`0` 永不选候选、数字后/分隔符后/无预编辑/非数字键一律放行。
- `tests/config/config_store_tests.cpp`：配置 schema v16 的迁移（v14/v15 缺键 → 默认开启）、往返持久化、非法取值与缺键拒绝。
- `tests/ipc/candidate_toggle_tests.cpp`（CTest `owo.ipc.candidate_toggles`）：真实核心服务按配置关闭智能/表情/概念候选，验证 `1234`、`leisi`、`chongfuzhixing` 的候选列表逐项变化，且其他候选族不受影响。
- TSF 全套 `owo.tsf.*`（dll_smoke、language_context、input_method_context、candidate_window_order、pinyin_cursor、key_routing）6/6 通过。
- `benchmarks/unified_candidate_benchmark.cpp`：逐输入输出候选数、标记命中与延迟分位（Debug/本次机器：普通拼音约 0.03 ms，声调/混合解码约 0.5–0.8 ms，短字面/概念/数字约 0.07–0.14 ms，表情键约 0.37 ms，56 字节混合输入约 0.19 ms，均远低于同机运行时的整句解析）。它还带 **200 ms 的 p95 冒烟上限**：一旦「带纠错的完整解析」重新混入统一入口（曾造成 Debug 下 60–260 ms），CTest 会直接失败。

命令（Ninja 单配置构建目录 `build/ninja-debug`）：

```powershell
cmake --build build\ninja-debug --target owo_unified_input_tests owo_smart_candidates_tests owo_expressive_candidates_tests owo_concept_candidates_tests owo_core_service_contract owo_unified_candidate_benchmark
build\ninja-debug\owo_unified_input_tests.exe
build\ninja-debug\owo_smart_candidates_tests.exe
build\ninja-debug\owo_expressive_candidates_tests.exe
build\ninja-debug\owo_concept_candidates_tests.exe
build\ninja-debug\owo_core_service_contract.exe
build\ninja-debug\owo_unified_candidate_benchmark.exe
```

本机备注：PATH 中的 `cmake` 是 Cygwin 构建，不认识 Visual Studio 生成器，因此 `build/ninja-debug` 由 Visual Studio 2022 `vcvars64` + `Visual Studio 18 Insiders` 自带的 CMake/Ninja 配置：

```powershell
call "<VS2022>\VC\Auxiliary\Build\vcvars64.bat"
"<VS18>\Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin\cmake.exe" -S . -B build\ninja-debug -G Ninja `
  -DCMAKE_MAKE_PROGRAM="<VS18>\...\CMake\Ninja\ninja.exe" -DCMAKE_CXX_COMPILER=cl `
  -DCMAKE_BUILD_TYPE=Debug -DBUILD_TESTING=ON
```

基线核对：`owo.engine.candidates` 在本次改动前的 `origin/master` 干净检出上同样失败（"a complete shorter reading displaced an incomplete learned word"），属于既有问题，与本切片无关。

## 8. 后续工作

1. TSF 预编辑路由：**数字已接入**——`src/tsf/key_routing.cpp` 提供纯函数 `route_digit_key`（候选序号优先 > 数字紧跟字母时并入预编辑 > 交给宿主；`1-5` 在引擎里按声调折叠，其余数字构成混合输入；`0` 永不选择候选），`should_eat_key` 与 `OnKeyDown` 共用同一判定，`tests/tsf/key_routing_tests.cpp` 覆盖候选优先、声调/混合数字并入、数字/分隔符/无预编辑时放行、请求挂起时 `0` 仍并入等分支。仍未做：`=`、`+` 等符号的路由（与中文标点转换冲突，需要独立设置项），以及必须的**真实 GUI 按键验收**（本工作树只能做到构建 + 单测 + DLL 冒烟，不能替代真机输入验证）。
2. 媒体素材插入：为工作台/插件定义素材请求与插入协议（剪贴板 + 用户确认、或富文本嵌入），并保持 TSF 不加载生成模型。
3. 语义概念候选：当前是「描述读音精确匹配本地术语表」；真正的语义检索需要 ModelHost 提供本地嵌入或概念索引后再接入，在此之前不做云端概念查询。
4. 术语表下发：核心服务已支持 `--concept-glossary <TSV>`（替换内置起步表，坏文件降级为内置表）；剩余工作是把它接到设置中心与项目语感词库（团队交付物 → 项目术语表），并提供逐条查看/启停。
5. 数值单位一致性：在数值纠错规则之上补充单位/量纲检查，保持纯本地。
