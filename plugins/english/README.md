# OwO 英文输入

这是独立的进程插件，与系统键盘透传方案“纯净键盘”不同。在「插件管理器」导入包，按风险提示单独授权 `input.scheme`，安装后手动启用插件，再到「输入方案」开启「英文输入」。需要配置时，从顶部方案下拉菜单选「英文输入」；「候选与显示」「词频学习」「词库管理器」「快捷键」只编辑该方案自己的设置。

- 单词支持前缀补全、大小写、撇号和连字符。原样输入总在首页第一位。数字键或点击候选才会应用补全/拼写建议；空格和普通标点会原样提交当前输入并保留分隔符，Enter 原样提交当前输入，Esc 取消。拼写建议和补全可独立关闭。
- 手动词库为 UTF-8 TSV：每行一个英文单词，可在 Tab 后加 1–1000 的权重；可以从词库管理器导入、导出或移除。现阶段仅为 ASCII 英文词提供候选；非 ASCII 字符按原样输入。
- 只有明确选择且成功上屏的候选才会进入英文学习词库；直接按空格或 Enter 不会记录。可关闭学习、单独导出或清除学习记录。导出文件可能包含私人词汇。
- 输入法进程不联网，不读取应用周围文本、剪贴板或密码框。仅显式下载字典时，由设置中心从固定 HTTPS 地址获取并校验字节数与 SHA-256。下载的词库可单独移除，不修改全拼词库或词频。

## 词库来源

| 词库 | 用途 | 许可与出处 |
| --- | --- | --- |
| wordfreq 3.1.1 English Large | 随包默认频率 | [wordfreq 3.1.1](https://pypi.org/project/wordfreq/3.1.1/)，数据 [CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/)；[上游许可和语料署名](assets/NOTICE.wordfreq.md) |
| LibreOffice 美式英语 | 可选词表 | [SCOWL 及上游许可](https://github.com/LibreOffice/dictionaries/blob/4fa94195b8136364dd40bf2b0366a0fe32058899/en/README_en_US.txt) |
| LibreOffice 英式英语 | 可选词表 | [上游说明与 LGPL 许可](https://github.com/LibreOffice/dictionaries/blob/e7f163feb2beaf526135132d8716e68e19d2716e/en/README_en_GB.txt) |

wordfreq 数据保留上游 cBpack 内容，仅去除 gzip 外层；没有转换为 CSV。OwO 的本地候选读取其中可用的英文单词，不等同于 wordfreq Python 模块的分词 API。两份 LibreOffice 下载词库仅使用显式词条，不执行 Hunspell 词形规则。它们提供词汇覆盖，不冒充词频来源。

## 构建

```powershell
cmake --build --preset windows-release --target owo_english_package
./scripts/package_plugins.ps1 -PluginIds owo.plugin.english
```

构建无需联网或 Python。仅在复核原始数据时，用 `tools/english/extract_wordfreq.py` 从固定 SHA-256 的官方 wheel 重现随包数据和署名。
