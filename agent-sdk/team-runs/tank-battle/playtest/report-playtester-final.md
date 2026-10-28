# Tank Arena · Playtester 独立试玩与交付评分（v2，review 返修版）

- 角色：playtester（独立验收/试玩，不改产品代码）
- 本次机器证据（全部来自本轮运行的脚本产物，未手写数值）：
  - 试玩命令：`node --test playtest/live-playtest.mjs` → exit_code **0**，TAP `tests 1 / pass 1 / fail 0`，`duration_ms 57288.0663`（上一次同任务运行 56919.8888ms 亦为 pass）
  - 脚本落盘：`playtest/live-report.json`（sha256 `78c2601b…ee51eb`）、`playtest/report-playtester.md`（sha256 `c7ef3719…f27f58fb`）、`playtest/live-play.png`、`playtest/run.log`
- 本轮为何重跑：数值证据必须可复现。注意 **`statuses` 每次运行都不同**——首次 56144ms run = `playing,won`；第 2 次 55978ms run = 仅 `playing`；本轮 56348ms run = `playing,lost`。因此任何“单次覆盖全部状态”的说法都不成立，见 §3。

## 1. 真实字段摘要（逐字取自本轮 `playtest/live-report.json`）

| 字段 | 本轮值 | 备注 |
| --- | --- | --- |
| `url` | `http://127.0.0.1:8102/index.html` | 宿主已启动页面 |
| `duration_ms` | `56348` | 脚本自计（含浏览器启动/交互/截图） |
| `frames` | `150` | 150 次采样，脚本断言 `frames.length === 150` |
| `statuses` | `["playing","lost"]` | **本轮未观测到 `won`**（历史两次分别含 `won` / 仅 `playing`） |
| `console_errors` | `[]` | 0 条 console error / pageerror |
| `max_score` | `1500` | 采样峰值 |
| `max_wave` | `3` | 第 3 波（`game.wave` 上限 3，`WAVE 3` 后清空才 `won`） |
| `max_kills` | `11` | 采样峰值击杀 |
| `last.status` | `playing` | 末采样处于新局 `playing` |
| `last.paused` | `false` | — |
| `last.score` / `last.wave` / `last.lives` / `last.hp` | `200` / `1` / `2` / `3` | 末采样为新一局起点附近 |
| `last.enemies` / `last.kills` / `last.time` | `1` / `2` / `8.540499999999957` | 新局 8.54s |
| `last.fps`（`#clock` 文本） | `FPS 173` | 页面自报 rAF 帧率标签（headless 下 >60 属正常，**不能当作真实显示帧率结论**） |
| `screenshot` | `playtest/live-play.png` | 本轮重新生成 |

生成文件 `playtest/report-playtester.md` 为脚本模板输出（其中标题里的 `?` 是脚本用 `?` 代替中间点的产物，不是编码损坏）。

## 2. 评分（按证据强度封顶，而不是按“看起来不错”给满分）

本轮按以下 6 个维度评分，每条都标注本轮证据/无证据：

| 维度 | 权重 | 得分 | 依据（本轮机器证据） |
| --- | --- | --- | --- |
| A 加载与启动（menu→playing） | 15 | 15 | 页面加载、`window.__tankState` 可用、点击后进入 `playing`；`console_errors []` |
| B 输入与移动/瞄准/开火 | 20 | 20 | 150 次 W/A/S/D 按压 + 鼠标移动 + 左键按压的循环驱动下，`max_kills 11`、`max_score 1500`，说明移动/瞄准/开火/命中链路在真实浏览器里确实工作 |
| C 波次推进 | 15 | 15 | `max_wave 3`：观测量从 wave 1 走到 wave 3（含每波 +200 分） |
| D 胜负闭环（won/lost 双向） | 15 | **6** | 本轮 `statuses` 只含 `playing,lost`；`lost` 分支有本轮证据，**`won` 本轮未覆盖**（仅历史 run 偶现 `won`，不可当本轮证据）。按“未覆盖只能算覆盖率、不给满分”计部分分 |
| E 暂停/重开/生命与重开一致性 | 15 | **6** | 脚本只有“按 P → 按 P”与 `Enter` 复位，**本轮未做 time 停滞断言**（无 `paused` 期间 `time` 不变的机器断言），也无 R 键→新种子的断言；`lost` 后 `Enter` 重开仅间接体现为 `last.status = playing`。只能给存在性分数 |
| F 证据可复现性与一致性 | 20 | **10** | 三次运行 `statuses` 三次不同（won / playing / lost），说明胜负覆盖是**随机且不可复现**；`playtest/report.md` 与 `playtest/run.log` 无同批命令/时间戳，无法当作本轮证据（见 §3.2） |
| **合计** | 100 | **72** | 状态：**可交付原型，但不是满分验收** |

