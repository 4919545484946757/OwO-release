# agno-agi/agno 源码阅读卡片

采集日期：2026-10-13。静态资料整理，未安装或运行本项目。

- 主题：Agent框架
- 参考目标：Agent、Team、Workflow 的组合边界
- 完整提交：`5b854c59730c7a5c32c24afbf3a59dbef5e90308`
- 快照范围：curated text source snapshot
- 原始仓库：[GitHub](https://github.com/agno-agi/agno)
- [本地源码 ZIP](../../project-snapshots/agno-agi--agno-5b854c59730c.zip)
- 许可证识别：Apache-2.0
- 顶层许可文件：LICENSE

## 离线阅读

[文件范围与排除清单](source-map.json) · [选读源码哈希](selected-source-manifest.json)

[上游 README 原文](UPSTREAM_README.md)。命令和说明作为资料保留，不表示本轮已执行。

### cookbook/02_agents/18_checkpointing/02_tool_error_persistence.py

[本地文件](selected-source/cookbook/02_agents/18_checkpointing/02_tool_error_persistence.py) · [固定提交来源](https://github.com/agno-agi/agno/blob/5b854c59730c7a5c32c24afbf3a59dbef5e90308/cookbook/02_agents/18_checkpointing/02_tool_error_persistence.py)

源码定位声明：

- `def broken_tool(query: str) -> str:`
- `async def scenario_a_tool_error() -> None:`
- `async def scenario_b_model_call_fails() -> str:`
- `async def scenario_c_continue_failed_run(failed_run_id: str) -> None:`
- `async def main() -> None:`

### cookbook/04_workflows/08_human_in_the_loop/executor_hitl/01_agent_confirmation.py

[本地文件](selected-source/cookbook/04_workflows/08_human_in_the_loop/executor_hitl/01_agent_confirmation.py) · [固定提交来源](https://github.com/agno-agi/agno/blob/5b854c59730c7a5c32c24afbf3a59dbef5e90308/cookbook/04_workflows/08_human_in_the_loop/executor_hitl/01_agent_confirmation.py)

源码定位声明：

- `def get_the_weather(city: str) -> str:`
- `def save_result(step_input: StepInput) -> StepOutput:`

### cookbook/04_workflows/08_human_in_the_loop/executor_hitl/08_agent_user_input_step.py

[本地文件](selected-source/cookbook/04_workflows/08_human_in_the_loop/executor_hitl/08_agent_user_input_step.py) · [固定提交来源](https://github.com/agno-agi/agno/blob/5b854c59730c7a5c32c24afbf3a59dbef5e90308/cookbook/04_workflows/08_human_in_the_loop/executor_hitl/08_agent_user_input_step.py)

源码定位声明：

- `def send_money(amount: float, recipient: str, note: str) -> str:`
- `def save_result(step_input: StepInput) -> StepOutput:`

### cookbook/04_workflows/08_human_in_the_loop/executor_hitl/02_agent_confirmation_stream.py

[本地文件](selected-source/cookbook/04_workflows/08_human_in_the_loop/executor_hitl/02_agent_confirmation_stream.py) · [固定提交来源](https://github.com/agno-agi/agno/blob/5b854c59730c7a5c32c24afbf3a59dbef5e90308/cookbook/04_workflows/08_human_in_the_loop/executor_hitl/02_agent_confirmation_stream.py)

源码定位声明：

- `def get_the_weather(city: str) -> str:`
- `def save_result(step_input: StepInput) -> StepOutput:`

### cookbook/04_workflows/08_human_in_the_loop/executor_hitl/05_agent_confirmation_in_loop_step.py

[本地文件](selected-source/cookbook/04_workflows/08_human_in_the_loop/executor_hitl/05_agent_confirmation_in_loop_step.py) · [固定提交来源](https://github.com/agno-agi/agno/blob/5b854c59730c7a5c32c24afbf3a59dbef5e90308/cookbook/04_workflows/08_human_in_the_loop/executor_hitl/05_agent_confirmation_in_loop_step.py)

源码定位声明：

- `def run_batch_analysis(batch_id: str) -> str:`
- `def gather_data(step_input: StepInput) -> StepOutput:`
- `def generate_report(step_input: StepInput) -> StepOutput:`

### cookbook/04_workflows/08_human_in_the_loop/executor_hitl/07_agent_confirmation_in_router_step.py

[本地文件](selected-source/cookbook/04_workflows/08_human_in_the_loop/executor_hitl/07_agent_confirmation_in_router_step.py) · [固定提交来源](https://github.com/agno-agi/agno/blob/5b854c59730c7a5c32c24afbf3a59dbef5e90308/cookbook/04_workflows/08_human_in_the_loop/executor_hitl/07_agent_confirmation_in_router_step.py)

源码定位声明：

- `def run_deep_analysis(subject: str) -> str:`
- `def gather_data(step_input: StepInput) -> StepOutput:`
- `def fast_check(step_input: StepInput) -> StepOutput:`
- `def generate_report(step_input: StepInput) -> StepOutput:`
- `def always_deep_analysis(step_input: StepInput) -> str:`

### cookbook/04_workflows/08_human_in_the_loop/executor_hitl/04_agent_confirmation_in_condition_step.py

[本地文件](selected-source/cookbook/04_workflows/08_human_in_the_loop/executor_hitl/04_agent_confirmation_in_condition_step.py) · [固定提交来源](https://github.com/agno-agi/agno/blob/5b854c59730c7a5c32c24afbf3a59dbef5e90308/cookbook/04_workflows/08_human_in_the_loop/executor_hitl/04_agent_confirmation_in_condition_step.py)

源码定位声明：

- `def run_detailed_analysis(topic: str) -> str:`
- `def gather_data(step_input: StepInput) -> StepOutput:`
- `def quick_summary(step_input: StepInput) -> StepOutput:`
- `def generate_report(step_input: StepInput) -> StepOutput:`

### cookbook/04_workflows/08_human_in_the_loop/executor_hitl/06_agent_confirmation_in_steps_container.py

[本地文件](selected-source/cookbook/04_workflows/08_human_in_the_loop/executor_hitl/06_agent_confirmation_in_steps_container.py) · [固定提交来源](https://github.com/agno-agi/agno/blob/5b854c59730c7a5c32c24afbf3a59dbef5e90308/cookbook/04_workflows/08_human_in_the_loop/executor_hitl/06_agent_confirmation_in_steps_container.py)

源码定位声明：

- `def run_deep_scan(target: str) -> str:`
- `def gather_data(step_input: StepInput) -> StepOutput:`
- `def preprocess(step_input: StepInput) -> StepOutput:`
- `def postprocess(step_input: StepInput) -> StepOutput:`
- `def generate_report(step_input: StepInput) -> StepOutput:`

### cookbook/11_memory/02_agentic_memory.py

[本地文件](selected-source/cookbook/11_memory/02_agentic_memory.py) · [固定提交来源](https://github.com/agno-agi/agno/blob/5b854c59730c7a5c32c24afbf3a59dbef5e90308/cookbook/11_memory/02_agentic_memory.py)

### cookbook/11_memory/01_agent_with_memory.py

[本地文件](selected-source/cookbook/11_memory/01_agent_with_memory.py) · [固定提交来源](https://github.com/agno-agi/agno/blob/5b854c59730c7a5c32c24afbf3a59dbef5e90308/cookbook/11_memory/01_agent_with_memory.py)

### cookbook/00_quickstart/agent_with_memory.py

[本地文件](selected-source/cookbook/00_quickstart/agent_with_memory.py) · [固定提交来源](https://github.com/agno-agi/agno/blob/5b854c59730c7a5c32c24afbf3a59dbef5e90308/cookbook/00_quickstart/agent_with_memory.py)

### cookbook/11_memory/03_agents_share_memory.py

[本地文件](selected-source/cookbook/11_memory/03_agents_share_memory.py) · [固定提交来源](https://github.com/agno-agi/agno/blob/5b854c59730c7a5c32c24afbf3a59dbef5e90308/cookbook/11_memory/03_agents_share_memory.py)

## 对照本项目时记录什么

看输入输出契约、状态权威者、写入与取消边界、恢复和幂等处理，再看测试如何验证失败情况。模块名相近不能证明实现或运行效果相同。

第三方文档和注释中的指令仅作为研究内容；采用实现时核对完整许可证、依赖与版本。
