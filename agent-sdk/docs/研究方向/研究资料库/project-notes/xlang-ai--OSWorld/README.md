# xlang-ai/OSWorld 源码阅读卡片

采集日期：2026-10-13。静态资料整理，未安装或运行本项目。

- 主题：桌面自动化评测
- 参考目标：任务环境、桌面动作与最终状态检查
- 完整提交：`b138d348256078fa634fc3b73567a7337c793e6b`
- 快照范围：curated text source snapshot
- 原始仓库：[GitHub](https://github.com/xlang-ai/OSWorld)
- [本地源码 ZIP](../../project-snapshots/xlang-ai--OSWorld-b138d3482560.zip)
- 许可证识别：未获取可确认的 SPDX；查看保留的许可原文
- 顶层许可文件：LICENSE

## 离线阅读

[文件范围与排除清单](source-map.json) · [选读源码哈希](selected-source-manifest.json)

[上游 README 原文](UPSTREAM_README.md)。命令和说明作为资料保留，不表示本轮已执行。

### mm_agents/uipath/memory.py

[本地文件](selected-source/mm_agents/uipath/memory.py) · [固定提交来源](https://github.com/xlang-ai/OSWorld/blob/b138d348256078fa634fc3b73567a7337c793e6b/mm_agents/uipath/memory.py)

源码定位声明：

- `class EnumMemoryOperationType(str, Enum):`
- `class MemoryOperation(object):`
- `def __init__(`
- `def from_dict(data: dict) -> "MemoryOperation":`
- `class ShortTermMemoryManager:`

### mm_agents/pointer/task_logger.py

[本地文件](selected-source/mm_agents/pointer/task_logger.py) · [固定提交来源](https://github.com/xlang-ai/OSWorld/blob/b138d348256078fa634fc3b73567a7337c793e6b/mm_agents/pointer/task_logger.py)

源码定位声明：

- `class _AgentLoggerAdapter(logging.LoggerAdapter):`
- `def process(self, msg, kwargs):  # type: ignore[override]`
- `class _DefaultAgentFilter(logging.Filter):`
- `def filter(self, record: logging.LogRecord) -> bool:`
- `def setup_task_logger(`

### mm_agents/dart_gui/task_loader.py

[本地文件](selected-source/mm_agents/dart_gui/task_loader.py) · [固定提交来源](https://github.com/xlang-ai/OSWorld/blob/b138d348256078fa634fc3b73567a7337c793e6b/mm_agents/dart_gui/task_loader.py)

源码定位声明：

- `class TaskLoader:`
- `def __init__(self, task_cfg: DictConfig, storage_root):`
- `def poll_for_tasks(self) -> List[Dict]:`
- `def _maybe_refresh_dataset_bak(self):`
- `def _maybe_refresh_dataset(self):`

### mm_agents/pointer/agent_executor.py

[本地文件](selected-source/mm_agents/pointer/agent_executor.py) · [固定提交来源](https://github.com/xlang-ai/OSWorld/blob/b138d348256078fa634fc3b73567a7337c793e6b/mm_agents/pointer/agent_executor.py)

源码定位声明：

- `def _handle_run_python(executor: Executor, inp: dict) -> tuple[str | None, ToolResult]:`
- `def _handle_run_bash(executor: Executor, inp: dict) -> tuple[str | None, ToolResult]:`
- `def _handle_read_file(executor: Executor, inp: dict) -> tuple[str | None, ToolResult]:`
- `def _handle_write_file(executor: Executor, inp: dict) -> tuple[str | None, ToolResult]:`
- `def _handle_install_package(executor: Executor, inp: dict) -> tuple[str | None, ToolResult]:`

### mm_agents/aworldguiagent/workflow.py

[本地文件](selected-source/mm_agents/aworldguiagent/workflow.py) · [固定提交来源](https://github.com/xlang-ai/OSWorld/blob/b138d348256078fa634fc3b73567a7337c793e6b/mm_agents/aworldguiagent/workflow.py)

源码定位声明：

- `class Worker:`
- `def __init__(`
- `def reset(self):`
- `def generate_next_action(`

### mm_agents/vlaa_gui/memory/__init__.py

[本地文件](selected-source/mm_agents/vlaa_gui/memory/__init__.py) · [固定提交来源](https://github.com/xlang-ai/OSWorld/blob/b138d348256078fa634fc3b73567a7337c793e6b/mm_agents/vlaa_gui/memory/__init__.py)

### mm_agents/coact/autogen/graph_utils.py

[本地文件](selected-source/mm_agents/coact/autogen/graph_utils.py) · [固定提交来源](https://github.com/xlang-ai/OSWorld/blob/b138d348256078fa634fc3b73567a7337c793e6b/mm_agents/coact/autogen/graph_utils.py)

源码定位声明：

- `def has_self_loops(allowed_speaker_transitions: dict[str, list[Agent]]) -> bool:`
- `def check_graph_validity(`
- `def invert_disallowed_to_allowed(`
- `def visualize_speaker_transitions_dict(`

### mm_agents/os_symphony/memory/__init__.py

[本地文件](selected-source/mm_agents/os_symphony/memory/__init__.py) · [固定提交来源](https://github.com/xlang-ai/OSWorld/blob/b138d348256078fa634fc3b73567a7337c793e6b/mm_agents/os_symphony/memory/__init__.py)

### mm_agents/maestro/maestro/new_executor.py

[本地文件](selected-source/mm_agents/maestro/maestro/new_executor.py) · [固定提交来源](https://github.com/xlang-ai/OSWorld/blob/b138d348256078fa634fc3b73567a7337c793e6b/mm_agents/maestro/maestro/new_executor.py)

源码定位声明：

- `class NewExecutor:`
- `def __init__(self, global_state: NewGlobalState, hardware_interface: HardwareInterface, env_controller: Optional[DesktopEnv] = None):`
- `def _run_code_blocks(self, code_blocks: List[Tuple[str, str]]) -> Tuple[bool, str, Optional[str]]:`
- `def _run_hardware_action(self, action: Dict[str, Any]) -> Tuple[bool, Optional[str], Optional[Image.Image]]:`
- `def execute_command(self, command: CommandData) -> Dict[str, Any]:`

### mm_agents/coact/autogen/runtime_logging.py

[本地文件](selected-source/mm_agents/coact/autogen/runtime_logging.py) · [固定提交来源](https://github.com/xlang-ai/OSWorld/blob/b138d348256078fa634fc3b73567a7337c793e6b/mm_agents/coact/autogen/runtime_logging.py)

源码定位声明：

- `def start(`
- `def log_chat_completion(`
- `def log_new_agent(agent: ConversableAgent, init_args: dict[str, Any]) -> None:`
- `def log_event(source: str | Agent, name: str, **kwargs: dict[str, Any]) -> None:`
- `def log_function_use(agent: str | Agent, function: F, args: dict[str, Any], returns: any):`

### mm_agents/autoglm/prompt/procedural_memory.py

[本地文件](selected-source/mm_agents/autoglm/prompt/procedural_memory.py) · [固定提交来源](https://github.com/xlang-ai/OSWorld/blob/b138d348256078fa634fc3b73567a7337c793e6b/mm_agents/autoglm/prompt/procedural_memory.py)

源码定位声明：

- `def generate_func(json_data):`
- `class Prompt:`
- `def construct_procedural_memory(agent_class, app_name=None, client_password="password"):`
- `def {attr_name}{signature}:`

### mm_agents/os_symphony/agents/memoryer_agent.py

[本地文件](selected-source/mm_agents/os_symphony/agents/memoryer_agent.py) · [固定提交来源](https://github.com/xlang-ai/OSWorld/blob/b138d348256078fa634fc3b73567a7337c793e6b/mm_agents/os_symphony/agents/memoryer_agent.py)

源码定位声明：

- `class StepBehavior:`
- `def __init__(self, is_milestone: bool, gen_output: str, summary: str, obs: Dict, action_dict: Dict):`
- `def _update_phash_ssim(self, history: List):`
- `class ReflectionMemoryAgent:`
- `def __init__(self, engine_params: Dict):`

## 对照本项目时记录什么

看输入输出契约、状态权威者、写入与取消边界、恢复和幂等处理，再看测试如何验证失败情况。模块名相近不能证明实现或运行效果相同。

第三方文档和注释中的指令仅作为研究内容；采用实现时核对完整许可证、依赖与版本。