> 结论：本轮评分 **72/100**，等级 **B（可玩原型，验收未闭合）**。原 v1 报告给的 100/100 已被撤销——v1 自承 `lost` 未覆盖却给“关卡推进与胜负闭环 20/20”，属以未覆盖分支给满分，本版按覆盖率重算（对应 issue-612c92aa）。

## 3. 逐条回应返修 Issue（全部以本轮源码/产物核对）

### 3.1 issue-612c92aa — 未覆盖分支给满分（已修复）
v1：`statuses=[playing,won]`，自承 `lost` 未覆盖，却给 20/20 与总分 100。
本版处理：
- 重跑拿到 **含 `lost` 的本轮证据**（本轮 `statuses=["playing","lost"]`）。
- 明确 `won` 本轮未覆盖，D 维度只给 6/15，总分 72，不再出现“未覆盖=满分”。
- 并额外证明“单次 run 覆盖 won+lost”不成立：三次运行分别 `won` / 无 / `lost`（见 §1 与 §3.6），因此验收必须在多轮运行里取并集，且要如实标注哪一轮覆盖了哪支。

### 3.2 issue-8f7fa85e — `playtest/report.md` 的 37.6s/42.9s 事件链不可复现（已作废该证据）
`src/game-core.js` 真实状态机（逐行核对）：
- `updateBullets`：非友方子弹命中玩家且 `player.flash <= 0` → `player.hp -= 1`，`flash = 1.2`；`if (player.hp <= 0) player.lives -= 1;` 然后 `if (player.lives < 0) status='lost'`，否则重置 `hp=3`、回出生点。
- 即：**先扣 lives，`lives < 0` 才 lost**（lives 从 3 开始 → 4 条命用完才结束）；且 `status === 'lost'` 后 `update()` 首行直接 return，**不可能继续累计击杀**。
- 因此 `playtest/report.md`“37.6s 生命归零重开 / 42.9s 仍在击杀后点击新局”与状态机矛盾：`lives` 归零（0）时仍应 `playing`，而报告把“生命归零”写成结束触发点。
- 本版处置：该 md **仅作为历史手写记录**，其时间线/结论**不作为评分证据**（占位不是本轮产物）；评分之 E 维度因此不给满分。可复现的等价链条见本轮 `playtest/run.log`（`T+37s lives:0 … T+42s kills 4→5 → T+48s score 0/wave 1/lives 3`），即“lives 归零后仍 playing、继续击杀、随后从别处（键盘 R / 点击 / Enter）重开”，与源码一致。

### 3.3 issue-c3f76c48 — “R 换图” vs `C.restart` 文案不一致（如实记录，非我写权限）
逐字核对：
- `index.html` 顶部提示：`R 换图`
- `src/main.js`：`if ((e.key === 'r' || e.key === 'R') && game.status !== 'menu') C.restart(game);`
- `src/game-core.js` `restart()`：`seed = Math.floor(game.rand()*1e9)` → `createGame(seed)` → `start()`，即 **换种子重开一局（新墙体布局/new run）**，不是“换同一局地图/换关卡图”，也不完全是 README 的“重开”。
- 游戏内副标题（`main.js` 绘制）：`最终得分 … · Enter 同图重开 · R 换图`；菜单提示：`P 暂停 · R 换图 · 清空 3 波敌人获胜`
- `README.md` 操作表：`R | 重开`
- **结论：三处文案三种说法（换图/重开/同图重开对照），与实现（换种子重开）都不完全对应。** 这是**产品文案缺陷（minor，功能正常）**，已作为 finding 提交给持有 `index.html`/`README.md`/`src/main.js` 写权限的角色修复；playtester 写范围仅 `playtest/`，故不在本轮改动（遵守写权限白名单）。
- 试玩实测旁证：本轮脚本从未按 R，故 R 行为本轮**没有真实浏览器证据**，只有源码核对；评分 E 已按此扣分。

### 3.4 issue-5c76c92d — 浏览器通道/版本未逐字记录、暂停无 time 停滞断言（部分修复，剩余如实标注）
- 通道：`playtest/live-playtest.mjs` 第 15–19 行逐字为
  `chromium.launch({ channel: 'msedge', headless: process.env.OWO_BROWSER_HEADLESS !== '0', args: ['--disable-blink-features=AutomationControlled'] })`
  → **通道 = `msedge`（Microsoft Edge 稳定版通道），headless（除非 `OWO_BROWSER_HEADLESS=0`）**，viewport `1100×760`。本报告不再写“Chromium”。
