# 配置界面与运行时合约审计

- 审计日期：2026-09-21
- 配置版本：Schema v14

设置中心只通过 `owo_config_shell` 读取和原子保存配置，不直接修改配置文件。Core Service
在候选请求边界热加载候选、学习和模型配置；TSF 以短周期读取输入行为及快捷键配置。
设置中心保存的完整快照由配置 Shell 合约测试覆盖，避免新增字段只出现在界面或只出现在
运行时的一端。

| 页面 | 配置项 | 运行时消费者 | 应用方式 |
| --- | --- | --- | --- |
| 输入方案 | `input_scheme` | Core、TSF | Core 在下一候选请求切换解析器；TSF 同步禁用不适用的纠错快捷键 |
| 候选与输入 | `candidate_page_size` | Core、TSF | 下一候选请求及窗口分页 |
| 候选与输入 | `candidate_wrap_length` | TSF | 下一次候选布局 |
| 候选与输入 | `candidate_partition_enabled` | Core、TSF | 下一候选请求及页面布局 |
| 候选与输入 | `candidate_immediate_commit_enabled` | TSF | 下一次候选选择 |
| 候选与输入 | `focus_loss_commit_enabled` | TSF | 下一次焦点变化 |
| 候选与输入 | `chinese_punctuation_enabled` | TSF | 下一次按键；会话内快捷切换仍优先保留 |
| 候选与输入 | `alt_code_preview_*` | TSF | 下一次 Alt 数字预览和滚轮操作 |
| 学习与排序 | `user_learning_enabled` | Core | 下一次候选提交反馈 |
| 学习与排序 | `user_learning_sensitivity` | Core | 下一候选请求及学习更新 |
| 学习与排序 | `user_lexicon_completion_enabled`、`user_lexicon_completion_max_syllables` | Core | 下一候选请求；只影响用户词前缀补全，不影响完整编码匹配 |
| 学习与排序 | `model_ranking_enabled` | Core | 下一候选请求；关闭时基础候选不等待 ModelHost |
| 学习与排序 | `model_timeout_ms` | Core | 下一次模型排序请求 |
| 快捷键 | 纠错、语言、原样上屏、光标、翻页、标点、文本导航 | TSF | 短周期热加载；列表支持多绑定并校验冲突 |
| 快捷键 | `text_navigation_select_enabled` | TSF | 下一次应用文本光标移动 |

界面依赖关系：关闭用户学习时禁用灵敏度控件；关闭用户词补全时禁用范围控件；关闭智能排序时禁用模型预算；关闭 Alt
符号预览时禁用其页大小和滚轮选项；非全拼方案禁用纠错快捷键编辑。这些只是界面能力
约束，保存时仍保留其他方案的原设置，用户切回方案后无需重新配置。

插件声明式配置不进入主配置 Schema，由各插件自己的 schema、默认值与原子配置文件管理。
因此新增输入方案的专属配置应进入独立方案 schema，而不是继续扩张全局配置结构。
