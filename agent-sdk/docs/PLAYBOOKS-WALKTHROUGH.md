# 任务剧本 Walkthrough（从零到自主推进）

> 目标：10 分钟走完一次完整闭环。所有命令与输出均为本机 2026-11-18 实测。
> 前提：`owo-agent` 已构建；在 `agent-sdk/` 目录下执行。

## 0. 准备（一次性）

```powershell
# 打开剧本工具组（可选功能，默认关闭）
$settings = "$env:USERPROFILE\.owo\settings.json"   # 或你的 <workspace>/settings.json
# 在 settings.json 里加：
# { "tool_capabilities": { "playbooks": true } }
```

## 1. 起一个剧本

```powershell
owo-agent playbook init demo
# 已生成剧本：playbooks\demo.json（3 步）
```

骨架是 `plan(model) → check(check，命令留空=人工判定) → deliver(human)`。
或者从模板派生：

```powershell
owo-agent playbook init review2 --from playbooks/code-review.json
```

## 2. 校验与看计划

```powershell
owo-agent playbook validate playbooks/demo.json
# 剧本通过：demo（demo）· 3 步
#   - plan    model  确认任务范围
#   ...

owo-agent playbook plan playbooks/demo.json
# 执行计划：… · 进度 0/3（0%）
#   1. plan  model  确认任务范围（无前置）
#      判据：列出输入、产出与不做的事
```

如果剧本里有 `rm -rf` 这类命令，`validate` 会给出风险提示（`--strict` 视为失败）。

## 3. 让 Agent 执行（工具侧）

模型侧工具（需要第 0 步开启）：

| 工具 | 作用 |
|---|---|
| `playbook_next {file}` | 拿下一步（含判据/命令/风险提示） |
| `playbook_record {file, step, outcome, note}` | 记录人工/模型结论 |
| `playbook_check {file, step}` | 执行该步命令（exit 0=通过） |
| `playbook_run {file}` | 一次跑完所有连续 check 步骤 |

典型循环：`next` →（check 用 `playbook_run`/`playbook_check`，model 用普通工具做）→ `record` → `next`。

## 4. 自主推进（宿主按剧本判定完成）

```text
/advance --playbook playbooks/demo.json
```

- 每轮宿主读取剧本状态，把当前步骤拼进提示；
- 模型只推进这一步并记录；
- **全部步骤了结才算完成**（模型自报 `GOAL_DONE` 不作数）；
- 无可用步骤（前置失败/跳过）时停下并说明；
- 失败/停滞走 `/advance` 的有界重试与停滞检测。

随时看进度：

```text
/goal status
# 目标：按剧本推进：… · 1/3（33%）· 下一步 check
#   剧本：demo（demo）· 1/3（33%）· 下一步 check
```

## 5. 留痕与对比

```powershell
# 中途快照
owo-agent playbook archive playbooks/demo.json --marker before-refactor

# 完成时会自动留档 complete-<时间戳>.json
# 看历史
owo-agent playbook history playbooks/demo.json
# 对比两次运行（谁退化了）
owo-agent playbook diff playbooks/demo.json --from before-refactor --to complete-1763xxxx
```

## 6. 桌面看板

打开桌面端 → 工作台「自动化 → 任务剧本」：

- 卡片：进度条、下一步、失败/阻塞计数、高风险命令数；
- 展开：逐步状态 + 命令复制 + 风险提示 + 通过/失败/跳过记录；
- 历史运行：归档与「退化」标记。

## 7. 收尾检查

```powershell
owo-agent playbook validate playbooks       # 目录批量校验（CI 友好）
owo-agent playbook list                     # 一览 + 风险计数
```

## 常见问题

| 现象 | 原因与处理 |
|---|---|
| `剧本工具未启用` | 第 0 步没开；改 settings 后重启核心 |
| `步骤 w 没有声明 command` | 该 check 是人工判定，用 `playbook_record` 记录 |
| `/advance --playbook` 立即停止 | 没有可执行步骤（前置失败/跳过导致阻塞），先看 `playbook status` |
| 面板记录后进度没动 | 点「刷新」，或确认状态文件在工作区 `playbooks/` 下 |
