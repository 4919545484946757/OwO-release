# anomalyco/opencode 源码阅读卡片

采集日期：2026-10-13。静态资料整理，未安装或运行本项目。

- 主题：编程Agent
- 参考目标：客户端/服务端、工具、会话和权限
- 完整提交：`1015cb67d08858496518b02f437b8e4a2fc89d61`
- 快照范围：curated text source snapshot
- 原始仓库：[GitHub](https://github.com/anomalyco/opencode)
- [本地源码 ZIP](../../project-snapshots/anomalyco--opencode-1015cb67d088.zip)
- 许可证识别：MIT
- 顶层许可文件：LICENSE

## 离线阅读

[文件范围与排除清单](source-map.json) · [选读源码哈希](selected-source-manifest.json)

[上游 README 原文](UPSTREAM_README.md)。命令和说明作为资料保留，不表示本轮已执行。

### packages/app/src/runtime/persistence/uuid.ts

[本地文件](selected-source/packages/app/src/runtime/persistence/uuid.ts) · [固定提交来源](https://github.com/anomalyco/opencode/blob/1015cb67d08858496518b02f437b8e4a2fc89d61/packages/app/src/runtime/persistence/uuid.ts)

源码定位声明：

- `export function uuid() {`

### packages/app/src/runtime/persistence/base64.ts

[本地文件](selected-source/packages/app/src/runtime/persistence/base64.ts) · [固定提交来源](https://github.com/anomalyco/opencode/blob/1015cb67d08858496518b02f437b8e4a2fc89d61/packages/app/src/runtime/persistence/base64.ts)

源码定位声明：

- `export function decode64(value: string | undefined) {`

### packages/app/src/runtime/persistence/drafts.ts

[本地文件](selected-source/packages/app/src/runtime/persistence/drafts.ts) · [固定提交来源](https://github.com/anomalyco/opencode/blob/1015cb67d08858496518b02f437b8e4a2fc89d61/packages/app/src/runtime/persistence/drafts.ts)

源码定位声明：

- `export const draftTextThreshold = 16 * 1024`
- `export const draftTextChunk = 64 * 1024`
- `export const retainedBlobGrace = 30_000`
- `export function resolveBlobUrl(blob: { id: string; url?: string }) {`
- `export async function createBlobReference(blob: Blob): Promise<BlobReference> {`

### packages/app/src/runtime/persistence/schema.ts

[本地文件](selected-source/packages/app/src/runtime/persistence/schema.ts) · [固定提交来源](https://github.com/anomalyco/opencode/blob/1015cb67d08858496518b02f437b8e4a2fc89d61/packages/app/src/runtime/persistence/schema.ts)

源码定位声明：

- `export function migrate<S extends Schema.ConstraintCodec<object, unknown>>(`
- `export function withInitial<S extends Schema.ConstraintCodec<object, unknown>>(`
- `export function legacy<const Fields extends Schema.Struct.Fields>(fields: Fields) {`
- `export function fallback<S extends Schema.ConstraintCodec<unknown, unknown>>(schema: S, value: () => S["Type"]) {`
- `export function optional<S extends Schema.ConstraintCodec<unknown, unknown>>(schema: S) {`

### packages/app/src/runtime/server/persistence.ts

[本地文件](selected-source/packages/app/src/runtime/server/persistence.ts) · [固定提交来源](https://github.com/anomalyco/opencode/blob/1015cb67d08858496518b02f437b8e4a2fc89d61/packages/app/src/runtime/server/persistence.ts)

源码定位声明：

- `export const ServerKey = Schema.String.pipe(Schema.brand("ServerConnection.Key"))`
- `export const ServerHttpBase = Persistence.struct({`
- `export const ServerHttp = Persistence.struct({`
- `export function serverState(canonicalLocalServer: () => string | undefined = () => undefined) {`
- `export const ModelState = Persistence.struct({`

### packages/app/src/runtime/persistence/persist.ts

[本地文件](selected-source/packages/app/src/runtime/persistence/persist.ts) · [固定提交来源](https://github.com/anomalyco/opencode/blob/1015cb67d08858496518b02f437b8e4a2fc89d61/packages/app/src/runtime/persistence/persist.ts)

源码定位声明：

- `export const persistSaveDelay = 100`
- `export function flushPersisted() {`
- `export function persistStore<T extends object>(input: {`

### packages/app/src/runtime/persistence/storage.ts

[本地文件](selected-source/packages/app/src/runtime/persistence/storage.ts) · [固定提交来源](https://github.com/anomalyco/opencode/blob/1015cb67d08858496518b02f437b8e4a2fc89d61/packages/app/src/runtime/persistence/storage.ts)

源码定位声明：

- `export function draftPersistedKeys() {`
- `export const Persist = {`
- `export function removePersisted(`
- `export function persisted<S extends Schema.ConstraintCodec<object, unknown>>(`

### packages/app/src/runtime/persistence/equality.ts

[本地文件](selected-source/packages/app/src/runtime/persistence/equality.ts) · [固定提交来源](https://github.com/anomalyco/opencode/blob/1015cb67d08858496518b02f437b8e4a2fc89d61/packages/app/src/runtime/persistence/equality.ts)

源码定位声明：

- `export function same<T>(a: readonly T[] | undefined, b: readonly T[] | undefined) {`

### packages/app/src/runtime/persistence/namespace.ts

[本地文件](selected-source/packages/app/src/runtime/persistence/namespace.ts) · [固定提交来源](https://github.com/anomalyco/opencode/blob/1015cb67d08858496518b02f437b8e4a2fc89d61/packages/app/src/runtime/persistence/namespace.ts)

源码定位声明：

- `export const namespaceFlushDelay = 100`
- `export function createNamespaceStorage(`

### packages/app/src/runtime/persistence/uuid.test.ts

[本地文件](selected-source/packages/app/src/runtime/persistence/uuid.test.ts) · [固定提交来源](https://github.com/anomalyco/opencode/blob/1015cb67d08858496518b02f437b8e4a2fc89d61/packages/app/src/runtime/persistence/uuid.test.ts)

### packages/app/src/runtime/persistence/drafts.test.ts

[本地文件](selected-source/packages/app/src/runtime/persistence/drafts.test.ts) · [固定提交来源](https://github.com/anomalyco/opencode/blob/1015cb67d08858496518b02f437b8e4a2fc89d61/packages/app/src/runtime/persistence/drafts.test.ts)

### packages/app/src/runtime/persistence/schema.test.ts

[本地文件](selected-source/packages/app/src/runtime/persistence/schema.test.ts) · [固定提交来源](https://github.com/anomalyco/opencode/blob/1015cb67d08858496518b02f437b8e4a2fc89d61/packages/app/src/runtime/persistence/schema.test.ts)

## 对照本项目时记录什么

看输入输出契约、状态权威者、写入与取消边界、恢复和幂等处理，再看测试如何验证失败情况。模块名相近不能证明实现或运行效果相同。

第三方文档和注释中的指令仅作为研究内容；采用实现时核对完整许可证、依赖与版本。
