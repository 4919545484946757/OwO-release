# 真实功能测试记录（2026-11-18 18:20 · UI 可视化轮）

> 环境：演示 daemon（`http://127.0.0.1:49030`，`%TEMP%\owo-demo-agent` 工作区，
> 模型提供商未配置），真实 HTTP + 真实浏览器（Playwright/Edge headless）。
> 每条结论都有当轮实测输出；空命中/空列表是"没有数据"而非"功能坏了"。

## 1. 剧本面板 UI 全生命周期点击穿透（此前只测过 CLI 与 API 层）

| 步骤 | 操作（真实点击） | 结果 |
|---|---|---|
| 1 | 展开 code-review 卡 | 服务端初始态 `scope=passed build=failed tests=…pending` |
| 2 | 对 pending 的 check 步骤 `tests` 填备注并点「通过」 | — |
| 3 | 读 `/playbooks/status` | **`tests=passed`**（点击真实落盘） |
| 4 | 检查步骤条 | 绿点从 1 → **2**（UI 与服务端同步） |
| 5 | 点「保存快照」 | 归档数 3，最新 `ui-1794996613518` |
| 6 | 下拉选 `run-2 → ui-*` 点「对比两次」 | 显示 **`tests pending → passed`** |
| 7 | 点「重置」两次（二次确认） | 全部回到 pending；自动快照 `reset-*` 恰 1 份 |
| — | console 错误 | **0** |

证据：`ui-e2e-playbook.png`（本目录）。

## 2. 未测 API 的真实 HTTP 测试

| 端点 | 结果 |
|---|---|
| `GET /memory/observations` | 200，100 条观察 |
| `GET /memory/recall?q=playbook&top_k=3` | 200，0 命中（演示区无语义记忆，预期） |
| `GET /memory/recall`（缺 q） | **400 拒绝**（参数校验有效） |
| `GET /usage` | 200（模型未配置，字段为空） |
| `GET /workflows/user` | 200 |
| `GET /health` | 200 |

## 3. 可视化盘点（inventory.js，25 个面板）

用"图形元素计数 vs 文本块计数"实测：**24 个面板 0 图形**（唯一例外 home 2 个、
playbook 4 个进度条）。最重灾区：capabilities 184 文本/0 图、sharedcore 104/0、
memory 100/0、project-launcher 67/0、about 60/0。

> 注意：observability 的 41 文本/0 图是**空数据**导致（面板本有 SVG 折线图，
> 演示区无回合数据时不画）——"没数据"与"没图"要分开处理。

## 4. 本轮新增可视化（详见 commit `0a3ecd5b`）

- SharedCore 任务 DAG（SVG 分层 + 依赖连线 + 状态色 + 图例）：真机 5 节点/3 连线，
  分层坐标正确、无重叠、0 错误。
- 剧本步骤条（状态圆点 + 轨道）：真机 5 点/4 线，连线与圆点中心对齐 ±1px，0 错误。

## 5. 结论与遗留

- 剧本面板的 UI 交互链路（记录/归档/对比/重置）**首次完成真实点击级验证**；
  memory/usage/workflows/health API 首次完成真实 HTTP 验证。
- 仍待做：observability 需要有数据后的折线图验证（需真实模型回合）；
  capabilities/memory/project-launcher 的可视化重构；深色主题。
