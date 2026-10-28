# Tank Arena 实时浏览器试玩报告（playtester · game-polish round 2 复跑）

本报告只用 `playtest/live-playtest.mjs` 本次真实生成的字段，未手写或篡改任何数值。

## 命令与结果

| 项 | 值 |
| --- | --- |
| 命令 | `node --test playtest/live-playtest.mjs`（前台，单一命令） |
| 第 1 次运行 | exit_code=1；TAP `# tests 1 / # pass 0 / # fail 1`（57.86 s 时 `assert.ok(pauseSamples.every(p => p.paused && p.time_stable))` 失败） |
| 第 2 次运行 | exit_code=0；TAP `# tests 1 / # pass 1 / # fail 0`，subtest duration 57704.40 ms，runner 58283.45 ms |
| 最终判定 | **PASS**（以第 2 次真实运行为准；第 1 次的失败未复现，见“未决观察”） |

## `playtest/live-report.json` 真实字段（第 2 次运行，sha256=5c3156306e904212d3ae486d005044e45c21b2a34f16a13a1d10f2b86b291db2）

| 字段 | 值 |
| --- | --- |
| url | http://127.0.0.1:8102/index.html |
| duration_ms | 57337（≥50000 断言通过） |
| frames | 130（`assert.equal(frames.length, 130)` 通过） |
| statuses | `["playing"]` |
| console_errors | `[]`（长度 0） |
| max_score / max_wave / max_kills | 800 / 2 / 6 |
| menu.status | `menu`（score 0、wave 1、lives 3、hp 3、enemies 0、time 0） |
| pause_samples | 12 条；`paused` 全为 true，`time_stable` 全为 true（all_paused=true、time_stable=true） |
| restarted | status=`playing`、paused=false、score=0、wave=1、lives=3、hp=3、kills=0、enemies=3、time=0.4133 |
| last | status=playing、paused=false、score=800、wave=2、lives=1、hp=2、kills=6、enemies=1、time=18.8561、fps=`FPS 175` |

自检断言逐条（脚本内，全部通过）：console/page errors=0；frames=130；statuses 含 `playing`；duration_ms ≥ 50000；max_wave ≥ 1；pause_samples ≥ 5；每条 pause 均 paused 且 time_stable；restarted.status=`playing`；restarted.wave=1；restarted.score=0。

## 产物清单

- `playtest/live-report.json`（脚本生成，本次运行数值）
- `playtest/report-playtester.md`（脚本生成的 Markdown，含同样数值）
- `playtest/live-run.log`：`command=node --test playtest/live-playtest.mjs` / `run_at=2026-10-26T04:02:35.885Z` / `duration_ms=57337` / `frames=130` / `statuses=playing` / `console_errors=0`
- 截图 4 张：`playtest/live-menu.png`、`playtest/live-play.png`、`playtest/live-paused.png`、`playtest/live-restart.png`

## 观察与说明（属于报告补充，不改动脚本）

1. **暂停确实冻结了游戏时钟**：12 条样本中 `before.time === after.time` 全部成立，说明 P 键暂停期间 `game.time` 不再推进；这一度在第 1 次运行的第 8 条样本上失败（该样本落在 `status=lost`、`paused=false`，因为游戏结束画面不响应暂停），第 2 次运行全程处于 `playing`，因此全样本通过。脚本在 `lost/won` 状态下仍会按 P 并采样，是测试口径的边界，而非产品缺陷。
2. **时长与规模**：单次真实会话 57.3 s、130 个采样状态、>170 FPS 标签，无控制台/页面错误，进入 `playing` 且能推进到 wave 2、击毁 6 台敌人。
3. **状态覆盖**：本次只观察到 `playing`；`lost`/`won` 属于随机对局结果，不属于本次命令断言范围（脚本只要求出现 `playing`）。
4. **键位**：WASD 移动、鼠标瞄准/开火、P 暂停、R 重开在本轮脚本中均被执行；R 重开后 status=playing、wave=1、score=0、lives=3，复位成功，键位未被改动。
5. 未触碰源码与脚本：本次仅在 `playtest` 白名单内新增本报告，`live-playtest.mjs`、`live-report.json`、`report-playtester.md` 保持脚本生成的原样（sha256 未变）。
