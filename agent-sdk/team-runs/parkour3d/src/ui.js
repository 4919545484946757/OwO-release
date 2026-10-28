/**
 * src/ui.js —— HUD / 计时 / 得分 / restart 的纯逻辑模块
 *
 * 设计要点：
 * - 不 import three.js，也不假设存在真实 DOM（Node 中可直接 import 断言）。
 * - 所有函数内部只操作传入的 state 与可选的 DOM 适配器（documentRef）。
 * - 计时按 dt 逐帧累加，显示格式 mm:ss.cs（分:秒.厘秒）。
 * - restart 重置计时、得分、玩家与关卡状态（通过可选钩子）并开始新一局。
 * - 保留「WASD 移动 / 空格 跳跃」键盘提示文案。
 */

/** 键盘操作提示文案（空格 / WASD） */
export const CONTROL_HINT = 'WASD / 方向键 移动 · 空格 跳跃 · R 重新开始';

/** HUD DOM 元素 id 约定 */
export const HUD_IDS = Object.freeze({
  root: 'hud',
  time: 'time',
  score: 'score',
  status: 'status',
  controls: 'controls',
  overlay: 'overlay',
  overlayText: 'overlay-text',
  restart: 'restart'
});

const STATUS_LABELS = Object.freeze({
  ready: 'READY',
  running: 'RUNNING',
  paused: 'PAUSED',
  won: 'CLEARED',
  lost: 'RETRY'
});

/** 状态码 → 大写可读文案 */
export function statusLabel(status) {
  const key = String(status ?? '').toLowerCase();
  return STATUS_LABELS[key] ?? String(status ?? '').toUpperCase();
}

/** 创建一局 HUD / 计时 / 得分状态（不含任何 DOM 依赖） */
export function createHudState(overrides = {}) {
  return {
    time: 0,
    score: 0,
    status: 'ready',
    runId: 1,
    bestScore: 0,
    ...overrides
  };
}

/** 逐帧推进计时：time += dt（非法 / 非正 dt 忽略，保证逐帧累加稳定） */
export function tickTimer(state, dt) {
  if (!state || typeof state !== 'object') return state;
  const delta = Number(dt);
  if (!Number.isFinite(delta) || delta <= 0) return state;
  const current = Number(state.time);
  state.time = (Number.isFinite(current) && current > 0 ? current : 0) + delta;
  return state;
}

/** 加分：score += n，分数下限为 0 */
export function addScore(state, n) {
  if (!state || typeof state !== 'object') return state;
  const delta = Number(n);
  if (!Number.isFinite(delta)) return state;
  const current = Number(state.score);
  const base = Number.isFinite(current) && current > 0 ? current : 0;
  state.score = Math.max(0, base + delta);
  return state;
}

const pad2 = (value) => String(Math.trunc(Math.abs(value))).padStart(2, '0');

/** 秒 → mm:ss.cs（分:秒.厘秒），四舍五入到厘秒以避免浮点漂移 */
export function formatTime(seconds) {
  const raw = Number(seconds);
  const safe = Number.isFinite(raw) && raw > 0 ? raw : 0;
  const totalCs = Math.round(safe * 100);
  const cs = totalCs % 100;
  const totalSeconds = Math.floor(totalCs / 100);
  const secs = totalSeconds % 60;
  const mins = Math.floor(totalSeconds / 60);
  return `${pad2(mins)}:${pad2(secs)}.${pad2(cs)}`;
}

/** 状态 → 一行 HUD 文本（含时间、分数与操作提示） */
export function formatHud(state) {
  const s = state || {};
  const time = formatTime(s.time);
  const score = Math.max(0, Math.trunc(Number(s.score) || 0));
  const label = statusLabel(s.status);
  const run = Number(s.runId) > 1 ? `RUN ${Math.trunc(Number(s.runId))}` : '';
  return [
    `TIME ${time}`,
    `SCORE ${score}`,
    label,
    run,
    CONTROL_HINT
  ].filter(Boolean).join('  ');
}

/** overlay 文案：按状态给出提示（不覆盖 restart 按钮本身） */
export function overlayText(state) {
  const label = String(state && state.status ? state.status : 'ready').toLowerCase();
  if (label === 'won') return `通关！用时 ${formatTime(state.time)} · 得分 ${Math.trunc(Number(state.score) || 0)}`;
  if (label === 'lost') return '掉落了！按 R 或点击「重新开始」再来一局';
  if (label === 'paused') return '已暂停 · 按 P 继续';
  if (label === 'running') return '';
  return '按 空格 跳跃，WASD / 方向键移动，抵达终点获胜';
}

/**
 * restart：重置计时、得分与状态标记，开始新一局。
 * 玩家与关卡状态由调用方通过钩子注入（避免本模块依赖 three.js / DOM）。
 */
export function onRestart(state, hooks = {}) {
  const target = state && typeof state === 'object' ? state : createHudState();
  const runId = Number(target.runId);
  target.time = 0;
  target.score = 0;
  target.status = 'running';
  target.runId = (Number.isFinite(runId) && runId > 0 ? Math.trunc(runId) : 1) + 1;

  if (typeof hooks.resetPlayer === 'function') hooks.resetPlayer(target);
  if (typeof hooks.resetWorld === 'function') hooks.resetWorld(target);
  return target;
}

/** 用可选的 documentRef 绑定 HUD 元素；缺省时全部为 null，逻辑照常可测 */
export function createHud(documentRef) {
  const get = (id) =>
    documentRef && typeof documentRef.getElementById === 'function'
      ? documentRef.getElementById(id)
      : null;
  return {
    root: get(HUD_IDS.root),
    time: get(HUD_IDS.time),
    score: get(HUD_IDS.score),
    status: get(HUD_IDS.status),
    controls: get(HUD_IDS.controls),
    overlay: get(HUD_IDS.overlay),
    overlayText: get(HUD_IDS.overlayText),
    restart: get(HUD_IDS.restart)
  };
}

function setText(el, text) {
  if (!el || typeof el !== 'object') return;
  el.textContent = String(text);
}

function toggleClass(el, name, on) {
  if (!el || !el.classList || typeof el.classList.toggle !== 'function') return;
  el.classList.toggle(name, Boolean(on));
}

/** 把 state 同步到 HUD 元素（无 DOM 时安全降级） */
export function updateHud(hud, state) {
  if (!hud || !state) return hud;
  const score = Math.max(0, Math.trunc(Number(state.score) || 0));
  setText(hud.time, formatTime(state.time));
  setText(hud.score, String(score));
  setText(hud.status, statusLabel(state.status));
  setText(hud.controls, CONTROL_HINT);
  setText(hud.overlayText, overlayText(state));
  toggleClass(hud.overlay, 'is-visible', String(state.status).toLowerCase() !== 'running');
  return hud;
}

/** 可选：把 restart 按钮点击接到调用方处理器 */
export function bindHudEvents(hud, handlers = {}) {
  const btn = hud && hud.restart;
  if (btn && typeof btn.addEventListener === 'function' && typeof handlers.onRestart === 'function') {
    btn.addEventListener('click', () => handlers.onRestart());
  }
  return hud;
}

export default {
  CONTROL_HINT,
  HUD_IDS,
  statusLabel,
  createHudState,
  tickTimer,
  addScore,
  formatTime,
  formatHud,
  overlayText,
  onRestart,
  createHud,
  updateHud,
  bindHudEvents
};
