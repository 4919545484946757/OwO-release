# 桌面端 UI：能力必须可见（surfacing 清单）

> 原则：**后端存在但界面上找不到的能力，等于没有**。新增 API 字段/能力时，
> 要么在界面露出入口或状态，要么在能力登记表里明确标 Experimental 并说明看不到。

## 1. 现状对照（能力 → 界面入口）

| 能力 | 界面位置 | 状态 |
|---|---|---|
| 插件列表 / 启停 | 工具页 →「插件与技能」→ 插件 | ✅ |
| 插件包格式（Agent Plugins 1.0.0 / OwO manifest） | 插件卡芯片 | ✅ 2026-11-18 补 |
| 插件 MCP 服务器（含多服务器） | 插件卡底部（`plugin/demo（stdio）`） | ✅ 2026-11-18 补 |
| 插件 hooks 注入（事件/匹配器/fail_mode） | 插件卡「hooks 注入 N 条」可展开 | ✅ 2026-11-18 补 |
| 插件工具被隐藏状态 | 插件卡「工具已隐藏」芯片 | ✅ 2026-11-18 补 |
| 插件市场（远端目录刷新/安装/更新） | 插件页「热门推荐」+「插件市场」面板 | ✅ |
| 已安装插件与签名状态 | 「插件与技能」列表（`/plugins` 的 `signed`） | ✅ 截图 `docs/qa/evidence/ui-plugin-signed-20261118.png` |
| 技能列表与健康度 | 「插件与技能」→ 技能 | ✅ |
| 自定义工作流（模板/草稿/评审/健康/attention） | 工作台 →「自定义工作流」面板 | ✅ |
| 工作流运行（run/rerun/diff/验证） | 「工作流」面板 | ✅ |
| 目标与计划（waves/步骤/尝试/输出） | 工作台 →「目标与计划」 | ✅ |
| 任务剧本 | 工作台 →「自动化 → 任务剧本」面板 | ✅ 2026-11-18 |
| ├ 新建 / 列表 / 进度 / 风险与失败类型 | 同上（`POST /playbooks/init`、`GET /playbooks`） | ✅ 截图 `docs/qa/evidence/ui-playbook-create-20261118.png` |
| ├ 展开：命令/重试预算/逐步记录 | 同上（`GET /playbooks/status`、`POST /playbooks/record`） | ✅ 截图 `docs/qa/evidence/ui-playbook-panel-20261118.png` |
| ├ 保存快照 / 历史与退化对比 | 同上（`POST /playbooks/archive`、`GET /playbooks/history`） | ✅ 截图 `ui-playbook-archive-20261118.png`、`ui-playbook-compare-20261118.png` |
| └ 风险提示（高风险命令） | 列表计数 + 步骤行提示 | ✅ 截图 `ui-playbook-risk-20261118.png` |
| 团队协作（任务图/产物/干预） | 「团队协作」页 + 面板 | ✅ |
| 权限与审批中心 | 「权限中心」面板 | ✅ |
| 运行指标 / 诊断 / 评测 | 对应面板 | ✅ |

## 2. 检查清单（新增能力时）

1. 后端新增字段 → 同步 `desktop/web/app.js` 或对应 panel 的渲染；
2. 新增能力 → 面板 `nav()` 有入口、空态有说明、错误态可读；
3. 敏感字段（env/headers/绝对路径/凭据）**不下发到前端**；需要展示时只给名称/数量；
4. 改完用真实页面验证（下节），不要只靠读代码。

## 3. 验证方法（真实页面 + 真实渲染函数）

工作台是磁盘加载的静态站（ADR-003），改 `desktop/web/*.js` 刷新即生效；浏览器模式
（非 Electron 壳）不需要 pairing。

```powershell
# 1) 起一个隔离的 daemon（临时 workspace/data，复制示例插件）
$root = Join-Path $env:TEMP ("owo-ui-" + [guid]::NewGuid().ToString('N').Substring(0,8))
New-Item -ItemType Directory -Path (Join-Path $root 'ws\plugins') -Force | Out-Null
Copy-Item agent-sdk\plugins\owo-demo-kit (Join-Path $root 'ws\plugins\owo-demo-kit') -Recurse
$env:OWO_AGENT_DATA = Join-Path $root 'data'
agent-sdk\target\debug\owo-agent.exe serve --port 47813 --workspace (Join-Path $root 'ws')

# 2) 用 Playwright（playwright-core + 系统 Edge）在真实页面上下文里调用真实渲染函数：
#    await page.evaluate(async () => { const data = await api('/plugins');
#      data.plugins.forEach(p => box.appendChild(pluginCard(p))); })
#    再对 #probeBox 截图（见本轮验证记录）。
```

为什么用这种方式而不是直接截图整页：工作台是长页面 + 多级导航，`#pluginGrid` 在未打开
工具页时不可见；在页面上下文里调用**同一份** `pluginCard` + 真实 `/plugins` 数据，
验证的是真实渲染路径与真实 CSS，而不是另写一份预览。

## 4. 已知未 surfaced（下一批）

- 技能来源（插件携带 vs 宿主安装）：`/skills` 目前不下发来源，界面无法区分；
- 插件安装/回滚的历史与审计：`plugin_api` 只给当前状态，界面上没有"最近一次操作"；
- 目标推进的停滞/重试账本：目前只在 CLI `/goal status`，桌面端没有对应展示。
