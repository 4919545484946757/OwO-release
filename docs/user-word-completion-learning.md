# 用户词补全与学习映射隔离

问题：曾选中长补全候选后，TSF 会反馈“实际消耗的短拼音＋完整候选文本”。Core 过去只检查输入能否解析，便把这对数据写入精确用户词库。例如完整用户词 `xiufugaiwenti → 修复该问题` 补全并被选择后，又留下 `xiufu → 修复该问题`；关闭补全也无法阻止后者被精确召回。

当前修复：

- Core 学习入口检查输出字数与来源解析路径的音节数。无法对应时只记录全局使用频次，不创建短拼音精确映射或短拼音上下文映射。正常提交先检查单路径，只有不匹配才补查最多 32 条无纠错路径。
- 候选生成对精确用户词采用相同检查，忽略历史遗留的不匹配映射。检查所有精确分割，`xian → 西安` 仍使用 `xi'an` 来源，而不是错误绑定到单音节 `xian`。
- 原有用户词数据不删除、不迁移；完整输入、不完整末音节和用户显式配置允许的长补全仍可使用。补全选择的使用频次仍保留，但不会升级成短输入的精确词。
- 该限制由解析结果的 `phonetic_assistance` 能力决定；编码类输入方案不要求“一音节一字”，其短编码整句映射不受影响。IPC、用户数据格式和 TSF 无需变化。

这里的检查是音节／字数对应性约束，不是逐字读音校验；它不能证明所有同长度历史映射的读音正确。开启长补全后，允许范围内仍会出现长词，这是显式补全而非精确用户词。

专项测试：旧污染记录、补全开关和 0/2/3 音节上限、完整与末音节不完整输入、`xian` 多分割、编码输入、候选分区开关、8/48 候选预算、模型备选开关；Core 独立管道验证反馈之后的落盘数据、上下文及全局频次。

```powershell
cmake --build --preset windows-release --target owo_user_lexicon_alignment_tests owo_user_frequency_tests owo_core_service_contract owo_core_service
ctest --test-dir build/windows-release -C Release -R '^owo\.(engine\.(user_lexicon_alignment|user_frequency)|ipc\.core_service_contract)$' --output-on-failure
```

本次不执行全量自检，不清空用户词库，不修改用户配置。部署仅替换 Core，并先保留上一版二进制以便回退。
