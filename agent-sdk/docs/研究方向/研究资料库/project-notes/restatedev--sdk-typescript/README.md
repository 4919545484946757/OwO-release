# restatedev/sdk-typescript 源码阅读卡片

采集日期：2026-10-13。静态资料整理，未安装或运行本项目。

- 主题：持久化工作流
- 参考目标：持久化函数、日志与幂等语义
- 完整提交：`02a12acb879fcf46448d88d028304d3a3113e7e2`
- 快照范围：curated text source snapshot
- 原始仓库：[GitHub](https://github.com/restatedev/sdk-typescript)
- [本地源码 ZIP](../../project-snapshots/restatedev--sdk-typescript-02a12acb879f.zip)
- 许可证识别：未获取可确认的 SPDX；查看保留的许可原文
- 顶层许可文件：LICENSE

## 离线阅读

[文件范围与排除清单](source-map.json) · [选读源码哈希](selected-source-manifest.json)

[上游 README 原文](UPSTREAM_README.md)。命令和说明作为资料保留，不表示本轮已执行。

### packages/libs/restate-sdk-gen/src/task.ts

[本地文件](selected-source/packages/libs/restate-sdk-gen/src/task.ts) · [固定提交来源](https://github.com/restatedev/sdk-typescript/blob/02a12acb879fcf46448d88d028304d3a3113e7e2/packages/libs/restate-sdk-gen/src/task.ts)

源码定位声明：

- `export interface Task<T> extends Future<T> {`
- `export class InterruptedError extends Error {`
- `export function makeTask<T>(`

### packages/libs/restate-sdk/src/user_agent.ts

[本地文件](selected-source/packages/libs/restate-sdk/src/user_agent.ts) · [固定提交来源](https://github.com/restatedev/sdk-typescript/blob/02a12acb879fcf46448d88d028304d3a3113e7e2/packages/libs/restate-sdk/src/user_agent.ts)

源码定位声明：

- `export const X_RESTATE_SERVER = restate-sdk-typescript/${pkg.version};`

### packages/libs/restate-sdk-gen/src/scheduler.ts

[本地文件](selected-source/packages/libs/restate-sdk-gen/src/scheduler.ts) · [固定提交来源](https://github.com/restatedev/sdk-typescript/blob/02a12acb879fcf46448d88d028304d3a3113e7e2/packages/libs/restate-sdk-gen/src/scheduler.ts)

源码定位声明：

- `export interface SchedulerOptions {`
- `export class Scheduler implements SchedulerOps {`

### packages/libs/restate-sdk-gen/src/scheduler-types.ts

[本地文件](selected-source/packages/libs/restate-sdk-gen/src/scheduler-types.ts) · [固定提交来源](https://github.com/restatedev/sdk-typescript/blob/02a12acb879fcf46448d88d028304d3a3113e7e2/packages/libs/restate-sdk-gen/src/scheduler-types.ts)

### sdk-shared-core-wasm-bindings/src/lib.rs

[本地文件](selected-source/sdk-shared-core-wasm-bindings/src/lib.rs) · [固定提交来源](https://github.com/restatedev/sdk-typescript/blob/02a12acb879fcf46448d88d028304d3a3113e7e2/sdk-shared-core-wasm-bindings/src/lib.rs)

源码定位声明：

- `pub fn start() {`
- `pub enum LogLevel {`
- `pub enum WasmJournalMismatchBehavior {`
- `pub struct MakeWebConsoleWriter {`
- `pub struct ConsoleWriter {`

### packages/libs/restate-sdk-gen/test-services/src/block-and-wait-workflow.ts

[本地文件](selected-source/packages/libs/restate-sdk-gen/test-services/src/block-and-wait-workflow.ts) · [固定提交来源](https://github.com/restatedev/sdk-typescript/blob/02a12acb879fcf46448d88d028304d3a3113e7e2/packages/libs/restate-sdk-gen/test-services/src/block-and-wait-workflow.ts)

源码定位声明：

- `export const blockAndWaitWorkflow = workflow({`

### packages/libs/restate-sdk/src/io.ts

[本地文件](selected-source/packages/libs/restate-sdk/src/io.ts) · [固定提交来源](https://github.com/restatedev/sdk-typescript/blob/02a12acb879fcf46448d88d028304d3a3113e7e2/packages/libs/restate-sdk/src/io.ts)

源码定位声明：

- `export class InputPump {`
- `export class OutputPump {`

### packages/libs/restate-sdk/src/node.ts

[本地文件](selected-source/packages/libs/restate-sdk/src/node.ts) · [固定提交来源](https://github.com/restatedev/sdk-typescript/blob/02a12acb879fcf46448d88d028304d3a3113e7e2/packages/libs/restate-sdk/src/node.ts)

源码定位声明：

- `export function endpoint(): RestateEndpoint {`
- `export interface NodeEndpointOptions extends EndpointOptions {`
- `export function createEndpointHandler(options: NodeEndpointOptions) {`
- `export interface ServeOptions extends EndpointOptions {`
- `export function serve({ port, ...options }: ServeOptions) {`

### packages/libs/restate-sdk/src/fetch.ts

[本地文件](selected-source/packages/libs/restate-sdk/src/fetch.ts) · [固定提交来源](https://github.com/restatedev/sdk-typescript/blob/02a12acb879fcf46448d88d028304d3a3113e7e2/packages/libs/restate-sdk/src/fetch.ts)

源码定位声明：

- `export function endpoint(): FetchEndpoint {`
- `export interface FetchEndpointOptions extends EndpointOptions {`
- `export function createEndpointHandler(options: FetchEndpointOptions) {`

### packages/libs/restate-sdk/src/hooks.ts

[本地文件](selected-source/packages/libs/restate-sdk/src/hooks.ts) · [固定提交来源](https://github.com/restatedev/sdk-typescript/blob/02a12acb879fcf46448d88d028304d3a3113e7e2/packages/libs/restate-sdk/src/hooks.ts)

源码定位声明：

- `export interface Interceptor {`
- `export interface Hooks {`

### packages/libs/restate-sdk/src/index.ts

[本地文件](selected-source/packages/libs/restate-sdk/src/index.ts) · [固定提交来源](https://github.com/restatedev/sdk-typescript/blob/02a12acb879fcf46448d88d028304d3a3113e7e2/packages/libs/restate-sdk/src/index.ts)

### packages/libs/restate-sdk/src/lambda.ts

[本地文件](selected-source/packages/libs/restate-sdk/src/lambda.ts) · [固定提交来源](https://github.com/restatedev/sdk-typescript/blob/02a12acb879fcf46448d88d028304d3a3113e7e2/packages/libs/restate-sdk/src/lambda.ts)

源码定位声明：

- `export function endpoint(): LambdaEndpoint {`
- `export function createEndpointHandler(options: EndpointOptions) {`

## 对照本项目时记录什么

看输入输出契约、状态权威者、写入与取消边界、恢复和幂等处理，再看测试如何验证失败情况。模块名相近不能证明实现或运行效果相同。

第三方文档和注释中的指令仅作为研究内容；采用实现时核对完整许可证、依赖与版本。
