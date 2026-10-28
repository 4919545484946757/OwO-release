import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

import {
  CHECKPOINT_BONUS,
  ELEMENT_IDS,
  INITIAL_HUD_STATE,
  PROGRESS_POINTS,
  VICTORY_BONUS,
  computeScore,
  createHud,
  createHudState,
  formatHud,
  formatScore,
  formatTime,
  updateHud
} from '../src/ui.js';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');

/** 注入用的元素桩：只实现 ui.js 会用到的最小 DOM 表面，不依赖真实 DOM。 */
function stubElement(id) {
  const classes = new Set();
  const listeners = new Map();
  return {
    id,
    textContent: '',
    hidden: false,
    dataset: {},
    style: {},
    classList: {
      add: (name) => classes.add(name),
      remove: (name) => classes.delete(name),
      contains: (name) => classes.has(name),
      toggle: (name, force) => {
        if (force === undefined) {
          if (classes.has(name)) classes.delete(name);
          else classes.add(name);
        } else if (force) {
          classes.add(name);
        } else {
          classes.delete(name);
        }
        return classes.has(name);
      }
    },
    addEventListener(type, handler) {
      if (!listeners.has(type)) listeners.set(type, []);
      listeners.get(type).push(handler);
    },
    dispatch(type) {
      for (const handler of listeners.get(type) || []) {
        handler({ type, target: this, preventDefault() {} });
      }
    },
    listenerCount(type) {
      return (listeners.get(type) || []).length;
    }
  };
}

function stubDocument(ids) {
  const elementIds = ids || [
    ELEMENT_IDS.root,
    ELEMENT_IDS.score,
    ELEMENT_IDS.time,
    ELEMENT_IDS.status,
    ELEMENT_IDS.overlay,
    ELEMENT_IDS.overlayTitle,
    ELEMENT_IDS.overlayText,
    ELEMENT_IDS.restart
  ];
  const elements = new Map(elementIds.map((id) => [id, stubElement(id)]));
  return {
    elements,
    getElementById(id) {
      return elements.has(id) ? elements.get(id) : null;
    }
  };
}

const EMPTY_STATE = Object.freeze({
  score: 0,
  time: 0,
  progress: 0,
  checkpoints: 0,
  status: 'ready',
  finished: false
});

test('计时和得分: formatTime 累加与格式化稳定', () => {
  assert.equal(formatTime(0), '00:00.0');
  assert.equal(formatTime(1.5), '00:01.5');
  assert.equal(formatTime(65.43), '01:05.4');
  assert.equal(formatTime(3599.9), '59:59.9');
  assert.equal(formatTime(3600.5), '60:00.5');
  assert.equal(formatTime(-3), '00:00.0');
  assert.equal(formatTime(undefined), '00:00.0');
  assert.equal(formatTime(Number.NaN), '00:00.0');
  assert.equal(formatTime(65.43), formatTime(65.43));
});

test('计时和得分: formatScore 取整且非负', () => {
  assert.equal(formatScore(0), '0');
  assert.equal(formatScore(1234.6), '1235');
  assert.equal(formatScore(-42), '0');
  assert.equal(formatScore(undefined), '0');
  assert.equal(formatScore('250'), '250');
  assert.ok(!formatScore(-0.4).startsWith('-'));
});

test('计时和得分: formatHud 输出稳定并包含得分/时间/状态', () => {
  assert.equal(formatHud(), '得分 0 | 时间 00:00.0 | 状态 准备中');
  assert.equal(formatHud({ score: 0, time: 0, status: 'ready' }), '得分 0 | 时间 00:00.0 | 状态 准备中');
  assert.equal(
    formatHud({ score: 1200, time: 65.43, status: 'running' }),
    '得分 1200 | 时间 01:05.4 | 状态 进行中'
  );
  assert.equal(formatHud({ score: -50, time: -1, status: 'weird' }), '得分 0 | 时间 00:00.0 | 状态 weird');
  assert.equal(formatHud({ score: 7, time: 2 }), '得分 7 | 时间 00:02.0 | 状态 准备中');
});

