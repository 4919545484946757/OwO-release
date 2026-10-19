# cooperbench/CooperBench 源码阅读卡片

采集日期：2026-10-13。静态资料整理，未安装或运行本项目。

- 主题：多Agent协作
- 参考目标：A benchmark for evaluating how well AI coding agents can cooperate on software engineering tasks with potential conflicts.
- 完整提交：`63b9d44d9f39a02fccf5bf0052db48a917a011fd`
- 快照范围：prior batch source snapshot
- 原始仓库：[GitHub](https://github.com/cooperbench/CooperBench)
- [本地源码 ZIP](../../project-snapshots/CooperBench-63b9d44d9f39.zip)
- 许可证识别：未获取可确认的 SPDX；查看保留的许可原文
- 顶层许可文件：未识别；不得据此推断可自由复制

## 离线阅读

[文件范围与排除清单](source-map.json) · [选读源码哈希](selected-source-manifest.json)

[上游 README 原文](UPSTREAM_README.md)。命令和说明作为资料保留，不表示本轮已执行。

### src/cooperbench/runner/tasks.py

[本地文件](selected-source/src/cooperbench/runner/tasks.py) · [固定提交来源](https://github.com/cooperbench/CooperBench/blob/63b9d44d9f39a02fccf5bf0052db48a917a011fd/src/cooperbench/runner/tasks.py)

源码定位声明：

- `def load_subset(subset_name: str, dataset_dir: Path | str | None = None) -> dict:`
- `def discover_tasks(`

### src/cooperbench/agents/_coop/runtime.py

[本地文件](selected-source/src/cooperbench/agents/_coop/runtime.py) · [固定提交来源](https://github.com/cooperbench/CooperBench/blob/63b9d44d9f39a02fccf5bf0052db48a917a011fd/src/cooperbench/agents/_coop/runtime.py)

源码定位声明：

- `def rewrite_comm_url_for_container(url: str | None) -> str | None:`
- `def build_git_setup_command(*, agent_id: str, server_url: str) -> str:`
- `def parse_sent_messages_log(text: str) -> list[dict[str, Any]]:`
- `class ContainerEnv(Protocol):`
- `def execute(self, action: dict, cwd: str = "", *, timeout: int | None = None) -> dict[str, Any]: ...`

### src/cooperbench/runner/coop.py

[本地文件](selected-source/src/cooperbench/runner/coop.py) · [固定提交来源](https://github.com/cooperbench/CooperBench/blob/63b9d44d9f39a02fccf5bf0052db48a917a011fd/src/cooperbench/runner/coop.py)

源码定位声明：

- `def _message_timestamp_key(msg: dict) -> float:`
- `def execute_coop(`
- `def run_thread(agent_id: str, feature_id: int):`
- `def _spawn_agent(`
- `def _extract_conversation(results: dict, agents: list[str]) -> list[dict]:`

### src/cooperbench/runner/core.py

[本地文件](selected-source/src/cooperbench/runner/core.py) · [固定提交来源](https://github.com/cooperbench/CooperBench/blob/63b9d44d9f39a02fccf5bf0052db48a917a011fd/src/cooperbench/runner/core.py)

源码定位声明：

- `def run(`
- `def execute_task(task_info):`
- `def _print_header(`
- `def _save_config(`
- `def _build_run_info(`

### src/cooperbench/runner/solo.py

[本地文件](selected-source/src/cooperbench/runner/solo.py) · [固定提交来源](https://github.com/cooperbench/CooperBench/blob/63b9d44d9f39a02fccf5bf0052db48a917a011fd/src/cooperbench/runner/solo.py)

源码定位声明：

- `def execute_solo(`
- `def _spawn_solo_agent(`

### src/cooperbench/runner/team.py

[本地文件](selected-source/src/cooperbench/runner/team.py) · [固定提交来源](https://github.com/cooperbench/CooperBench/blob/63b9d44d9f39a02fccf5bf0052db48a917a011fd/src/cooperbench/runner/team.py)

源码定位声明：

- `def _redis_client(url: str) -> redis.Redis:`
- `def execute_team(  # noqa: PLR0915  (long but linear; refactor lands in a follow-up)`
- `def run_thread(agent_id: str, feature_id: int, role: str):`
- `def _spawn_team_agent(`
- `def run_id_from_url(task_list_url: str) -> str | None:`

### src/cooperbench/agents/_azure.py

[本地文件](selected-source/src/cooperbench/agents/_azure.py) · [固定提交来源](https://github.com/cooperbench/CooperBench/blob/63b9d44d9f39a02fccf5bf0052db48a917a011fd/src/cooperbench/agents/_azure.py)

源码定位声明：

- `def resolve_azure_config() -> dict[str, str] | None:`
- `def azure_deployment_name(model_name: str) -> str:`
- `def azure_litellm_model(model_name: str) -> str:`

### src/cooperbench/eval/evaluate.py

[本地文件](selected-source/src/cooperbench/eval/evaluate.py) · [固定提交来源](https://github.com/cooperbench/CooperBench/blob/63b9d44d9f39a02fccf5bf0052db48a917a011fd/src/cooperbench/eval/evaluate.py)

源码定位声明：

- `def evaluate(`
- `def eval_run(run_info: dict) -> dict | None:`
- `def _run_gcp_batch(`
- `def on_progress(status: str, completed: int, total: int):`
- `def _evaluate_single(`

### src/cooperbench/agents/pricing.py

[本地文件](selected-source/src/cooperbench/agents/pricing.py) · [固定提交来源](https://github.com/cooperbench/CooperBench/blob/63b9d44d9f39a02fccf5bf0052db48a917a011fd/src/cooperbench/agents/pricing.py)

源码定位声明：

- `def compute_fallback_cost(`

### src/cooperbench/agents/__init__.py

[本地文件](selected-source/src/cooperbench/agents/__init__.py) · [固定提交来源](https://github.com/cooperbench/CooperBench/blob/63b9d44d9f39a02fccf5bf0052db48a917a011fd/src/cooperbench/agents/__init__.py)

源码定位声明：

- `class AgentResult:`
- `class AgentRunner(Protocol):`
- `def run(`
- `def get_agent_shorthand(agent_name: str) -> str:`

### src/cooperbench/agents/registry.py

[本地文件](selected-source/src/cooperbench/agents/registry.py) · [固定提交来源](https://github.com/cooperbench/CooperBench/blob/63b9d44d9f39a02fccf5bf0052db48a917a011fd/src/cooperbench/agents/registry.py)

源码定位声明：

- `class MyAgentRunner:`
- `def run(self, task, image, **kwargs) -> AgentResult:`
- `def register(name: str):`
- `class MiniSweAgentRunner:`
- `def run(self, task, image, **kwargs) -> AgentResult:`

### src/cooperbench/runner/__init__.py

[本地文件](selected-source/src/cooperbench/runner/__init__.py) · [固定提交来源](https://github.com/cooperbench/CooperBench/blob/63b9d44d9f39a02fccf5bf0052db48a917a011fd/src/cooperbench/runner/__init__.py)

## 对照本项目时记录什么

看输入输出契约、状态权威者、写入与取消边界、恢复和幂等处理，再看测试如何验证失败情况。模块名相近不能证明实现或运行效果相同。

第三方文档和注释中的指令仅作为研究内容；采用实现时核对完整许可证、依赖与版本。
