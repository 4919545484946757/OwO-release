import test from 'node:test';
import assert from 'node:assert/strict';

import { createClock, loadFocusFlow } from './support/browser-env.js';

// script.js 是经典脚本（没有 export 语句），这里用 vm 加载后再从 window.FocusFlow
// 取同一套纯逻辑，保证测试用的就是浏览器里跑的那份代码。
const {
  DEFAULT_DURATION_MINUTES,
  MAX_DURATION_MINUTES,
  MIN_DURATION_MINUTES,
  clampDurationMinutes,
  createFocusTimer,
  formatDurationLabel,
  formatTime,
} = loadFocusFlow().api;

test('clampDurationMinutes 把输入收敛到 1–90 分钟', () => {
  assert.equal(clampDurationMinutes(25), 25);
  assert.equal(clampDurationMinutes('45'), 45);
  assert.equal(clampDurationMinutes(0), MIN_DURATION_MINUTES);
  assert.equal(clampDurationMinutes(-12), MIN_DURATION_MINUTES);
  assert.equal(clampDurationMinutes(500), MAX_DURATION_MINUTES);
  assert.equal(clampDurationMinutes(24.6), 25);
});

test('clampDurationMinutes 对非法输入回落到默认 25 分钟', () => {
  assert.equal(clampDurationMinutes(undefined), DEFAULT_DURATION_MINUTES);
  assert.equal(clampDurationMinutes(null), DEFAULT_DURATION_MINUTES);
  assert.equal(clampDurationMinutes(''), DEFAULT_DURATION_MINUTES);
  assert.equal(clampDurationMinutes('abc'), DEFAULT_DURATION_MINUTES);
  assert.equal(clampDurationMinutes(Number.NaN), DEFAULT_DURATION_MINUTES);
  assert.equal(clampDurationMinutes(Number.POSITIVE_INFINITY), DEFAULT_DURATION_MINUTES);
});

test('formatTime 输出 mm:ss 并兜底非法输入', () => {
  assert.equal(formatTime(0), '00:00');
  assert.equal(formatTime(9), '00:09');
  assert.equal(formatTime(59), '00:59');
  assert.equal(formatTime(60), '01:00');
  assert.equal(formatTime(1500), '25:00');
  assert.equal(formatTime(5400), '90:00');
  assert.equal(formatTime(-30), '00:00');
  assert.equal(formatTime('75'), '01:15');
  assert.equal(formatTime(Number.NaN), '00:00');
});

test('formatDurationLabel 生成中文时长标签', () => {
  assert.equal(formatDurationLabel(25), '25 分钟');
  assert.equal(formatDurationLabel(200), '90 分钟');
  assert.equal(formatDurationLabel('x'), '25 分钟');
});

test('计时器初始状态：未运行、满时长、进度为 0', () => {
  const clock = createClock();
  const timer = createFocusTimer({ durationMinutes: 25, now: clock.now });
  const state = timer.getState();

  assert.equal(state.durationSeconds, 1500);
  assert.equal(state.remainingSeconds, 1500);
  assert.equal(state.running, false);
  assert.equal(state.completed, false);
  assert.equal(state.progress, 0);
});

test('start 之后按真实流逝时间递减', () => {
  const clock = createClock();
  const timer = createFocusTimer({ durationMinutes: 25, now: clock.now });

  const started = timer.start();
  assert.equal(started.running, true);

  clock.advance(10);
  const afterTen = timer.tick();
  assert.equal(afterTen.remainingSeconds, 1490);
  assert.equal(formatTime(afterTen.remainingSeconds), '24:50');

  clock.advance(50);
  assert.equal(timer.tick().remainingSeconds, 1440);
  assert.ok(Math.abs(timer.getState().progress - 0.04) < 1e-9, '进度应约为 4%');
});

