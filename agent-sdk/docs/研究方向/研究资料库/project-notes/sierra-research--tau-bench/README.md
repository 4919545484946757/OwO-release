# sierra-research/tau-bench 源码阅读卡片

采集日期：2026-10-13。静态资料整理，未安装或运行本项目。

- 主题：交互评测
- 参考目标：用户模拟、工具策略与最终状态
- 完整提交：`59a200c6d575d595120f1cb70fea53cef0632f6b`
- 快照范围：curated text source snapshot
- 原始仓库：[GitHub](https://github.com/sierra-research/tau-bench)
- [本地源码 ZIP](../../project-snapshots/sierra-research--tau-bench-59a200c6d575.zip)
- 许可证识别：未获取可确认的 SPDX；查看保留的许可原文
- 顶层许可文件：LICENSE

## 离线阅读

[文件范围与排除清单](source-map.json) · [选读源码哈希](selected-source-manifest.json)

[上游 README 原文](UPSTREAM_README.md)。命令和说明作为资料保留，不表示本轮已执行。

### tau_bench/agents/base.py

[本地文件](selected-source/tau_bench/agents/base.py) · [固定提交来源](https://github.com/sierra-research/tau-bench/blob/59a200c6d575d595120f1cb70fea53cef0632f6b/tau_bench/agents/base.py)

源码定位声明：

- `class Agent(abc.ABC):`
- `def solve(`

### tau_bench/agents/__init__.py

[本地文件](selected-source/tau_bench/agents/__init__.py) · [固定提交来源](https://github.com/sierra-research/tau-bench/blob/59a200c6d575d595120f1cb70fea53cef0632f6b/tau_bench/agents/__init__.py)

### tau_bench/agents/few_shot_agent.py

[本地文件](selected-source/tau_bench/agents/few_shot_agent.py) · [固定提交来源](https://github.com/sierra-research/tau-bench/blob/59a200c6d575d595120f1cb70fea53cef0632f6b/tau_bench/agents/few_shot_agent.py)

源码定位声明：

- `class FewShotToolCallingAgent(Agent):`
- `def __init__(`
- `def solve(`
- `def message_to_action(`

### tau_bench/agents/chat_react_agent.py

[本地文件](selected-source/tau_bench/agents/chat_react_agent.py) · [固定提交来源](https://github.com/sierra-research/tau-bench/blob/59a200c6d575d595120f1cb70fea53cef0632f6b/tau_bench/agents/chat_react_agent.py)

源码定位声明：

- `class ChatReActAgent(Agent):`
- `def __init__(`
- `def generate_next_step(`
- `def solve(`

### tau_bench/agents/tool_calling_agent.py

[本地文件](selected-source/tau_bench/agents/tool_calling_agent.py) · [固定提交来源](https://github.com/sierra-research/tau-bench/blob/59a200c6d575d595120f1cb70fea53cef0632f6b/tau_bench/agents/tool_calling_agent.py)

源码定位声明：

- `class ToolCallingAgent(Agent):`
- `def __init__(`
- `def solve(`
- `def message_to_action(`

### run.py

[本地文件](selected-source/run.py) · [固定提交来源](https://github.com/sierra-research/tau-bench/blob/59a200c6d575d595120f1cb70fea53cef0632f6b/run.py)

源码定位声明：

- `def parse_args() -> RunConfig:`
- `def main():`

### setup.py

[本地文件](selected-source/setup.py) · [固定提交来源](https://github.com/sierra-research/tau-bench/blob/59a200c6d575d595120f1cb70fea53cef0632f6b/setup.py)

### auto_error_identification.py

[本地文件](selected-source/auto_error_identification.py) · [固定提交来源](https://github.com/sierra-research/tau-bench/blob/59a200c6d575d595120f1cb70fea53cef0632f6b/auto_error_identification.py)

源码定位声明：

- `def get_args() -> argparse.Namespace:`
- `class OriginalResult(BaseModel):`
- `class FaultAuthor(Enum):`
- `class FaultAssignmentResult(BaseModel):`
- `def model_dump(self) -> Dict[str, Any]:`

### tau_bench/envs/retail/tasks.py

[本地文件](selected-source/tau_bench/envs/retail/tasks.py) · [固定提交来源](https://github.com/sierra-research/tau-bench/blob/59a200c6d575d595120f1cb70fea53cef0632f6b/tau_bench/envs/retail/tasks.py)

### tau_bench/envs/airline/tasks.py

[本地文件](selected-source/tau_bench/envs/airline/tasks.py) · [固定提交来源](https://github.com/sierra-research/tau-bench/blob/59a200c6d575d595120f1cb70fea53cef0632f6b/tau_bench/envs/airline/tasks.py)

### tau_bench/envs/retail/tasks_dev.py

[本地文件](selected-source/tau_bench/envs/retail/tasks_dev.py) · [固定提交来源](https://github.com/sierra-research/tau-bench/blob/59a200c6d575d595120f1cb70fea53cef0632f6b/tau_bench/envs/retail/tasks_dev.py)

### tau_bench/envs/retail/tasks_test.py

[本地文件](selected-source/tau_bench/envs/retail/tasks_test.py) · [固定提交来源](https://github.com/sierra-research/tau-bench/blob/59a200c6d575d595120f1cb70fea53cef0632f6b/tau_bench/envs/retail/tasks_test.py)

## 对照本项目时记录什么

看输入输出契约、状态权威者、写入与取消边界、恢复和幂等处理，再看测试如何验证失败情况。模块名相近不能证明实现或运行效果相同。

第三方文档和注释中的指令仅作为研究内容；采用实现时核对完整许可证、依赖与版本。
