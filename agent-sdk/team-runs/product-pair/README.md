# FocusFlow · 专注计时器

一个可以直接用浏览器打开的单页产品落地页，产品本体是一个**专注计时器**：开始 / 暂停 / 重置、1–90 分钟可调时长、结束时的横幅 + 提示音 + 标题提醒，以及适配手机与桌面的响应式布局。

零依赖、零构建、纯前端，所有计时都在浏览器本地完成。

## 快速开始

### 直接用浏览器打开

双击 `index.html`（或在浏览器地址栏打开该文件）即可使用，无需安装任何依赖、无需启动服务器。

> `script.js` 是**经典脚本**（`<script src="script.js" defer>`，IIFE + `window.FocusFlow`），**不是** ES Module。
> 这是刻意的取舍：Chrome / Edge / Firefox 在 `file://` 协议下会以 CORS（origin `null`）拒绝加载 `type="module"`
> 脚本，用模块写法双击打开时 `initFocusApp` 根本不会执行，开始 / 暂停 / 重置 / 时长调节 / 完成提示全部失效。

如果所在环境（例如企业策略或某些浏览器扩展）禁止执行本地脚本，也可以用任意静态服务器打开：

```bash
npx serve .            # 或者
python -m http.server 8080
```

### 运行测试

```bash
npm test        # 等价于 node --test
```

测试覆盖：时长收敛（1–90 分钟）、时间格式化、倒计时推进、暂停/继续、重置、完成回调只触发一次、调整时长后重置；
静态资源与页面挂载点检查；以及用最小 fake DOM 覆盖 `initFocusApp` 的 `开始 → 暂停 → 重置 → 调整时长` 调用链、
按钮文案切换、横幅 / toast / 标题显隐、预设按钮 `aria-pressed`，和「提示音在用户手势里解锁」的行为。

## 功能

- **开始 / 暂停 / 重置**：一个主按钮在「开始专注 → 暂停 → 继续 → 再来一轮」之间自动切换；`重置` 随时回到本轮初始时长。
- **可调时长**：滑块支持 1–90 分钟，另有 15 / 25 / 45 / 60 分钟预设按钮；调整时长会重置当前这一轮。
- **完成提示**：倒计时归零后卡片内横幅出现、播放一声短提示音（浏览器允许时）、底部 toast 提醒，并把页面标题改成 `✅ 专注完成`；运行中标题实时显示剩余时间。
- **可视化进度**：SVG 圆环随时长收缩，剩余时间用等宽数字展示，避免跳动。
- **响应式布局**：≥960px 时文案与计时器左右分栏，窄屏自动改为单列居中；≤640px 时导航换行、控制按钮竖排。
- **可访问性**：跳过链接、`aria-pressed` 状态、`role="timer"`、`role="status"` 与 `aria-live` 提示，并支持 `prefers-reduced-motion`。

## 文件结构

| 文件 | 说明 |
| --- | --- |
| `index.html` | 落地页结构与计时器组件（含全部挂载点 id） |
| `styles.css` | 深色主题样式、计时环、按钮与响应式断点 |
| `script.js` | 经典脚本（IIFE）：倒计时引擎（纯逻辑）+ 浏览器 DOM 绑定，暴露 `window.FocusFlow` |
| `package.json` | 项目元信息与 `npm test` 脚本 |
| `tests/focus-timer.test.js` | 计时引擎单元测试（使用假时钟） |
| `tests/page.test.js` | 交付文件静态检查 + `initFocusApp` 的 fake DOM 绑定测试 |
| `tests/support/browser-env.js` | 用 `node:vm` 加载经典脚本，并提供 fake DOM / fake AudioContext / 假时钟 |
| `README.md` | 本文档 |

## 核心 API（`window.FocusFlow`）

```html
<script src="script.js" defer></script>
```

```js
const { createFocusTimer, formatTime, clampDurationMinutes } = window.FocusFlow;

const timer = createFocusTimer({
  durationMinutes: 25,
  onTick: (state) => console.log(formatTime(state.remainingSeconds)),
  onComplete: () => console.log('专注完成'),
});

timer.start();          // 开始，返回状态快照
timer.tick();           // 推进倒计时（页面里每 250ms 调用一次）
timer.pause();          // 暂停
timer.reset();          // 重置到本轮时长
timer.toggle();         // 开始 / 暂停切换
timer.setDurationMinutes(45); // 调整时长并重置
timer.getState();       // { durationSeconds, remainingSeconds, running, completed, progress }
```

- `durationMinutes` 会经 `clampDurationMinutes()` 收敛到 1–90 的整数分钟。
- 时间来源 `now` 可注入，因此测试中可以用假时钟精确推进秒数。
- `script.js` 没有 `export` 语句：浏览器里从 `window.FocusFlow` 取用；Node 侧测试用 `node:vm` 在隔离上下文执行同一份文件（见 `tests/support/browser-env.js`）。
- 只有在存在 `document` 的浏览器环境下才会自动初始化页面交互；`initFocusApp(doc, { now })` 也可手动调用并注入时间来源。

## 设计取舍

- 时长限制在 1–90 分钟：低于 1 分钟难以形成专注，超过 90 分钟则更接近「连续工作」而非专注冲刺。
- 调整时长即重置：避免出现「剩余时间大于新时长」这类不一致状态。
- 提示音用 Web Audio 现场合成，不引入音频文件，也不会因为缺少资源而报错。`AudioContext` 在点击开始（用户手势）时创建并 `resume()`，
  倒计时结束时复用同一个实例播放，避免自动播放策略把 context 卡在 `suspended` 导致没声音；不支持时静默降级，横幅 / toast / 标题提示照常出现。

## License

MIT
