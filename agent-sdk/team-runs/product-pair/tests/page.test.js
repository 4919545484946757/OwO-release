import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, existsSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

import { createClock, createFakeAudioContextClass, createTimerPage, loadFocusFlow } from './support/browser-env.js';

const root = dirname(dirname(fileURLToPath(import.meta.url)));
const read = (name) => readFileSync(join(root, name), 'utf8');

test('交付文件齐全', () => {
  for (const file of ['index.html', 'styles.css', 'script.js', 'package.json', 'README.md']) {
    assert.ok(existsSync(join(root, file)), `缺少 ${file}`);
    assert.ok(read(file).trim().length > 0, `${file} 不应为空`);
  }
});

test('index.html 引用样式与脚本，并包含计时器所需挂载点', () => {
  const html = read('index.html');
  assert.match(html, /<link rel="stylesheet" href="styles\.css">/);
  // 必须是经典脚本：file:// 下浏览器会以 CORS（origin 'null'）拒绝 type="module"。
  assert.match(html, /<script src="script\.js" defer><\/script>/);
  assert.doesNotMatch(html, /type="module"/);
  for (const id of [
    'time-display',
    'status-text',
    'progress-ring',
    'toggle-btn',
    'reset-btn',
    'duration-input',
    'duration-value',
    'complete-banner',
  ]) {
    assert.ok(html.includes(`id="${id}"`), `index.html 缺少 id="${id}"`);
  }
  assert.match(html, /data-timer/);
  assert.match(html, /data-preset="25"/);
  assert.match(html, /<meta name="viewport" content="width=device-width, initial-scale=1">/);
});

test('package.json 提供 node --test 测试脚本', () => {
  const pkg = JSON.parse(read('package.json'));
  assert.equal(pkg.type, 'module');
  assert.match(pkg.scripts.test, /node --test/);
});

test('styles.css 包含响应式断点与计时环样式', () => {
  const css = read('styles.css');
  assert.match(css, /@media \(max-width: 960px\)/);
  assert.match(css, /@media \(max-width: 640px\)/);
  assert.match(css, /\.dial__progress/);
  assert.match(css, /\.complete-banner/);
  assert.match(css, /\.toast/);
});

test('script.js 是经典脚本：无 import / export，直接挂在 window.FocusFlow 上', () => {
  const source = read('script.js');
  assert.doesNotMatch(source, /^\s*export\s/m, 'script.js 不应有 export 语句');
  assert.doesNotMatch(source, /^\s*import\s.+from\s/m, 'script.js 不应有 import 语句');
  assert.match(source, /root\.FocusFlow = api;/);

  const { api } = loadFocusFlow();
  for (const name of ['createFocusTimer', 'clampDurationMinutes', 'formatTime', 'initFocusApp']) {
    assert.equal(typeof api[name], 'function', `FocusFlow 应暴露 ${name}()`);
  }
});

test('initFocusApp 绑定开始 / 暂停 / 重置：按钮文案与状态同步切换', () => {
  const { doc, els } = createTimerPage();
  const clock = createClock();
  const { api } = loadFocusFlow({ document: doc });
  const app = api.initFocusApp(doc, { now: clock.now });
  assert.ok(app, 'initFocusApp 应返回应用实例');

  assert.equal(els.timeDisplay.textContent, '25:00');
  assert.equal(els.toggle.textContent, '开始专注');
  assert.equal(els.banner.hidden, true);

  els.toggle.click();
  assert.equal(els.toggle.textContent, '暂停');
  assert.equal(els.toggle.getAttribute('aria-pressed'), 'true');
  assert.equal(els.status.textContent, '专注中…');
  assert.equal(els.toggle.classList.contains('is-running'), true);

  clock.advance(7);
  app.timer.tick();
  assert.equal(els.timeDisplay.textContent, '24:53');

  els.toggle.click();
  assert.equal(els.toggle.textContent, '继续');
  assert.equal(els.toggle.getAttribute('aria-pressed'), 'false');
  assert.equal(els.status.textContent, '已暂停');
  assert.equal(els.toggle.classList.contains('is-running'), false);

  els.reset.click();
  assert.equal(els.toggle.textContent, '开始专注');
  assert.equal(els.timeDisplay.textContent, '25:00');
  assert.equal(els.status.textContent, '准备开始');

  app.stop();
});

test('initFocusApp：调整时长会重置，倒计时按真实流逝递减（file:// 双击打开的核心路径）', () => {
  const { doc, els } = createTimerPage();
  const clock = createClock();
  const { api } = loadFocusFlow({ document: doc });
  const app = api.initFocusApp(doc, { now: clock.now });

  els.presets[2].click();
  assert.equal(els.timeDisplay.textContent, '45:00');
  assert.equal(els.durationValue.textContent, '45 分钟');
  assert.equal(els.presets[2].getAttribute('aria-pressed'), 'true');
  assert.equal(els.presets[1].getAttribute('aria-pressed'), 'false');
  assert.equal(els.input.value, '45');

  els.input.value = '10';
  els.input.dispatch('input');
  assert.equal(els.timeDisplay.textContent, '10:00');
  assert.equal(els.durationValue.textContent, '10 分钟');
  assert.ok(els.presets.every((button) => button.getAttribute('aria-pressed') === 'false'));

  els.toggle.click();
  clock.advance(5);
  app.timer.tick();
  assert.equal(els.timeDisplay.textContent, '09:55');
  assert.match(doc.title, /09:55/);

  app.stop();
});

test('initFocusApp：完成时横幅 + toast + 标题，并复用用户手势里解锁的 AudioContext', () => {
  const audio = createFakeAudioContextClass();
  const { doc, els } = createTimerPage();
  const clock = createClock();
  const { api } = loadFocusFlow({ document: doc, window: { AudioContext: audio.FakeAudioContext } });
  const app = api.initFocusApp(doc, { now: clock.now });

  els.toggle.click(); // 用户手势：此刻应创建并 resume AudioContext
  assert.equal(audio.created.length, 1, '开始时应创建 AudioContext');
  assert.equal(audio.created[0].state, 'running', '开始时应 resume 解锁音频');

  clock.advance(1500);
  app.timer.tick();

  assert.equal(els.timeDisplay.textContent, '00:00');
  assert.equal(els.banner.hidden, false);
  assert.equal(els.status.textContent, '专注完成');
  assert.equal(els.toggle.textContent, '再来一轮');
  assert.equal(els.card.classList.contains('is-completed'), true);
  assert.equal(els.toast.hidden, false);
  assert.match(els.toast.textContent, /专注完成/);
  assert.match(doc.title, /专注完成/);
  assert.equal(audio.created.length, 1, '提示音应复用同一个 AudioContext');
  assert.equal(audio.oscillators.length, 1);
  assert.equal(audio.oscillators[0].stopped, 1);

  app.stop();
});
