# OpenBMB/ChatDev 源码阅读卡片

采集日期：2026-10-13。静态资料整理，未安装或运行本项目。

- 主题：多Agent编程
- 参考目标：软件开发流程与角色通信
- 完整提交：`4fb2db0ea90375ce1059f44fe03ffbd191a7a169`
- 快照范围：curated text source snapshot
- 原始仓库：[GitHub](https://github.com/OpenBMB/ChatDev)
- [本地源码 ZIP](../../project-snapshots/OpenBMB--ChatDev-4fb2db0ea903.zip)
- 许可证识别：Apache-2.0
- 顶层许可文件：LICENSE

## 离线阅读

[文件范围与排除清单](source-map.json) · [选读源码哈希](selected-source-manifest.json)

[上游 README 原文](UPSTREAM_README.md)。命令和说明作为资料保留，不表示本轮已执行。

### runtime/node/executor/agent_executor.py

[本地文件](selected-source/runtime/node/executor/agent_executor.py) · [固定提交来源](https://github.com/OpenBMB/ChatDev/blob/4fb2db0ea90375ce1059f44fe03ffbd191a7a169/runtime/node/executor/agent_executor.py)

源码定位声明：

- `class AgentNodeExecutor(NodeExecutor):`
- `def execute(self, node: Node, inputs: List[Message]) -> List[Message]:`
- `def _prepare_prompt_messages(`
- `def _prepare_message_conversation(`
- `def _prepare_call_options(self, node: Node) -> Dict[str, Any]:`

### runtime/node/executor/subgraph_executor.py

[本地文件](selected-source/runtime/node/executor/subgraph_executor.py) · [固定提交来源](https://github.com/OpenBMB/ChatDev/blob/4fb2db0ea90375ce1059f44fe03ffbd191a7a169/runtime/node/executor/subgraph_executor.py)

源码定位声明：

- `class SubgraphNodeExecutor(NodeExecutor):`
- `def __init__(self, context, subgraphs: dict):`
- `def execute(self, node: Node, inputs: List[Message]) -> List[Message]:`

### workflow/graph.py

[本地文件](selected-source/workflow/graph.py) · [固定提交来源](https://github.com/OpenBMB/ChatDev/blob/4fb2db0ea90375ce1059f44fe03ffbd191a7a169/workflow/graph.py)

源码定位声明：

- `class ExecutionError(RuntimeError):`
- `class GraphExecutor:`
- `def __init__(`
- `def request_cancel(self, reason: Optional[str] = None) -> None:`
- `def is_cancelled(self) -> bool:`

### workflow/graph_context.py

[本地文件](selected-source/workflow/graph_context.py) · [固定提交来源](https://github.com/OpenBMB/ChatDev/blob/4fb2db0ea90375ce1059f44fe03ffbd191a7a169/workflow/graph_context.py)

源码定位声明：

- `class GraphContext:`
- `def __init__(self, config: GraphConfig) -> None:`
- `def name(self) -> str:`
- `def log_level(self):`
- `def metadata(self) -> Dict[str, Any]:`

### workflow/graph_manager.py

[本地文件](selected-source/workflow/graph_manager.py) · [固定提交来源](https://github.com/OpenBMB/ChatDev/blob/4fb2db0ea90375ce1059f44fe03ffbd191a7a169/workflow/graph_manager.py)

源码定位声明：

- `class GraphManager:`
- `def __init__(self, graph: "GraphContext") -> None:`
- `def build_graph_structure(self) -> None:`
- `def _instantiate_nodes(self) -> None:`
- `def _build_subgraph(self, node_id: str) -> None:`

### workflow/subgraph_loader.py

[本地文件](selected-source/workflow/subgraph_loader.py) · [固定提交来源](https://github.com/OpenBMB/ChatDev/blob/4fb2db0ea90375ce1059f44fe03ffbd191a7a169/workflow/subgraph_loader.py)

源码定位声明：

- `def _resolve_candidate_paths(file_path: str, parent_source: str | None) -> List[Path]:`
- `def _resolve_existing_path(candidates: List[Path]) -> Path:`
- `def _load_graph_dict(path: Path) -> Dict[str, Any]:`
- `def load_subgraph_config(file_path: str, *, parent_source: str | None = None) -> Tuple[Dict[str, Any], Dict[str, Any], str]:`

### runtime/node/agent/memory/__init__.py

[本地文件](selected-source/runtime/node/agent/memory/__init__.py) · [固定提交来源](https://github.com/OpenBMB/ChatDev/blob/4fb2db0ea90375ce1059f44fe03ffbd191a7a169/runtime/node/agent/memory/__init__.py)

### runtime/node/agent/memory/registry.py

[本地文件](selected-source/runtime/node/agent/memory/registry.py) · [固定提交来源](https://github.com/OpenBMB/ChatDev/blob/4fb2db0ea90375ce1059f44fe03ffbd191a7a169/runtime/node/agent/memory/registry.py)

源码定位声明：

- `class MemoryStoreRegistration:`
- `def _ensure_builtins_loaded() -> None:`
- `def register_memory_store(`
- `def get_memory_store_registration(name: str) -> MemoryStoreRegistration:`
- `def iter_memory_store_registrations() -> Dict[str, MemoryStoreRegistration]:`

### runtime/node/agent/memory/embedding.py

[本地文件](selected-source/runtime/node/agent/memory/embedding.py) · [固定提交来源](https://github.com/OpenBMB/ChatDev/blob/4fb2db0ea90375ce1059f44fe03ffbd191a7a169/runtime/node/agent/memory/embedding.py)

源码定位声明：

- `class EmbeddingBase(ABC):`
- `def __init__(self, embedding_config: EmbeddingConfig):`
- `def get_embedding(self, text):`
- `def _preprocess_text(self, text: str) -> str:`
- `def _chunk_text(self, text: str, max_length: int = 500) -> List[str]:`

### runtime/node/agent/memory/file_memory.py

[本地文件](selected-source/runtime/node/agent/memory/file_memory.py) · [固定提交来源](https://github.com/OpenBMB/ChatDev/blob/4fb2db0ea90375ce1059f44fe03ffbd191a7a169/runtime/node/agent/memory/file_memory.py)

源码定位声明：

- `class FileMemory(MemoryBase):`
- `def __init__(self, store: MemoryStoreConfig):`
- `def load(self) -> None:`
- `def save(self) -> None:`
- `def retrieve(`

### runtime/node/agent/memory/mem0_memory.py

[本地文件](selected-source/runtime/node/agent/memory/mem0_memory.py) · [固定提交来源](https://github.com/OpenBMB/ChatDev/blob/4fb2db0ea90375ce1059f44fe03ffbd191a7a169/runtime/node/agent/memory/mem0_memory.py)

源码定位声明：

- `def _get_mem0_client(config: Mem0MemoryConfig):`
- `class Mem0Memory(MemoryBase):`
- `def __init__(self, store: MemoryStoreConfig):`
- `def load(self) -> None:`
- `def save(self) -> None:`

### runtime/node/agent/memory/memory_base.py

[本地文件](selected-source/runtime/node/agent/memory/memory_base.py) · [固定提交来源](https://github.com/OpenBMB/ChatDev/blob/4fb2db0ea90375ce1059f44fe03ffbd191a7a169/runtime/node/agent/memory/memory_base.py)

源码定位声明：

- `class MemoryContentSnapshot:`
- `def to_dict(self) -> Dict[str, Any]:`
- `def from_dict(cls, payload: Dict[str, Any] | None) -> "MemoryContentSnapshot | None":`
- `def from_message(cls, message: Message | str | None) -> "MemoryContentSnapshot | None":`
- `def from_messages(cls, messages: List[Message]) -> "MemoryContentSnapshot | None":`

## 对照本项目时记录什么

看输入输出契约、状态权威者、写入与取消边界、恢复和幂等处理，再看测试如何验证失败情况。模块名相近不能证明实现或运行效果相同。

第三方文档和注释中的指令仅作为研究内容；采用实现时核对完整许可证、依赖与版本。
