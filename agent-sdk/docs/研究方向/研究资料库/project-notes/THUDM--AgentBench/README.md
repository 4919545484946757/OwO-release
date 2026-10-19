# THUDM/AgentBench 源码阅读卡片

采集日期：2026-10-13。静态资料整理，未安装或运行本项目。

- 主题：通用Agent评测
- 参考目标：交互环境、控制器与评估
- 完整提交：`d1e4a10db08c87075c78972e48ecc182be03e2d5`
- 快照范围：curated text source snapshot
- 原始仓库：[GitHub](https://github.com/THUDM/AgentBench)
- [本地源码 ZIP](../../project-snapshots/THUDM--AgentBench-d1e4a10db08c.zip)
- 许可证识别：未获取可确认的 SPDX；查看保留的许可原文
- 顶层许可文件：LICENSE

## 离线阅读

[文件范围与排除清单](source-map.json) · [选读源码哈希](selected-source-manifest.json)

[上游 README 原文](UPSTREAM_README.md)。命令和说明作为资料保留，不表示本轮已执行。

### src/server/tasks/knowledgegraph/api.py

[本地文件](selected-source/src/server/tasks/knowledgegraph/api.py) · [固定提交来源](https://github.com/THUDM/AgentBench/blob/d1e4a10db08c87075c78972e48ecc182be03e2d5/src/server/tasks/knowledgegraph/api.py)

源码定位声明：

- `class Variable:`
- `def __init__(self, type, program):`
- `def __hash__(self) -> int:`
- `def __eq__(self, __value: object) -> bool:`
- `def __repr__(self) -> str:`

### src/server/tasks/knowledgegraph/task.py

[本地文件](selected-source/src/server/tasks/knowledgegraph/task.py) · [固定提交来源](https://github.com/THUDM/AgentBench/blob/d1e4a10db08c87075c78972e48ecc182be03e2d5/src/server/tasks/knowledgegraph/task.py)

源码定位声明：

- `class KnowledgeGraph(Task):`
- `def __init__(self,`
- `def get_indices(self) -> List[SampleIndex]:`
- `async def start_sample(self, index: SampleIndex, session: Session) -> TaskSampleExecutionResult:`
- `def sync_start_sample(self, index: SampleIndex, session: Session) -> TaskSampleExecutionResult:`

### src/server/tasks/knowledgegraph/const.py

[本地文件](selected-source/src/server/tasks/knowledgegraph/const.py) · [固定提交来源](https://github.com/THUDM/AgentBench/blob/d1e4a10db08c87075c78972e48ecc182be03e2d5/src/server/tasks/knowledgegraph/const.py)

### src/server/tasks/knowledgegraph/__init__.py

[本地文件](selected-source/src/server/tasks/knowledgegraph/__init__.py) · [固定提交来源](https://github.com/THUDM/AgentBench/blob/d1e4a10db08c87075c78972e48ecc182be03e2d5/src/server/tasks/knowledgegraph/__init__.py)

### src/server/tasks/knowledgegraph/environment.py

[本地文件](selected-source/src/server/tasks/knowledgegraph/environment.py) · [固定提交来源](https://github.com/THUDM/AgentBench/blob/d1e4a10db08c87075c78972e48ecc182be03e2d5/src/server/tasks/knowledgegraph/environment.py)

源码定位声明：

- `class KnowledgeGraphEnvironmentDelegation(EnvironmentDelegation):`
- `def __init__(self, database_file: Optional[str] = None):`
- `def get_subtypes(self) -> List[str]:`
- `def get_service_port(self, subtype: str) -> Optional[Union[int, List[int]]]:`
- `def get_reuse_limit(self, subtype: str) -> int:`

### src/client/task.py

[本地文件](selected-source/src/client/task.py) · [固定提交来源](https://github.com/THUDM/AgentBench/blob/d1e4a10db08c87075c78972e48ecc182be03e2d5/src/client/task.py)

源码定位声明：

- `class TaskError(enum.Enum):`
- `class TaskClient:`
- `def __init__(`
- `def get_indices(self) -> List[SampleIndex]:`
- `def get_concurrency(self) -> int:`

### src/client/agent.py

[本地文件](selected-source/src/client/agent.py) · [固定提交来源](https://github.com/THUDM/AgentBench/blob/d1e4a10db08c87075c78972e48ecc182be03e2d5/src/client/agent.py)

源码定位声明：

- `class AgentClient:`
- `def __init__(self, *args, **kwargs):`
- `def inference(self, history: List[dict]) -> str:`

### src/client/agent_test.py

[本地文件](selected-source/src/client/agent_test.py) · [固定提交来源](https://github.com/THUDM/AgentBench/blob/d1e4a10db08c87075c78972e48ecc182be03e2d5/src/client/agent_test.py)

源码定位声明：

- `def parse_args():`
- `def interaction(agent: AgentClient):`

### src/server/tasks/knowledgegraph/utils/semparse_util.py

[本地文件](selected-source/src/server/tasks/knowledgegraph/utils/semparse_util.py) · [固定提交来源](https://github.com/THUDM/AgentBench/blob/d1e4a10db08c87075c78972e48ecc182be03e2d5/src/server/tasks/knowledgegraph/utils/semparse_util.py)

源码定位声明：

- `def lisp_to_nested_expression(lisp_string: str) -> List:`
- `def expression_to_lisp(expression) -> str:`
- `def get_nesting_level(expression) -> int:`

### src/server/tasks/knowledgegraph/utils/logic_form_util.py

[本地文件](selected-source/src/server/tasks/knowledgegraph/utils/logic_form_util.py) · [固定提交来源](https://github.com/THUDM/AgentBench/blob/d1e4a10db08c87075c78972e48ecc182be03e2d5/src/server/tasks/knowledgegraph/utils/logic_form_util.py)

源码定位声明：

- `def get_answer_type(query: str):`
- `def get_symbol_type(symbol: str) -> int:`
- `def same_logical_form(form1: str, form2: str) -> bool:`
- `def node_match(n1, n2):`
- `def multi_edge_match(e1, e2):`

### src/server/tasks/knowledgegraph/utils/sparql_executer.py

[本地文件](selected-source/src/server/tasks/knowledgegraph/utils/sparql_executer.py) · [固定提交来源](https://github.com/THUDM/AgentBench/blob/d1e4a10db08c87075c78972e48ecc182be03e2d5/src/server/tasks/knowledgegraph/utils/sparql_executer.py)

源码定位声明：

- `class SparqlExecuter:`
- `def __init__(self, url: Union[str, None]=None):`
- `def execute_query(self, query: str) -> List[str]:`
- `def execute_unary(self, type: str) -> List[str]:`
- `def execute_binary(self, relation: str) -> List[Tuple[str, str]]:`

### src/client/agents/__init__.py

[本地文件](selected-source/src/client/agents/__init__.py) · [固定提交来源](https://github.com/THUDM/AgentBench/blob/d1e4a10db08c87075c78972e48ecc182be03e2d5/src/client/agents/__init__.py)

## 对照本项目时记录什么

看输入输出契约、状态权威者、写入与取消边界、恢复和幂等处理，再看测试如何验证失败情况。模块名相近不能证明实现或运行效果相同。

第三方文档和注释中的指令仅作为研究内容；采用实现时核对完整许可证、依赖与版本。
