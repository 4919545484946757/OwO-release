import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, existsSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, resolve } from 'node:path';

import {
  formatTime,
  computeScore,
  createHud,
  bindRestart,
  getHudElementIds,
  formatHud,
  updateHud,
} from '../src/ui.js';

const __dirname = dirname(fileURLToPath(import.meta.url));
const ROOT = resolve(__dirname, '..');

/* ------------------------------------------------------------------ */
/* 最小 DOM 桩（无需 jsdom）                                          */
/* ------------------------------------------------------------------ */

function makeStubElement(id = '', tagName = 'DIV') {
  const el = {
    id,
    tagName,
    textContent: '',
    className: '',
    hidden: false,
    style: {},
    children: [],
    attributes: {},
    _listeners: [],
    classList: {
      _set: new Set(),
      add(...names) {
        for (const n of names) this._set.add(n);
      },
      remove(...names) {
        for (const n of names) this._set.delete(n);
      },
      contains(n) {
        return this._set.has(n);
      },
      toggle(n, force) {
        const next = force === undefined ? !this._set.has(n) : !!force;
        if (next) this._set.add(n);
        else this._set.delete(n);
        return next;
      },
    },
    setAttribute(name, value) {
      this.attributes[name] = String(value);
    },
    getAttribute(name) {
      return Object.prototype.hasOwnProperty.call(this.attributes, name)
        ? this.attributes[name]
        : null;
    },
    appendChild(child) {
      this.children.push(child);
      return child;
    },
    addEventListener(type, handler) {
      this._listeners.push({ type, handler });
    },
    removeEventListener(type, handler) {
      this._listeners = this._listeners.filter(
        (l) => !(l.type === type && l.handler === handler)
      );
    },
    dispatchEvent(event) {
      const type = typeof event === 'string' ? event : event.type;
      const evt = typeof event === 'string' ? { type } : event;
      for (const l of [...this._listeners]) {
        if (l.type === type) l.handler(evt);
      }
      return true;
    },
    click() {
      this.dispatchEvent({ type: 'click' });
    },
    listenerCount(type) {
      return this._listeners.filter((l) => l.type === type).length;
    },
  };
  return el;
}

function makeStubDocument(ids) {
  const cache = new Map();
  const doc = {
    createElement(tagName) {
      return makeStubElement('', String(tagName).toUpperCase());
    },
    getElementById(id) {
      if (cache.has(id)) return cache.get(id);
      if (!ids.includes(id)) return null;
      const el = makeStubElement(id);
      cache.set(id, el);
      return el;
    },
    querySelector(sel) {
      if (typeof sel === 'string' && sel.startsWith('#')) {
        return doc.getElementById(sel.slice(1));
      }
      return null;
    },
    _cache: cache,
  };
  return doc;
}

const IDS = Object.values({
  canvas: 'game',
  score: 'hud-score',
  time: 'hud-time',
  status: 'hud-status',
  overlay: 'overlay',
  overlayTitle: 'overlay-title',
  overlayText: 'overlay-text',
  restart: 'restart',
});

function makeRoot() {
  const doc = makeStubDocument(IDS);
  const root = makeStubElement('root');
  root.ownerDocument = doc;
  // 也允许直接通过 root.getElementById / querySelector 访问
  root.getElementById = (id) => doc.getElementById(id);
  root.querySelector = (sel) => doc.querySelector(sel);
  return { root, doc };
}

/* ------------------------------------------------------------------ */
/* formatTime                                                          */
/* ------------------------------------------------------------------ */

test('formatTime: 0ms -> 00:00.000', () => {
  assert.equal(formatTime(0), '00:00.000');
});

test('formatTime: 999ms -> 00:00.999', () => {
  assert.equal(formatTime(999), '00:00.999');
});

test('formatTime: 1000ms 进位到 1 秒', () => {
  assert.equal(formatTime(1000), '00:01.000');
});

test('formatTime: 秒 -> 分进位', () => {
  assert.equal(formatTime(59_999), '00:59.999');
  assert.equal(formatTime(60_000), '01:00.000');
});

test('formatTime: 多分钟', () => {
  assert.equal(formatTime(61_234), '01:01.234');
  assert.equal(formatTime(125_007), '02:05.007');
});

