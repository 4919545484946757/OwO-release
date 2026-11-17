# 工作流优先：自定义流程 + 按进度自主推进

> 状态：策略层已落地（2026-11-18）；模板层与确定性步骤执行是下一步。
> 事实优先级：源码与测试 > 本文。

## 1. 核心不是插件

插件解决的是"**能扩展**"，但用户真正要的是"**把某类任务稳定做完**"。这两件事的差别：

| | 插件（扩展点） | 工作流（核心） |
|---|---|---|
| 单位 | 一个能力（工具/技能/hook） | 一类任务（从输入到可验收产出） |
| 成功标准 | 能力被正确调用 | **任务按进度推进到终态** |
| 失败表现 | 工具报错 | 卡住、原地打转、半成品、预算烧光 |
| 谁驱动 | 模型临时决定 | 流程 + 进度账本 + 推进策略 |

所以我们把工作流当核心：**用户定义"这类任务怎么做"，agent 按进度自己往下推**，
插件只是工作流在需要时调用的能力。

## 2. 三层模型

```text
① 模板层（按域）    workflow template：步骤 / 验收标准 / 允许能力 / 人工节点
        ↓ 实例化
② 运行层（一次跑）  run + progress ledger：第几轮、当前步骤、连续失败、连续同状态
        ↓ 每轮结束
③ 推进层（自主）    advance policy：Continue / Retry / Stall / Failed / Done / Aborted
```

三层各自可独立测试，出问题时能定位到"模板定义错了 / 进度记错了 / 策略判错了"。

## 3. 现有资产映射（不重复造）

| 层 | 已有实现 | 说明 |
|---|---|---|
| 模板 | `owo_agent_protocol::TeamTemplate` + `core::user_workflow` | 角色 DAG、写范围、验收断言、评审/健康检查；CLI `owo-agent workflow` 全链路 |
| 运行 | `core::goal`（GoalRunner/GoalRunState/StepRecord） | 计划→步骤→尝试→验收收据、critic、黑板、持久化 |
| 推进 | **本轮新增** `owo_agent_contracts::goal_progress` | 纯策略：按"进度有没有动"决定下一步 |
| 入口 | CLI `/goal`（daemon + local） | 每轮一回合，宿主状态作为进度信号 |

## 4. 本轮落地：按进度推进（不是按轮数）

`goal_progress` 是纯逻辑模块（无 IO），规则：

| 本轮结果 | 决策 |
|---|---|
| 宿主接受（Accepted/ResponseComplete + 完成标记） | `Done` |
| 回合中止 | `Aborted` |
| 回合失败 | 连续失败 ≤ `max_retries` → `Retry`；超过 → `Failed`（交给人） |
| 宿主状态与上轮相同 | 连续同状态 < `stall_window` → `Continue`；达到 → `Stall`（停滞升级） |
| 宿主状态变化 | `Continue`（进度在动） |

**停滞检测**是这一层的核心价值：目标没失败、但连续 N 轮宿主状态一模一样，说明模型
在原地打转；继续自动推进只会烧预算。`Stall` 会带着状态与轮数停下，并提示"拆分目标 /
补完成标准 / 换更具体的下一步"。

配置（env，可按工作流覆盖）：

| 变量 | 默认 | 含义 |
|---|---|---|
| `OWO_GOAL_MAX_RETRIES` | 2 | 连续失败允许的自动重试次数（0 = 首次失败即停） |
| `OWO_GOAL_STALL_WINDOW` | 3 | 连续多少轮同状态判停滞（0 = 关闭检测） |
| `OWO_GOAL_MAX_ITERATIONS` | 不限 | 总轮数上限（既有） |

可见性：`/goal status` 现在会打印进度行（连续失败次数 / 连续同状态轮数）；停滞与
失败停止时终端给出可操作建议，而不是静默退出。

## 5. 下一步（按价值排序）

1. **域模板**：把"这类任务怎么做"变成可复用模板（步骤 + 每步验收 + 允许工具），
   一个命令从模板起一个 run；与 `TeamTemplate` 的角色 DAG 打通。
2. **确定性步骤不走模型**：模板里能由命令/校验器判定的步骤直接执行并记账，
   模型只处理开放式步骤——这是"高效"的主要来源。
3. **运行记录持久化**：进度账本随会话落库（当前 daemon 已持久化目标与轮次，
   账本重启后从 `Default` 重建）；跨进程 resume 后停滞计数不丢。
4. **同一策略接桌面/API**：把 `goal_progress` 应用到 daemon 的目标推进接口，
   让桌面端与 CLI 的"会不会停"完全一致。
5. **按域的验收标准**：工作流模板声明 `done criteria`，由宿主校验器判定，
   模型只能"申请完成"，不能自己宣布完成（现有 `goal_claim_is_accepted` 已是这个方向）。

## 6. 验收

```powershell
cd agent-sdk
cargo test -p owo-agent-contracts --lib -- goal_progress   # 策略规则（7 条）
cargo test -p owo-agent-cli --bin owo-agent -- goal        # 接线与提示（9 条）
```

代码位置：

- 策略：`crates/owo-agent-contracts/src/goal_progress.rs`
- 接线：`crates/owo-agent-cli/src/support/goal.rs`（`goal_advance` / `goal_progress_line`）
- 循环：`crates/owo-agent-cli/src/commands/repl_daemon.rs`（`run_goal_loop`）、
  `crates/owo-agent-cli/src/commands/repl.rs`（本地路径）
