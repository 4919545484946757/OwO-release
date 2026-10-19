# mem0ai/mem0 源码阅读卡片

采集日期：2026-10-13。静态资料整理，未安装或运行本项目。

- 主题：长期记忆
- 参考目标：记忆提取、检索、更新与删除
- 完整提交：`d26ba5e4518cef7fff3c66d4f715b21adf9feabe`
- 快照范围：curated text source snapshot
- 原始仓库：[GitHub](https://github.com/mem0ai/mem0)
- [本地源码 ZIP](../../project-snapshots/mem0ai--mem0-d26ba5e4518c.zip)
- 许可证识别：未获取可确认的 SPDX；查看保留的许可原文
- 顶层许可文件：LICENSE

## 离线阅读

[文件范围与排除清单](source-map.json) · [选读源码哈希](selected-source-manifest.json)

[上游 README 原文](UPSTREAM_README.md)。命令和说明作为资料保留，不表示本轮已执行。

### integrations/pi-agent-plugin/src/memory/tools.ts

[本地文件](selected-source/integrations/pi-agent-plugin/src/memory/tools.ts) · [固定提交来源](https://github.com/mem0ai/mem0/blob/d26ba5e4518cef7fff3c66d4f715b21adf9feabe/integrations/pi-agent-plugin/src/memory/tools.ts)

源码定位声明：

- `export function buildToolExecute(`
- `export function registerMemoryTool(`

### integrations/pi-agent-plugin/src/memory/scoping.ts

[本地文件](selected-source/integrations/pi-agent-plugin/src/memory/scoping.ts) · [固定提交来源](https://github.com/mem0ai/mem0/blob/d26ba5e4518cef7fff3c66d4f715b21adf9feabe/integrations/pi-agent-plugin/src/memory/scoping.ts)

源码定位声明：

- `export function detectAppId(cwd: string): string {`
- `export function detectRunId(sessionFile: string | undefined): string {`
- `export function resolveSearchFilters(`
- `export function resolveAddParams(`

### integrations/pi-agent-plugin/src/memory/formatting.ts

[本地文件](selected-source/integrations/pi-agent-plugin/src/memory/formatting.ts) · [固定提交来源](https://github.com/mem0ai/mem0/blob/d26ba5e4518cef7fff3c66d4f715b21adf9feabe/integrations/pi-agent-plugin/src/memory/formatting.ts)

### integrations/pi-agent-plugin/src/memory/scoping.test.ts

[本地文件](selected-source/integrations/pi-agent-plugin/src/memory/scoping.test.ts) · [固定提交来源](https://github.com/mem0ai/mem0/blob/d26ba5e4518cef7fff3c66d4f715b21adf9feabe/integrations/pi-agent-plugin/src/memory/scoping.test.ts)

### cli/node/src/agent-detect.ts

[本地文件](selected-source/cli/node/src/agent-detect.ts) · [固定提交来源](https://github.com/mem0ai/mem0/blob/d26ba5e4518cef7fff3c66d4f715b21adf9feabe/cli/node/src/agent-detect.ts)

源码定位声明：

- `export function detectAgentCaller(): string | null {`

### integrations/pi-agent-plugin/src/entry.ts

[本地文件](selected-source/integrations/pi-agent-plugin/src/entry.ts) · [固定提交来源](https://github.com/mem0ai/mem0/blob/d26ba5e4518cef7fff3c66d4f715b21adf9feabe/integrations/pi-agent-plugin/src/entry.ts)

源码定位声明：

- `export function resolveUserId(configUserId: string): string {`

### integrations/pi-agent-plugin/src/index.ts

[本地文件](selected-source/integrations/pi-agent-plugin/src/index.ts) · [固定提交来源](https://github.com/mem0ai/mem0/blob/d26ba5e4518cef7fff3c66d4f715b21adf9feabe/integrations/pi-agent-plugin/src/index.ts)

### integrations/pi-agent-plugin/src/types.ts

[本地文件](selected-source/integrations/pi-agent-plugin/src/types.ts) · [固定提交来源](https://github.com/mem0ai/mem0/blob/d26ba5e4518cef7fff3c66d4f715b21adf9feabe/integrations/pi-agent-plugin/src/types.ts)

源码定位声明：

- `export interface Mem0Config {`
- `export interface ScopeContext {`
- `export interface CustomCategory {`
- `export const DEFAULT_CUSTOM_CATEGORIES: CustomCategory[] = [`

### integrations/pi-agent-plugin/src/prompt.ts

[本地文件](selected-source/integrations/pi-agent-plugin/src/prompt.ts) · [固定提交来源](https://github.com/mem0ai/mem0/blob/d26ba5e4518cef7fff3c66d4f715b21adf9feabe/integrations/pi-agent-plugin/src/prompt.ts)

源码定位声明：

- `export const MEMORY_POLICY = <mem0-memory-policy>`

### integrations/pi-agent-plugin/src/commands.ts

[本地文件](selected-source/integrations/pi-agent-plugin/src/commands.ts) · [固定提交来源](https://github.com/mem0ai/mem0/blob/d26ba5e4518cef7fff3c66d4f715b21adf9feabe/integrations/pi-agent-plugin/src/commands.ts)

源码定位声明：

- `export function registerCommands(`

### integrations/pi-agent-plugin/src/telemetry.ts

[本地文件](selected-source/integrations/pi-agent-plugin/src/telemetry.ts) · [固定提交来源](https://github.com/mem0ai/mem0/blob/d26ba5e4518cef7fff3c66d4f715b21adf9feabe/integrations/pi-agent-plugin/src/telemetry.ts)

源码定位声明：

- `export function captureEvent(`
- `export function captureToolEvent(`
- `export function captureCommandEvent(`
- `export function _getEventQueue(): Record<string, unknown>[] {`
- `export function _resetForTesting(): void {`

### integrations/pi-agent-plugin/src/entry.test.ts

[本地文件](selected-source/integrations/pi-agent-plugin/src/entry.test.ts) · [固定提交来源](https://github.com/mem0ai/mem0/blob/d26ba5e4518cef7fff3c66d4f715b21adf9feabe/integrations/pi-agent-plugin/src/entry.test.ts)

## 对照本项目时记录什么

看输入输出契约、状态权威者、写入与取消边界、恢复和幂等处理，再看测试如何验证失败情况。模块名相近不能证明实现或运行效果相同。

第三方文档和注释中的指令仅作为研究内容；采用实现时核对完整许可证、依赖与版本。
