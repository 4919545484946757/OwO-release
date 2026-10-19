# PrefectHQ/prefect 源码阅读卡片

采集日期：2026-10-13。静态资料整理，未安装或运行本项目。

- 主题：持久化工作流
- 参考目标：任务、flow、状态、重试与可观察性
- 完整提交：`302beb37eb79cd6ba37fcaa9fa002c4429b45702`
- 快照范围：curated text source snapshot
- 原始仓库：[GitHub](https://github.com/PrefectHQ/prefect)
- [本地源码 ZIP](../../project-snapshots/PrefectHQ--prefect-302beb37eb79.zip)
- 许可证识别：未获取可确认的 SPDX；查看保留的许可原文
- 顶层许可文件：LICENSE

## 离线阅读

[文件范围与排除清单](source-map.json) · [选读源码哈希](selected-source-manifest.json)

[上游 README 原文](UPSTREAM_README.md)。命令和说明作为资料保留，不表示本轮已执行。

### src/prefect/task_runners.py

[本地文件](selected-source/src/prefect/task_runners.py) · [固定提交来源](https://github.com/PrefectHQ/prefect/blob/302beb37eb79cd6ba37fcaa9fa002c4429b45702/src/prefect/task_runners.py)

源码定位声明：

- `class TaskRunner(abc.ABC, Generic[F]):`
- `def __init__(self):`
- `def name(self) -> str:`
- `def duplicate(self) -> Self:`
- `def submit(`

### src/prefect/runtime/task_run.py

[本地文件](selected-source/src/prefect/runtime/task_run.py) · [固定提交来源](https://github.com/PrefectHQ/prefect/blob/302beb37eb79cd6ba37fcaa9fa002c4429b45702/src/prefect/runtime/task_run.py)

源码定位声明：

- `def __getattr__(name: str) -> Any:`
- `def __dir__() -> list[str]:`
- `def get_id() -> str | None:`
- `def get_tags() -> list[str]:`
- `def get_run_count() -> int:`

### src/prefect/runner/_flow_run_executor.py

[本地文件](selected-source/src/prefect/runner/_flow_run_executor.py) · [固定提交来源](https://github.com/PrefectHQ/prefect/blob/302beb37eb79cd6ba37fcaa9fa002c4429b45702/src/prefect/runner/_flow_run_executor.py)

源码定位声明：

- `class ProcessStarter(Protocol):`
- `async def start(`
- `class FlowRunExecutionResult:`
- `class FlowRunExecutor:`
- `def __init__(`

### src/prefect/runner/_workspace_runtime.py

[本地文件](selected-source/src/prefect/runner/_workspace_runtime.py) · [固定提交来源](https://github.com/PrefectHQ/prefect/blob/302beb37eb79cd6ba37fcaa9fa002c4429b45702/src/prefect/runner/_workspace_runtime.py)

源码定位声明：

- `class WorkspaceSupervisorConfig(BaseModel):`
- `class PreparedWorkspaceManifest(BaseModel):`
- `def write_private_model(path: Path, model: BaseModel) -> None:`
- `def read_model(path: Path, model: type[ModelT]) -> ModelT:`

### src/prefect/runner/_workspace_runtime_bootstrap.py

[本地文件](selected-source/src/prefect/runner/_workspace_runtime_bootstrap.py) · [固定提交来源](https://github.com/PrefectHQ/prefect/blob/302beb37eb79cd6ba37fcaa9fa002c4429b45702/src/prefect/runner/_workspace_runtime_bootstrap.py)

源码定位声明：

- `def _run_engine(entrypoint: str) -> int:`
- `def _read_manifest(path: Path) -> dict[str, Any]:`
- `def _absolute_file_entrypoint(manifest: dict[str, Any]) -> str:`
- `async def _run_sync(call: Callable[..., Any], *args: Any) -> Any:`
- `async def _load_flow(manifest: dict[str, Any]) -> Flow[Any, Any]:`

### ui-v2/src/graphs/factories/nodeTaskRun.ts

[本地文件](selected-source/ui-v2/src/graphs/factories/nodeTaskRun.ts) · [固定提交来源](https://github.com/PrefectHQ/prefect/blob/302beb37eb79cd6ba37fcaa9fa002c4429b45702/ui-v2/src/graphs/factories/nodeTaskRun.ts)

源码定位声明：

- `export async function taskRunContainerFactory(`

### src/integrations/prefect-ray/prefect_ray/task_runners.py

[本地文件](selected-source/src/integrations/prefect-ray/prefect_ray/task_runners.py) · [固定提交来源](https://github.com/PrefectHQ/prefect/blob/302beb37eb79cd6ba37fcaa9fa002c4429b45702/src/integrations/prefect-ray/prefect_ray/task_runners.py)

源码定位声明：

- `def shout(number):`
- `def count_to(highest_number):`
- `def shout(number):`
- `def count_to(highest_number):`
- `class PrefectRayFuture(PrefectWrappedFuture[R, "ray.ObjectRef"]):`

### src/integrations/prefect-dask/prefect_dask/task_runners.py

[本地文件](selected-source/src/integrations/prefect-dask/prefect_dask/task_runners.py) · [固定提交来源](https://github.com/PrefectHQ/prefect/blob/302beb37eb79cd6ba37fcaa9fa002c4429b45702/src/integrations/prefect-dask/prefect_dask/task_runners.py)

源码定位声明：

- `def shout(number):`
- `def count_to(highest_number):`
- `def shout(number):`
- `def count_to(highest_number):`
- `class PrefectDaskFuture(PrefectWrappedFuture[R, distributed.Future]):`

### src/prefect/agent.py

[本地文件](selected-source/src/prefect/agent.py) · [固定提交来源](https://github.com/PrefectHQ/prefect/blob/302beb37eb79cd6ba37fcaa9fa002c4429b45702/src/prefect/agent.py)

### src/prefect/tasks.py

[本地文件](selected-source/src/prefect/tasks.py) · [固定提交来源](https://github.com/PrefectHQ/prefect/blob/302beb37eb79cd6ba37fcaa9fa002c4429b45702/src/prefect/tasks.py)

源码定位声明：

- `class TaskRunNameCallbackWithParameters(Protocol):`
- `def is_callback_with_parameters(cls, callable: Callable[..., str]) -> TypeIs[Self]:`
- `def __call__(self, parameters: dict[str, Any]) -> str: ...`
- `class TaskOptions(TypedDict, total=False):`
- `def task_input_hash(`

### src/prefect/task_runs.py

[本地文件](selected-source/src/prefect/task_runs.py) · [固定提交来源](https://github.com/PrefectHQ/prefect/blob/302beb37eb79cd6ba37fcaa9fa002c4429b45702/src/prefect/task_runs.py)

源码定位声明：

- `class TaskRunWaiter:`
- `async def test_task():`
- `async def main():`
- `def __init__(self):`
- `def start(self) -> None:`

### src/prefect/task_engine.py

[本地文件](selected-source/src/prefect/task_engine.py) · [固定提交来源](https://github.com/PrefectHQ/prefect/blob/302beb37eb79cd6ba37fcaa9fa002c4429b45702/src/prefect/task_engine.py)

源码定位声明：

- `def _create_task_run_locally(`
- `class TaskRunTimeoutError(TimeoutError):`
- `class BaseTaskRunEngine(Generic[P, R]):`
- `def __post_init__(self) -> None:`
- `def state(self) -> State:`

## 对照本项目时记录什么

看输入输出契约、状态权威者、写入与取消边界、恢复和幂等处理，再看测试如何验证失败情况。模块名相近不能证明实现或运行效果相同。

第三方文档和注释中的指令仅作为研究内容；采用实现时核对完整许可证、依赖与版本。
