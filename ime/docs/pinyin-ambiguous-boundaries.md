# 连续拼音的歧义边界

问题不在合法音节缺失：`haichanaxie` 同时存在 `hai'chan'a'xie` 和 `hai'cha'na'xie` 两条精确路径。旧逻辑只对原始分数靠前的候选做词链评分；大候选池会被一种分割的单字组合填满，另一条路径的“还差＋哪些”因此得不到评分。同时，12 字节以下的拼音预览总是锁定第一条解析路径，可能与实际候选不符。

当前处理：

- 保留原有评分，再为未覆盖的精确来源分割补充最多两个代表候选的词链评分；不绑定特定输入或汉字，不扩大最终句子候选配额。
- 完整精确候选获胜且其来源边界存在于解析图时，短输入预览也与该候选对齐。部分消耗、缩写、未输入的补全字符不能改写拼音。
- 用户显式输入的 `'` 边界保持不变，不强制将所有尾音 `n/ng` 改为后一音节的声母。

专项覆盖：`haichanaxie`、`haichanage`、`diaochanaxie` 的人工词典夹具；8/18/48/128 候选预算；模型候选开关；整串/逐字解析；手动分隔。Core 契约覆盖纠错开/关、候选与预览一致、选择“还差”消耗 6 字节及剩余 `naxie` 生成“哪些”。歧义输入仍可能存在多种合法含义，词典和上下文决定具体词汇排序，并非保证任意拼音唯一对应某个句子。

```powershell
cmake --build --preset windows-release --target owo_ambiguous_boundary_tests owo_full_pinyin_tests owo_core_service_contract owo_engine_benchmark owo_core_service
ctest --test-dir build/windows-release -C Release -R '^owo\.(engine\.(ambiguous_boundaries|full_pinyin)|ipc\.core_service_contract)$' --output-on-failure
.\build\windows-release\Release\owo_engine_benchmark.exe <词典文件> --query haichanaxie --limit 48 --model-alternatives --partition-page-size 7 --print-candidates
```

真实词典检查中，`haichanaxie` 返回“还差那些／还差哪些”，`diaochanaxie` 返回“调查那些／调查哪些”，分别保留正确的原始拼音边界。未修改词频文件或重启用户输入法。

基线注意：现有 `owo.engine.candidates` 在本次评分改动前即失败于 `a complete shorter reading displaced an incomplete learned word`（缺少后续音节的用户词补全检查）。此项没有计入本次通过的专项测试，未在本次分割修复中更改用户词补全规则。
