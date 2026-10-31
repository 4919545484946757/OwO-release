# Wubi86 Phase 1 完成与验证报告

日期：2026-10-30（Asia/Shanghai）

**结论：本轮 Phase 1 验收完成，范围为 GB2312 核心完整码、来源内扩展数据、完整参考字根形体、独立词典、编码及 Trie；没有进入 Phase 2。**

原有 Engine 候选回归仍为 **FAIL**，本轮重新在原始 HEAD 复现同一错误。按本轮“没有引入新回归”的验收要求处理，未修改拼音代码；不能将本报告解读为全部 Engine 测试通过。范围外扩展汉字不声称全部有可靠编码。

## 本轮解决的原有问题

1. 不再用 11 字审核清单充当完整单字数据。引入固定版本 MIT 显式全码资源，第三列为完整码，其后为来源简码，不依长度/最长记录推断。已实际覆盖 GB2312 一级 3,755 字、二级 3,008 字，共 6,763/6,763。
2. 交叉审查发现辅助全码源的 发/ntyc、靥/dddd 两处错误；分别用固定 Jidian 与 Rime 的双重记录核实为 ntcy、dddl。修正记录、原值及见证均保留；导入时自动验证。
3. 处理主表未提供的 式/aad 等完整码缺项，保留原短码。6762 个核心字符全码有 Jidian 记录见证；式/aad 来自显式全码源，并符合 工+弋+末笔字型识别码 d。不是将 aa 延长猜码。
4. 正式区分 full、short、phrase；所有未经核实的四位部件别名也隔离，如 艹/copp。标准 艹/aghh 可用。
5. 增加完整参考字根资源：25 键、257 个含变体的形体，均有独立 ID、键位、原字体槽位及精确 SVG 路径。非 Unicode 字形不再缺失，也不冒充 Unicode 字符。
6. 对原始词典全部记录分类，逐词组按完整单字码交叉验证。隔离不符合规则的旧快捷词码和混合文本，不改写为虚构“标准编码”。
7. 增加全部核心字符、全部词条编码桶、全部前缀、全部词组及图形字根测试；原有机械测试保留并增强标准数据断言。

## 实际数据统计

| 项目 | 实际统计 |
|---|---:|
| GB2312 一级覆盖 | 3755 / 3755 |
| GB2312 二级覆盖 | 3008 / 3008 |
| GB2312 合计覆盖 | 6763 / 6763 |
| 有完整码的可移植字符 | 20924 |
| 核心以外字符 | 14161 |
| 全部 CJK 统一汉字 | 20902 |
| 兼容汉字 | 21 |
| 〇 | 1 |
| 完整码记录 | 20925 |
| 三位完整码记录 | 1587 |
| 四位完整码记录 | 19338 |
| 独立单字简码记录 | 4183 |
| 多完整码字符 | 1（戈/agnt、agny） |
| 核验一致的词组记录 | 62586 |
| 生产词典记录总数 | 87694 |
| 全词典重码编码数 | 8594 |
| 全词典重码关联记录数 | 19582 |
| 单字完整码的重码编码数 | 2906 |
| 单字完整码的重码关联记录数 | 6510 |
| 原始主表记录 | 89260 |
| 格式异常隔离 | 2 |
| 重复合并 | 3 |
| 原始有效唯一记录 | 89255 |
| 未核实单字别名隔离 | 1504 |
| 无完整字符支持的词组隔离 | 33 |
| 词组编码不一致隔离 | 35 |
| 有效唯一记录中隔离合计 | 1572 |
| 主表通过记录 | 87683 |
| 显式全码源补入记录 | 11（权重 0） |
| 辅助源私用区字符排除 | 80 |
| 字根键位 | 25 |
| 图形字根及变体条目 | 257 |

87,683 + 11 = 87,694。11 项补入记录保存在 normalization-audit.json 的 source_additions；每条隔离记录及原码、权重、原因保存在 quarantine.json。五条缺失权重明确置 0；原有效权重均保留，重复 (code,text) 取最大值。权重是上游候选偏好，不是声称测量得到的语料词频。

核心以外 14,161 字符由 14,139 个额外 CJK 统一汉字、21 个兼容字及 〇 组成。源中的 80 个私用槽没有稳定字符身份，全部不进入编码表。大于来源覆盖范围的扩展区字符/未核实别名不补造编码；这是扩展范围限制，不是 GB2312 缺口。

## 词组、简码及多编码策略

完整码来自显式全码字段；简码来自独立字段，可不以完整码为前缀（例如 我/q、以/c）。三位完整码（中/khk、好/vbg）不会被误当作简码。已核实备选完整码保留；对每个字的完整码按字典序排列，词组枚举全部备选的取码片段并去重。超过变体限额返回错误，不随机选择或静默截断。

词组取码：两字 2+2，三字 1+1+2，四字及以上取第一、二、三和最后一字首码。只用 full 记录，包括未参与取码的中间字也必须有完整码。62,586 条生产词组逐条生成并比对原码，全部一致。原表部分 足旁 词组使用不同旧快捷码，未当作标准生成结果；与拉丁/标点词组、缺少可靠单字码的记录一起保留在隔离文件。未引入其他五笔版本、用户词库、扩展词库或拼音词条。

## 字根数据及编码规则

参考源 riku-ri/wubi-kb 的 25 键全部形体已转换，共 257 项（含变体，不等同于 257 个基本字根）。完整图形可在 data/wubi86/roots-chart.svg 独立检查；API 为 Wubi86CodeTable::root_glyphs / root_glyph。每项有 W86-key-index ID、键位、原字体槽、SVG 轮廓。原槽 U+9752 在该字体里代表 青字头，不会被当成整个“青”。原 TeX 的对齐填充字符不作为字根。

采用精确字体轮廓转换而非生成图形。保留字体内 Arphic-1999 许可证、原始版权、上游修改声明及本轮格式转换说明。没有安装字体、执行 TeX 或实现候选 UI。取大优先、分散/连接/交叉原则、键名字、成字字根、单笔字、末笔字型识别码及词组规则说明位于 CODING-RULES.md。异常单字以验证后的表项为准；没有声称从未知字符图片自动拆字。

## 代码和文件

本轮沿用既有库和编码器/词典/Trie，没有重新实现已有功能。