test('pause 冻结剩余时间，再 start 从暂停处继续', () => {
  const clock = createClock();
  const timer = createFocusTimer({ durationMinutes: 5, now: clock.now });

  timer.start();
  clock.advance(30);
  const paused = timer.pause();
  assert.equal(paused.running, false);
  assert.equal(paused.remainingSeconds, 270);
  assert.equal(paused.completed, false);

  // 暂停期间时间流逝不应影响剩余时间。
  clock.advance(120);
  assert.equal(timer.tick().remainingSeconds, 270);

  timer.start();
  clock.advance(15);
  assert.equal(timer.tick().remainingSeconds, 255);
});

test('reset 回到初始时长并清除运行/完成状态', () => {
  const clock = createClock();
  const timer = createFocusTimer({ durationMinutes: 10, now: clock.now });

  timer.start();
  clock.advance(60);
  timer.pause();
  const reset = timer.reset();

  assert.equal(reset.running, false);
  assert.equal(reset.completed, false);
  assert.equal(reset.remainingSeconds, 600);
  assert.equal(reset.progress, 0);
});

test('倒计时归零时停止并触发完成回调（只触发一次）', () => {
  const clock = createClock();
  const completions = [];
  const timer = createFocusTimer({
    durationMinutes: 1,
    now: clock.now,
    onComplete: (state) => completions.push(state),
  });

  timer.start();
  clock.advance(59);
  assert.equal(timer.tick().remainingSeconds, 1);
  assert.equal(completions.length, 0);

  clock.advance(1);
  const finished = timer.tick();
  assert.equal(finished.remainingSeconds, 0);
  assert.equal(finished.running, false);
  assert.equal(finished.completed, true);
  assert.equal(finished.progress, 1);
  assert.equal(completions.length, 1);
  assert.equal(completions[0].remainingSeconds, 0);

  // 已完成后再 tick 不应重复触发完成回调。
  clock.advance(30);
  timer.tick();
  assert.equal(completions.length, 1);
});

test('完成后再 start 会重置为新的一轮时长', () => {
  const clock = createClock();
  const timer = createFocusTimer({ durationMinutes: 1, now: clock.now });

  timer.start();
  clock.advance(60);
  timer.tick();
  assert.equal(timer.getState().completed, true);

  const restarted = timer.start();
  assert.equal(restarted.completed, false);
  assert.equal(restarted.running, true);
  assert.equal(restarted.remainingSeconds, 60);
});

test('toggle 在开始与暂停之间切换', () => {
  const clock = createClock();
  const timer = createFocusTimer({ durationMinutes: 3, now: clock.now });

  assert.equal(timer.toggle().running, true);
  clock.advance(20);
  assert.equal(timer.toggle().running, false);
  assert.equal(timer.getState().remainingSeconds, 160);
});

test('setDurationMinutes 调整时长并重置本轮', () => {
  const clock = createClock();
  const timer = createFocusTimer({ durationMinutes: 25, now: clock.now });

  timer.start();
  clock.advance(45);
  timer.tick();

  const updated = timer.setDurationMinutes(45);
  assert.equal(updated.durationSeconds, 2700);
  assert.equal(updated.remainingSeconds, 2700);
  assert.equal(updated.running, false);
  assert.equal(updated.progress, 0);

  assert.equal(timer.setDurationMinutes(999).durationSeconds, MAX_DURATION_MINUTES * 60);
  assert.equal(timer.setDurationMinutes(-4).durationSeconds, MIN_DURATION_MINUTES * 60);
});

test('onTick 每次状态变化都会收到快照', () => {
  const clock = createClock();
  const snapshots = [];
  const timer = createFocusTimer({
    durationMinutes: 2,
    now: clock.now,
    onTick: (state) => snapshots.push(state.remainingSeconds),
  });

  timer.start();
  clock.advance(5);
  timer.tick();
  timer.pause();
  timer.reset();

  assert.equal(snapshots.length, 4);
  assert.deepEqual(snapshots, [120, 115, 115, 120]);
});
