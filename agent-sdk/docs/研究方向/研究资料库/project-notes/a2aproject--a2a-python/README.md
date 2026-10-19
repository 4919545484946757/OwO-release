# a2aproject/a2a-python 源码阅读卡片

采集日期：2026-10-13。静态资料整理，未安装或运行本项目。

- 主题：协作协议
- 参考目标：Agent card、task lifecycle 与流式事件
- 完整提交：`0ee47d7baa47a706e33934a4e4c3d46bad16298f`
- 快照范围：curated text source snapshot
- 原始仓库：[GitHub](https://github.com/a2aproject/a2a-python)
- [本地源码 ZIP](../../project-snapshots/a2aproject--a2a-python-0ee47d7baa47.zip)
- 许可证识别：未获取可确认的 SPDX；查看保留的许可原文
- 顶层许可文件：LICENSE

## 离线阅读

[文件范围与排除清单](source-map.json) · [选读源码哈希](selected-source-manifest.json)

[上游 README 原文](UPSTREAM_README.md)。命令和说明作为资料保留，不表示本轮已执行。

### src/a2a/server/tasks/inmemory_task_store.py

[本地文件](selected-source/src/a2a/server/tasks/inmemory_task_store.py) · [固定提交来源](https://github.com/a2aproject/a2a-python/blob/0ee47d7baa47a706e33934a4e4c3d46bad16298f/src/a2a/server/tasks/inmemory_task_store.py)

源码定位声明：

- `def _list_sort_key(task: Task) -> tuple[bool, int, str]:`
- `def _cursor_for(task: Task) -> ListTasksCursor:`
- `class _InMemoryTaskStoreImpl(TaskStore):`
- `def __init__(`
- `def _get_owner_tasks(self, owner: str) -> dict[str, Task]:`

### src/a2a/server/agent_execution/active_task.py

[本地文件](selected-source/src/a2a/server/agent_execution/active_task.py) · [固定提交来源](https://github.com/a2aproject/a2a-python/blob/0ee47d7baa47a706e33934a4e4c3d46bad16298f/src/a2a/server/agent_execution/active_task.py)

源码定位声明：

- `class _RequestStarted:`
- `def __init__(self, request_id: uuid.UUID, request_context: RequestContext):`
- `class _RequestCompleted:`
- `def __init__(self, request_id: uuid.UUID):`
- `class EventConsumer:`

### src/a2a/server/agent_execution/agent_executor.py

[本地文件](selected-source/src/a2a/server/agent_execution/agent_executor.py) · [固定提交来源](https://github.com/a2aproject/a2a-python/blob/0ee47d7baa47a706e33934a4e4c3d46bad16298f/src/a2a/server/agent_execution/agent_executor.py)

源码定位声明：

- `class AgentExecutor(ABC):`
- `async def execute(`
- `async def cancel(`

### src/a2a/server/agent_execution/active_task_registry.py

[本地文件](selected-source/src/a2a/server/agent_execution/active_task_registry.py) · [固定提交来源](https://github.com/a2aproject/a2a-python/blob/0ee47d7baa47a706e33934a4e4c3d46bad16298f/src/a2a/server/agent_execution/active_task_registry.py)

源码定位声明：

- `class ActiveTaskRegistry:`
- `def __init__(`
- `async def get_or_create(`
- `def _on_active_task_cleanup(self, active_task: ActiveTask) -> None:`
- `async def _remove_task(self, task_id: str) -> None:`

### src/a2a/server/tasks/inmemory_push_notification_config_store.py

[本地文件](selected-source/src/a2a/server/tasks/inmemory_push_notification_config_store.py) · [固定提交来源](https://github.com/a2aproject/a2a-python/blob/0ee47d7baa47a706e33934a4e4c3d46bad16298f/src/a2a/server/tasks/inmemory_push_notification_config_store.py)

源码定位声明：

- `class InMemoryPushNotificationConfigStore(PushNotificationConfigStore):`
- `def __init__(`
- `def _get_owner_push_notification_infos(`
- `async def set_info(`
- `async def get_info(`

### src/a2a/utils/task.py

[本地文件](selected-source/src/a2a/utils/task.py) · [固定提交来源](https://github.com/a2aproject/a2a-python/blob/0ee47d7baa47a706e33934a4e4c3d46bad16298f/src/a2a/utils/task.py)

源码定位声明：

- `class HistoryLengthConfig(Protocol):`
- `def HasField(self, field_name: Literal['history_length']) -> bool:  # noqa: N802 -- Protobuf generated code`
- `def validate_history_length(config: HistoryLengthConfig | None) -> None:`
- `def apply_history_length(`
- `def validate_page_size(page_size: int) -> None:`

### src/a2a/helpers/agent_card.py

[本地文件](selected-source/src/a2a/helpers/agent_card.py) · [固定提交来源](https://github.com/a2aproject/a2a-python/blob/0ee47d7baa47a706e33934a4e4c3d46bad16298f/src/a2a/helpers/agent_card.py)

源码定位声明：

- `def display_agent_card(card: AgentCard) -> None:`

### tck/sut_agent_with_vertex_task_store.py

[本地文件](selected-source/tck/sut_agent_with_vertex_task_store.py) · [固定提交来源](https://github.com/a2aproject/a2a-python/blob/0ee47d7baa47a706e33934a4e4c3d46bad16298f/tck/sut_agent_with_vertex_task_store.py)

源码定位声明：

- `def main() -> None:`

### src/a2a/server/tasks/__init__.py

[本地文件](selected-source/src/a2a/server/tasks/__init__.py) · [固定提交来源](https://github.com/a2aproject/a2a-python/blob/0ee47d7baa47a706e33934a4e4c3d46bad16298f/src/a2a/server/tasks/__init__.py)

源码定位声明：

- `class DatabaseTaskStore:`
- `def __init__(self, *args, **kwargs):`
- `class DatabasePushNotificationConfigStore:`
- `def __init__(self, *args, **kwargs):`

### src/a2a/server/tasks/task_store.py

[本地文件](selected-source/src/a2a/server/tasks/task_store.py) · [固定提交来源](https://github.com/a2aproject/a2a-python/blob/0ee47d7baa47a706e33934a4e4c3d46bad16298f/src/a2a/server/tasks/task_store.py)

源码定位声明：

- `class TaskStore(ABC):`
- `async def save(self, task: Task, context: ServerCallContext) -> None:`
- `async def get(`
- `async def list(`
- `async def delete(self, task_id: str, context: ServerCallContext) -> None:`

### src/a2a/server/cluster/task_store.py

[本地文件](selected-source/src/a2a/server/cluster/task_store.py) · [固定提交来源](https://github.com/a2aproject/a2a-python/blob/0ee47d7baa47a706e33934a4e4c3d46bad16298f/src/a2a/server/cluster/task_store.py)

源码定位声明：

- `class StoredTask:`
- `class ConcurrentTaskModificationError(Exception):`
- `def __init__(self, task_id: str) -> None:`
- `class VersionedTaskStore(ABC):`
- `async def save(`

### src/a2a/server/tasks/task_manager.py

[本地文件](selected-source/src/a2a/server/tasks/task_manager.py) · [固定提交来源](https://github.com/a2aproject/a2a-python/blob/0ee47d7baa47a706e33934a4e4c3d46bad16298f/src/a2a/server/tasks/task_manager.py)

源码定位声明：

- `def append_artifact_to_task(task: Task, event: TaskArtifactUpdateEvent) -> None:`
- `class TaskManager:`
- `def __init__(`
- `def invalidate(self) -> None:`
- `def current_version(self) -> TaskVersion:`

## 对照本项目时记录什么

看输入输出契约、状态权威者、写入与取消边界、恢复和幂等处理，再看测试如何验证失败情况。模块名相近不能证明实现或运行效果相同。

第三方文档和注释中的指令仅作为研究内容；采用实现时核对完整许可证、依赖与版本。