| 本轮修改/新增文件 | 职责 |
|---|---|
| CMakeLists.txt | 原 Phase 1 配置上追加 coverage 可执行目标和 data_integrity CTest；相对 HEAD 共新增 44 行 |
| include/owo/wubi86/wubi86_code_table.h | 增加全图形字根查询类型与接口 |
| src/wubi86/wubi86_code_table.cpp | 连接完整字根资源；既有单字完整码/简码快照机制沿用 |
| src/wubi86/wubi86_roots.inc | 自动生成的 257 项键位及 SVG 路径 |
| tools/wubi86/normalize.py | 固定来源、显式全码解析、核心覆盖、修正见证、词组验证、分类和隔离 |
| tools/wubi86/generate_roots.py | 从固定字体/参考映射离线提取形体，无 TeX 执行 |
| tools/wubi86/verify_data.py | 只读核验生成文件、全部原始下载哈希、GB2312 精确位置及字根 ID |
| tests/wubi86/wubi86_tests.cpp | 保留机械测试；加入新全码后的正向断言，继续阻止 copp 污染 |
| tests/wubi86/wubi86_coverage_tests.cpp | 全核心、全词典、全前缀、全词组、字根及性能测试 |
| data/wubi86/main.tsv | 独立生产查询资源 |
| data/wubi86/full-codes.tsv | 显式完整码及备选完整码 |
| data/wubi86/gb2312.tsv | 全部核心字符及 GB2312 字节位置 |
| data/wubi86/full-code-corrections.json | 原始错误和有见证的修正 |
| data/wubi86/input-lock.json | 转换所读取原始文件的固定 SHA-256 |
| data/wubi86/output-sha256.json | 生成结果完整性摘要 |
| data/wubi86/normalization-audit.json | 精确统计、补入项、修正、异常和私用区排除 |
| data/wubi86/quarantine.json | 每条未采用主表记录及原因 |
| data/wubi86/roots.json、roots-chart.svg | 全参考形体、键位与独立核查图 |
| data/wubi86/LICENSE.pywubi、LICENSE.roots、ROOTS-NOTICE.md、font-modifications.adoc | 辅助全码与字根资源的许可证、版权和修改声明 |
| data/wubi86/downloads.json | 全部下载凭据 |
| data/wubi86/SOURCES.md、CODING-RULES.md | 数据语义、规则、来源、再现与边界 |
| docs/wubi86/PLAN.md、REPORT.md | 本轮实施记录和报告 |
| docs/wubi86/validation-results.json、baseline-test.log | 实际测试输出及原始 HEAD 失败证据 |

既有 wubi86_encoder、wubi86_dictionary、wubi86_trie、wubi86_types 实现继续复用，没有因为数据补齐另造接口。旧 reviewed-full-codes.json 保留为备选码审核证据，并校验其 source_commit；它不再限制整个编码表覆盖。

## 构建与实际验证

已有工具：MSVC 19.44 / Visual Studio 2022、Windows SDK 10.0.26100.0、CMake、CTest、Python、FontTools/Pillow。没有安装软件、下载或执行安装程序、修改系统环境或注册服务。

- 五笔库及两个测试可执行目标：编译 PASS。
- Engine 静态库：重新构建 PASS。
- 五笔 CTest：3/3 PASS。
- Engine CTest：15/16 PASS；候选测试 FAIL，基线同错。
- 联合运行：19 项，18 PASS、1 FAIL，无新增 Engine 失败。
- MSVC /W4 和 /analyze：PASS，无诊断；独立分析构建缓存 CMAKE_CXX_FLAGS=/analyze，项目 EnablePREfast=true。
- Python py_compile：PASS。
- 两个转换脚本离线重复运行：所有生成文件 SHA 相同，PASS。
- 全部 65 个下载位置、字节数与哈希核验：PASS。
- git diff --check、新源码空白检查：PASS。未发现项目额外专用 lint 入口，不冒充已运行不存在的脚本。

最新联合执行结果（来自 validation-results.json）：

| CTest | 结果 | 秒 |
|---|---|---:|
| owo.engine.input_method_composition | PASS | 0.028 |
| owo.engine.input_method_provider | PASS | 0.026 |
| owo.engine.full_pinyin | PASS | 10.355 |
| owo.engine.candidates | FAIL | 0.171 |
| owo.engine.ambiguous_boundaries | PASS | 0.358 |
| owo.engine.dictionary_bridges | PASS | 0.589 |
| owo.engine.phrase_chains | PASS | 0.209 |
| owo.engine.candidate_partition | PASS | 0.026 |
| owo.engine.candidate_search_budget | PASS | 0.270 |
| owo.engine.lookup_reuse | PASS | 0.180 |
| owo.engine.local_corrections | PASS | 5.182 |
| owo.engine.user_lexicon_alignment | PASS | 0.200 |
| owo.engine.candidate_scheduler | PASS | 0.033 |
| owo.engine.binary_lexicon | PASS | 0.035 |
| owo.engine.user_frequency | PASS | 0.248 |
| owo.engine.user_word_retention | PASS | 0.981 |
| owo.wubi86.phase1 | PASS | 1.706 |
| owo.wubi86.coverage | PASS | 8.118 |
| owo.wubi86.data_integrity | PASS | 0.319 |

覆盖内容包括：257 个图形字根身份和逐键数量、GB2312 全部字符（含末字齄）、全码/简码 Golden、多编码与重码、两/三/四/多字词、编码/UTF-8/损坏数据/频率溢出/冲突/事务加载、全部精确桶和前缀的候选/排序/limit，以及 20,000 次原有查询测试。旧合成 甲/乙/丙 测试明确只证明机制，不充当标准证据。

全量生产数据测试的实际输出：

```
PASS core=6763 roots=257 rows=87694 phrases=62586 exact_codes=76706 prefixes=85921 load_ms=1407 total_ms=7497
```

该轮 Debug 加载 1,407 ms，完整核心、词组和全查询验证 7,497 ms。它是本机这一次观测，不是跨机器吞吐保证。性能保护阈值为加载 30 秒、全验证 120 秒，防止卡死；没有降低原测试或造出性能成绩。

Engine 失败信息：`a complete shorter reading displaced an incomplete learned word`。重新构建原始基线 `0c30b077b7fe5d883ac109e6157690ebf61d9f0c` 的候选测试，同样 FAIL。基线位于 build/wubi86-baseline / build/wubi86-baseline-build。原有源码仅 CMakeLists.txt 有增量；拼音/Engine 源码没有改动，不能修饰成“拼音回归全部通过”。

## 下载清单

全部 65 个下载文件（本轮新增 58 个，复用此前 7 个）仅保存在 env/wubi86/。research-urls.json 是本地生成的下载索引，不是下载输入。原始资源不删除；正式数据为本地转换产物。CNMan、viewer、ibus 等未通过用途/授权判断的资源只用于调查，没有混进生产词典。每一文件的精确 URL、版本、大小、SHA-256、许可证及实际用途如下；机器可读同 downloads.json。

### commit.json

- 保存路径：`env/wubi86/commit.json`
- 来源 URL：https://api.github.com/repos/KyleBing/rime-wubi86-jidian/commits/master
- 版本/Commit：`4d08110b8080f6577dad733ada7e5ae5c5e878fd`
- 大小：5191 bytes
- SHA-256：`8fdf43fb36c8274fdb7cff241a7acb060b7ab81947a6f69fe97e9a596d78809b`
- 许可证：GitHub API metadata; no license grant assumed
- 使用目的：pin upstream commit

### README.md