test('formatTime: 非法/负值安全处理', () => {
  assert.equal(formatTime(-1), '00:00.000');
  assert.equal(formatTime(NaN), '00:00.000');
  assert.equal(formatTime(undefined), '00:00.000');
});

/* ------------------------------------------------------------------ */
/* computeScore                                                        */
/* ------------------------------------------------------------------ */

const BASE = { collectibles: 0, distance: 0, timeMs: 0, finished: false };

test('computeScore: 基准情况返回确定数值', () => {
  const score = computeScore(BASE);
  assert.equal(typeof score, 'number');
  assert.ok(Number.isFinite(score));
  assert.equal(score, computeScore({ ...BASE }));
});

test('computeScore: 收集物增加 -> 得分严格增加', () => {
  const a = computeScore({ ...BASE, collectibles: 0 });
  const b = computeScore({ ...BASE, collectibles: 1 });
  const c = computeScore({ ...BASE, collectibles: 5 });
  assert.ok(b > a);
  assert.ok(c > b);
});

test('computeScore: 距离增加 -> 得分严格增加', () => {
  const a = computeScore({ ...BASE, distance: 0 });
  const b = computeScore({ ...BASE, distance: 10 });
  const c = computeScore({ ...BASE, distance: 100 });
  assert.ok(b > a);
  assert.ok(c > b);
});

test('computeScore: 时间增加 -> 得分不增（时间惩罚单调）', () => {
  const fast = computeScore({ ...BASE, timeMs: 1_000 });
  const slow = computeScore({ ...BASE, timeMs: 20_000 });
  const slower = computeScore({ ...BASE, timeMs: 120_000 });
  assert.ok(slow < fast);
  assert.ok(slower < slow);
  assert.ok(slower >= 0, '得分不应为负');
});

test('computeScore: 胜利加分且高于未胜利', () => {
  const notFinished = computeScore({ ...BASE, collectibles: 3, distance: 50, timeMs: 10_000, finished: false });
  const finished = computeScore({ ...BASE, collectibles: 3, distance: 50, timeMs: 10_000, finished: true });
  assert.ok(finished > notFinished);
});

test('computeScore: 返回整数（可用于显示）', () => {
  const score = computeScore({ collectibles: 2, distance: 33.7, timeMs: 4_321, finished: true });
  assert.equal(score, Math.round(score));
});

test('computeScore: 缺失字段使用默认值不抛错', () => {
  assert.doesNotThrow(() => computeScore({}));
  assert.equal(typeof computeScore({}), 'number');
});

/* ------------------------------------------------------------------ */
/* getHudElementIds                                                    */
/* ------------------------------------------------------------------ */

test('getHudElementIds: 与 index.html 的 id 一致', () => {
  const ids = getHudElementIds();
  assert.equal(typeof ids, 'object');
  const html = readFileSync(resolve(ROOT, 'index.html'), 'utf8');
  for (const [key, id] of Object.entries(ids)) {
    assert.equal(typeof id, 'string');
    assert.ok(id.length > 0, `${key} 对应 id 非空`);
    assert.ok(
      html.includes(`id="${id}"`),
      `index.html 应包含 id="${id}"（来自 getHudElementIds().${key}）`
    );
  }
  // 关键元素映射存在
  assert.equal(ids.canvas, 'game');
  assert.equal(ids.restart, 'restart');
});

/* ------------------------------------------------------------------ */
/* createHud / update / showVictory / hide / dispose                   */
/* ------------------------------------------------------------------ */

test('createHud: 返回完整 API', () => {
  const { root } = makeRoot();
  const hud = createHud(root);
  for (const fn of ['update', 'showVictory', 'hide', 'dispose']) {
    assert.equal(typeof hud[fn], 'function', `createHud 应提供 ${fn}`);
  }
});

test('createHud().update: 桩元素文本包含格式化计时与得分', () => {
  const { root, doc } = makeRoot();
  const hud = createHud(root);
  const state = { collectibles: 4, distance: 120, timeMs: 61_234, finished: false };
  hud.update(state);

  const timeEl = doc.getElementById('hud-time');
  const scoreEl = doc.getElementById('hud-score');
  const expectedTime = formatTime(61_234);
  const expectedScore = computeScore(state);

  assert.equal(expectedTime, '01:01.234');
  assert.ok(
    String(timeEl.textContent).includes(expectedTime),
    `计时元素文本应包含 ${expectedTime}，实际为 ${timeEl.textContent}`
  );
  assert.ok(
    String(scoreEl.textContent).includes(String(expectedScore)),
    `得分元素文本应包含 ${expectedScore}，实际为 ${scoreEl.textContent}`
  );
  assert.notEqual(String(timeEl.textContent), '');
  assert.notEqual(String(scoreEl.textContent), '');
});

