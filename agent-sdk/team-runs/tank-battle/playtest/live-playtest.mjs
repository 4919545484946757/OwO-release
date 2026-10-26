import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
const require = createRequire(import.meta.url);
const { chromium } = require('playwright');
import fs from 'node:fs';
import path from 'node:path';

const base = process.env.PLAYTEST_URL || 'http://127.0.0.1:8102/index.html';

test('real browser playtest: menu, move, aim, fire, hit, wave, pause and restart', { timeout: 120000 }, async () => {
  const errors = [];
  const frames = [];
  const pauseSamples = [];
  const started = Date.now();
  const browser = await chromium.launch({
    channel: 'msedge',
    headless: process.env.OWO_BROWSER_HEADLESS !== '0',
    args: ['--disable-blink-features=AutomationControlled'],
  });
  const context = await browser.newContext({ viewport: { width: 1100, height: 760 } });
  const page = await context.newPage();
  page.on('console', (msg) => {
    if (msg.type() === 'error' && !msg.text().includes('favicon')) errors.push(msg.text());
  });
  page.on('pageerror', (err) => errors.push(String(err)));

  await page.goto(base, { waitUntil: 'domcontentloaded', timeout: 30000 });
  await page.waitForFunction(() => typeof window.__tankState === 'function');
  const canvas = page.locator('#game');
  const menu = await page.evaluate(() => window.__tankState());
  await page.screenshot({ path: 'playtest/live-menu.png', fullPage: false });
  await canvas.click({ position: { x: 480, y: 320 } });
  await page.waitForTimeout(400);

  const keys = ['KeyW', 'KeyA', 'KeyS', 'KeyD'];
  for (let i = 0; i < 130; i++) {
    const key = keys[i % keys.length];
    await page.keyboard.down(key);
    await page.waitForTimeout(170);
    await page.keyboard.up(key);
    const box = await canvas.boundingBox();
    const x = box.x + box.width * (0.35 + (i % 5) * 0.08);
    const y = box.y + box.height * (0.3 + (i % 7) * 0.055);
    await page.mouse.move(x, y);
    await page.mouse.down();
    await page.waitForTimeout(150);
    await page.mouse.up();
    if (i % 11 === 0) {
      await page.keyboard.press('KeyP');
      await page.waitForTimeout(180);
      const pausedBefore = await page.evaluate(() => window.__tankState());
      await page.screenshot({ path: 'playtest/live-paused.png', fullPage: false });
      await page.waitForTimeout(220);
      const pausedAfter = await page.evaluate(() => window.__tankState());
      pauseSamples.push({ paused: pausedBefore.paused, time_stable: pausedBefore.time === pausedAfter.time, before: pausedBefore, after: pausedAfter });
      await page.keyboard.press('KeyP');
      await page.waitForTimeout(80);
    }
    const state = await page.evaluate(() => ({
      ...window.__tankState(),
      fps: document.querySelector('#clock')?.textContent || '',
    }));
    frames.push(state);
    if (state.status === 'lost' || state.status === 'won') {
      await page.keyboard.press('Enter');
      await page.waitForTimeout(250);
    }
  }

  const shotPath = path.resolve('playtest/live-play.png');
  await page.screenshot({ path: shotPath, fullPage: false });
  await page.keyboard.press('KeyR');
  await page.waitForTimeout(400);
  const restarted = await page.evaluate(() => window.__tankState());
  await page.screenshot({ path: 'playtest/live-restart.png', fullPage: false });
  const statuses = [...new Set(frames.map((f) => f.status))];
  const report = {
    url: base,
    duration_ms: Date.now() - started,
    frames: frames.length,
    statuses,
    console_errors: errors,
    max_score: Math.max(...frames.map((f) => f.score || 0)),
    max_wave: Math.max(...frames.map((f) => f.wave || 0)),
    max_kills: Math.max(...frames.map((f) => f.kills || 0)),
    last: frames[frames.length - 1],
    menu,
    pause_samples: pauseSamples,
    restarted,
    screenshots: ['playtest/live-menu.png', 'playtest/live-play.png', 'playtest/live-paused.png', 'playtest/live-restart.png'],
    screenshot: 'playtest/live-play.png',
  };
  fs.writeFileSync('playtest/live-report.json', JSON.stringify(report, null, 2));
  fs.writeFileSync('playtest/live-run.log', `command=node --test playtest/live-playtest.mjs\nrun_at=${new Date().toISOString()}\nduration_ms=${report.duration_ms}\nframes=${report.frames}\nstatuses=${report.statuses.join(',')}\nconsole_errors=${report.console_errors.length}\n`);
  const md = [
    '# Tank Arena - real browser playtest (generated)',
    '',
    `- URL: ${report.url}`,
    `- Duration: ${report.duration_ms} ms`,
    `- Frames/states sampled: ${report.frames}`,
    `- Statuses observed: ${report.statuses.join(', ')}`,
    `- Console/page errors: ${report.console_errors.length}`,
    `- Max score: ${report.max_score}`,
    `- Max wave: ${report.max_wave}`,
    `- Max kills: ${report.max_kills}`,
    `- Last FPS label: ${report.last.fps}`,
    `- Screenshot: ${report.screenshot}`,
    '- Menu screenshot: playtest/live-menu.png',
    '- Paused screenshot: playtest/live-paused.png',
    '- Restart screenshot: playtest/live-restart.png',
    `- Pause samples: ${report.pause_samples.length}; all_paused=${report.pause_samples.every((p) => p.paused)}; time_stable=${report.pause_samples.every((p) => p.time_stable)}`,
    `- R restart: role=${report.restarted.status}; wave=${report.restarted.wave}; score=${report.restarted.score}; lives=${report.restarted.lives}`,
    '',
    'This Markdown file is generated by `playtest/live-playtest.mjs` from the actual run',
    'and must not be replaced with hand-written numbers. The script asserts zero console',
    'errors, 130 sampled states, an observed `playing` state, a >=50s session, a real',
    'pause with stable game time, and an R-key restart with a fresh run.',
    '',
  ].join('\n');
  fs.writeFileSync('playtest/report-playtester.md', md);
  await browser.close();

  assert.equal(errors.length, 0, 'console/page errors: ' + JSON.stringify(errors));
  assert.equal(frames.length, 130);
  assert.ok(statuses.includes('playing'), 'page must enter playing state');
  assert.ok(report.duration_ms >= 50000, 'play session must last at least 50 seconds');
  assert.ok(report.max_wave >= 1);
  assert.ok(pauseSamples.length >= 5);
  assert.ok(pauseSamples.every((p) => p.paused && p.time_stable));
  assert.equal(restarted.status, 'playing');
  assert.equal(restarted.wave, 1);
  assert.equal(restarted.score, 0);
});