- 保存路径：`env/wubi86/README.md`
- 来源 URL：https://raw.githubusercontent.com/KyleBing/rime-wubi86-jidian/4d08110b8080f6577dad733ada7e5ae5c5e878fd/README.md
- 版本/Commit：`4d08110b8080f6577dad733ada7e5ae5c5e878fd`
- 大小：20290 bytes
- SHA-256：`38e2a697df4bd35b0005091eabb70f01b19ab0c2f05b227676c103370a12ace2`
- 许可证：MIT
- 使用目的：source/version scope review

### LICENSE

- 保存路径：`env/wubi86/LICENSE`
- 来源 URL：https://raw.githubusercontent.com/KyleBing/rime-wubi86-jidian/4d08110b8080f6577dad733ada7e5ae5c5e878fd/LICENSE
- 版本/Commit：`4d08110b8080f6577dad733ada7e5ae5c5e878fd`
- 大小：1061 bytes
- SHA-256：`e2396e96d15f17d1a1020e9798e5271edcef8749dcdf80f7284262659ab7ee53`
- 许可证：MIT
- 使用目的：license and attribution

### wubi86_jidian.dict.yaml

- 保存路径：`env/wubi86/wubi86_jidian.dict.yaml`
- 来源 URL：https://raw.githubusercontent.com/KyleBing/rime-wubi86-jidian/4d08110b8080f6577dad733ada7e5ae5c5e878fd/wubi86_jidian.dict.yaml
- 版本/Commit：`4d08110b8080f6577dad733ada7e5ae5c5e878fd`
- 大小：1320330 bytes
- SHA-256：`89df3685e4a0c3d319a1c56a3a64e3df4ee3bfa3ba3892a6dadc77b21c989c5b`
- 许可证：MIT
- 使用目的：only production lookup data source; normalized offline

### roots-reference.html

- 保存路径：`env/wubi86/roots-reference.html`
- 来源 URL：https://hantang.github.io/search-wubi/wiki/roots/
- 版本/Commit：`unversioned reference snapshot 2026-10-30; exact SHA-256 retained`
- 大小：49933 bytes
- SHA-256：`e7c3e26b34bcf6398d3ace693c6c9770ce6f9182fc8ca6c370ba1737d314b3d4`
- 许可证：Page says Copyleft 2024-2026; no specific license verified; not redistributed
- 使用目的：consult named root/key facts; glyph forms not copied

### wubi86-tup-excerpt.pdf

- 保存路径：`env/wubi86/wubi86-tup-excerpt.pdf`
- 来源 URL：https://www.tup.tsinghua.edu.cn/upload/books/yz/046215-01.pdf
- 版本/Commit：`unversioned reference snapshot 2026-10-30; exact SHA-256 retained`
- 大小：2802817 bytes
- SHA-256：`cc16d83252c434a7cbf1d5b73e5a438f6aaf88f28d458deb8ed45494339c8baf`
- 许可证：Publisher copyright; no redistribution license verified; not redistributed
- 使用目的：consult version/coverage background

### office-tup-excerpt.pdf

- 保存路径：`env/wubi86/office-tup-excerpt.pdf`
- 来源 URL：https://www.tup.tsinghua.edu.cn/upload/books/yz/033229-01.pdf
- 版本/Commit：`unversioned reference snapshot 2026-10-30; exact SHA-256 retained`
- 大小：5291179 bytes
- SHA-256：`1f5edb7b9dbfee6a61b5f249293fff35c6ae4e997063b38a634665a4ba910ae6`
- 许可证：Publisher copyright; no redistribution license verified; not redistributed
- 使用目的：consult keyboard/root background

### pythonwubi-README.md

- 保存路径：`env/wubi86/pythonwubi-README.md`
- 来源 URL：https://raw.githubusercontent.com/sfyc23/python-wubi/42c6ac6b4d2285acc7f39594274a4644cdd358ff/README.md
- 版本/Commit：`42c6ac6b4d2285acc7f39594274a4644cdd358ff`
- 大小：8248 bytes
- SHA-256：`a77c8ed399b1973ddc5b897a0ce0cd92f009fdc1d6e414e4da80384ec42ae501`
- 许可证：MIT
- 使用目的：explicit full-code source; resource third column is full code, subsequent columns are source short forms; source implementation read for semantics

### pythonwubi-LICENSE

- 保存路径：`env/wubi86/pythonwubi-LICENSE`
- 来源 URL：https://raw.githubusercontent.com/sfyc23/python-wubi/42c6ac6b4d2285acc7f39594274a4644cdd358ff/LICENSE
- 版本/Commit：`42c6ac6b4d2285acc7f39594274a4644cdd358ff`
- 大小：1072 bytes
- SHA-256：`2798c535a0816f138ea1791f229eb659400d9ce19a0317bb8f0c0b52745a3b27`
- 许可证：MIT
- 使用目的：explicit full-code source; resource third column is full code, subsequent columns are source short forms; source implementation read for semantics

### pythonwubi-pywubi_core.py

- 保存路径：`env/wubi86/pythonwubi-pywubi_core.py`
- 来源 URL：https://raw.githubusercontent.com/sfyc23/python-wubi/42c6ac6b4d2285acc7f39594274a4644cdd358ff/pywubi/core.py
- 版本/Commit：`42c6ac6b4d2285acc7f39594274a4644cdd358ff`
- 大小：2920 bytes
- SHA-256：`fbb73590d7e7e2e6581bffc5be9c9c49e0457f88385d900c91b791c27eeb2153`
- 许可证：MIT
- 使用目的：explicit full-code source; resource third column is full code, subsequent columns are source short forms; source implementation read for semantics

### pythonwubi-pywubi_loader.py

- 保存路径：`env/wubi86/pythonwubi-pywubi_loader.py`
- 来源 URL：https://raw.githubusercontent.com/sfyc23/python-wubi/42c6ac6b4d2285acc7f39594274a4644cdd358ff/pywubi/loader.py
- 版本/Commit：`42c6ac6b4d2285acc7f39594274a4644cdd358ff`
- 大小：3984 bytes
- SHA-256：`29d93c8d66665f4d1f0f72571cfae8dfa7f6469f6a74a67948287b39ddab1208`
- 许可证：MIT
- 使用目的：explicit full-code source; resource third column is full code, subsequent columns are source short forms; source implementation read for semantics

### pythonwubi-pywubi_data_wubi_86.json

- 保存路径：`env/wubi86/pythonwubi-pywubi_data_wubi_86.json`
- 来源 URL：https://raw.githubusercontent.com/sfyc23/python-wubi/42c6ac6b4d2285acc7f39594274a4644cdd358ff/pywubi/data/wubi_86.json
- 版本/Commit：`42c6ac6b4d2285acc7f39594274a4644cdd358ff`
- 大小：337904 bytes
- SHA-256：`c0179f40392629a11fda59cf7c1293d22ff821e5f513cbcf972c075d564c1783`
- 许可证：MIT
- 使用目的：explicit full-code source; resource third column is full code, subsequent columns are source short forms; source implementation read for semantics

### pythonwubi-resource_86版_全码.txt

