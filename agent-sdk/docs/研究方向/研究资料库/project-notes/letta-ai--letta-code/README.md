# letta-ai/letta-code 源码阅读卡片

采集日期：2026-10-13。静态资料整理，未安装或运行本项目。

- 主题：有状态Agent
- 参考目标：当前 Letta Agent harness、server 与记忆集成
- 完整提交：`2943efe47e8c49e8ef2b78df100ba8e262c2415a`
- 快照范围：curated text source snapshot
- 原始仓库：[GitHub](https://github.com/letta-ai/letta-code)
- [本地源码 ZIP](../../project-snapshots/letta-ai--letta-code-2943efe47e8c.zip)
- 许可证识别：未获取可确认的 SPDX；查看保留的许可原文
- 顶层许可文件：LICENSE

## 离线阅读

[文件范围与排除清单](source-map.json) · [选读源码哈希](selected-source-manifest.json)

[上游 README 原文](UPSTREAM_README.md)。命令和说明作为资料保留，不表示本轮已执行。

### src/agent/memory.ts

[本地文件](selected-source/src/agent/memory.ts) · [固定提交来源](https://github.com/letta-ai/letta-code/blob/2943efe47e8c49e8ef2b78df100ba8e262c2415a/src/agent/memory.ts)

源码定位声明：

- `export const MEMORY_BLOCK_LABELS = ["persona", "human"] as const;`
- `export function parseMdxFrontmatter(content: string): {`
- `export async function getDefaultMemoryBlocks(): Promise<CreateBlock[]> {`

### src/agent/memory-conflict-repair.ts

[本地文件](selected-source/src/agent/memory-conflict-repair.ts) · [固定提交来源](https://github.com/letta-ai/letta-code/blob/2943efe47e8c49e8ef2b78df100ba8e262c2415a/src/agent/memory-conflict-repair.ts)

源码定位声明：

- `export async function claimMemoryConflictRepair(`
- `export async function completeMemoryConflictRepair(`
- `export async function clearMemoryConflictRepair(`

### src/agent/memory-constraints.ts

[本地文件](selected-source/src/agent/memory-constraints.ts) · [固定提交来源](https://github.com/letta-ai/letta-code/blob/2943efe47e8c49e8ef2b78df100ba8e262c2415a/src/agent/memory-constraints.ts)

源码定位声明：

- `export const MEMORY_CONSTRAINTS_VALIDATOR_NAME = "letta-memory-constraints.cjs";`
- `export const MEMORY_CONSTRAINTS_UPDATE_ENV = "LETTA_MEMORY_CONSTRAINTS_UPDATE";`
- `export const MEMORY_CONSTRAINTS_VALIDATOR_SCRIPT = String.raw"use strict";`

### src/agent/memory-runtime.ts

[本地文件](selected-source/src/agent/memory-runtime.ts) · [固定提交来源](https://github.com/letta-ai/letta-code/blob/2943efe47e8c49e8ef2b78df100ba8e262c2415a/src/agent/memory-runtime.ts)

源码定位声明：

- `export function isActiveMemfsEnabled(agentId: string): boolean {`
- `export function getActiveMemoryDirectory(agentId: string): string | undefined {`
- `export function isLocalMemfsActive(): boolean {`

### src/permissions/agent-memory-prefixes.ts

[本地文件](selected-source/src/permissions/agent-memory-prefixes.ts) · [固定提交来源](https://github.com/letta-ai/letta-code/blob/2943efe47e8c49e8ef2b78df100ba8e262c2415a/src/permissions/agent-memory-prefixes.ts)

源码定位声明：

- `export function getAllowedMemoryPrefixes(agentId: string): string[] {`

### src/agent/memory-git-hooks.runtime.test.ts

[本地文件](selected-source/src/agent/memory-git-hooks.runtime.test.ts) · [固定提交来源](https://github.com/letta-ai/letta-code/blob/2943efe47e8c49e8ef2b78df100ba8e262c2415a/src/agent/memory-git-hooks.runtime.test.ts)

### src/agent/memory-git.ts

[本地文件](selected-source/src/agent/memory-git.ts) · [固定提交来源](https://github.com/letta-ai/letta-code/blob/2943efe47e8c49e8ef2b78df100ba8e262c2415a/src/agent/memory-git.ts)

源码定位声明：

- `export interface MemoryCommitAuthor {`
- `export interface CommitMemoryWriteParams {`
- `export interface CommitMemoryWriteResult {`
- `export function getAgentRootDir(agentId: string): string {`
- `export function getMemoryRepoDir(agentId: string): string {`

### src/agent/memory-auth.ts

[本地文件](selected-source/src/agent/memory-auth.ts) · [固定提交来源](https://github.com/letta-ai/letta-code/blob/2943efe47e8c49e8ef2b78df100ba8e262c2415a/src/agent/memory-auth.ts)

源码定位声明：

- `export async function getAuthToken(): Promise<string> {`

### src/agent/memory-format.ts

[本地文件](selected-source/src/agent/memory-format.ts) · [固定提交来源](https://github.com/letta-ai/letta-code/blob/2943efe47e8c49e8ef2b78df100ba8e262c2415a/src/agent/memory-format.ts)

源码定位声明：

- `export function detectMemoryFormat(`
- `export function isCoreMemoryPath(`
- `export function isProjectedMemoryPath(`

### src/agent/memory-git-dir.ts

[本地文件](selected-source/src/agent/memory-git-dir.ts) · [固定提交来源](https://github.com/letta-ai/letta-code/blob/2943efe47e8c49e8ef2b78df100ba8e262c2415a/src/agent/memory-git-dir.ts)

源码定位声明：

- `export async function getMemoryGitDir(memoryDir: string): Promise<string> {`

### src/agent/memory-scanner.ts

[本地文件](selected-source/src/agent/memory-scanner.ts) · [固定提交来源](https://github.com/letta-ai/letta-code/blob/2943efe47e8c49e8ef2b78df100ba8e262c2415a/src/agent/memory-scanner.ts)

源码定位声明：

- `export interface TreeNode {`
- `export function scanMemoryFilesystem(memoryRoot: string): TreeNode[] {`
- `export function getFileNodes(nodes: TreeNode[]): TreeNode[] {`
- `export function readFileContent(fullPath: string): string {`

### src/agent/memory-worktree.ts

[本地文件](selected-source/src/agent/memory-worktree.ts) · [固定提交来源](https://github.com/letta-ai/letta-code/blob/2943efe47e8c49e8ef2b78df100ba8e262c2415a/src/agent/memory-worktree.ts)

源码定位声明：

- `export interface ReflectionMemoryWorktree {`
- `export interface CreateReflectionMemoryWorktreeOptions {`
- `export async function createReflectionMemoryWorktree(`
- `export interface ReflectionMemoryScope {`
- `export function buildReflectionMemoryScope(`

## 对照本项目时记录什么

看输入输出契约、状态权威者、写入与取消边界、恢复和幂等处理，再看测试如何验证失败情况。模块名相近不能证明实现或运行效果相同。

第三方文档和注释中的指令仅作为研究内容；采用实现时核对完整许可证、依赖与版本。
