# uiuc-kang-lab/InjecAgent 源码阅读卡片

采集日期：2026-10-13。静态资料整理，未安装或运行本项目。

- 主题：安全评测
- 参考目标：工具输出注入、攻击样本与评估脚本
- 完整提交：`f19c9f2c79a41046eb13c03c51a24c567a8ffa07`
- 快照范围：curated text source snapshot
- 原始仓库：[GitHub](https://github.com/uiuc-kang-lab/InjecAgent)
- [本地源码 ZIP](../../project-snapshots/uiuc-kang-lab--InjecAgent-f19c9f2c79a4.zip)
- 许可证识别：未获取可确认的 SPDX；查看保留的许可原文
- 顶层许可文件：未识别；不得据此推断可自由复制

## 离线阅读

[文件范围与排除清单](source-map.json) · [选读源码哈希](selected-source-manifest.json)

[上游 README 原文](UPSTREAM_README.md)。命令和说明作为资料保留，不表示本轮已执行。

### src/evaluate_prompted_agent.py

[本地文件](selected-source/src/evaluate_prompted_agent.py) · [固定提交来源](https://github.com/uiuc-kang-lab/InjecAgent/blob/f19c9f2c79a41046eb13c03c51a24c567a8ffa07/src/evaluate_prompted_agent.py)

源码定位声明：

- `def predict_one_case(system_prompt, user_prompt, params, item, model_class, tool_dict, step_2=False):`
- `def main(params):`

### src/evaluate_finetuned_agent.py

[本地文件](selected-source/src/evaluate_finetuned_agent.py) · [固定提交来源](https://github.com/uiuc-kang-lab/InjecAgent/blob/f19c9f2c79a41046eb13c03c51a24c567a8ffa07/src/evaluate_finetuned_agent.py)

源码定位声明：

- `class Function(BaseModel):`
- `class ChatCompletionMessageToolCall(BaseModel):`
- `class FunctionCall(BaseModel):`
- `class ChatCompletionMessage(BaseModel):`
- `def predict_one_case(system_prompt, user_prompt, params, item, tool_dict, step_2=False):`

### src/prompts/agent_prompts.py

[本地文件](selected-source/src/prompts/agent_prompts.py) · [固定提交来源](https://github.com/uiuc-kang-lab/InjecAgent/blob/f19c9f2c79a41046eb13c03c51a24c567a8ffa07/src/prompts/agent_prompts.py)

### src/utils.py

[本地文件](selected-source/src/utils.py) · [固定提交来源](https://github.com/uiuc-kang-lab/InjecAgent/blob/f19c9f2c79a41046eb13c03c51a24c567a8ffa07/src/utils.py)

源码定位声明：

- `def get_score(output_files):`
- `def transform_tool_format_gpt(tool):`
- `def get_tool_dict(gpt_format=False):`
- `def extract_content(text):`
- `def get_simulated_attacker_tool_response(attacker_tool, attacker_tool_parameters, tool_dict):`

### src/models.py

[本地文件](selected-source/src/models.py) · [固定提交来源](https://github.com/uiuc-kang-lab/InjecAgent/blob/f19c9f2c79a41046eb13c03c51a24c567a8ffa07/src/models.py)

源码定位声明：

- `class BaseModel:`
- `def __init__(self):`
- `def prepare_input(self, sys_prompt,  user_prompt_filled):`
- `def call_model(self, model_input):`
- `class ClaudeModel(BaseModel):`

### src/params.py

[本地文件](selected-source/src/params.py) · [固定提交来源](https://github.com/uiuc-kang-lab/InjecAgent/blob/f19c9f2c79a41046eb13c03c51a24c567a8ffa07/src/params.py)

源码定位声明：

- `def define_arguments_prompted(parser):`
- `def define_arguments_finetuned(parser):`
- `def parse_arguments(agent_type=None):`

### src/output_parsing.py

[本地文件](selected-source/src/output_parsing.py) · [固定提交来源](https://github.com/uiuc-kang-lab/InjecAgent/blob/f19c9f2c79a41046eb13c03c51a24c567a8ffa07/src/output_parsing.py)

源码定位声明：

- `def output_parser(paragraph):`
- `def truncate_at_marker(text, marker="[/INST]"):`
- `def detect_overly_long_sequences(text):`
- `def detect_repetitive_language(text, n_gram = 8, n_rep = 10):`
- `def evaluate_output_prompted(output, target_tool, attacker_tools, user_tool, attacker_instruction, word_num=500):`

### src/prompts/prompt_template.py

[本地文件](selected-source/src/prompts/prompt_template.py) · [固定提交来源](https://github.com/uiuc-kang-lab/InjecAgent/blob/f19c9f2c79a41046eb13c03c51a24c567a8ffa07/src/prompts/prompt_template.py)

### src/prompts/generation_prompts.py

[本地文件](selected-source/src/prompts/generation_prompts.py) · [固定提交来源](https://github.com/uiuc-kang-lab/InjecAgent/blob/f19c9f2c79a41046eb13c03c51a24c567a8ffa07/src/prompts/generation_prompts.py)

## 对照本项目时记录什么

看输入输出契约、状态权威者、写入与取消边界、恢复和幂等处理，再看测试如何验证失败情况。模块名相近不能证明实现或运行效果相同。

第三方文档和注释中的指令仅作为研究内容；采用实现时核对完整许可证、依赖与版本。