- 保存路径：`env/wubi86/pythonwubi-resource_86版_全码.txt`
- 来源 URL：https://raw.githubusercontent.com/sfyc23/python-wubi/42c6ac6b4d2285acc7f39594274a4644cdd358ff/resource/86%E7%89%88_%E5%85%A8%E7%A0%81.txt
- 版本/Commit：`42c6ac6b4d2285acc7f39594274a4644cdd358ff`
- 大小：308533 bytes
- SHA-256：`c201906161fc4d5d94b7f16519607bb2297a6fa365cfbf9b5b961b53dec8ddd7`
- 许可证：MIT
- 使用目的：explicit full-code source; resource third column is full code, subsequent columns are source short forms; source implementation read for semantics

### viewer-README.md

- 保存路径：`env/wubi86/viewer-README.md`
- 来源 URL：https://raw.githubusercontent.com/sdcb/wubi86-viewer/77c521c2e4e3918fb32af5c0eaef5aec63847939/README.md
- 版本/Commit：`77c521c2e4e3918fb32af5c0eaef5aec63847939`
- 大小：3197 bytes
- SHA-256：`57d33d026e6be7e66cddb0d13044e2337ed4cbb516acccfd1af8993d404874bb`
- 许可证：AGPL-3.0 code; third-party data retains original licenses, unresolved for these tables
- 使用目的：format/conflict investigation only; not imported

### viewer-LICENSE

- 保存路径：`env/wubi86/viewer-LICENSE`
- 来源 URL：https://raw.githubusercontent.com/sdcb/wubi86-viewer/77c521c2e4e3918fb32af5c0eaef5aec63847939/LICENSE
- 版本/Commit：`77c521c2e4e3918fb32af5c0eaef5aec63847939`
- 大小：34523 bytes
- SHA-256：`0d96a4ff68ad6d4b6f1f30f713b18d5184912ba8dd389f86aa7710db079abcb0`
- 许可证：AGPL-3.0 code; third-party data retains original licenses, unresolved for these tables
- 使用目的：format/conflict investigation only; not imported

### viewer-data-src_data-chars.tsv

- 保存路径：`env/wubi86/viewer-data-src_data-chars.tsv`
- 来源 URL：https://raw.githubusercontent.com/sdcb/wubi86-viewer/77c521c2e4e3918fb32af5c0eaef5aec63847939/data-src/data-chars.tsv
- 版本/Commit：`77c521c2e4e3918fb32af5c0eaef5aec63847939`
- 大小：1078901 bytes
- SHA-256：`9694872e66985dc169672e6481ba3d6dafc4c19db45fa365dd0fc82d598048c6`
- 许可证：AGPL-3.0 code; third-party data retains original licenses, unresolved for these tables
- 使用目的：format/conflict investigation only; not imported

### viewer-data-src_data-wubi-v86.tsv

- 保存路径：`env/wubi86/viewer-data-src_data-wubi-v86.tsv`
- 来源 URL：https://raw.githubusercontent.com/sdcb/wubi86-viewer/77c521c2e4e3918fb32af5c0eaef5aec63847939/data-src/data-wubi-v86.tsv
- 版本/Commit：`77c521c2e4e3918fb32af5c0eaef5aec63847939`
- 大小：713635 bytes
- SHA-256：`0557bfcb2e18699d2ebab2234ef90491734d7cfb064f09fb8ef1331ea17e8fb7`
- 许可证：AGPL-3.0 code; third-party data retains original licenses, unresolved for these tables
- 使用目的：format/conflict investigation only; not imported

### rootskb-README.adoc

- 保存路径：`env/wubi86/rootskb-README.adoc`
- 来源 URL：https://raw.githubusercontent.com/riku-ri/wubi-kb/c65590f6a524fa7cb9459d2095be87d22bd839fc/README.adoc
- 版本/Commit：`c65590f6a524fa7cb9459d2095be87d22bd839fc`
- 大小：1170 bytes
- SHA-256：`fdf29acc8316f49095efe1b0636d54d47062391319918f36bc2ec47b92b231ae`
- 许可证：No standalone TeX license found; factual key/glyph relations extracted only
- 使用目的：root/key factual mapping investigation; no TeX code copied or executed

### rootskb-font.adoc

- 保存路径：`env/wubi86/rootskb-font.adoc`
- 来源 URL：https://raw.githubusercontent.com/riku-ri/wubi-kb/c65590f6a524fa7cb9459d2095be87d22bd839fc/font.adoc
- 版本/Commit：`c65590f6a524fa7cb9459d2095be87d22bd839fc`
- 大小：7830 bytes
- SHA-256：`65896904255a33da86cc625c0058a311f5dab601332d56abc6d23f453d4b4b85`
- 许可证：Arphic-1999 (font derivative); notices retained
- 使用目的：exact graphical-root outlines and license verification; no font installation

### rootskb-rls_font.md

- 保存路径：`env/wubi86/rootskb-rls_font.md`
- 来源 URL：https://raw.githubusercontent.com/riku-ri/wubi-kb/c65590f6a524fa7cb9459d2095be87d22bd839fc/rls/font.md
- 版本/Commit：`c65590f6a524fa7cb9459d2095be87d22bd839fc`
- 大小：625 bytes
- SHA-256：`1c7720215dd97495855a5c9452ba6daa9fecaa429c5daec01039eefd8926d3e2`
- 许可证：No standalone TeX license found; factual key/glyph relations extracted only
- 使用目的：root/key factual mapping investigation; no TeX code copied or executed

### rootskb-wubi-86-kb.tex

- 保存路径：`env/wubi86/rootskb-wubi-86-kb.tex`
- 来源 URL：https://raw.githubusercontent.com/riku-ri/wubi-kb/c65590f6a524fa7cb9459d2095be87d22bd839fc/wubi-86-kb.tex
- 版本/Commit：`c65590f6a524fa7cb9459d2095be87d22bd839fc`
- 大小：1326 bytes
- SHA-256：`1f01f631c6a737bd2be8cd3183b3e967ce892906e4fdebf0b76a94ae216ab53f`
- 许可证：No standalone TeX license found; factual key/glyph relations extracted only
- 使用目的：root/key factual mapping investigation; no TeX code copied or executed

### rootskb-kb.tex

- 保存路径：`env/wubi86/rootskb-kb.tex`
- 来源 URL：https://raw.githubusercontent.com/riku-ri/wubi-kb/c65590f6a524fa7cb9459d2095be87d22bd839fc/kb.tex
- 版本/Commit：`c65590f6a524fa7cb9459d2095be87d22bd839fc`
- 大小：3661 bytes
- SHA-256：`c226c4fc85f0ea77a9b7b4bbe51e771fe6619fd83176f2647da43f2bbb022fb8`
- 许可证：No standalone TeX license found; factual key/glyph relations extracted only
- 使用目的：root/key factual mapping investigation; no TeX code copied or executed

### rootskb-kb_g.tex

