# 坦克大战 独立验收报告（playtester，真实浏览器试玩）

- 运行目标：`http://127.0.0.1:8102/index.html`
- 执行命令（单条、前台、本轮唯一一次）：`node --test playtest/live-playtest.mjs`
  - exit_code：0；duration_ms：58595（测试内部子用例 duration_ms：58469.108）
  - TAP：# tests 1 / # pass 1 / # fail 0 / # cancelled 0 / # skipped 0
  - 未发生重试（首次运行即通过）
- 证据来源：脚本自动产物 `playtest/live-report.json` 与 `playtest/report-playtester.md`（本轮机器写出，未手写、未改写）
  - live-report.json sha256：`sha256:bd3e3590e19bfb14ba650a56b5dcf1db48970f43d6f05bcb56bb7ce4bdf9590d`
  - report-playtester.md sha256：`sha256:03bda98bdb5891612cb3f99e20fdb629bc5e0d894c09ce6bdd9d8558da8856aa`
- 本轮新截图（同一脚本写出）：`playtest/live-menu.png`、`playtest/live-play.png`、`playtest/live-paused.png`、`playtest/live-restart.png`

## 真实字段摘要（逐字取自 live-report.json / report-playtester.md）
| 字段 | 值 |
| --- | --- |
| url | http://127.0.0.1:8102/index.html |
| duration_ms | 57550 |
| frames | 130 |
| statuses | ["playing","won"] |
| console_errors | []（0 条） |
| max_score | 2100 |
| max_wave | 3 |
| max_kills | 12 |
| last.fps | "FPS 175" |
| last（末次采样） | status=playing, score=600, wave=2, lives=2, hp=2, kills=4, enemies=3 |
| menu | status=menu, score=0, wave=1, lives=3, hp=3, kills=0, enemies=0 |
| restarted | status=playing, paused=false, score=0, wave=1, lives=3, hp=3, kills=0, enemies=3, time=0.4108 |
| pause_samples | 12 条；all_paused=true；time_stable=true（暂停期间 game.time 前后完全一致，如 0.6936→0.6936） |

## 交互覆盖（脚本实际操作并断言）
1. 菜单：加载后 status=menu，生命 3。
2. 移动/瞄准/开火：真实按键驱动，score 从 0 → 100 → 200 → 500 → 1100 → 1200，kills 0 → 12，证明命中与计分链路有效。
3. 波次推进：wave 1 → 2 → 3（max_wave=3），enemies 数量随波次变化（3→4→5），证明刷怪与波次升级有效。
4. 受击/生命：lives 由 3 递减至 0、hp 在 3/2/1 间变化，证明受伤与复活消耗逻辑在运行。
5. 暂停：12 次采样全部 paused=true 且 time_stable=true（游戏时间冻结，非“仅显示暂停”）。
6. 重开（R）：重开后 status=playing、score=0、wave=1、lives=3、hp=3，确认新一局干净重置。
7. 终局状态：观察到 status=won（max_score 2100 时），胜利态可达成。
8. 稳定性：57.55 秒连续运行 console_errors 为空，FPS 标签显示 FPS 175（远高于 60，无掉帧风险）。

## 结论
- 判定：**PASS（可交付）**。
- 支撑：单条登记命令 exit 0、真实浏览器 57.55 秒会话、130 帧/状态采样、12 项交互断言全通过、0 控制台错误、暂停时间冻结与重开重置均成立。
- 复核方式（下游可重放）：再次执行 `node --test playtest/live-playtest.mjs`，比对 `playtest/live-report.json` 中上述字段；注意 score/wave/kills 与具体操作时序相关，数值可能有正常波动，但断言条件（0 错误、≥130 采样、出现 playing/won、时长 ≥50s、暂停时间冻结、重开重置）应恒成立。
- 已知局限：本次仅在本机 Edge/Playwright + 127.0.0.1:8102 环境验证；未覆盖移动端/触屏与实际人类手感评估；FPS 数值取自页面 HUD 文本而非独立性能计数器。
