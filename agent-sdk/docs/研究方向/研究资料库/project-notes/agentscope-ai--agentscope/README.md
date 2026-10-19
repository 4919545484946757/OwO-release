# agentscope-ai/agentscope 源码阅读卡片

采集日期：2026-10-13。静态资料整理，未安装或运行本项目。

- 主题：多Agent框架
- 参考目标：消息、运行时及协作管理
- 完整提交：`bcf908f6d851457ea4ca7debe6674a2a05d98d14`
- 快照范围：curated text source snapshot
- 原始仓库：[GitHub](https://github.com/agentscope-ai/agentscope)
- [本地源码 ZIP](../../project-snapshots/agentscope-ai--agentscope-bcf908f6d851.zip)
- 许可证识别：Apache-2.0
- 顶层许可文件：LICENSE

## 离线阅读

[文件范围与排除清单](source-map.json) · [选读源码哈希](selected-source-manifest.json)

[上游 README 原文](UPSTREAM_README.md)。命令和说明作为资料保留，不表示本轮已执行。

### src/agentscope/state/_task.py

[本地文件](selected-source/src/agentscope/state/_task.py) · [固定提交来源](https://github.com/agentscope-ai/agentscope/blob/bcf908f6d851457ea4ca7debe6674a2a05d98d14/src/agentscope/state/_task.py)

源码定位声明：

- `class Task(BaseModel):`

### src/agentscope/permission/_rule.py

[本地文件](selected-source/src/agentscope/permission/_rule.py) · [固定提交来源](https://github.com/agentscope-ai/agentscope/blob/bcf908f6d851457ea4ca7debe6674a2a05d98d14/src/agentscope/permission/_rule.py)

源码定位声明：

- `class PermissionRule(BaseModel):`

### src/agentscope/permission/_types.py

[本地文件](selected-source/src/agentscope/permission/_types.py) · [固定提交来源](https://github.com/agentscope-ai/agentscope/blob/bcf908f6d851457ea4ca7debe6674a2a05d98d14/src/agentscope/permission/_types.py)

源码定位声明：

- `class PermissionMode(Enum):`
- `class PermissionBehavior(Enum):`

### src/agentscope/permission/_engine.py

[本地文件](selected-source/src/agentscope/permission/_engine.py) · [固定提交来源](https://github.com/agentscope-ai/agentscope/blob/bcf908f6d851457ea4ca7debe6674a2a05d98d14/src/agentscope/permission/_engine.py)

源码定位声明：

- `class PermissionEngine:`
- `def __init__(`
- `def add_rule(self, rule: PermissionRule) -> None:`
- `async def check_permission(`
- `async def _check_default(`

### src/agentscope/permission/__init__.py

[本地文件](selected-source/src/agentscope/permission/__init__.py) · [固定提交来源](https://github.com/agentscope-ai/agentscope/blob/bcf908f6d851457ea4ca7debe6674a2a05d98d14/src/agentscope/permission/__init__.py)

### src/agentscope/permission/_context.py

[本地文件](selected-source/src/agentscope/permission/_context.py) · [固定提交来源](https://github.com/agentscope-ai/agentscope/blob/bcf908f6d851457ea4ca7debe6674a2a05d98d14/src/agentscope/permission/_context.py)

源码定位声明：

- `class AdditionalWorkingDirectory(BaseModel):`
- `class PermissionContext(BaseModel):`

### src/agentscope/permission/_decision.py

[本地文件](selected-source/src/agentscope/permission/_decision.py) · [固定提交来源](https://github.com/agentscope-ai/agentscope/blob/bcf908f6d851457ea4ca7debe6674a2a05d98d14/src/agentscope/permission/_decision.py)

源码定位声明：

- `class PermissionDecision:`

### src/agentscope/app/access/_policy.py

[本地文件](selected-source/src/agentscope/app/access/_policy.py) · [固定提交来源](https://github.com/agentscope-ai/agentscope/blob/bcf908f6d851457ea4ca7debe6674a2a05d98d14/src/agentscope/app/access/_policy.py)

源码定位声明：

- `class ResourceKind(StrEnum):`
- `class ResourcePermission(StrEnum):`
- `class ResourceRef(BaseModel):`
- `class ResourceAccessPolicyBase(ABC):`
- `async def list_accessible(`

### src/agentscope/tool/_task/__init__.py

[本地文件](selected-source/src/agentscope/tool/_task/__init__.py) · [固定提交来源](https://github.com/agentscope-ai/agentscope/blob/bcf908f6d851457ea4ca7debe6674a2a05d98d14/src/agentscope/tool/_task/__init__.py)

### src/agentscope/tool/_task/_get_task.py

[本地文件](selected-source/src/agentscope/tool/_task/_get_task.py) · [固定提交来源](https://github.com/agentscope-ai/agentscope/blob/bcf908f6d851457ea4ca7debe6674a2a05d98d14/src/agentscope/tool/_task/_get_task.py)

源码定位声明：

- `class _TaskGetParams(BaseModel):`
- `class TaskGet(_TaskToolBase):`
- `async def call(`

### src/agentscope/tool/_task/_list_task.py

[本地文件](selected-source/src/agentscope/tool/_task/_list_task.py) · [固定提交来源](https://github.com/agentscope-ai/agentscope/blob/bcf908f6d851457ea4ca7debe6674a2a05d98d14/src/agentscope/tool/_task/_list_task.py)

源码定位声明：

- `class _TaskListParams(ParamsBase):`
- `class TaskList(_TaskToolBase):`
- `async def call(self, _agent_state: AgentState) -> ToolChunk:`

### src/agentscope/tool/_task/_create_task.py

[本地文件](selected-source/src/agentscope/tool/_task/_create_task.py) · [固定提交来源](https://github.com/agentscope-ai/agentscope/blob/bcf908f6d851457ea4ca7debe6674a2a05d98d14/src/agentscope/tool/_task/_create_task.py)

源码定位声明：

- `class _TaskCreateParams(BaseModel):`
- `class TaskCreate(_TaskToolBase):`
- `async def call(`

## 对照本项目时记录什么

看输入输出契约、状态权威者、写入与取消边界、恢复和幂等处理，再看测试如何验证失败情况。模块名相近不能证明实现或运行效果相同。

第三方文档和注释中的指令仅作为研究内容；采用实现时核对完整许可证、依赖与版本。
