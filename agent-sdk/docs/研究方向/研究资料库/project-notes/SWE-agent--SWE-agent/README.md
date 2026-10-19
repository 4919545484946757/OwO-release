# SWE-agent/SWE-agent 源码阅读卡片

采集日期：2026-10-13。静态资料整理，未安装或运行本项目。

- 主题：编程Agent
- 参考目标：Agent-computer interface 与补丁评测
- 完整提交：`3ea751c087f32b16e039a2233dd6eefecef325d5`
- 快照范围：curated text source snapshot
- 原始仓库：[GitHub](https://github.com/SWE-agent/SWE-agent)
- [本地源码 ZIP](../../project-snapshots/SWE-agent--SWE-agent-3ea751c087f3.zip)
- 许可证识别：MIT
- 顶层许可文件：LICENSE

## 离线阅读

[文件范围与排除清单](source-map.json) · [选读源码哈希](selected-source-manifest.json)

[上游 README 原文](UPSTREAM_README.md)。命令和说明作为资料保留，不表示本轮已执行。

### sweagent/run/hooks/swe_bench_evaluate.py

[本地文件](selected-source/sweagent/run/hooks/swe_bench_evaluate.py) · [固定提交来源](https://github.com/SWE-agent/SWE-agent/blob/3ea751c087f32b16e039a2233dd6eefecef325d5/sweagent/run/hooks/swe_bench_evaluate.py)

源码定位声明：

- `class SweBenchEvaluate(RunHook):`
- `def __init__(self, output_dir: Path, subset: str, split: str, continuous_submission_every: int = 0) -> None:`
- `def run_id(self) -> str:`
- `def _get_sb_call(self, preds_path: Path, submit_only: bool = False) -> list[str]:`
- `def check_running_calls(self) -> None:`

### sweagent/types.py

[本地文件](selected-source/sweagent/types.py) · [固定提交来源](https://github.com/SWE-agent/SWE-agent/blob/3ea751c087f32b16e039a2233dd6eefecef325d5/sweagent/types.py)

源码定位声明：

- `class StepOutput(BaseModel):`
- `def to_template_format_dict(self) -> dict[str, str | int | float | bool | None]:`
- `class TrajectoryStep(TypedDict):`
- `class _HistoryItem(TypedDict):`
- `class HistoryItem(_HistoryItem, total=False):`

### sweagent/__init__.py

[本地文件](selected-source/sweagent/__init__.py) · [固定提交来源](https://github.com/SWE-agent/SWE-agent/blob/3ea751c087f32b16e039a2233dd6eefecef325d5/sweagent/__init__.py)

源码定位声明：

- `def get_agent_commit_hash() -> str:`
- `def get_rex_commit_hash() -> str:`
- `def get_rex_version() -> str:`
- `def get_agent_version_info() -> str:`
- `def impose_rex_lower_bound() -> None:`

### sweagent/__main__.py

[本地文件](selected-source/sweagent/__main__.py) · [固定提交来源](https://github.com/SWE-agent/SWE-agent/blob/3ea751c087f32b16e039a2233dd6eefecef325d5/sweagent/__main__.py)

### sweagent/exceptions.py

[本地文件](selected-source/sweagent/exceptions.py) · [固定提交来源](https://github.com/SWE-agent/SWE-agent/blob/3ea751c087f32b16e039a2233dd6eefecef325d5/sweagent/exceptions.py)

源码定位声明：

- `class FormatError(Exception):`
- `class FunctionCallingFormatError(FormatError):`
- `def __init__(`
- `class ContextWindowExceededError(Exception):`
- `class CostLimitExceededError(Exception):`

### sweagent/run/run.py

[本地文件](selected-source/sweagent/run/run.py) · [固定提交来源](https://github.com/SWE-agent/SWE-agent/blob/3ea751c087f32b16e039a2233dd6eefecef325d5/sweagent/run/run.py)

源码定位声明：

- `def get_cli():`
- `def main(args: list[str] | None = None):`

### sweagent/utils/log.py

[本地文件](selected-source/sweagent/utils/log.py) · [固定提交来源](https://github.com/SWE-agent/SWE-agent/blob/3ea751c087f32b16e039a2233dd6eefecef325d5/sweagent/utils/log.py)

源码定位声明：

- `def _interpret_level(level: int | str | None, *, default=logging.DEBUG) -> int:`
- `def register_thread_name(name: str) -> None:`
- `class _RichHandlerWithEmoji(RichHandler):`
- `def __init__(self, emoji: str, *args, **kwargs):`
- `def get_level_text(self, record: logging.LogRecord) -> Text:`

### sweagent/run/common.py

[本地文件](selected-source/sweagent/run/common.py) · [固定提交来源](https://github.com/SWE-agent/SWE-agent/blob/3ea751c087f32b16e039a2233dd6eefecef325d5/sweagent/run/common.py)

源码定位声明：

- `def _shorten_strings(data, *, max_length=30):`
- `class AutoCorrectSuggestion:`
- `def __init__(`
- `def show(self, args: list[str]) -> bool:`
- `def format(self) -> str:`

### sweagent/tools/tools.py

[本地文件](selected-source/sweagent/tools/tools.py) · [固定提交来源](https://github.com/SWE-agent/SWE-agent/blob/3ea751c087f32b16e039a2233dd6eefecef325d5/sweagent/tools/tools.py)

源码定位声明：

- `class ToolFilterConfig(BaseModel):`
- `class ToolConfig(BaseModel):`
- `def use_function_calling(self) -> bool:`
- `def state_commands(self) -> list[str]:`
- `def commands(self) -> list[Command]:`

### sweagent/tools/utils.py

[本地文件](selected-source/sweagent/tools/utils.py) · [固定提交来源](https://github.com/SWE-agent/SWE-agent/blob/3ea751c087f32b16e039a2233dd6eefecef325d5/sweagent/tools/utils.py)

源码定位声明：

- `def _guard_multiline_input(action: str, match_fct: Callable[[str], re.Match | None]) -> str:`
- `def _should_quote(value: Any, command: Command) -> bool:`
- `def get_signature(cmd):`
- `def generate_command_docs(`

### sweagent/utils/files.py

[本地文件](selected-source/sweagent/utils/files.py) · [固定提交来源](https://github.com/SWE-agent/SWE-agent/blob/3ea751c087f32b16e039a2233dd6eefecef325d5/sweagent/utils/files.py)

源码定位声明：

- `def load_file(path: Path | str | None) -> Any:`

### sweagent/agent/agents.py

[本地文件](selected-source/sweagent/agent/agents.py) · [固定提交来源](https://github.com/SWE-agent/SWE-agent/blob/3ea751c087f32b16e039a2233dd6eefecef325d5/sweagent/agent/agents.py)

源码定位声明：

- `class TemplateConfig(BaseModel):`
- `def model_post_init(self, __context):`
- `def validate_template_jinja_syntax(self) -> Self:`
- `def warnings(self) -> Self:`
- `class DefaultAgentConfig(BaseModel):`

## 对照本项目时记录什么

看输入输出契约、状态权威者、写入与取消边界、恢复和幂等处理，再看测试如何验证失败情况。模块名相近不能证明实现或运行效果相同。

第三方文档和注释中的指令仅作为研究内容；采用实现时核对完整许可证、依赖与版本。
