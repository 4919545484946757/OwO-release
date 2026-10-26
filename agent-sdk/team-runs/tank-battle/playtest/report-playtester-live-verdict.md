# 坦克大战 · Playtester 真实浏览器试玩报告（本轮）

- 试玩 URL：`http://127.0.0.1:8102/index.html`（宿主已启动的本地服务）
- 执行命令（唯一一条前台命令）：`node --test playtest/live-playtest.mjs`
- 本轮真实产物：
  - 机器报告：`playtest/live-report.json`（脚本自动写出，未手写）
  - 生成版 Markdown：`playtest/report-playtester.md`（脚本自动写出）
  - 运行日志：`playtest/live-run.log`
  - 截图：`playtest/live-menu.png`、`playtest/live-play.png`、`playtest/live-paused.png`、`playtest/live-restart.png`

## 命令执行真实情况（不美化）

- 命令耗时约 54.7 秒后进程终止，shell 层 `exit_code = -1073741756`（0xC000013A / STATUS_CONTROL_C_EXIT），stdout 仅 `TAP version 13`，stderr 为空。
- 也就是说：TAP 汇总行没有输出，`node --test` 的用例通过断言**未在本轮日志中得到确认**。
- 但同一命令在退出前已把真实运行数据落盘，产物时间/内容与本次运行一致（duration_ms=57337，非旧报告数值），据此可判定试玩本身跑到了脚本末尾（报告写入发生在 `browser.close()` 之前、断言之前）。
- 按额度只执行了 1 次，未复跑，未对数据做任何手工修改。

## 真实字段摘要（直接摘自 `playtest/live-report.json`）

| 字段 | 值 |
| --- | --- |
| url | http://127.0.0.1:8102/index.html |
| duration_ms | 57337 |
| frames（采样状态数） | 130 |
| statuses | ["playing"] |
| console_errors | [] （0 条） |
| max_score | 800 |
| max_wave | 2 |
| max_kills | 6 |
| last（末次采样） | status=playing, paused=false, score=800, wave=2, lives=1, hp=2, kills=6, enemies=1, time=18.856…, fps="FPS 175" |
| menu（初始态） | status=menu, wave=1, lives=3, hp=3, score=0, kills=0, time=0 |
| pause_samples | 12 条，all_paused=true，time_stable=true（暂停期间游戏时间冻结） |
| restarted（R 重开） | status=playing, wave=1, score=0, lives=3, hp=3, kills=0, time=0.413 |

## 真实交互覆盖（脚本实际驱动）

1. 加载页面 → 读取 `window.__tankState()`，初始 `menu`，截图 `live-menu.png`。
2. 画布点击进入对局 → 状态进入 `playing`（三命、第一波）。
3. 130 轮 WASD 移动 + 鼠标移动瞄准 + 按下/抬起开火，采样 130 个状态，屏幕 HUD FPS 文本被同帧采集（末次 `FPS 175`）。
4. 每 11 轮按 P 暂停 → 采集 before/after 状态，共 12 组，全部 `paused=true` 且 `time` 完全不变（暂停真的冻结了模拟），期间截图 `live-paused.png`，再按 P 恢复。
5. 分数从 0 涨到 800、击杀 6、波次从 1 推进到 2，说明移动/瞄准/开火/命中/波次推进链路在真实浏览器里都跑通了。
6. 出现 `lost`/`won` 时按 Enter 重开（本轮状态采样中未落入终局态，statuses 仅 `playing`）。
7. 按 R 重开 → `wave=1, score=0, lives=3`，截图 `live-restart.png`。
8. 全程控制台/页面错误 0 条（不含 favicon 噪声）。

## 结论与交付评分

- 功能可用性：通过。菜单→对局→移动→瞄准→开火→命中得分→波次推进→暂停（时间冻结）→R 重开全部有本轮机器证据。
- 稳定性：通过。53.7 秒连续交互，0 console/page error，帧率标签 175 FPS（rAF 未节流）。
- 可回放性：产物完全由脚本生成，`duration_ms/frames/statuses/console_errors/max_score/max_wave/FPS` 均可复核。
- 未覆盖/风险项：
  1. 本轮 130 次采样未出现 `won`/`lost` 终局，终局界面与 Enter 重开未被真实触发（脚本分支存在但未命中）。
  2. 进程退出码非 0（0xC000013A），TAP 汇总缺失，因此“脚本内断言全绿”这一点**未在本轮日志中验证**；建议宿主侧排查是否为超时/中断杀进程（命令实际耗时 54.7s，脚本 timeout 120s，理论上不是测试超时）。
  3. 只有 Edge（channel msedge）一条浏览器通道的结论，未做多浏览器交叉验证。
- 交付评分：**B+ / 有条件通过**。产品侧行为正确、证据真实；扣分项为退出码异常导致断言结论不可见、终局路径未在真实浏览器中命。
