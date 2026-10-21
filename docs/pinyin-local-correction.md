# 长拼音的局部纠错

`gongnnegkaiguan` 原先会得到“功能开干／功能开挂”，而不是“功能开关”。原因是长输入允许两处纠错，但路径裁剪优先输出两次编辑的结果；有限的纠错配额被改动正确后缀的路径占满，遗漏了只修复中间 `nneg` 的路径。

修复采用通用规则，不绑定具体拼音或汉字：

- 优先保留一次编辑的修复，同时交替分配一次／两次编辑的路径预算，避免任一类完全占满名额。
- 在同等音节长度的两次编辑路径中，先覆盖不同的首个纠错位置，再补充同一位置的变体；保留长输入两处真实错误的恢复能力。
- 在分区候选链路中，同一原始分割和同一纠正后读音已有完整字典词时，抑制其非用户词的逐字／短词组合，避免组合句分区重新把完整字典词挤到后面。精确匹配、其他读音和用户词不受这条规则影响。
- 不修改用户输入的拼音文本，候选保留原始来源区间和消耗长度；纠错关闭时不注入修复路径。不增加编辑距离、Beam 宽度或返回路径上限。

专项覆盖中间音节换位 `nneg`、`negn`，不同前后缀，8/16/32 条解析预算，8/48 条候选预算和模型备选开关。原有 `wangwangshituobukuaigauanxi` 两处纠错、短拼音纠错与歧义边界回归保留。Core 独立管道测试覆盖基础结果后的纠错补全、原始拼音预览、完整消耗 15 字节以及关闭纠错。

```powershell
cmake --build --preset windows-release --target owo_local_correction_tests owo_full_pinyin_tests owo_ambiguous_boundary_tests owo_core_service_contract owo_core_service
ctest --test-dir build/windows-release -C Release -R '^owo\.(engine\.(local_corrections|full_pinyin|ambiguous_boundaries)|ipc\.core_service_contract)$' --output-on-failure
.\build\windows-release\Release\owo_engine_benchmark.exe .\build\windows-release\rime-ice-cn-2026.06.30-v2.owolx --query gongnnegkaiguan --limit 48 --partition-page-size 7 --model-alternatives --print-candidates
```

本次仅运行专项验证，不执行全量自检，不修改词典、用户词频、配置、插件协议或运行中的已安装服务。真实词典下 `gongnnegkaiguan` 首候选为“功能开关”。多处错误可能同时对应多种合法读音，不保证任意歧义输入唯一对应某个词。