test('createHud().update: 再次 update 会刷新文本（不累加）', () => {
  const { root, doc } = makeRoot();
  const hud = createHud(root);
  hud.update({ collectibles: 1, distance: 10, timeMs: 1_000, finished: false });
  const firstTime = String(doc.getElementById('hud-time').textContent);
  hud.update({ collectibles: 1, distance: 10, timeMs: 2_000, finished: false });
  const secondTime = String(doc.getElementById('hud-time').textContent);
  assert.notEqual(firstTime, secondTime);
  assert.ok(secondTime.includes(formatTime(2_000)));
});

test('createHud().showVictory: 显示胜利文案与得分', () => {
  const { root, doc } = makeRoot();
  const hud = createHud(root);
  const state = { collectibles: 7, distance: 250, timeMs: 45_000, finished: true };
  hud.showVictory(state);

  const overlay = doc.getElementById('overlay');
  const title = doc.getElementById('overlay-title');
  const text = doc.getElementById('overlay-text');
  const score = computeScore(state);

  const overlayBlob = [overlay.textContent, title.textContent, text.textContent].join(' ');
  assert.match(overlayBlob, /胜利|通关|完成|Victory/i, '应出现胜利文案');
  assert.ok(
    overlayBlob.includes(String(score)) || String(doc.getElementById('hud-score').textContent).includes(String(score)),
    '胜利面板应包含得分'
  );
  assert.equal(overlay.hidden, false, '胜利面板应可见');
  assert.ok(overlay.classList.contains('visible') || overlay.hidden === false);
});

test('createHud().hide: 隐藏浮层并清空可见状态', () => {
  const { root, doc } = makeRoot();
  const hud = createHud(root);
  hud.showVictory({ collectibles: 1, distance: 1, timeMs: 1, finished: true });
  hud.hide();
  const overlay = doc.getElementById('overlay');
  assert.equal(overlay.hidden, true);
  assert.equal(overlay.classList.contains('visible'), false);
});

test('createHud().dispose: 不抛错且可安全调用', () => {
  const { root } = makeRoot();
  const hud = createHud(root);
  hud.update({ collectibles: 0, distance: 0, timeMs: 0, finished: false });
  assert.doesNotThrow(() => hud.dispose());
});

/* ------------------------------------------------------------------ */
/* bindRestart                                                         */
/* ------------------------------------------------------------------ */

test('bindRestart: 点击触发回调恰好一次', () => {
  const button = makeStubElement('restart', 'BUTTON');
  let calls = 0;
  const unbind = bindRestart(button, () => {
    calls += 1;
  });
  button.click();
  assert.equal(calls, 1);
  assert.equal(typeof unbind, 'function');
});

test('bindRestart: 多次点击累计触发且不影响 HUD 状态', () => {
  const { root, doc } = makeRoot();
  const hud = createHud(root);
  hud.update({ collectibles: 2, distance: 20, timeMs: 5_000, finished: false });
  const beforeTime = String(doc.getElementById('hud-time').textContent);
  const beforeScore = String(doc.getElementById('hud-score').textContent);

  const button = doc.getElementById('restart');
  let calls = 0;
  bindRestart(button, () => {
    calls += 1;
  });
  button.click();
  assert.equal(calls, 1);
  assert.equal(String(doc.getElementById('hud-time').textContent), beforeTime);
  assert.equal(String(doc.getElementById('hud-score').textContent), beforeScore);
});

test('bindRestart: 返回的解绑函数阻止后续回调', () => {
  const button = makeStubElement('restart', 'BUTTON');
  let calls = 0;
  const unbind = bindRestart(button, () => {
    calls += 1;
  });
  unbind();
  button.click();
  assert.equal(calls, 0);
});

