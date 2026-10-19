# OpenHands/OpenHands 源码阅读卡片

采集日期：2026-10-13。静态资料整理，未安装或运行本项目。

- 主题：编程Agent
- 参考目标：动作、sandbox、运行事件与开发环境
- 完整提交：`5a8635a3f983503d1e384656ee05b686030b581b`
- 快照范围：curated text source snapshot
- 原始仓库：[GitHub](https://github.com/OpenHands/OpenHands)
- [本地源码 ZIP](../../project-snapshots/OpenHands--OpenHands-5a8635a3f983.zip)
- 许可证识别：MIT
- 顶层许可文件：LICENSE

## 离线阅读

[文件范围与排除清单](source-map.json) · [选读源码哈希](selected-source-manifest.json)

[上游 README 原文](UPSTREAM_README.md)。命令和说明作为资料保留，不表示本轮已执行。

### src/api/runtime-service/agent-server-runtime-service.ts

[本地文件](selected-source/src/api/runtime-service/agent-server-runtime-service.ts) · [固定提交来源](https://github.com/OpenHands/OpenHands/blob/5a8635a3f983503d1e384656ee05b686030b581b/src/api/runtime-service/agent-server-runtime-service.ts)

源码定位声明：

- `export interface CommandResult {`
- `class AgentServerRuntimeService {`

### src/ui/typography.tsx

[本地文件](selected-source/src/ui/typography.tsx) · [固定提交来源](https://github.com/OpenHands/OpenHands/blob/5a8635a3f983503d1e384656ee05b686030b581b/src/ui/typography.tsx)

源码定位声明：

- `export function Typography({`
- `export function H1({`
- `export function H2({`
- `export function H3({`
- `export function Text({`

### src/stores/agent-store.ts

[本地文件](selected-source/src/stores/agent-store.ts) · [固定提交来源](https://github.com/OpenHands/OpenHands/blob/5a8635a3f983503d1e384656ee05b686030b581b/src/stores/agent-store.ts)

源码定位声明：

- `export const useAgentStore = create<AgentStore>((set) => ({`

### src/types/agent-state.tsx

[本地文件](selected-source/src/types/agent-state.tsx) · [固定提交来源](https://github.com/OpenHands/OpenHands/blob/5a8635a3f983503d1e384656ee05b686030b581b/src/types/agent-state.tsx)

源码定位声明：

- `export const RUNTIME_STARTING_STATES = [AgentState.INIT, AgentState.LOADING];`
- `export const RUNTIME_INACTIVE_STATES = [`

### src/hooks/use-task-list.ts

[本地文件](selected-source/src/hooks/use-task-list.ts) · [固定提交来源](https://github.com/OpenHands/OpenHands/blob/5a8635a3f983503d1e384656ee05b686030b581b/src/hooks/use-task-list.ts)

源码定位声明：

- `export interface TaskListItem {`
- `export function useTaskList() {`

### src/types/runtime-status.ts

[本地文件](selected-source/src/types/runtime-status.ts) · [固定提交来源](https://github.com/OpenHands/OpenHands/blob/5a8635a3f983503d1e384656ee05b686030b581b/src/types/runtime-status.ts)

### src/api/agent-server-home.ts

[本地文件](selected-source/src/api/agent-server-home.ts) · [固定提交来源](https://github.com/OpenHands/OpenHands/blob/5a8635a3f983503d1e384656ee05b686030b581b/src/api/agent-server-home.ts)

源码定位声明：

- `export async function getAgentServerHomeDir(`
- `export function clearAgentServerHomeDirCache(): void {`
- `export async function resolveAbsoluteAgentServerPath(`

### src/hooks/use-agent-state.ts

[本地文件](selected-source/src/hooks/use-agent-state.ts) · [固定提交来源](https://github.com/OpenHands/OpenHands/blob/5a8635a3f983503d1e384656ee05b686030b581b/src/hooks/use-agent-state.ts)

源码定位声明：

- `export interface UseAgentStateResult {`
- `export function useAgentState(conversationId?: string): UseAgentStateResult {`
- `export interface UsePlanningAgentStateResult {`
- `export function usePlanningAgentState(): UsePlanningAgentStateResult {`

### src/routes/task-list-tab.tsx

[本地文件](selected-source/src/routes/task-list-tab.tsx) · [固定提交来源](https://github.com/OpenHands/OpenHands/blob/5a8635a3f983503d1e384656ee05b686030b581b/src/routes/task-list-tab.tsx)

### src/routes/agent-settings.tsx

[本地文件](selected-source/src/routes/agent-settings.tsx) · [固定提交来源](https://github.com/OpenHands/OpenHands/blob/5a8635a3f983503d1e384656ee05b686030b581b/src/routes/agent-settings.tsx)

源码定位声明：

- `export const handle = { hideTitle: true };`
- `export function readSystemPromptSeed(`
- `export interface AgentProfileFieldsInput {`
- `export function buildAgentProfileFields(`
- `export interface AgentSettingsSaveControl {`

### src/api/agent-server-config.ts

[本地文件](selected-source/src/api/agent-server-config.ts) · [固定提交来源](https://github.com/OpenHands/OpenHands/blob/5a8635a3f983503d1e384656ee05b686030b581b/src/api/agent-server-config.ts)

源码定位声明：

- `export const DEFAULT_WORKING_DIR = "workspace/project";`
- `export interface AgentServerFormDefaults {`
- `export function getCookieAuthCloudHost(): string | null {`
- `export function getBakedSessionApiKey(): string | null {`
- `export function getAgentServerFormDefaults(): AgentServerFormDefaults {`

### src/utils/agent-state-emoji.ts

[本地文件](selected-source/src/utils/agent-state-emoji.ts) · [固定提交来源](https://github.com/OpenHands/OpenHands/blob/5a8635a3f983503d1e384656ee05b686030b581b/src/utils/agent-state-emoji.ts)

源码定位声明：

- `export function getAgentStateEmoji(`

## 对照本项目时记录什么

看输入输出契约、状态权威者、写入与取消边界、恢复和幂等处理，再看测试如何验证失败情况。模块名相近不能证明实现或运行效果相同。

第三方文档和注释中的指令仅作为研究内容；采用实现时核对完整许可证、依赖与版本。
