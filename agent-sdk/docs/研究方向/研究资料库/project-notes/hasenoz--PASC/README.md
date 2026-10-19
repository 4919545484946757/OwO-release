# hasenoz/PASC 源码阅读卡片

采集日期：2026-10-13。静态资料整理，未安装或运行本项目。

- 主题：多Agent协作
- 参考目标：Replication package for "Coordinating Agents on a Shared Git Workspace: An Empirical Study of Action-Space Observation for Agentic Software Engineering". Includes the PASC system source, per-configuration predictions and evaluation outputs on SWE-Bench Pro, and the analysis scripts that produce the paper's tables.
- 完整提交：`ae1ee633e91cdf3716bb6bfc809a54df0977486e`
- 快照范围：source, documentation, analysis scripts; excludes FINAL_results/
- 原始仓库：[GitHub](https://github.com/hasenoz/PASC)
- [本地源码 ZIP](../../project-snapshots/PASC-source-ae1ee633e91c.zip)
- 许可证识别：MIT
- 顶层许可文件：LICENSE, THIRD_PARTY_LICENSES

## 离线阅读

[文件范围与排除清单](source-map.json) · [选读源码哈希](selected-source-manifest.json)

[上游 README 原文](UPSTREAM_README.md)。命令和说明作为资料保留，不表示本轮已执行。

### mini-swe-agent/src/minisweagent/__init__.py

[本地文件](selected-source/mini-swe-agent/src/minisweagent/__init__.py) · [固定提交来源](https://github.com/hasenoz/PASC/blob/ae1ee633e91cdf3716bb6bfc809a54df0977486e/mini-swe-agent/src/minisweagent/__init__.py)

源码定位声明：

- `class Model(Protocol):`
- `def query(self, messages: list[dict[str, str]], **kwargs) -> dict: ...`
- `def query_text(self, messages: list[dict[str, Any]], **kwargs) -> str: ...`
- `def format_message(self, **kwargs) -> dict: ...`
- `def format_observation_messages(`

### mini-swe-agent/src/minisweagent/__main__.py

[本地文件](selected-source/mini-swe-agent/src/minisweagent/__main__.py) · [固定提交来源](https://github.com/hasenoz/PASC/blob/ae1ee633e91cdf3716bb6bfc809a54df0977486e/mini-swe-agent/src/minisweagent/__main__.py)

### mini-swe-agent/src/minisweagent/exceptions.py

[本地文件](selected-source/mini-swe-agent/src/minisweagent/exceptions.py) · [固定提交来源](https://github.com/hasenoz/PASC/blob/ae1ee633e91cdf3716bb6bfc809a54df0977486e/mini-swe-agent/src/minisweagent/exceptions.py)

源码定位声明：

- `class InterruptAgentFlow(Exception):`
- `def __init__(self, *messages: dict):`
- `class Submitted(InterruptAgentFlow):`
- `class LimitsExceeded(InterruptAgentFlow):`
- `class UserInterruption(InterruptAgentFlow):`

### mini-swe-agent/src/minisweagent/run/mini.py

[本地文件](selected-source/mini-swe-agent/src/minisweagent/run/mini.py) · [固定提交来源](https://github.com/hasenoz/PASC/blob/ae1ee633e91cdf3716bb6bfc809a54df0977486e/mini-swe-agent/src/minisweagent/run/mini.py)

源码定位声明：

- `def main(`

### mini-swe-agent/src/minisweagent/utils/log.py

[本地文件](selected-source/mini-swe-agent/src/minisweagent/utils/log.py) · [固定提交来源](https://github.com/hasenoz/PASC/blob/ae1ee633e91cdf3716bb6bfc809a54df0977486e/mini-swe-agent/src/minisweagent/utils/log.py)

源码定位声明：

- `def _setup_root_logger() -> None:`
- `def add_file_handler(path: Path | str, level: int = logging.DEBUG, *, print_path: bool = True) -> None:`

### mini-swe-agent/src/minisweagent/run/__init__.py

[本地文件](selected-source/mini-swe-agent/src/minisweagent/run/__init__.py) · [固定提交来源](https://github.com/hasenoz/PASC/blob/ae1ee633e91cdf3716bb6bfc809a54df0977486e/mini-swe-agent/src/minisweagent/run/__init__.py)

### mini-swe-agent/src/minisweagent/agents/default.py

[本地文件](selected-source/mini-swe-agent/src/minisweagent/agents/default.py) · [固定提交来源](https://github.com/hasenoz/PASC/blob/ae1ee633e91cdf3716bb6bfc809a54df0977486e/mini-swe-agent/src/minisweagent/agents/default.py)

源码定位声明：

- `class AgentConfig(BaseModel):`
- `class DefaultAgent:`
- `def __init__(self, model: Model, env: Environment, *, config_class: type = AgentConfig, **kwargs):`
- `def get_template_vars(self, **kwargs) -> dict:`
- `def _render_template(self, template: str) -> str:`

### mini-swe-agent/src/minisweagent/utils/__init__.py

[本地文件](selected-source/mini-swe-agent/src/minisweagent/utils/__init__.py) · [固定提交来源](https://github.com/hasenoz/PASC/blob/ae1ee633e91cdf3716bb6bfc809a54df0977486e/mini-swe-agent/src/minisweagent/utils/__init__.py)

### mini-swe-agent/src/minisweagent/agents/__init__.py

[本地文件](selected-source/mini-swe-agent/src/minisweagent/agents/__init__.py) · [固定提交来源](https://github.com/hasenoz/PASC/blob/ae1ee633e91cdf3716bb6bfc809a54df0977486e/mini-swe-agent/src/minisweagent/agents/__init__.py)

源码定位声明：

- `def get_agent_class(spec: str) -> type[Agent]:`
- `def get_agent(model: Model, env: Environment, config: dict, *, default_type: str = "") -> Agent:`

### mini-swe-agent/src/minisweagent/config/__init__.py

[本地文件](selected-source/mini-swe-agent/src/minisweagent/config/__init__.py) · [固定提交来源](https://github.com/hasenoz/PASC/blob/ae1ee633e91cdf3716bb6bfc809a54df0977486e/mini-swe-agent/src/minisweagent/config/__init__.py)

源码定位声明：

- `def get_config_path(config_spec: str | Path) -> Path:`
- `def _key_value_spec_to_nested_dict(config_spec: str) -> dict:`
- `def get_config_from_spec(config_spec: str | Path) -> dict:`

### mini-swe-agent/src/minisweagent/models/__init__.py

[本地文件](selected-source/mini-swe-agent/src/minisweagent/models/__init__.py) · [固定提交来源](https://github.com/hasenoz/PASC/blob/ae1ee633e91cdf3716bb6bfc809a54df0977486e/mini-swe-agent/src/minisweagent/models/__init__.py)

源码定位声明：

- `class GlobalModelStats:`
- `def __init__(self):`
- `def add(self, cost: float) -> None:`
- `def cost(self) -> float:`
- `def n_calls(self) -> int:`

### mini-swe-agent/src/minisweagent/run/hello_world.py

[本地文件](selected-source/mini-swe-agent/src/minisweagent/run/hello_world.py) · [固定提交来源](https://github.com/hasenoz/PASC/blob/ae1ee633e91cdf3716bb6bfc809a54df0977486e/mini-swe-agent/src/minisweagent/run/hello_world.py)

源码定位声明：

- `def main(`

## 对照本项目时记录什么

看输入输出契约、状态权威者、写入与取消边界、恢复和幂等处理，再看测试如何验证失败情况。模块名相近不能证明实现或运行效果相同。

第三方文档和注释中的指令仅作为研究内容；采用实现时核对完整许可证、依赖与版本。