- 保存路径：`env/wubi86/rootskb-kb_g.tex`
- 来源 URL：https://raw.githubusercontent.com/riku-ri/wubi-kb/c65590f6a524fa7cb9459d2095be87d22bd839fc/kb/g.tex
- 版本/Commit：`c65590f6a524fa7cb9459d2095be87d22bd839fc`
- 大小：217 bytes
- SHA-256：`ea1db2775605f130e622c0e0a17bcaec2bf5ff428d229654cc6ab086fdb2aaf6`
- 许可证：No standalone TeX license found; factual key/glyph relations extracted only
- 使用目的：root/key factual mapping investigation; no TeX code copied or executed

### rootskb-kb_a.tex

- 保存路径：`env/wubi86/rootskb-kb_a.tex`
- 来源 URL：https://raw.githubusercontent.com/riku-ri/wubi-kb/c65590f6a524fa7cb9459d2095be87d22bd839fc/kb/a.tex
- 版本/Commit：`c65590f6a524fa7cb9459d2095be87d22bd839fc`
- 大小：319 bytes
- SHA-256：`176b6bf8169cda9c19db36dbd8634f9ae7725b2be39ceefc1bf75a55168b3c25`
- 许可证：No standalone TeX license found; factual key/glyph relations extracted only
- 使用目的：root/key factual mapping investigation; no TeX code copied or executed

### rootskb-kb_n.tex

- 保存路径：`env/wubi86/rootskb-kb_n.tex`
- 来源 URL：https://raw.githubusercontent.com/riku-ri/wubi-kb/c65590f6a524fa7cb9459d2095be87d22bd839fc/kb/n.tex
- 版本/Commit：`c65590f6a524fa7cb9459d2095be87d22bd839fc`
- 大小：384 bytes
- SHA-256：`cb1706b6715739e57bc809f854a9b66cf3e806cc1220cf62b3ba0faf1d81a284`
- 许可证：No standalone TeX license found; factual key/glyph relations extracted only
- 使用目的：root/key factual mapping investigation; no TeX code copied or executed

### rootskb-releases.json

- 保存路径：`env/wubi86/rootskb-releases.json`
- 来源 URL：https://api.github.com/repos/riku-ri/wubi-kb/releases
- 版本/Commit：`c65590f6a524fa7cb9459d2095be87d22bd839fc`
- 大小：20661 bytes
- SHA-256：`b61ecb25bb5a9d502a3c607a51cba72218d2278e66d740564a803376833a9fe9`
- 许可证：GitHub API metadata; not production data
- 使用目的：pin upstream revision / enumerate resource files

### rootskb-kb_b.tex

- 保存路径：`env/wubi86/rootskb-kb_b.tex`
- 来源 URL：https://raw.githubusercontent.com/riku-ri/wubi-kb/c65590f6a524fa7cb9459d2095be87d22bd839fc/kb/b.tex
- 版本/Commit：`c65590f6a524fa7cb9459d2095be87d22bd839fc`
- 大小：265 bytes
- SHA-256：`0318fd1fe60756a53b69a4a37de3196c53512d306401a34cb7c1387fe854c42b`
- 许可证：No standalone TeX license found; factual key/glyph relations extracted only
- 使用目的：root/key factual mapping investigation; no TeX code copied or executed

### rootskb-kb_c.tex

- 保存路径：`env/wubi86/rootskb-kb_c.tex`
- 来源 URL：https://raw.githubusercontent.com/riku-ri/wubi-kb/c65590f6a524fa7cb9459d2095be87d22bd839fc/kb/c.tex
- 版本/Commit：`c65590f6a524fa7cb9459d2095be87d22bd839fc`
- 大小：206 bytes
- SHA-256：`2408ac6b4975131647255ceeafeec0e4bad9ad49f812749cee69f560d40cea9d`
- 许可证：No standalone TeX license found; factual key/glyph relations extracted only
- 使用目的：root/key factual mapping investigation; no TeX code copied or executed

### rootskb-kb_d.tex

- 保存路径：`env/wubi86/rootskb-kb_d.tex`
- 来源 URL：https://raw.githubusercontent.com/riku-ri/wubi-kb/c65590f6a524fa7cb9459d2095be87d22bd839fc/kb/d.tex
- 版本/Commit：`c65590f6a524fa7cb9459d2095be87d22bd839fc`
- 大小：285 bytes
- SHA-256：`49a6001a68c907afd1d89381193d9c9696b7dc93d0373cf520aef732261f3906`
- 许可证：No standalone TeX license found; factual key/glyph relations extracted only
- 使用目的：root/key factual mapping investigation; no TeX code copied or executed

### rootskb-kb_e.tex

- 保存路径：`env/wubi86/rootskb-kb_e.tex`
- 来源 URL：https://raw.githubusercontent.com/riku-ri/wubi-kb/c65590f6a524fa7cb9459d2095be87d22bd839fc/kb/e.tex
- 版本/Commit：`c65590f6a524fa7cb9459d2095be87d22bd839fc`
- 大小：320 bytes
- SHA-256：`1ff17b366d1af392e5e14154d297452268d52a9de8f604acdda53f0749615773`
- 许可证：No standalone TeX license found; factual key/glyph relations extracted only
- 使用目的：root/key factual mapping investigation; no TeX code copied or executed

### rootskb-kb_f.tex

- 保存路径：`env/wubi86/rootskb-kb_f.tex`
- 来源 URL：https://raw.githubusercontent.com/riku-ri/wubi-kb/c65590f6a524fa7cb9459d2095be87d22bd839fc/kb/f.tex
- 版本/Commit：`c65590f6a524fa7cb9459d2095be87d22bd839fc`
- 大小：268 bytes
- SHA-256：`4550356fb3f90f8cf2185e27fc0b90cc635f85f97f0602bc0f3bda561066c28c`
- 许可证：No standalone TeX license found; factual key/glyph relations extracted only
- 使用目的：root/key factual mapping investigation; no TeX code copied or executed

### rootskb-kb_h.tex

- 保存路径：`env/wubi86/rootskb-kb_h.tex`
- 来源 URL：https://raw.githubusercontent.com/riku-ri/wubi-kb/c65590f6a524fa7cb9459d2095be87d22bd839fc/kb/h.tex
- 版本/Commit：`c65590f6a524fa7cb9459d2095be87d22bd839fc`
- 大小：257 bytes
- SHA-256：`228c7f918a088aa8a85629c7e766bc0fe92727994acfa8f4c6170ebf8e066c19`
- 许可证：No standalone TeX license found; factual key/glyph relations extracted only
- 使用目的：root/key factual mapping investigation; no TeX code copied or executed

### rootskb-kb_i.tex

- 保存路径：`env/wubi86/rootskb-kb_i.tex`
- 来源 URL：https://raw.githubusercontent.com/riku-ri/wubi-kb/c65590f6a524fa7cb9459d2095be87d22bd839fc/kb/i.tex
- 版本/Commit：`c65590f6a524fa7cb9459d2095be87d22bd839fc`
- 大小：265 bytes
- SHA-256：`3c7264471299a83c690d5a8f5a4e6d935fedc97b41ee9dbc2de80a968af40310`
- 许可证：No standalone TeX license found; factual key/glyph relations extracted only
- 使用目的：root/key factual mapping investigation; no TeX code copied or executed

