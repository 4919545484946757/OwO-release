# 局部纠错与缩写证据

## 修复内容

`gongnnegjiekou` 原本已有 `gong/neng/jie/kou` 的交换字母纠错路径，
但 Core 将 `gong/n/ne/g/jie/kou` 的单字补全组合误判为可靠缩写证据。
当这些组合及前缀候选填满第一页时，按需纠错被跳过。

现在，多音节组合中的两个及以上缩写必须有多字字典词覆盖，才能阻止
按需纠错。仅靠几个独立单字补全，不再视为词汇证据。完整字典词、
精确拼音、短尾部补全仍保留原规则；关闭纠错时不会生成修复路径。

没有增加特定输入串或候选文本映射，也未扩大解析/搜索预算。

## 验证

- 新增 `owo.ipc.local_correction_contract`，用故意缺少整句词条、包含大量
  高频单字的隔离词典覆盖局部纠错、延后细化、开关切换和展开列表。
- 原门控下 `gongnnegjiekou`、`gongnegnjiekou` 回归失败，修复后通过。
- 保留 `bugd`、`wsm`、`rwky` 及正确完整拼音的字典候选。
- 真实只读词典中，`gongnnegjiekou` 首候选恢复为“功能接口”，预览为
  `gong'nneg'jie'kou`，候选消耗完整 14 字节输入。
- 本机更新后的 Core 返回“功能接口”为首候选；日常 ModelHost 的智能排序
  细化完成后首候选保持一致。未向真实学习记录写入测试选择。
- `owo.engine.full_pinyin`、`owo.engine.local_corrections`、
  `owo.ipc.local_correction_contract`、`owo.ipc.core_config_contract` 通过。
- 较大测试 `owo.ipc.core_service_contract` 的既有
  `long suffix/local bridge Core contract failed` 断言仍失败；恢复原门控
  构建也复现相同错误（退出码 9），非本次引入。未修改该无关学习断言。

本次未运行全量自检，未改变用户词库或智能排序配置。
