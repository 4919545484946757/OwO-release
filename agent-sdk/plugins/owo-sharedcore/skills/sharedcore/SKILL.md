---
name: sharedcore
description: 多 Agent 共享核心上下文的并行开发协调：初始化项目上下文、拆任务 DAG、计算并行批次、生成子 Agent 简报、回收结果并推进。用户说"用 SharedCore 并行开发/多 agent 一起做"时使用。
---

# SharedCore

你正在使用 **SharedCore** 插件：把项目目标、规则、契约和任务进度存进工作区的
`.sharedcore/`，让多个子 Agent 在**同一份结构化上下文**下并行开发，而不是共享聊天记录。

## 何时使用

- 用户要求"多 Agent / 并行 / 一起开发 / 分头做"，或任务可以拆成互不依赖的模块；
- 需要跨多个子任务保持一致的规则、接口契约、写范围；
- 任务较大、需要先规划再并行执行。

单个小改动**不要**用 SharedCore，直接做即可。

## 标准流程（一次对话内）

1. **初始化**（每个项目一次）
   `sharedcore_init`：workspace、name、goal、baseline_commit、architecture、rules。
   规则写清楚：接口先定契约、禁止越出写范围、测试必须过。
2. **拆任务**
   为每个可独立交付的模块调用 `sharedcore_task_add`：title、depends_on、
   write_scope（**必须明确到目录/文件前缀**）、read_scope、acceptance。
   跨模块的共享契约（API/数据结构）单独建一个前置任务。
3. **算批次**
   `sharedcore_plan`：返回 `batches`（可并行的任务组）、`blocked`、`conflicts`。
   同一批次内任务的写范围不应重叠；有冲突就拆范围或加依赖。
4. **并行执行**
   对每个可运行任务：
   a. `sharedcore_claim`（owner=子 Agent 名，branch/worktree 可空）；
   b. `sharedcore_brief` 生成该任务的简报；
   c. 用宿主的 **`fanout_subagents`**（一批）或 `subagent`（单个）启动子 Agent，
      提示词 = brief 原文 + "在独立 worktree 中完成，只改 write_scope 内文件"；
   d. 子 Agent 返回后，`sharedcore_update` 记 done/failed，附证据与原因。
5. **下一批**
   重新 `sharedcore_plan`，直到没有可运行任务。
6. **收尾**
   `sharedcore_status` 汇总：完成数、失败/阻塞、契约版本、最近事件；向用户报告。

## 工具边界

| 工具 | 作用 |
|---|---|
| `sharedcore_init` | 建项目上下文（重复调用需 reset） |
| `sharedcore_status` | 摘要 + 批次 + 阻塞 + 最近事件 |
| `sharedcore_context` | 规则/架构/契约切片（构造提示用） |
| `sharedcore_task_add` | 加任务（T-001 起） |
| `sharedcore_plan` | 并行批次与冲突检测 |
| `sharedcore_claim` | 领取任务（检查依赖） |
| `sharedcore_update` | 状态迁移 + 自动事件 |
| `sharedcore_contract` | 契约提案/接受（expected_version CAS） |
| `sharedcore_event` | 追加共享事件 |
| `sharedcore_brief` | 生成子 Agent 简报 |

## 约束

- **本插件不执行命令、不联网**：只读写 `<workspace>/.sharedcore/`。git、测试、构建由宿主
  工具执行（照常走权限/审批）。
- **写范围是硬约束**：子 Agent 只改 `write_scope` 内文件；需要跨范围时先报告，
  由主 Agent 新增任务或扩展范围。
- **契约变更走提案**：接口/数据结构变化先 `sharedcore_contract` propose，受影响任务
  确认后 accept；不要直接改接口。
- **不共享对话**：给子 Agent 的上下文 = brief（规则 + 相关契约 + 任务契约），不是整段聊天。
- 子 Agent 的并发数受宿主 `fanout_subagents` 限制；不要超过 `plan` 给出的批次大小。
