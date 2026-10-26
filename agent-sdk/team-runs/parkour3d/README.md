# PARKOUR 3D · WorkSwarm

一个无构建步骤的 3D 跑酷小游戏：从起点平台出发，跨过台阶、移动平台与障碍，抵达终点即通关。
three.js 通过 CDN + importmap 加载，所有玩法逻辑都是可被 `node --test` 直接 import 的纯 JS 模块。

## 目录结构

```
index.html               # DOM 骨架：canvas、HUD、overlay + importmap(three) + module 入口
styles.css               # HUD / overlay / canvas 样式
src/world.js             # 关卡数据 + 世界构建 + 碰撞体 + 移动平台 + 终点/掉落判定
src/player.js            # 玩家状态 + 输入归一化 + 物理步进（加速度/摩擦/跳跃/重力/AABB 阻挡）+ 重生
src/ui.js                # HUD 状态 + 计时格式化 + 得分 + overlay 文案 + restart
src/main.js              # 集成层：three.js 加载、世界/角色装配、判定接线、HUD 绑定、RAF 主循环
tests/world.test.mjs     # 关卡/判定单元测试
tests/player.test.mjs    # 物理/输入/重生单元测试
tests/ui.test.mjs        # HUD/计时/得分/restart 单元测试
tests/integration.test.mjs # 集成测试：world + player + ui + main 一起跑
```

## 运行方式

### 1. 打开游戏（浏览器）

需要有网络（从 CDN 拉 three.js）。任选一个静态服务器：

```bash
python -m http.server 8080
# 然后访问 http://localhost:8080/index.html
```

或：

```bash
npx serve .
```

不要用 `file://` 直接打开：ES module 与 importmap 在 `file://` 下会被浏览器拦截。

### 2. 跑测试（Node，无需 any 依赖 / 无需网络）

```bash
node --test
```

只跑集成测试：

```bash
node --test tests/integration.test.mjs
```

测试里不会访问 DOM、也不会请求 CDN：three.js 用 fake 替身注入（`createGame({ THREE: fakeThree })`），
`src/main.js` 的浏览器自启动只在 `window` + `document.getElementById` 都存在时才会触发。

## 操作

| 按键 | 动作 |
| --- | --- |
| `W` / `↑` | 前进（-Z） |
| `S` / `↓` | 后退 |
| `A` / `←` | 左移 |
| `D` / `→` | 右移 |
| `空格` | 跳跃（仅落地时，按住不会连跳） |
| `R` / 「重新开始」按钮 | 重开一局 |

## 模块接口（集成层实际用到的部分）

### `src/world.js`

| 接口 | 说明 |
| --- | --- |
| `WORLD_SPEC` | 默认关卡：9 个平台、3 个障碍、2 个移动平台、1 个终点触发器、`killY=-6`、`spawn/respawn` |
| `buildWorld(THREE, def)` | 装配入口，返回 `{ group, platforms, obstacles, movers, goal, colliders, spawn, respawn, killY, update(), evaluatePlayer() ... }`；`THREE = null` 时只产出数据 |
| `world.update(dt, elapsed)` | 推进移动平台（`position = base + axis * amplitude * sin(time * speed + phase)`），并写入每帧 `delta` |
| `world.evaluatePlayer(position)` | 统一判定：`{ win: true }` / `{ reset: true, respawn }` / `null` |
| `getCollisionData(world)` | 全部碰撞体（平台 / 移动平台 / 障碍），带 `standable` / `solid` / `moving` / `delta` |

### `src/player.js`

| 接口 | 说明 |
| --- | --- |
| `PLAYER` | 角色参数（半径 0.35、高 1.7、重力 -26、跳跃初速 9.5、最大速度 6 …） |
| `createPlayerState({ respawnPoint })` | 建立玩家状态，`state.position` 是**脚底**位置 |
| `inputFromKeys(keys)` | 把键名集合/数组归一化为 `{ forward, right, jump }`（WASD / 方向键 / 空格） |
| `stepPlayer(state, input, dt, ctx)` | 推进一帧物理：移动平台跟随 → 加速/摩擦 → 跳跃/重力 → AABB 阻挡 → 落地/撞头 → 兜底重生 |
| `resetPlayer(state, { respawnPoint })` | 重置到重生点 |
| `syncMesh(mesh, state)` | 把脚底位置 + yaw 写进 three.js 网格 |

