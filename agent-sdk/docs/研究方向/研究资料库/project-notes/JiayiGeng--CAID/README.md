# JiayiGeng/CAID 源码阅读卡片

采集日期：2026-10-13。静态资料整理，未安装或运行本项目。

- 主题：多Agent协作
- 参考目标：Code repo for paper: Effective Strategies for Asynchronous Software Engineering Agents
- 完整提交：`f364d9e95727c2e2ba3dbf23f2d6de52c5f3d5fa`
- 快照范围：prior batch source snapshot
- 原始仓库：[GitHub](https://github.com/JiayiGeng/CAID)
- [本地源码 ZIP](../../project-snapshots/CAID-f364d9e95727.zip)
- 许可证识别：未获取可确认的 SPDX；查看保留的许可原文
- 顶层许可文件：未识别；不得据此推断可自由复制

## 离线阅读

[文件范围与排除清单](source-map.json) · [选读源码哈希](selected-source-manifest.json)

[上游 README 原文](UPSTREAM_README.md)。命令和说明作为资料保留，不表示本轮已执行。

### tasks/base.py

[本地文件](selected-source/tasks/base.py) · [固定提交来源](https://github.com/JiayiGeng/CAID/blob/f364d9e95727c2e2ba3dbf23f2d6de52c5f3d5fa/tasks/base.py)

源码定位声明：

- `class TaskModule(ABC):`
- `def get_docker_image(self):`
- `def get_work_dir(self):`
- `def get_workspace_config(self):`
- `def load_task_data(self):`

### core/subagent.py

[本地文件](selected-source/core/subagent.py) · [固定提交来源](https://github.com/JiayiGeng/CAID/blob/f364d9e95727c2e2ba3dbf23f2d6de52c5f3d5fa/core/subagent.py)

源码定位声明：

- `class SubAgentRunner:`
- `def __init__(`
- `def log(self, message):`
- `def can_accept_more_tasks(self):`
- `def update_task(self, new_subagent):`

### tasks/commit0.py

[本地文件](selected-source/tasks/commit0.py) · [固定提交来源](https://github.com/JiayiGeng/CAID/blob/f364d9e95727c2e2ba3dbf23f2d6de52c5f3d5fa/tasks/commit0.py)

源码定位声明：

- `class Commit0Config:`
- `class Commit0Task(TaskModule):`
- `def __init__(self, config):`
- `def get_docker_image(self):`
- `def get_work_dir(self):`

### tasks/__init__.py

[本地文件](selected-source/tasks/__init__.py) · [固定提交来源](https://github.com/JiayiGeng/CAID/blob/f364d9e95727c2e2ba3dbf23f2d6de52c5f3d5fa/tasks/__init__.py)

### tasks/paperbench.py

[本地文件](selected-source/tasks/paperbench.py) · [固定提交来源](https://github.com/JiayiGeng/CAID/blob/f364d9e95727c2e2ba3dbf23f2d6de52c5f3d5fa/tasks/paperbench.py)

源码定位声明：

- `class PaperbenchConfig:`
- `class PaperbenchTask(TaskModule):`
- `def __init__(self, config):`
- `def get_docker_image(self):`
- `def get_work_dir(self):`

### judge/judge_runner.py

[本地文件](selected-source/judge/judge_runner.py) · [固定提交来源](https://github.com/JiayiGeng/CAID/blob/f364d9e95727c2e2ba3dbf23f2d6de52c5f3d5fa/judge/judge_runner.py)

源码定位声明：

- `def run(`
- `async def _run():`

### config.py

[本地文件](selected-source/config.py) · [固定提交来源](https://github.com/JiayiGeng/CAID/blob/f364d9e95727c2e2ba3dbf23f2d6de52c5f3d5fa/config.py)

源码定位声明：

- `class WorkflowConfig:`
- `class PaperInfo:`
- `def to_dict(self):`
- `class TaskNode:`
- `def to_dict(self):`

### run_infer.py

[本地文件](selected-source/run_infer.py) · [固定提交来源](https://github.com/JiayiGeng/CAID/blob/f364d9e95727c2e2ba3dbf23f2d6de52c5f3d5fa/run_infer.py)

源码定位声明：

- `async def run_workflow_inner(task, workflow_config, task_module, multi_agent=True, **kwargs):`
- `async def run_workflow(task, workflow_config, task_module, multi_agent=True, **kwargs):`
- `def main(task="commit0", model=None, multi_agent=True,`

### core/utils.py

[本地文件](selected-source/core/utils.py) · [固定提交来源](https://github.com/JiayiGeng/CAID/blob/f364d9e95727c2e2ba3dbf23f2d6de52c5f3d5fa/core/utils.py)

源码定位声明：

- `class PanelVisualizer(ConversationVisualizerBase):`
- `def __init__(self, highlight_regex=None, skip_user_messages=False):`
- `def on_event(self, event):`
- `def apply_highlighting(self, text):`
- `def format_metrics_subtitle(self):`

### core/manager.py

[本地文件](selected-source/core/manager.py) · [固定提交来源](https://github.com/JiayiGeng/CAID/blob/f364d9e95727c2e2ba3dbf23f2d6de52c5f3d5fa/core/manager.py)

源码定位声明：

- `class Manager:`
- `def __init__(`
- `def log(self, message):`
- `def save_events(self, phase, event_start_idx=0):`
- `def setup_workspace(self):`

### core/patches.py

[本地文件](selected-source/core/patches.py) · [固定提交来源](https://github.com/JiayiGeng/CAID/blob/f364d9e95727c2e2ba3dbf23f2d6de52c5f3d5fa/core/patches.py)

源码定位声明：

- `def patched_convert(self, content, role):`
- `def patched_model_dump(self, **kwargs):`

### core/__init__.py

[本地文件](selected-source/core/__init__.py) · [固定提交来源](https://github.com/JiayiGeng/CAID/blob/f364d9e95727c2e2ba3dbf23f2d6de52c5f3d5fa/core/__init__.py)

## 对照本项目时记录什么

看输入输出契约、状态权威者、写入与取消边界、恢复和幂等处理，再看测试如何验证失败情况。模块名相近不能证明实现或运行效果相同。

第三方文档和注释中的指令仅作为研究内容；采用实现时核对完整许可证、依赖与版本。
