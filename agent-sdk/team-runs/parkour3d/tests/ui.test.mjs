// tests/ui.test.mjs —— 先于实现写就的 HUD / 计时 / 得分 / restart 测试
// 运行：node --test tests/ui.test.mjs
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

import {
  CONTROL_HINT,
  createHudState,
  tickTimer,
  addScore,
  formatTime,
  formatHud,
  statusLabel,
  onRestart,
  createHud,
  updateHud
} from '../src/ui.js';

const readText = (rel) => readFileSync(new URL(`../${rel}`, import.meta.url), 'utf8');

// 极简 DOM 适配器桩：只实现 getElementById / textContent / classList.toggle
function makeStubDocument(ids) {
  const nodes = new Map();
  const classes = new Set();
  return {
    classes,
    getElementById(id) {
      if (!ids.includes(id)) return null;
      if (!nodes.has(id)) {
        nodes.set(id, {
          id,
          textContent: '',
          classList: {
            toggle(name, on) {
              if (on) classes.add(name);
              else classes.delete(name);
            }
          }
        });
      }
      return nodes.get(id);
    }
  };
}

test('tickTimer 按 dt 逐帧累加计时', () => {
  const state = createHudState();
  assert.equal(state.time, 0);

  for (let i = 0; i < 90; i += 1) tickTimer(state, 1 / 60);
  assert.ok(Math.abs(state.time - 1.5) < 1e-9, `期望约 1.5s，实际 ${state.time}`);

  tickTimer(state, 0.25);
  assert.ok(Math.abs(state.time - 1.75) < 1e-9);

  // 非法 dt 不应破坏计时
  tickTimer(state, 0);
  tickTimer(state, -5);
  tickTimer(state, Number.NaN);
  tickTimer(state, undefined);
  assert.ok(Math.abs(state.time - 1.75) < 1e-9);
});

test('formatTime / formatHud 使用 mm:ss.cs 格式', () => {
  assert.equal(formatTime(0), '00:00.00');
  assert.equal(formatTime(1.5), '00:01.50');
  assert.equal(formatTime(65.25), '01:05.25');
  assert.equal(formatTime(-3), '00:00.00');

  const state = createHudState();
  for (let i = 0; i < 90; i += 1) tickTimer(state, 1 / 60);
  const hud = formatHud(state);
  assert.match(hud, /00:01\.50/);

  const later = createHudState();
  tickTimer(later, 65.25);
  assert.match(formatHud(later), /01:05\.25/);
});

test('addScore 累加分数且 HUD 文本包含该分值', () => {
  const state = createHudState();
  assert.equal(state.score, 0);

  addScore(state, 10);
  addScore(state, 25);
  assert.equal(state.score, 35);

  const hud = formatHud(state);
  assert.match(hud, /SCORE\s+35/);
  assert.ok(hud.includes('35'), `HUD 文本应包含分数：${hud}`);

  addScore(state, -5); // 分数不应为负
  assert.equal(state.score, 30);
  addScore(state, Number.NaN);
  assert.equal(state.score, 30);
});

test('HUD 文本包含空格 / WASD 操作提示', () => {
  const hud = formatHud(createHudState());
  assert.match(hud, /WASD/);
  assert.match(hud, /空格/);
  assert.match(CONTROL_HINT, /WASD/);
  assert.match(CONTROL_HINT, /空格/);
});

test('onRestart 重置计时与得分并标记新一局', () => {
  const state = createHudState();
  tickTimer(state, 42.5);
  addScore(state, 120);
  const beforeRun = state.runId;

  onRestart(state);

  assert.equal(state.time, 0);
  assert.equal(state.score, 0);
  assert.equal(state.status, 'running');
  assert.equal(state.runId, beforeRun + 1, 'restart 后应标记为新一局');

  const hud = formatHud(state);
  assert.match(hud, /00:00\.00/);
  assert.match(hud, /SCORE\s+0/);
  assert.match(hud, /RUNNING/);
});

test('onRestart 通过钩子同时重置玩家与关卡状态', () => {
  const state = createHudState();
  tickTimer(state, 10);
  addScore(state, 30);

  const calls = [];
  onRestart(state, {
    resetPlayer: (s) => calls.push(['player', s.time, s.score]),
    resetWorld: (s) => calls.push(['world', s.time, s.score])
  });

  assert.equal(calls.length, 2);
  assert.deepEqual(calls[0], ['player', 0, 0]);
  assert.deepEqual(calls[1], ['world', 0, 0]);
  assert.equal(state.status, 'running');
});

test('createHud / updateHud 只依赖可选的 DOM 适配器（Node 无 DOM 可断言）', () => {
  const doc = makeStubDocument(['hud', 'time', 'score', 'status', 'controls', 'overlay', 'restart', 'overlay-text']);
  const hud = createHud(doc);
  assert.equal(hud.time.id, 'time');
  assert.equal(hud.score.id, 'score');
  assert.equal(hud.restart.id, 'restart');

  // 一局进行中：计时累加 + 加分，HUD 同步到元素
  const state = createHudState({ status: 'running' });
  for (let i = 0; i < 90; i += 1) tickTimer(state, 1 / 60);
  addScore(state, 35);
  updateHud(hud, state);

  assert.equal(hud.time.textContent, '00:01.50');
  assert.equal(hud.score.textContent, '35');
  assert.match(hud.status.textContent, /RUNNING/);
  assert.match(hud.controls.textContent, /WASD/);
  assert.ok(!doc.classes.has('is-visible'), 'running 时应隐藏 overlay');

  state.status = 'won';
  updateHud(hud, state);
  assert.equal(statusLabel('won'), hud.status.textContent);
  assert.ok(doc.classes.has('is-visible'), '非 running 时应显示 overlay');

  // 无 document 时应安全降级，不抛异常
  const detached = createHud(undefined);
  assert.equal(detached.time, null);
  assert.doesNotThrow(() => updateHud(detached, state));
});

test('index.html 通过 CDN importmap 引入 three 并挂载 #app 与 HUD 容器', () => {
  const html = readText('index.html');
  assert.match(html, /<script type="importmap">/);
  assert.match(html, /"three":\s*"https:\/\/unpkg\.com\/three@0\.160\.0\/build\/three\.module\.js"/);
  assert.match(html, /<script type="module" src="\.\/src\/main\.js">/);
  assert.match(html, /id="app"/);
  assert.match(html, /<canvas id="game"/);
  for (const id of ['time', 'score', 'status', 'overlay', 'restart', 'hud']) {
    assert.ok(html.includes(`id="${id}"`), `index.html 缺少 #${id}`);
  }
  assert.match(html, /空格/);
  assert.match(html, /WASD/);
});

test('styles.css 提供全屏画布与 HUD 覆盖层样式', () => {
  const css = readText('styles.css');
  assert.match(css, /#game\s*\{[^}]*position:\s*fixed/);
  assert.match(css, /#game\s*\{[^}]*width:\s*100%/);
  assert.match(css, /#game\s*\{[^}]*height:\s*100%/);
  assert.match(css, /#hud\s*\{[^}]*position:\s*absolute/);
  assert.match(css, /#overlay\s*\{[^}]*position:\s*absolute/);
  assert.match(css, /#overlay\.is-visible/);
  assert.ok(css.includes('#time') && css.includes('#score'), 'HUD 数值样式应存在');
});