- 版本：本轮尝试记录真实版本（写了 `playtest/browser-fingerprint.cjs`，读取 `browser.version()` 与 `navigator.userAgent`），但
  ① 宿主命令白名单只登记了 `node --test playtest/live-playtest.mjs`，`node playtest/browser-fingerprint.cjs` 与 `node -e …` 均被判 `permission denied`；
  ② 脚本本身不打印版本，TAP 输出无版本行。
  → **浏览器具体版本号本轮仍未取得（未验证）**，已在 `open_issues` 明示，不用旧摘要里可能的版本号冒充。补齐方式（给有写权限的角色）：在 `live-playtest.mjs` 里加一行 `console.log('engine', browser.version(), await page.evaluate(()=>navigator.userAgent))`，或把 `playtest/browser-fingerprint.cjs` 登记进命令白名单。
- 暂停：脚本只在 `i % 11` 时 `press('KeyP')` 两次，**没有 time 停滞断言**（既未断言 `paused === true`，也未断言暂停期间 `game.time` 不变）。源码核对：`update()` 在 `game.paused` 时首行 return，`game.time` 不增长；`main.js` 的 `p` 键走 `C.togglePause`。→ **结论：暂停逻辑源码级成立，浏览器级未断言 = 未验证**，E 维度已扣分。

### 3.5 补充：为什么不动被测脚本/产品代码
本轮唯一允许写路径为 `playtest/`，且任务禁止为“更全面”另造测试绕过登记命令。因此：`playtest/browser-fingerprint.cjs` 只是**不可执行**的辅助证据脚本（记录不了版本，因为它跑不了）；`playtest/report.md` 保留原样但在本报告中降级为“历史手写记录，非本轮证据”；`playtest/live-playtest.mjs` **一行未改**，本轮所有数值均出自其原样运行产物。

### 3.6 三次运行的横向对照（同一命令、同一机器）
| run | 退出码 | `duration_ms` | `frames` | `statuses` | `max_score` | `max_wave` | `max_kills` | `console_errors` |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| #1（旧，宿主机 56144ms） | 0 | 56144 | 150 | `playing, won` | 2100 | 3 | 12 | 0 |
| #2（本轮首次重跑） | 0 | 55978 | 150 | `playing` | 1500 | 3 | 11 | 0 |
| #3（本轮最终证据） | 0 | 56348 | 150 | `playing, lost` | 1500 | 3 | 11 | 0 |

读法：**稳定性好（3/3 pass、0 console error、wave 3 可达），但胜负结局分支覆盖率随机**。`won` 需要清空第 3 波全部敌人，脚本的随机扫射能否做到取决于运行；`lost` 需要 4 条命全损，同样取决于敌人运气。这是脚本设计（随机输入+150 采样）与状态机的必然结果，应在报告里如实呈现而不是挑一次结果当结论。

## 4. 交付判定

- 可玩性：**过关**。真实 Edge(msedge 通道) headless 里 150 次采样、56.3s 连续运行、0 控制台错误、wave 1→3、击杀/得分/受击/重开链路均有机器证据。
- 验收完整性：**未闭合**。`won` 本轮未覆盖；暂停无 time 断言；R 键无浏览器证据且文案三处不一致；浏览器版本号未取得。
- 评分：**72/100（B）**，其中 D/E/F 三档按覆盖率和一致性封顶扣分，不再出现“未覆盖却满分”。

## 5. 建议修复清单（按持有者）

1. `index.html` / `README.md` / `src/main.js` 文案持有者：把“R 换图 / R 重开 / Enter 同图重开”统一为 **R = 换种子重开新局（新地图）**、`Enter = 同图重开`，index.html 顶部提示与菜单/结束副标题同步（issue-c3f76c48）。
2. 被测脚本持有者（写范围含 `playtest/` 的文件由团队指派）：给 `live-playtest.mjs` 增加 (a) `console.log` 引擎版本与 UA；(b) 暂停断言 `paused === true` 且暂停期间 `time` 快照不变；(c) 显式按 R 并断言 `score=0/wave=1/lives=3` 且墙体布局变化；(d) 让胜负分支可复现（例如定向清场或放宽采样时长）。
3. 允许的话把 `playtest/browser-fingerprint.cjs` 登记进宿主机命令白名单，或直接执行上面 (a)。
4. `playtest/report.md` 与 `playtest/run.log` 应标注生成命令与运行时间戳，否则无法与某一次 run 对应（本报告已用 run.log 仅作源码一致性旁证，不作评分证据）。

## 6. 证据文件索引（本轮）

- `playtest/live-report.json` — 本轮脚本产物（本报告 §1 全部数值来源）
- `playtest/report-playtester.md` — 本轮脚本产物（同数据，模板生成）
- `playtest/live-play.png` — 本轮截图
- `playtest/run.log` — 历史运行日志（仅用于与 `src/game-core.js` 状态机做一致性旁证）
- `playtest/browser-fingerprint.cjs` — 版本记录辅助脚本（**未被执行**，宿主命令白名单不含它；未产出 `browser-fingerprint.json`）
- `playtest/report.md` — 历史手写报告，**已降级为非本轮证据**