### rootskb-kb_j.tex

- 保存路径：`env/wubi86/rootskb-kb_j.tex`
- 来源 URL：https://raw.githubusercontent.com/riku-ri/wubi-kb/c65590f6a524fa7cb9459d2095be87d22bd839fc/kb/j.tex
- 版本/Commit：`c65590f6a524fa7cb9459d2095be87d22bd839fc`
- 大小：260 bytes
- SHA-256：`744b14de39411c743701e0189439b33ae7dbc2c08072b50fb322bc5d8ce804e7`
- 许可证：No standalone TeX license found; factual key/glyph relations extracted only
- 使用目的：root/key factual mapping investigation; no TeX code copied or executed

### rootskb-kb_k.tex

- 保存路径：`env/wubi86/rootskb-kb_k.tex`
- 来源 URL：https://raw.githubusercontent.com/riku-ri/wubi-kb/c65590f6a524fa7cb9459d2095be87d22bd839fc/kb/k.tex
- 版本/Commit：`c65590f6a524fa7cb9459d2095be87d22bd839fc`
- 大小：182 bytes
- SHA-256：`8fbcb9efe37622812228ec32781589711328e210ed822ce12346210a49b854ff`
- 许可证：No standalone TeX license found; factual key/glyph relations extracted only
- 使用目的：root/key factual mapping investigation; no TeX code copied or executed

### rootskb-kb_l.tex

- 保存路径：`env/wubi86/rootskb-kb_l.tex`
- 来源 URL：https://raw.githubusercontent.com/riku-ri/wubi-kb/c65590f6a524fa7cb9459d2095be87d22bd839fc/kb/l.tex
- 版本/Commit：`c65590f6a524fa7cb9459d2095be87d22bd839fc`
- 大小：303 bytes
- SHA-256：`0812568434f74618d5f20ad5c78115b21f7040c2805ab24039aad726d92ef9db`
- 许可证：No standalone TeX license found; factual key/glyph relations extracted only
- 使用目的：root/key factual mapping investigation; no TeX code copied or executed

### rootskb-kb_m.tex

- 保存路径：`env/wubi86/rootskb-kb_m.tex`
- 来源 URL：https://raw.githubusercontent.com/riku-ri/wubi-kb/c65590f6a524fa7cb9459d2095be87d22bd839fc/kb/m.tex
- 版本/Commit：`c65590f6a524fa7cb9459d2095be87d22bd839fc`
- 大小：256 bytes
- SHA-256：`b229f2116f85b1b6b813cec0ebc18d8b1a8b04e214a47840eba0a3b8b5e97ded`
- 许可证：No standalone TeX license found; factual key/glyph relations extracted only
- 使用目的：root/key factual mapping investigation; no TeX code copied or executed

### rootskb-kb_p.tex

- 保存路径：`env/wubi86/rootskb-kb_p.tex`
- 来源 URL：https://raw.githubusercontent.com/riku-ri/wubi-kb/c65590f6a524fa7cb9459d2095be87d22bd839fc/kb/p.tex
- 版本/Commit：`c65590f6a524fa7cb9459d2095be87d22bd839fc`
- 大小：224 bytes
- SHA-256：`edb74073a4d94da8fed907d4a0bc350f0d9f9835142d37b7283a0579c2bc16a6`
- 许可证：No standalone TeX license found; factual key/glyph relations extracted only
- 使用目的：root/key factual mapping investigation; no TeX code copied or executed

### rootskb-kb_q.tex

- 保存路径：`env/wubi86/rootskb-kb_q.tex`
- 来源 URL：https://raw.githubusercontent.com/riku-ri/wubi-kb/c65590f6a524fa7cb9459d2095be87d22bd839fc/kb/q.tex
- 版本/Commit：`c65590f6a524fa7cb9459d2095be87d22bd839fc`
- 大小：305 bytes
- SHA-256：`4147336a94f47c0aff8958aba2bb08e21f1d4c9c9097ba00338e84d51f12901f`
- 许可证：No standalone TeX license found; factual key/glyph relations extracted only
- 使用目的：root/key factual mapping investigation; no TeX code copied or executed

### rootskb-kb_r.tex

- 保存路径：`env/wubi86/rootskb-kb_r.tex`
- 来源 URL：https://raw.githubusercontent.com/riku-ri/wubi-kb/c65590f6a524fa7cb9459d2095be87d22bd839fc/kb/r.tex
- 版本/Commit：`c65590f6a524fa7cb9459d2095be87d22bd839fc`
- 大小：249 bytes
- SHA-256：`a3cae7c8a7a0d055c40a880a2649c7a31721c046ea860ef94a22397a522cbe11`
- 许可证：No standalone TeX license found; factual key/glyph relations extracted only
- 使用目的：root/key factual mapping investigation; no TeX code copied or executed

### rootskb-kb_s.tex

- 保存路径：`env/wubi86/rootskb-kb_s.tex`
- 来源 URL：https://raw.githubusercontent.com/riku-ri/wubi-kb/c65590f6a524fa7cb9459d2095be87d22bd839fc/kb/s.tex
- 版本/Commit：`c65590f6a524fa7cb9459d2095be87d22bd839fc`
- 大小：190 bytes
- SHA-256：`1361a20315b6d8bdbdf54c68c556f1e1055dcc46009d8c02de6e954126be8e00`
- 许可证：No standalone TeX license found; factual key/glyph relations extracted only
- 使用目的：root/key factual mapping investigation; no TeX code copied or executed

### rootskb-kb_t.tex

- 保存路径：`env/wubi86/rootskb-kb_t.tex`
- 来源 URL：https://raw.githubusercontent.com/riku-ri/wubi-kb/c65590f6a524fa7cb9459d2095be87d22bd839fc/kb/t.tex
- 版本/Commit：`c65590f6a524fa7cb9459d2095be87d22bd839fc`
- 大小：242 bytes
- SHA-256：`8c1246f735f9f2ffff03371442d2f290588d287168e0011d767b186e13fcc5fc`
- 许可证：No standalone TeX license found; factual key/glyph relations extracted only
- 使用目的：root/key factual mapping investigation; no TeX code copied or executed

### rootskb-kb_u.tex

- 保存路径：`env/wubi86/rootskb-kb_u.tex`
- 来源 URL：https://raw.githubusercontent.com/riku-ri/wubi-kb/c65590f6a524fa7cb9459d2095be87d22bd839fc/kb/u.tex
- 版本/Commit：`c65590f6a524fa7cb9459d2095be87d22bd839fc`
- 大小：284 bytes
- SHA-256：`a5220f0cd4425d20cbc6ee89f5e5f83c744bea772372137e419cd57de867a9ba`
- 许可证：No standalone TeX license found; factual key/glyph relations extracted only
- 使用目的：root/key factual mapping investigation; no TeX code copied or executed

### rootskb-kb_v.tex

