# temporalio/sdk-python 源码阅读卡片

采集日期：2026-10-13。静态资料整理，未安装或运行本项目。

- 主题：持久化工作流
- 参考目标：workflow/activity 分离、重放和取消
- 完整提交：`dca3a006dcd7a44114d8a36ec10359855ee427f7`
- 快照范围：curated text source snapshot
- 原始仓库：[GitHub](https://github.com/temporalio/sdk-python)
- [本地源码 ZIP](../../project-snapshots/temporalio--sdk-python-dca3a006dcd7.zip)
- 许可证识别：未获取可确认的 SPDX；查看保留的许可原文
- 顶层许可文件：LICENSE

## 离线阅读

[文件范围与排除清单](source-map.json) · [选读源码哈希](selected-source-manifest.json)

[上游 README 原文](UPSTREAM_README.md)。命令和说明作为资料保留，不表示本轮已执行。

### temporalio/workflow/_activities.py

[本地文件](selected-source/temporalio/workflow/_activities.py) · [固定提交来源](https://github.com/temporalio/sdk-python/blob/dca3a006dcd7a44114d8a36ec10359855ee427f7/temporalio/workflow/_activities.py)

源码定位声明：

- `class _AsyncioTask(asyncio.Task[AnyType]):`
- `class _AsyncioTask(Generic[AnyType], asyncio.Task):`
- `class ActivityHandle(_AsyncioTask[ReturnType]):  # type: ignore[type-var]`
- `class ActivityCancellationType(IntEnum):`
- `class ActivityConfig(TypedDict, total=False):`

### temporalio/worker/_workflow_instance.py

[本地文件](selected-source/temporalio/worker/_workflow_instance.py) · [固定提交来源](https://github.com/temporalio/sdk-python/blob/dca3a006dcd7a44114d8a36ec10359855ee427f7/temporalio/worker/_workflow_instance.py)

源码定位声明：

- `async def _shield_await(fut: asyncio.Future[Any]) -> Any:`
- `class WorkflowRunner(ABC):`
- `def prepare_workflow(self, defn: temporalio.workflow._Definition) -> None:`
- `def create_instance(self, det: WorkflowInstanceDetails) -> WorkflowInstance:`
- `def set_worker_level_failure_exception_types(`

### temporalio/workflow/__init__.py

[本地文件](selected-source/temporalio/workflow/__init__.py) · [固定提交来源](https://github.com/temporalio/sdk-python/blob/dca3a006dcd7a44114d8a36ec10359855ee427f7/temporalio/workflow/__init__.py)

### temporalio/bridge/src/runtime.rs

[本地文件](selected-source/temporalio/bridge/src/runtime.rs) · [固定提交来源](https://github.com/temporalio/sdk-python/blob/dca3a006dcd7a44114d8a36ec10359855ee427f7/temporalio/bridge/src/runtime.rs)

源码定位声明：

- `pub struct RuntimeRef {`
- `pub(crate) struct Runtime {`
- `pub struct TelemetryConfig {`
- `pub struct LoggingConfig {`
- `pub struct BufferedLogEntry {`

### temporalio/contrib/deepagents/workflow.py

[本地文件](selected-source/temporalio/contrib/deepagents/workflow.py) · [固定提交来源](https://github.com/temporalio/sdk-python/blob/dca3a006dcd7a44114d8a36ec10359855ee427f7/temporalio/contrib/deepagents/workflow.py)

### temporalio/contrib/openai_agents/workflow.py

[本地文件](selected-source/temporalio/contrib/openai_agents/workflow.py) · [固定提交来源](https://github.com/temporalio/sdk-python/blob/dca3a006dcd7a44114d8a36ec10359855ee427f7/temporalio/contrib/openai_agents/workflow.py)

### temporalio/worker/workflow_sandbox/_runner.py

[本地文件](selected-source/temporalio/worker/workflow_sandbox/_runner.py) · [固定提交来源](https://github.com/temporalio/sdk-python/blob/dca3a006dcd7a44114d8a36ec10359855ee427f7/temporalio/worker/workflow_sandbox/_runner.py)

源码定位声明：

- `class SandboxedWorkflowRunner(WorkflowRunner):`
- `def prepare_workflow(self, defn: temporalio.workflow._Definition) -> None:`
- `def create_instance(self, det: WorkflowInstanceDetails) -> WorkflowInstance:`
- `def set_worker_level_failure_exception_types(`
- `class _Instance(WorkflowInstance):`

### temporalio/contrib/google_adk_agents/workflow.py

[本地文件](selected-source/temporalio/contrib/google_adk_agents/workflow.py) · [固定提交来源](https://github.com/temporalio/sdk-python/blob/dca3a006dcd7a44114d8a36ec10359855ee427f7/temporalio/contrib/google_adk_agents/workflow.py)

### temporalio/runtime.py

[本地文件](selected-source/temporalio/runtime.py) · [固定提交来源](https://github.com/temporalio/sdk-python/blob/dca3a006dcd7a44114d8a36ec10359855ee427f7/temporalio/runtime.py)

源码定位声明：

- `class _RuntimeRef:`
- `def __init__(`
- `def default(self) -> Runtime:`
- `def prevent_default(self):`
- `def set_default(`

### temporalio/bridge/src/lib.rs

[本地文件](selected-source/temporalio/bridge/src/lib.rs) · [固定提交来源](https://github.com/temporalio/sdk-python/blob/dca3a006dcd7a44114d8a36ec10359855ee427f7/temporalio/bridge/src/lib.rs)

### temporalio/bridge/src/client.rs

[本地文件](selected-source/temporalio/bridge/src/client.rs) · [固定提交来源](https://github.com/temporalio/sdk-python/blob/dca3a006dcd7a44114d8a36ec10359855ee427f7/temporalio/bridge/src/client.rs)

源码定位声明：

- `pub struct ClientRef {`
- `pub struct ClientConfig {`
- `pub(crate) struct RpcCall {`
- `pub fn connect_client<'a>(`
- `pub(crate) fn rpc_req<P: prost::Message + Default>(call: RpcCall) -> PyResult<tonic::Request<P>> {`

### temporalio/bridge/src/metric.rs

[本地文件](selected-source/temporalio/bridge/src/metric.rs) · [固定提交来源](https://github.com/temporalio/sdk-python/blob/dca3a006dcd7a44114d8a36ec10359855ee427f7/temporalio/bridge/src/metric.rs)

源码定位声明：

- `pub struct MetricMeterRef {`
- `pub struct MetricAttributesRef {`
- `pub struct MetricCounterRef {`
- `pub struct MetricHistogramRef {`
- `pub struct MetricHistogramFloatRef {`

## 对照本项目时记录什么

看输入输出契约、状态权威者、写入与取消边界、恢复和幂等处理，再看测试如何验证失败情况。模块名相近不能证明实现或运行效果相同。

第三方文档和注释中的指令仅作为研究内容；采用实现时核对完整许可证、依赖与版本。