test('计时和得分: computeScore 随进度/检查点/终点增加且非负', () => {
  assert.equal(computeScore({ progress: 0 }), 0);
  assert.equal(computeScore({ progress: 0.5 }), Math.round(PROGRESS_POINTS * 0.5));
  assert.equal(computeScore({ progress: 0.5, checkpoints: 2 }), 500 + 2 * CHECKPOINT_BONUS);
  assert.equal(
    computeScore({ progress: 0.5, checkpoints: 2, finished: true }),
    500 + 2 * CHECKPOINT_BONUS + VICTORY_BONUS
  );
  assert.equal(computeScore({ progress: -1, checkpoints: -4 }), 0);
  assert.ok(computeScore({ progress: 0.9 }) >= 0);
  assert.ok(computeScore({ progress: 5, checkpoints: 0 }) <= PROGRESS_POINTS);
});

test('计时和得分: createHudState 归一化初始值', () => {
  const state = createHudState();
  assert.deepEqual(state, { ...EMPTY_STATE });
  assert.deepEqual(INITIAL_HUD_STATE, { ...EMPTY_STATE });
  const clamped = createHudState({ time: -9, progress: 3, checkpoints: -2, score: -5 });
  assert.equal(clamped.time, 0);
  assert.equal(clamped.progress, 1);
  assert.equal(clamped.checkpoints, 0);
  assert.ok(clamped.score >= 0);
});

test('计时和得分: createHud 初始渲染通过注入桩写入 DOM', () => {
  const doc = stubDocument();
  const hud = createHud(doc);
  assert.equal(doc.getElementById('score').textContent, '0');
  assert.equal(doc.getElementById('time').textContent, '00:00.0');
  assert.equal(doc.getElementById('status').textContent, '准备中');
  const overlay = doc.getElementById('overlay');
  assert.equal(overlay.hidden, true);
  assert.equal(overlay.classList.contains('hidden'), true);
  assert.deepEqual(hud.state, { ...EMPTY_STATE });
});

test('计时和得分: tick 累加运行时间并忽略非法增量', () => {
  const doc = stubDocument();
  const hud = createHud(doc);
  hud.tick(1.5);
  assert.equal(hud.state.time, 1.5);
  assert.equal(doc.getElementById('time').textContent, '00:01.5');
  assert.equal(hud.state.status, 'running');
  assert.equal(doc.getElementById('status').textContent, '进行中');

  hud.tick(0.25);
  assert.equal(hud.state.time, 1.75);
  assert.equal(doc.getElementById('time').textContent, '00:01.7');

  hud.tick(-5);
  hud.tick(Number.NaN);
  hud.tick('abc');
  assert.equal(hud.state.time, 1.75);
  assert.equal(doc.getElementById('time').textContent, '00:01.7');
});

test('计时和得分: 进度与检查点让得分单调不减，终点追加奖励', () => {
  const doc = stubDocument();
  const hud = createHud(doc);
  hud.setProgress(0.4);
  assert.equal(hud.state.score, 400);
  assert.equal(doc.getElementById('score').textContent, '400');

  hud.addCheckpoint();
  assert.equal(hud.state.score, 400 + CHECKPOINT_BONUS);
  hud.addCheckpoint();
  assert.equal(hud.state.score, 400 + 2 * CHECKPOINT_BONUS);

  hud.setProgress(0.2);
  assert.ok(hud.state.score >= 400 + 2 * CHECKPOINT_BONUS);

  hud.setProgress(0.9);
  assert.equal(hud.state.score, 900 + 2 * CHECKPOINT_BONUS);

  hud.finish();
  assert.equal(hud.state.finished, true);
  assert.equal(hud.state.status, 'victory');
  assert.equal(hud.state.score, 900 + 2 * CHECKPOINT_BONUS + VICTORY_BONUS);
  assert.equal(doc.getElementById('status').textContent, '胜利');
  assert.equal(doc.getElementById('overlay').hidden, false);
  assert.equal(doc.getElementById('overlay').classList.contains('hidden'), false);
  assert.ok(hud.state.score >= 0);
  assert.match(doc.getElementById('overlay-text').textContent, /得分/);
});