- 保存路径：`env/wubi86/rootskb-kb_v.tex`
- 来源 URL：https://raw.githubusercontent.com/riku-ri/wubi-kb/c65590f6a524fa7cb9459d2095be87d22bd839fc/kb/v.tex
- 版本/Commit：`c65590f6a524fa7cb9459d2095be87d22bd839fc`
- 大小：233 bytes
- SHA-256：`2954b121274d949e0e6d61ab3bd761aa43919d3b12213743b2323138052276f7`
- 许可证：No standalone TeX license found; factual key/glyph relations extracted only
- 使用目的：root/key factual mapping investigation; no TeX code copied or executed

### rootskb-kb_w.tex

- 保存路径：`env/wubi86/rootskb-kb_w.tex`
- 来源 URL：https://raw.githubusercontent.com/riku-ri/wubi-kb/c65590f6a524fa7cb9459d2095be87d22bd839fc/kb/w.tex
- 版本/Commit：`c65590f6a524fa7cb9459d2095be87d22bd839fc`
- 大小：206 bytes
- SHA-256：`212b44d495d0a6cf71142b0b5dad4b8797ff23442cd48a3ca2c39c7e522a96de`
- 许可证：No standalone TeX license found; factual key/glyph relations extracted only
- 使用目的：root/key factual mapping investigation; no TeX code copied or executed

### rootskb-kb_x.tex

- 保存路径：`env/wubi86/rootskb-kb_x.tex`
- 来源 URL：https://raw.githubusercontent.com/riku-ri/wubi-kb/c65590f6a524fa7cb9459d2095be87d22bd839fc/kb/x.tex
- 版本/Commit：`c65590f6a524fa7cb9459d2095be87d22bd839fc`
- 大小：241 bytes
- SHA-256：`1481dd9c1f0a177085244ac00020a45e37099a1c75b73fc28e3a7cabf87868f8`
- 许可证：No standalone TeX license found; factual key/glyph relations extracted only
- 使用目的：root/key factual mapping investigation; no TeX code copied or executed

### rootskb-kb_y.tex

- 保存路径：`env/wubi86/rootskb-kb_y.tex`
- 来源 URL：https://raw.githubusercontent.com/riku-ri/wubi-kb/c65590f6a524fa7cb9459d2095be87d22bd839fc/kb/y.tex
- 版本/Commit：`c65590f6a524fa7cb9459d2095be87d22bd839fc`
- 大小：249 bytes
- SHA-256：`ac0adf3e983c9bf933a7eed1b367dbd0e6df0abecaf548fc4209fdc462ab23ce`
- 许可证：No standalone TeX license found; factual key/glyph relations extracted only
- 使用目的：root/key factual mapping investigation; no TeX code copied or executed

### rootskb-UMingCN-wubi-86-kb.ttf

- 保存路径：`env/wubi86/rootskb-UMingCN-wubi-86-kb.ttf`
- 来源 URL：https://github.com/riku-ri/wubi-kb/releases/download/font/UMingCN-wubi-86-kb.ttf
- 版本/Commit：`font pre-release; binary pinned by SHA-256 (not necessarily identical to current source commit)`
- 大小：17530728 bytes
- SHA-256：`4b9d2a7a2473ff39e097565e8906875fbe1097deac7e0891c0b150244d71223a`
- 许可证：Arphic-1999 (font derivative); notices retained
- 使用目的：exact graphical-root outlines and license verification; no font installation

### rootskb-kb_o.tex

- 保存路径：`env/wubi86/rootskb-kb_o.tex`
- 来源 URL：https://raw.githubusercontent.com/riku-ri/wubi-kb/c65590f6a524fa7cb9459d2095be87d22bd839fc/kb/o.tex
- 版本/Commit：`c65590f6a524fa7cb9459d2095be87d22bd839fc`
- 大小：233 bytes
- SHA-256：`a9efe487a1cbe9ab1fae2d93114de4ce4b337b058202d4d31f70aec367c4ad6c`
- 许可证：No standalone TeX license found; factual key/glyph relations extracted only
- 使用目的：root/key factual mapping investigation; no TeX code copied or executed

### Arphic-1999.txt

- 保存路径：`env/wubi86/Arphic-1999.txt`
- 来源 URL：https://raw.githubusercontent.com/spdx/license-list-data/main/text/Arphic-1999.txt
- 版本/Commit：`snapshot pinned by SHA-256`
- 大小：6901 bytes
- SHA-256：`3a5e90c0957524a89e48203febcd4492ca4393678abaa7e5b4d70f3ff32b386d`
- 许可证：Arphic-1999 (font derivative); notices retained
- 使用目的：exact graphical-root outlines and license verification; no font installation

### cnman-commit.json

- 保存路径：`env/wubi86/cnman-commit.json`
- 来源 URL：https://api.github.com/repos/CNMan/UnicodeCJK-WuBi/commits/master
- 版本/Commit：`7f55c865a6acfdb1d298d90c42650748f358a3cf`
- 大小：27498 bytes
- SHA-256：`0a0c6ed5b95f0435a686da2a6e44ec457d2d2801486a2d407e58527a87c4d581`
- 许可证：GitHub API metadata; not production data
- 使用目的：pin upstream revision / enumerate resource files

### cnman-tree.json

- 保存路径：`env/wubi86/cnman-tree.json`
- 来源 URL：https://api.github.com/repos/CNMan/UnicodeCJK-WuBi/git/trees/master?recursive=1
- 版本/Commit：`7f55c865a6acfdb1d298d90c42650748f358a3cf`
- 大小：3617 bytes
- SHA-256：`aabbef0adc983d2fbda316276500862a54332da6c771c188e9e49d7c11c44ac1`
- 许可证：GitHub API metadata; not production data
- 使用目的：pin upstream revision / enumerate resource files

### rime-commit.json

- 保存路径：`env/wubi86/rime-commit.json`
- 来源 URL：https://api.github.com/repos/rime/rime-wubi/commits/master
- 版本/Commit：`152a0d3f3efe40cae216d1e3b338242446848d07`
- 大小：5069 bytes
- SHA-256：`8cd8c864c845e967756905c50602cdbb263eac6510b1dcc167d519090894639d`
- 许可证：GitHub API metadata; not production data
- 使用目的：pin upstream revision / enumerate resource files

### pythonwubi-commit.json

- 保存路径：`env/wubi86/pythonwubi-commit.json`
- 来源 URL：https://api.github.com/repos/sfyc23/python-wubi/commits/HEAD
- 版本/Commit：`42c6ac6b4d2285acc7f39594274a4644cdd358ff`
- 大小：52675 bytes
- SHA-256：`462bf46aaf98210cd462498b5a5788df22b1a07e733bb0f94a7f7a13bf3005b5`
- 许可证：GitHub API metadata; not production data
- 使用目的：pin upstream revision / enumerate resource files

### pythonwubi-tree.json