### `src/ui.js`

| 接口 | 说明 |
| --- | --- |
| `createHudState()` | `{ time, score, status, runId, bestScore }` |
| `tickTimer(state, dt)` / `addScore(state, n)` | 计时 / 加分（下限 0，支持负分惩罚） |
| `formatTime` / `formatHud` / `overlayText` | `mm:ss.cs` 时间、一行 HUD 文本、overlay 文案 |
| `onRestart(state, hooks)` | 重置计时/得分/状态并把 `runId + 1`，通过 `resetPlayer` / `resetWorld` 钩子重置玩家与关卡 |
| `createHud(documentRef)` / `updateHud(hud, state)` / `bindHudEvents` | DOM 绑定（无 DOM 时全部安全降级为 null） |

### `src/main.js`（本层新增）

| 接口 | 说明 |
| --- | --- |
| `THREE_CDN_URL` | `https://unpkg.com/three@0.160.0/build/three.module.js`，与 `index.html` 的 importmap 同版本 |
| `loadThree()` | 先 `import('three')`（走 importmap），失败回退 CDN URL；只在浏览器调用 |
| `createGame({ THREE?, level?, documentRef? })` | 构造游戏对象（world + player state + hud + 网格 + HUD 元素） |
| `stepGame(game, dt, input)` | 单帧：`world.update` → `stepPlayer` → `world.evaluatePlayer` → HUD/网格同步；`won` / `lost` 冻结 |
| `restartGame(game)` | 复用 `ui.onRestart` 重置全部状态并复位移动平台相位 |
| `snapshot(game)` | 局面快照（状态、时间、分数、重生次数、位置、HUD 文案），测试用 |
| `mount(THREE, documentRef)` | 浏览器装配：WebGLRenderer、Scene、灯光、相机跟随、键盘输入、`requestAnimationFrame` 主循环 |
| `boot()` | 浏览器启动入口，页面加载后自动执行（Node 环境不会触发） |

## 判定与 HUD 一致性

一帧的顺序固定，避免「先判定后移动」造成的穿透与状态错位：

1. `world.update(dt)` 推进移动平台，写入 `delta`；
2. `main.playerContext(world)` 把碰撞体（含 `delta`）转成 player 认识的 AABB；
3. `stepPlayer(state, input, dt, ctx)` 推进角色（跳跃 / 重力 / 阻挡 / 落地）；
4. `world.evaluatePlayer(state.position)` 统一判定：终点 → `won`；`y < killY` → 重生（`respawns + 1`、扣 50 分、回到 `world.respawn`）；
5. `syncGame`：`player.syncMesh` 更新网格，`ui.updateHud` 更新 `time / score / status / controls / overlay`。

- 掉落重生由 `world.killY` 驱动；`player` 的 `fallLimit` 设为 `killY - 3` 仅作异常兜底，避免两处阈值打架。
- 重生次数达到 `MAX_RESPAWNS = 5` 判负：`status = 'lost'`，overlay 显示重试文案。
- 通关：`status = 'won'`，得分 = `WIN_SCORE(1000) - floor(time * 10)`（不低于 0），overlay 显示「通关！用时 … 得分 …」。
- `status` 只有 `running` 时隐藏 overlay；`ready` / `won` / `lost` 都显示，与 `ui.updateHud` 的 `is-visible` 规则一致。
- 关卡几何全部来自 `world.js` 的 `WORLD_SPEC`（平台、障碍、移动平台、终点、`killY`、`respawn`）；`main.js` 不硬编码任何坐标，障碍作为实心 AABB 阻挡角色（`player` 把所有实心盒统一当碰撞体，因此障碍顶面也可短暂停留，不影响关卡通过性）。

## 已知边界

- 角色碰撞是一圈固定半径的竖直 AABB，不做斜坡/胶囊体滑动；移动平台只跟随平移，不传递旋转。
- 相机为固定偏移的第三人称跟随，不提供鼠标转视角。
- 无音效、无存档；`bestScore` 只保存在内存中。
