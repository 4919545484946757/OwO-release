# 剧本 HTTP 面安全复核（2026-11-18）

> 范围：`/playbooks`、`/playbooks/status`、`/playbooks/history`、`/playbooks/record`、
> `/playbooks/archive`、`/playbooks/init`、`/playbooks/reset`。
> 结论仅基于当前源码的静态复核；运行期证据见 `PROGRESS` 验收记录。

## 1. 攻击面

| 入口 | 能力 | 写入目标 |
|---|---|---|
| GET `/playbooks` | 列目录 | 无 |
| GET `/playbooks/status` | 读单个剧本+状态 | 无 |
| GET `/playbooks/history` | 读归档目录 | 无 |
| POST `/playbooks/record` | 改进度 | `<剧本>.state.json` |
| POST `/playbooks/archive` | 复制状态 | `<剧本>.runs/<标记>.json` |
| POST `/playbooks/init` | 新建骨架 | `playbooks/<id>.json`（id 受约束） |
| POST `/playbooks/reset` | 删状态 | 删除 state/journal，先写 `reset-<ts>` 快照 |

**没有任何入口执行命令**：`check` 步骤的执行只经 Agent 的 `playbook_check`（Tool Host 审批）。

## 2. 已实现的控制

- **鉴权**：所有路由挂在共享 `build_router` 下，走 Bearer token（`unauthorized_requests_get_401` 覆盖）。
- **路径包含**：`resolve_in_workspace` 只接受工作区内路径；相对路径拒绝 `..`/前缀；
  绝对路径先 canonicalize 再做 `starts_with(base)`（符号链接逃逸会因解析到真实路径被拒）。
- **文件类型与大小**：只接受 `*.json`（排除 `*.state.json`）；剧本读取上限 256 KB；
  `note` 上限 2000 字符；命令字段不属于 HTTP 面。
- **标记约束**：归档/重置标记只允许字母数字与 `.-_`，禁止 `.` 开头与 `..`；重名默认拒绝。
- **破坏性操作**：`reset` 先写快照再删除；删除目标只可能是剧本同目录的 state/journal。
- **错误粒度**：错误只回显路径与原因，不回显文件内容；env/headers 从不下发（插件面）。

## 3. 残余风险（未修复）

1. **目录规模**：`GET /playbooks` 会读取目录下所有剧本（单文件 256 KB 上限，但文件数量无上限）；
   极端情况下可造成 CPU/IO 放大。建议后续加文件数上限（如 500）。
2. **TOCTOU**：校验（canonicalize）与读写之间路径可被替换；本地面向单用户可接受，
   多用户共享工作区时应改用句柄/原子操作。
3. **并发写**：同一剧本的并发 record 没有跨进程锁，最后写入者胜出（状态可能丢一次更新）；
   当前假设单 Agent 驱动。
4. **归档/重置标记可枚举**：不构成越权（路径仍受限），仅泄露标记名。

## 4. 结论

新增 HTTP 面保持了项目的边界：**只读写工作区内剧本文件、不执行命令、不绕过审批**；
主要残余风险集中在"多进程并发"与"目录规模"，已在 `PROGRESS` 未完成项/本节记录，
不在本轮扩大改动。