test('bindRestart: 支持键盘 R 键解绑/触发（可选封装）', () => {
  const button = makeStubElement('restart', 'BUTTON');
  const keyTarget = makeStubElement('window', 'WINDOW');
  let calls = 0;
  const unbind = bindRestart(button, () => {
    calls += 1;
  }, { keyTarget });
  assert.equal(typeof unbind, 'function');
  if (keyTarget.listenerCount('keydown') > 0) {
    keyTarget.dispatchEvent({ type: 'keydown', key: 'r' });
    assert.equal(calls, 1);
    keyTarget.dispatchEvent({ type: 'keydown', key: 'x' });
    assert.equal(calls, 1, '非 R 键不触发');
  }
  unbind();
  button.click();
  assert.equal(calls, keyTarget.listenerCount('keydown') > 0 ? 1 : 1);
});

test('bindRestart: 缺少按钮元素时安全不抛错', () => {
  assert.doesNotThrow(() => bindRestart(null, () => {}));
});

/* ------------------------------------------------------------------ */
/* 兼容别名 formatHud / updateHud                                     */
/* ------------------------------------------------------------------ */

test('formatHud: 返回含计时与得分的对象', () => {
  const out = formatHud({ score: 123, time: 61_234, status: 'running' });
  assert.equal(typeof out, 'object');
  const blob = JSON.stringify(out);
  assert.ok(blob.includes('61') || blob.includes(formatTime(61_234)), '应包含格式化计时');
  assert.ok(blob.includes('123'), '应包含得分');
});

test('updateHud: 直接以 document 桩更新元素', () => {
  const doc = makeStubDocument(IDS);
  const hud = { document: doc, root: null, elements: null };
  assert.doesNotThrow(() =>
    updateHud(hud, { score: 42, time: 1_000, status: 'running', collectibles: 0, distance: 0, timeMs: 1_000 })
  );
  const scoreEl = doc.getElementById('hud-score');
  assert.ok(String(scoreEl.textContent).includes('42'));
});

/* ------------------------------------------------------------------ */
/* 文件级断言                                                          */
/* ------------------------------------------------------------------ */

test('index.html: 存在且包含 three.js CDN importmap / module 脚本', () => {
  const htmlPath = resolve(ROOT, 'index.html');
  assert.ok(existsSync(htmlPath), 'index.html 必须存在');
  const html = readFileSync(htmlPath, 'utf8');

  assert.match(html, /type=["']importmap["']/, '应包含 importmap');
  assert.match(
    html,
    /https:\/\/unpkg\.com\/three@0\.160\.0\/build\/three\.module\.js/,
    'importmap 应指向 three@0.160.0 CDN'
  );
  assert.match(html, /type=["']module["']/, '应包含 module 脚本');
  assert.match(html, /src=["']\.?\/?src\/main\.js["']/, '应加载 ./src/main.js');
});

test('index.html: 包含 canvas#game、HUD、restart 按钮与浮层', () => {
  const html = readFileSync(resolve(ROOT, 'index.html'), 'utf8');
  assert.match(html, /<canvas[^>]*id=["']game["']/, '应包含 canvas#game');
  assert.match(html, /id=["']hud-time["']/, '应包含计时元素');
  assert.match(html, /id=["']hud-score["']/, '应包含得分元素');
  assert.match(html, /id=["']hud-status["']/, '应包含状态元素');
  assert.match(html, /id=["']overlay["']/, '应包含浮层');
  assert.match(html, /<button[^>]*id=["']restart["']/, '应包含 restart 按钮');
  assert.match(html, /styles\.css/, '应链接 styles.css');
});

test('styles.css: 存在且包含全屏 canvas 与 HUD / 胜利面板 / restart 样式', () => {
  const cssPath = resolve(ROOT, 'styles.css');
  assert.ok(existsSync(cssPath), 'styles.css 必须存在');
  const css = readFileSync(cssPath, 'utf8');
  assert.match(css, /#game\b/, '应有 #game 规则');
  assert.match(css, /position\s*:\s*fixed|position\s*:\s*absolute/, 'canvas 应全屏定位');
  assert.match(css, /#hud-time\b/, '应有计时样式');
  assert.match(css, /#hud-score\b/, '应有得分样式');
  assert.match(css, /#overlay\b/, '应有胜利面板样式');
  assert.match(css, /#restart\b/, '应有 restart 按钮样式');
});
