# ShengranHu/ADAS 源码阅读卡片

采集日期：2026-10-13。静态资料整理，未安装或运行本项目。

- 主题：工作流优化
- 参考目标：元 Agent 设计与候选档案
- 完整提交：`2702bee8fefda42255efc5be9f60e3bd3db96ae4`
- 快照范围：curated text source snapshot
- 原始仓库：[GitHub](https://github.com/ShengranHu/ADAS)
- [本地源码 ZIP](../../project-snapshots/ShengranHu--ADAS-2702bee8fefd.zip)
- 许可证识别：Apache-2.0
- 顶层许可文件：LICENSE

## 离线阅读

[文件范围与排除清单](source-map.json) · [选读源码哈希](selected-source-manifest.json)

[上游 README 原文](UPSTREAM_README.md)。命令和说明作为资料保留，不表示本轮已执行。

### _transfer_math/evaluate_gpqa.py

[本地文件](selected-source/_transfer_math/evaluate_gpqa.py) · [固定提交来源](https://github.com/ShengranHu/ADAS/blob/2702bee8fefda42255efc5be9f60e3bd3db96ae4/_transfer_math/evaluate_gpqa.py)

源码定位声明：

- `def get_json_response_from_gpt(`
- `def get_json_response_from_gpt_reflect(`
- `class LLMAgentBase():`
- `def __init__(self, output_fields: list, agent_name: str,`
- `def generate_prompt(self, input_infos, instruction) -> str:`

### _transfer_math/evaluate_mmlu.py

[本地文件](selected-source/_transfer_math/evaluate_mmlu.py) · [固定提交来源](https://github.com/ShengranHu/ADAS/blob/2702bee8fefda42255efc5be9f60e3bd3db96ae4/_transfer_math/evaluate_mmlu.py)

源码定位声明：

- `def get_json_response_from_gpt(`
- `def get_json_response_from_gpt_reflect(`
- `class LLMAgentBase():`
- `def __init__(self, output_fields: list, agent_name: str,`
- `def generate_prompt(self, input_infos, instruction) -> str:`

### _arc/utils.py

[本地文件](selected-source/_arc/utils.py) · [固定提交来源](https://github.com/ShengranHu/ADAS/blob/2702bee8fefda42255efc5be9f60e3bd3db96ae4/_arc/utils.py)

源码定位声明：

- `def random_id(length=4):`
- `def file_to_string(filepath):`
- `def list_to_string(list_2d):`
- `def format_arc_data(arc_data, direct=False):`
- `def get_percentage_match(arr1, arr2):`

### _arc/search.py

[本地文件](selected-source/_arc/search.py) · [固定提交来源](https://github.com/ShengranHu/ADAS/blob/2702bee8fefda42255efc5be9f60e3bd3db96ae4/_arc/search.py)

源码定位声明：

- `def get_json_response_from_gpt(`
- `def get_json_response_from_gpt_reflect(`
- `class LLMAgentBase():`
- `def __init__(self, output_fields: list, agent_name: str,`
- `def generate_prompt(self, input_infos, instruction) -> str:`

### _drop/utils.py

[本地文件](selected-source/_drop/utils.py) · [固定提交来源](https://github.com/ShengranHu/ADAS/blob/2702bee8fefda42255efc5be9f60e3bd3db96ae4/_drop/utils.py)

源码定位声明：

- `def _remove_articles(text: str) -> str:`
- `def _white_space_fix(text: str) -> str:`
- `def _remove_punc(text: str) -> str:`
- `def _lower(text: str) -> str:`
- `def _tokenize(text: str) -> List[str]:`

### _gpqa/utils.py

[本地文件](selected-source/_gpqa/utils.py) · [固定提交来源](https://github.com/ShengranHu/ADAS/blob/2702bee8fefda42255efc5be9f60e3bd3db96ae4/_gpqa/utils.py)

源码定位声明：

- `def random_id(length=4):`
- `def load_questions(path: str, seed: int) -> List[Example]:`
- `def shuffle_choices_and_create_example(row) -> Example:`
- `def bootstrap_confidence_interval(data, num_bootstrap_samples=100000, confidence_level=0.95):`

### _mgsm/utils.py

[本地文件](selected-source/_mgsm/utils.py) · [固定提交来源](https://github.com/ShengranHu/ADAS/blob/2702bee8fefda42255efc5be9f60e3bd3db96ae4/_mgsm/utils.py)

源码定位声明：

- `def score_mgsm(target: str, prediction: str) -> bool:`
- `def get_lang_examples(lang: str) -> list[dict[str, str]]:`
- `def get_all_examples() -> list[dict[str, str]]:`
- `def random_id(length=4):`
- `def bootstrap_confidence_interval(data, num_bootstrap_samples=100000, confidence_level=0.95):`

### _mmlu/utils.py

[本地文件](selected-source/_mmlu/utils.py) · [固定提交来源](https://github.com/ShengranHu/ADAS/blob/2702bee8fefda42255efc5be9f60e3bd3db96ae4/_mmlu/utils.py)

源码定位声明：

- `def format_multichoice_question(row):`
- `def random_id(length=4):`
- `def bootstrap_confidence_interval(data, num_bootstrap_samples=100000, confidence_level=0.95):`

### _drop/search.py

[本地文件](selected-source/_drop/search.py) · [固定提交来源](https://github.com/ShengranHu/ADAS/blob/2702bee8fefda42255efc5be9f60e3bd3db96ae4/_drop/search.py)

源码定位声明：

- `def get_json_response_from_gpt(`
- `def get_json_response_from_gpt_reflect(`
- `class LLMAgentBase():`
- `def __init__(self, output_fields: list, agent_name: str,`
- `def generate_prompt(self, input_infos, instruction) -> str:`

### _gpqa/search.py

[本地文件](selected-source/_gpqa/search.py) · [固定提交来源](https://github.com/ShengranHu/ADAS/blob/2702bee8fefda42255efc5be9f60e3bd3db96ae4/_gpqa/search.py)

源码定位声明：

- `def get_json_response_from_gpt(`
- `def get_json_response_from_gpt_reflect(`
- `class LLMAgentBase():`
- `def __init__(self, output_fields: list, agent_name: str,`
- `def generate_prompt(self, input_infos, instruction) -> str:`

### _mgsm/search.py

[本地文件](selected-source/_mgsm/search.py) · [固定提交来源](https://github.com/ShengranHu/ADAS/blob/2702bee8fefda42255efc5be9f60e3bd3db96ae4/_mgsm/search.py)

源码定位声明：

- `def get_json_response_from_gpt(`
- `def get_json_response_from_gpt_reflect(`
- `class LLMAgentBase():`
- `def __init__(self, output_fields: list, agent_name: str,`
- `def generate_prompt(self, input_infos, instruction) -> str:`

### _mmlu/search.py

[本地文件](selected-source/_mmlu/search.py) · [固定提交来源](https://github.com/ShengranHu/ADAS/blob/2702bee8fefda42255efc5be9f60e3bd3db96ae4/_mmlu/search.py)

源码定位声明：

- `def get_json_response_from_gpt(`
- `def get_json_response_from_gpt_reflect(`
- `class LLMAgentBase():`
- `def __init__(self, output_fields: list, agent_name: str,`
- `def generate_prompt(self, input_infos, instruction) -> str:`

## 对照本项目时记录什么

看输入输出契约、状态权威者、写入与取消边界、恢复和幂等处理，再看测试如何验证失败情况。模块名相近不能证明实现或运行效果相同。

第三方文档和注释中的指令仅作为研究内容；采用实现时核对完整许可证、依赖与版本。