- 保存路径：`env/wubi86/pythonwubi-tree.json`
- 来源 URL：https://api.github.com/repos/sfyc23/python-wubi/git/trees/HEAD?recursive=1
- 版本/Commit：`42c6ac6b4d2285acc7f39594274a4644cdd358ff`
- 大小：5972 bytes
- SHA-256：`57c0f082ff01916bcf1ba24f1e3082c2a7f64b1c9ed08ec0ec2ec0e7cf86a614`
- 许可证：GitHub API metadata; not production data
- 使用目的：pin upstream revision / enumerate resource files

### rootskb-commit.json

- 保存路径：`env/wubi86/rootskb-commit.json`
- 来源 URL：https://api.github.com/repos/riku-ri/wubi-kb/commits/HEAD
- 版本/Commit：`c65590f6a524fa7cb9459d2095be87d22bd839fc`
- 大小：4920 bytes
- SHA-256：`750f8bff7773087df2ba0712e33b517496aef426adfe5ea903307eda5d6b277f`
- 许可证：GitHub API metadata; not production data
- 使用目的：pin upstream revision / enumerate resource files

### rootskb-tree.json

- 保存路径：`env/wubi86/rootskb-tree.json`
- 来源 URL：https://api.github.com/repos/riku-ri/wubi-kb/git/trees/HEAD?recursive=1
- 版本/Commit：`c65590f6a524fa7cb9459d2095be87d22bd839fc`
- 大小：7440 bytes
- SHA-256：`ec13cca1f7dc50b2618e27bda896fe1a586f208d190a9654e170931eb232db10`
- 许可证：GitHub API metadata; not production data
- 使用目的：pin upstream revision / enumerate resource files

### viewer-commit.json

- 保存路径：`env/wubi86/viewer-commit.json`
- 来源 URL：https://api.github.com/repos/sdcb/wubi86-viewer/commits/HEAD
- 版本/Commit：`77c521c2e4e3918fb32af5c0eaef5aec63847939`
- 大小：4518 bytes
- SHA-256：`11e1ca16126d18ee2ea0da961ab2ac1c280cbdcaa8812ac5a7d98471f7b83b16`
- 许可证：GitHub API metadata; not production data
- 使用目的：pin upstream revision / enumerate resource files

### viewer-tree.json

- 保存路径：`env/wubi86/viewer-tree.json`
- 来源 URL：https://api.github.com/repos/sdcb/wubi86-viewer/git/trees/HEAD?recursive=1
- 版本/Commit：`77c521c2e4e3918fb32af5c0eaef5aec63847939`
- 大小：2358363 bytes
- SHA-256：`656089a282abe1f623a1374cfa7f332a2384800d440e9ad7e3111b7e55036dec`
- 许可证：GitHub API metadata; not production data
- 使用目的：pin upstream revision / enumerate resource files

### haifeng-commit.json

- 保存路径：`env/wubi86/haifeng-commit.json`
- 来源 URL：https://api.github.com/repos/definite/ibus-table-chinese/commits/HEAD
- 版本/Commit：`bac13d268344311d48ad9bbc09c7df48b304066f`
- 大小：4133 bytes
- SHA-256：`cf9bfbc4e55df6969fe001649717555cb20fd53855475b56fc281ca931ab5659`
- 许可证：GitHub API metadata; not production data
- 使用目的：pin upstream revision / enumerate resource files

### haifeng-tree.json

- 保存路径：`env/wubi86/haifeng-tree.json`
- 来源 URL：https://api.github.com/repos/definite/ibus-table-chinese/git/trees/HEAD?recursive=1
- 版本/Commit：`bac13d268344311d48ad9bbc09c7df48b304066f`
- 大小：25152 bytes
- SHA-256：`ee75fe3d6210bfed8f669d7fe56b5736f411b5bb070610eb3c9cd332ae591ffa`
- 许可证：GitHub API metadata; not production data
- 使用目的：pin upstream revision / enumerate resource files

### cnman-README.md

- 保存路径：`env/wubi86/cnman-README.md`
- 来源 URL：https://raw.githubusercontent.com/CNMan/UnicodeCJK-WuBi/7f55c865a6acfdb1d298d90c42650748f358a3cf/README.md
- 版本/Commit：`7f55c865a6acfdb1d298d90c42650748f358a3cf`
- 大小：2238 bytes
- SHA-256：`d7e761216d8a2cb196f959f149b3bdbd72e8ac9d81a0088d4ba67e59a9e43feb`
- 许可证：No explicit data license found
- 使用目的：investigation only; rejected for production

### rime-README.md

- 保存路径：`env/wubi86/rime-README.md`
- 来源 URL：https://raw.githubusercontent.com/rime/rime-wubi/152a0d3f3efe40cae216d1e3b338242446848d07/README.md
- 版本/Commit：`152a0d3f3efe40cae216d1e3b338242446848d07`
- 大小：399 bytes
- SHA-256：`05dc392a3f6b2b93ea61411a909380615a2d99d018c6a041c4a6ee96ba74132e`
- 许可证：LGPL-3.0
- 使用目的：cross-check only, not imported into production data

### rime-LICENSE

- 保存路径：`env/wubi86/rime-LICENSE`
- 来源 URL：https://raw.githubusercontent.com/rime/rime-wubi/152a0d3f3efe40cae216d1e3b338242446848d07/LICENSE
- 版本/Commit：`152a0d3f3efe40cae216d1e3b338242446848d07`
- 大小：7651 bytes
- SHA-256：`da7eabb7bafdf7d3ae5e9f223aa5bdc1eece45ac569dc21b3b037520b4464768`
- 许可证：LGPL-3.0
- 使用目的：cross-check only, not imported into production data

### rime-wubi86.dict.yaml

- 保存路径：`env/wubi86/rime-wubi86.dict.yaml`
- 来源 URL：https://raw.githubusercontent.com/rime/rime-wubi/152a0d3f3efe40cae216d1e3b338242446848d07/wubi86.dict.yaml
- 版本/Commit：`152a0d3f3efe40cae216d1e3b338242446848d07`
- 大小：2289545 bytes
- SHA-256：`f833d86b72341fe82e069a425b6625f29ef85f1bc0f34f6fb7975fe514888b5a`
- 许可证：LGPL-3.0
- 使用目的：cross-check only, not imported into production data

## 剩余问题与最终验收

GB2312 核心编码没有缺项。参考字根的全部 257 个形体均可查询和独立核查。单字/词组、词典、Trie、全量测试及静态分析已满足本轮范围。

保留两个明确边界：来源外扩展汉字、未核实的扩展别名与私用区字符不强行编码；已隔离的旧词组快捷码不作为标准公式的编码。扩展源是可追溯来源数据，并不声称每个扩展字经独立国家标准认证。原有 Engine 候选缺陷仍未修复，原有 FAIL 如实保留；这不属于本轮允许修改范围，复现证据已交付。

本轮未新增拼音回归，未修改 FullPinyinSchema、CandidateGenerator、拼音词典、TSF、IPC、插件运行时、Settings Center。只改五笔相关内容及最小测试/构建注册。未提交 Git commit、推送、安装、打包或进入 Phase 2。停止并等待下一步指令。