test('restart: 归零得分/计时/玩家状态并返回新一局初始状态，可重复调用', () => {
  const doc = stubDocument();
  const hud = createHud(doc);
  hud.setProgress(0.5);
  hud.addCheckpoint();
  hud.tick(3.2);
  hud.finish();
  assert.ok(hud.state.score > 0);
  assert.equal(hud.state.finished, true);

  const fresh = hud.restart();
  assert.deepEqual(fresh, { ...EMPTY_STATE });
  assert.equal(hud.state.score, 0);
  assert.equal(hud.state.time, 0);
  assert.equal(hud.state.progress, 0);
  assert.equal(hud.state.checkpoints, 0);
  assert.equal(hud.state.status, 'ready');
  assert.equal(hud.state.finished, false);
  assert.equal(doc.getElementById('score').textContent, '0');
  assert.equal(doc.getElementById('time').textContent, '00:00.0');
  assert.equal(doc.getElementById('status').textContent, '准备中');
  assert.equal(doc.getElementById('overlay').hidden, true);

  const second = hud.restart();
  assert.deepEqual(second, fresh);

  hud.tick(0.5);
  assert.equal(hud.state.time, 0.5);

  hud.setProgress(0.7);
  hud.tick(1);
  doc.getElementById('restart').dispatch('click');
  assert.equal(hud.state.score, 0);
  assert.equal(hud.state.time, 0);
  assert.equal(doc.getElementById('time').textContent, '00:00.0');
});

test('restart: 未注入文档时也不抛错且返回初始状态', () => {
  const bare = createHud();
  assert.equal(bare.elements.score, null);
  bare.tick(2);
  bare.finish();
  assert.deepEqual(bare.restart(), { ...EMPTY_STATE });
  assert.doesNotThrow(() => updateHud(null, {}));
  assert.equal(updateHud(null, {}), null);
});

test('updateHud 用部分状态刷新桩元素并保持幂等', () => {
  const doc = stubDocument();
  const hud = createHud(doc);
  updateHud(hud, { score: 250, time: 12.24, status: 'running', progress: 0.25 });
  assert.equal(doc.getElementById('score').textContent, '250');
  assert.equal(doc.getElementById('time').textContent, '00:12.2');
  assert.equal(doc.getElementById('status').textContent, '进行中');
  assert.equal(hud.state.score >= 250, true);

  updateHud(hud);
  assert.equal(doc.getElementById('time').textContent, '00:12.2');
  assert.equal(doc.getElementById('score').textContent, String(hud.state.score));
});

test('index.html 引用 three.js CDN、styles.css 与画布/HUD 容器', () => {
  const html = readFileSync(path.join(ROOT, 'index.html'), 'utf8');
  assert.match(html, /<canvas id="game"/);
  assert.match(html, /three\.module\.js/);
  assert.match(html, /three@0\.160\.0/);
  assert.match(html, /href="\.\/styles\.css"/);
  for (const id of ['score', 'time', 'status', 'overlay', 'restart']) {
    assert.ok(html.includes(`id="${id}"`), `index.html 缺少 #${id}`);
  }
  assert.match(html, /<script type="module" src="\.\/src\/main\.js"><\/script>/);

  const css = readFileSync(path.join(ROOT, 'styles.css'), 'utf8');
  assert.match(css, /#game/);
  assert.match(css, /100v(h|w)/);
  assert.match(css, /\.hud/);
  assert.match(css, /\.overlay/);
  assert.match(css, /\.hidden/);

  const uiSource = readFileSync(path.join(ROOT, 'src/ui.js'), 'utf8');
  assert.equal(/from\s+['"]three['"]/.test(uiSource), false);
  assert.equal(/three\.module\.js/.test(uiSource), false);
  assert.equal(/\bTHREE\b/.test(uiSource), false);
  assert.equal(/^\s*[^/\s].*\bdocument\./m.test(uiSource), false);
});
