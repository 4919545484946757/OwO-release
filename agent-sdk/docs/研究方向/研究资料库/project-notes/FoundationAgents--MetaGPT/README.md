# FoundationAgents/MetaGPT 源码阅读卡片

采集日期：2026-10-13。静态资料整理，未安装或运行本项目。

- 主题：多Agent编程
- 参考目标：SOP、角色交接与工程产物
- 完整提交：`11cdf466d042aece04fc6cfd13b28e1a70341b1f`
- 快照范围：curated text source snapshot
- 原始仓库：[GitHub](https://github.com/FoundationAgents/MetaGPT)
- [本地源码 ZIP](../../project-snapshots/FoundationAgents--MetaGPT-11cdf466d042.zip)
- 许可证识别：MIT
- 顶层许可文件：LICENSE

## 离线阅读

[文件范围与排除清单](source-map.json) · [选读源码哈希](selected-source-manifest.json)

[上游 README 原文](UPSTREAM_README.md)。命令和说明作为资料保留，不表示本轮已执行。

### metagpt/ext/stanford_town/memory/agent_memory.py

[本地文件](selected-source/metagpt/ext/stanford_town/memory/agent_memory.py) · [固定提交来源](https://github.com/FoundationAgents/MetaGPT/blob/11cdf466d042aece04fc6cfd13b28e1a70341b1f/metagpt/ext/stanford_town/memory/agent_memory.py)

源码定位声明：

- `class BasicMemory(Message):`
- `def check_values(cls, values):`
- `def transform_time_field(self, time_field: Optional[datetime]) -> str:`
- `def summary(self):`
- `def save_to_dict(self) -> dict:`

### metagpt/environment/minecraft/mineflayer/mineflayer-collectblock/src/TaskQueue.ts

[本地文件](selected-source/metagpt/environment/minecraft/mineflayer/mineflayer-collectblock/src/TaskQueue.ts) · [固定提交来源](https://github.com/FoundationAgents/MetaGPT/blob/11cdf466d042aece04fc6cfd13b28e1a70341b1f/metagpt/environment/minecraft/mineflayer/mineflayer-collectblock/src/TaskQueue.ts)

源码定位声明：

- `export class TaskQueue {`

### metagpt/memory/memory.py

[本地文件](selected-source/metagpt/memory/memory.py) · [固定提交来源](https://github.com/FoundationAgents/MetaGPT/blob/11cdf466d042aece04fc6cfd13b28e1a70341b1f/metagpt/memory/memory.py)

源码定位声明：

- `class Memory(BaseModel):`
- `def add(self, message: Message):`
- `def add_batch(self, messages: Iterable[Message]):`
- `def get_by_role(self, role: str) -> list[Message]:`
- `def get_by_content(self, content: str) -> list[Message]:`

### metagpt/memory/__init__.py

[本地文件](selected-source/metagpt/memory/__init__.py) · [固定提交来源](https://github.com/FoundationAgents/MetaGPT/blob/11cdf466d042aece04fc6cfd13b28e1a70341b1f/metagpt/memory/__init__.py)

### metagpt/prompts/task_type.py

[本地文件](selected-source/metagpt/prompts/task_type.py) · [固定提交来源](https://github.com/FoundationAgents/MetaGPT/blob/11cdf466d042aece04fc6cfd13b28e1a70341b1f/metagpt/prompts/task_type.py)

### metagpt/strategy/task_type.py

[本地文件](selected-source/metagpt/strategy/task_type.py) · [固定提交来源](https://github.com/FoundationAgents/MetaGPT/blob/11cdf466d042aece04fc6cfd13b28e1a70341b1f/metagpt/strategy/task_type.py)

源码定位声明：

- `class TaskTypeDef(BaseModel):`
- `class TaskType(Enum):`
- `def type_name(self):`
- `def get_type(cls, type_name):`

### metagpt/memory/brain_memory.py

[本地文件](selected-source/metagpt/memory/brain_memory.py) · [固定提交来源](https://github.com/FoundationAgents/MetaGPT/blob/11cdf466d042aece04fc6cfd13b28e1a70341b1f/metagpt/memory/brain_memory.py)

源码定位声明：

- `class BrainMemory(BaseModel):`
- `def set_default_config(cls, config):`
- `class Config:`
- `def add_talk(self, msg: Message):`
- `def add_answer(self, msg: Message):`

### metagpt/actions/action_graph.py

[本地文件](selected-source/metagpt/actions/action_graph.py) · [固定提交来源](https://github.com/FoundationAgents/MetaGPT/blob/11cdf466d042aece04fc6cfd13b28e1a70341b1f/metagpt/actions/action_graph.py)

源码定位声明：

- `class ActionGraph:`
- `def __init__(self):`
- `def add_node(self, node):`
- `def add_edge(self, from_node: "ActionNode", to_node: "ActionNode"):`
- `def topological_sort(self):`

### metagpt/actions/execute_task.py

[本地文件](selected-source/metagpt/actions/execute_task.py) · [固定提交来源](https://github.com/FoundationAgents/MetaGPT/blob/11cdf466d042aece04fc6cfd13b28e1a70341b1f/metagpt/actions/execute_task.py)

源码定位声明：

- `class ExecuteTask(Action):`
- `async def run(self, *args, **kwargs):`

### metagpt/memory/memory_storage.py

[本地文件](selected-source/metagpt/memory/memory_storage.py) · [固定提交来源](https://github.com/FoundationAgents/MetaGPT/blob/11cdf466d042aece04fc6cfd13b28e1a70341b1f/metagpt/memory/memory_storage.py)

源码定位声明：

- `class MemoryStorage(object):`
- `def __init__(self, mem_ttl: int = MEM_TTL, embedding: BaseEmbedding = None):`
- `def is_initialized(self) -> bool:`
- `def recover_memory(self, role_id: str) -> list[Message]:`
- `def add(self, message: Message) -> bool:`

### metagpt/memory/longterm_memory.py

[本地文件](selected-source/metagpt/memory/longterm_memory.py) · [固定提交来源](https://github.com/FoundationAgents/MetaGPT/blob/11cdf466d042aece04fc6cfd13b28e1a70341b1f/metagpt/memory/longterm_memory.py)

源码定位声明：

- `class LongTermMemory(Memory):`
- `def recover_memory(self, role_id: str, rc: RoleContext):`
- `def add(self, message: Message):`
- `async def find_news(self, observed: list[Message], k=0) -> list[Message]:`
- `def persist(self):`

### metagpt/utils/graph_repository.py

[本地文件](selected-source/metagpt/utils/graph_repository.py) · [固定提交来源](https://github.com/FoundationAgents/MetaGPT/blob/11cdf466d042aece04fc6cfd13b28e1a70341b1f/metagpt/utils/graph_repository.py)

源码定位声明：

- `class GraphKeyword:`
- `class SPO(BaseModel):`
- `class GraphRepository(ABC):`
- `def __init__(self, name: str, **kwargs):`
- `async def insert(self, subject: str, predicate: str, object_: str):`

## 对照本项目时记录什么

看输入输出契约、状态权威者、写入与取消边界、恢复和幂等处理，再看测试如何验证失败情况。模块名相近不能证明实现或运行效果相同。

第三方文档和注释中的指令仅作为研究内容；采用实现时核对完整许可证、依赖与版本。
